#!/usr/bin/env python3
"""Motor de voz local para o Voz utilizando Chatterbox e o timbre clonado da Laura.

Roda como um serviço residente em segundo plano (processando pedidos via fila de arquivos
em estado/fila-motor/) para manter o modelo em memória e gerar áudio em ~6s.

100% local, offline, sem custos de API e sem envio de dados para servidores externos.
"""

import argparse
import fcntl
import io
import json
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

BASE = Path.home() / ".claude" / "voz"
ESTADO = BASE / "estado"
FILA_DIR = ESTADO / "fila-motor"
PID_FILE = ESTADO / "motor.pid"
LOCK_FILE = ESTADO / "motor.lock"
HEARTBEAT_FILE = ESTADO / "motor.heartbeat"
PARAR_FILE = ESTADO / "motor.parar"
CONDS_FILE = ESTADO / "laura_conds.pt"
REF_WAV = ESTADO / "laura_referencia.wav"
VENV_PYTHON = BASE / ".venv" / "bin" / "python"
CKPT_DIR = Path.home() / ".cache" / "huggingface" / "hub" / "models--ResembleAI--chatterbox" / "snapshots" / "5bb1f6ee58e50c3b8d408bc82a6d3740c2db6e18"


def _garantir_condicionais(model):
    """Garante que as condicionais de voz da Laura estão salvas em disco para carregamento rápido."""
    if CONDS_FILE.is_file() and CONDS_FILE.stat().st_size > 1000:
        return
    print("[motor_local] Extraindo condicionais da Laura a partir da referência...", flush=True)
    if not REF_WAV.is_file():
        som_padrao = BASE / "sons" / "terminei" / "Pronto por aqui, já pode conferir!.mp3"
        if som_padrao.is_file():
            subprocess.run(["ffmpeg", "-y", "-i", str(som_padrao), "-ar", "24000", "-ac", "1", str(REF_WAV)], check=True)
    model.prepare_conditionals(str(REF_WAV), exaggeration=0.35)
    model.conds.save(str(CONDS_FILE))
    print(f"[motor_local] Condicionais salvas em {CONDS_FILE}", flush=True)


def _caminho_ffmpeg():
    ff = "/opt/homebrew/bin/ffmpeg"
    if os.path.exists(ff):
        return ff
    return shutil.which("ffmpeg") or "ffmpeg"


def _converter_para_mp3(wav_bytes, sr=24000, tom=0.90, vel=0.91):
    """Converte o áudio WAV do Chatterbox para MP3 aplicando ajustes de tom e velocidade."""
    ffmpeg = _caminho_ffmpeg()
    cmd = [ffmpeg, "-y", "-hide_banner", "-loglevel", "error", "-f", "wav", "-i", "pipe:0"]
    if abs(tom - 1.0) > 0.01 or abs(vel - 1.0) > 0.01:
        atempo = max(0.5, min(2.0, vel / tom))
        filtro = f"asetrate={sr}*{tom:.4f},aresample={sr},atempo={atempo:.4f}"
        cmd.extend(["-af", filtro])
    cmd.extend(["-b:a", "128k", "-f", "mp3", "pipe:1"])
    r = subprocess.run(cmd, input=wav_bytes, capture_output=True)
    return r.stdout if r.returncode == 0 and r.stdout else wav_bytes


def rodar_daemon():
    """Loop do serviço residente em background."""
    os.environ["HF_HUB_OFFLINE"] = "1"
    ESTADO.mkdir(parents=True, exist_ok=True)
    FILA_DIR.mkdir(parents=True, exist_ok=True)

    # Garante instância única via flock exclusivo
    lock_fd = open(LOCK_FILE, "w", encoding="utf-8")
    try:
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except (BlockingIOError, OSError):
        print("[motor_local] Outra instância do motor de voz já está rodando. Encerrando.", flush=True)
        sys.exit(0)

    PID_FILE.write_text(str(os.getpid()), encoding="utf-8")
    PARAR_FILE.unlink(missing_ok=True)

    try:
        import perth
        if getattr(perth, "PerthImplicitWatermarker", None) is None:
            perth.PerthImplicitWatermarker = perth.DummyWatermarker
    except Exception:
        pass

    import soundfile as sf
    import torch
    from chatterbox.mtl_tts import ChatterboxMultilingualTTS, Conditionals

    torch.set_num_threads(10)
    print("[motor_local] Carregando Chatterbox na CPU (10 threads)...", flush=True)
    t0 = time.time()
    model = ChatterboxMultilingualTTS.from_local(str(CKPT_DIR), device="cpu")
    print(f"[motor_local] Modelo carregado em {time.time()-t0:.2f}s", flush=True)

    _garantir_condicionais(model)
    model.conds = Conditionals.load(str(CONDS_FILE), map_location="cpu").to("cpu")
    HEARTBEAT_FILE.write_text(str(time.time()), encoding="utf-8")
    print(f"[motor_local] Pronto! Aguardando pedidos em {FILA_DIR}...", flush=True)

    def _encerrar(signum=None, frame=None):
        print("[motor_local] Encerrando daemon...", flush=True)
        PID_FILE.unlink(missing_ok=True)
        HEARTBEAT_FILE.unlink(missing_ok=True)
        try:
            fcntl.flock(lock_fd, fcntl.LOCK_UN)
            lock_fd.close()
        except Exception:
            pass
        sys.exit(0)

    signal.signal(signal.SIGTERM, _encerrar)
    signal.signal(signal.SIGINT, _encerrar)

    ultimo_pulso = time.time()
    while True:
        try:
            if PARAR_FILE.is_file():
                PARAR_FILE.unlink(missing_ok=True)
                _encerrar()

            agora = time.time()
            if agora - ultimo_pulso >= 1.0:
                HEARTBEAT_FILE.write_text(str(agora), encoding="utf-8")
                ultimo_pulso = agora

            pedidos = sorted(FILA_DIR.glob("req_*.json"))
            if not pedidos:
                time.sleep(0.15)
                continue

            for p_req in pedidos:
                req_id = p_req.stem.replace("req_", "")
                p_resp_mp3 = FILA_DIR / f"resp_{req_id}.mp3"
                p_pronto = FILA_DIR / f"resp_{req_id}.pronto"

                try:
                    conteudo = json.loads(p_req.read_text(encoding="utf-8"))
                    texto = conteudo.get("texto", "").strip()
                    tom = float(conteudo.get("tom_voz", 0.90))
                    vel = float(conteudo.get("velocidade_voz", 0.91))
                    exag = float(conteudo.get("exaggeration", 0.35))
                    cfg_w = float(conteudo.get("cfg_weight", 0.5))

                    t1 = time.time()
                    wav = model.generate(
                        text=texto,
                        language_id="pt",
                        audio_prompt_path=None,
                        exaggeration=exag,
                        cfg_weight=cfg_w,
                    )
                    dur = time.time() - t1

                    buf_wav = io.BytesIO()
                    sf.write(buf_wav, wav.squeeze().cpu().numpy(), model.sr, format="WAV")
                    mp3_bytes = _converter_para_mp3(buf_wav.getvalue(), sr=model.sr, tom=tom, vel=vel)

                    p_resp_mp3.write_bytes(mp3_bytes)
                    p_pronto.write_text(json.dumps({"sucesso": True, "duracao": dur}), encoding="utf-8")
                    print(f"[motor_local] Sintetizado em {dur:.2f}s ({len(mp3_bytes)} bytes): {texto[:40]}...", flush=True)
                except Exception as e:
                    p_pronto.write_text(json.dumps({"sucesso": False, "erro": str(e)}), encoding="utf-8")
                finally:
                    p_req.unlink(missing_ok=True)

        except Exception:
            time.sleep(0.5)


def daemon_esta_rodando():
    """Verifica se o daemon está ativo testando a trava exclusiva do processo."""
    if not LOCK_FILE.is_file():
        return False
    try:
        fd = open(LOCK_FILE, "r+", encoding="utf-8")
        fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
        # Se obteve o lock, significa que o processo anterior morreu
        fcntl.flock(fd, fcntl.LOCK_UN)
        fd.close()
        return False
    except (BlockingIOError, OSError):
        # Bloqueio ativo: processo residente está em execução
        return True


def garantir_daemon_iniciado():
    """Se o daemon não estiver rodando, inicia em background e aguarda ficar pronto."""
    if daemon_esta_rodando() and HEARTBEAT_FILE.is_file():
        return True

    cmd = [str(VENV_PYTHON), str(Path(__file__).resolve()), "--daemon"]
    ESTADO.mkdir(parents=True, exist_ok=True)
    log_f = open(ESTADO / "motor_daemon.log", "a", encoding="utf-8")
    subprocess.Popen(
        cmd,
        stdout=log_f,
        stderr=subprocess.STDOUT,
        start_new_session=True,
    )

    inicio = time.time()
    while time.time() - inicio < 30.0:
        if daemon_esta_rodando() and HEARTBEAT_FILE.is_file():
            return True
        time.sleep(0.3)
    return False


def sintetizar(texto, tom_voz=0.90, velocidade_voz=0.91, timeout=40):
    """Envia pedido para o serviço de voz em background via fila de arquivos."""
    FILA_DIR.mkdir(parents=True, exist_ok=True)
    garantir_daemon_iniciado()

    req_id = f"{time.time():.6f}_{os.getpid()}"
    p_req = FILA_DIR / f"req_{req_id}.json"
    p_resp_mp3 = FILA_DIR / f"resp_{req_id}.mp3"
    p_pronto = FILA_DIR / f"resp_{req_id}.pronto"

    payload = {
        "texto": texto,
        "tom_voz": tom_voz,
        "velocidade_voz": velocidade_voz,
    }
    p_req.write_text(json.dumps(payload, ensure_ascii=False), encoding="utf-8")

    inicio = time.time()
    while time.time() - inicio < timeout:
        if p_pronto.is_file():
            try:
                status = json.loads(p_pronto.read_text(encoding="utf-8"))
                if not status.get("sucesso"):
                    raise RuntimeError(f"Erro na síntese local: {status.get('erro')}")
                if p_resp_mp3.is_file():
                    mp3_data = p_resp_mp3.read_bytes()
                    return mp3_data
            finally:
                p_resp_mp3.unlink(missing_ok=True)
                p_pronto.unlink(missing_ok=True)
                p_req.unlink(missing_ok=True)
        time.sleep(0.1)

    p_req.unlink(missing_ok=True)
    raise TimeoutError(f"Tempo limite excedido ({timeout}s) aguardando síntese de voz local.")


def parar_daemon():
    """Solicita a interrupção graciosa do daemon."""
    PARAR_FILE.touch()
    if PID_FILE.is_file():
        try:
            pid = int(PID_FILE.read_text(encoding="utf-8").strip())
            os.kill(pid, signal.SIGTERM)
        except Exception:
            pass
    for _ in range(20):
        if not daemon_esta_rodando():
            PID_FILE.unlink(missing_ok=True)
            HEARTBEAT_FILE.unlink(missing_ok=True)
            PARAR_FILE.unlink(missing_ok=True)
            print("Daemon de voz local encerrado.")
            return
        time.sleep(0.1)
    PID_FILE.unlink(missing_ok=True)
    HEARTBEAT_FILE.unlink(missing_ok=True)
    PARAR_FILE.unlink(missing_ok=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description="Motor de voz local Chatterbox (Laura).")
    parser.add_argument("--daemon", action="store_true", help="Executa como daemon servidor de voz.")
    parser.add_argument("--stop", action="store_true", help="Para o daemon se estiver rodando.")
    parser.add_argument("--status", action="store_true", help="Verifica se o daemon está rodando.")
    parser.add_argument("texto", nargs="?", help="Texto para sintetizar em modo cliente.")
    args = parser.parse_args()

    if args.daemon:
        rodar_daemon()
    elif args.stop:
        parar_daemon()
    elif args.status:
        if daemon_esta_rodando():
            print("Daemon de voz local está RODANDO.")
            sys.exit(0)
        else:
            print("Daemon de voz local NÃO está rodando.")
            sys.exit(1)
    elif args.texto:
        t0 = time.time()
        print(f"Sintetizando: \"{args.texto}\"...")
        audio = sintetizar(args.texto)
        out_mp3 = ESTADO / "teste_cliente.mp3"
        out_mp3.write_bytes(audio)
        print(f"Áudio recebido ({len(audio)} bytes) em {time.time()-t0:.2f}s e salvo em {out_mp3}")
        subprocess.run(["afplay", str(out_mp3)])
    else:
        parser.print_help()
