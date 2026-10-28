"""Peças compartilhadas pelo gerador de frases e pela narração."""

import json
import os
import re
import shutil
import subprocess
import time
import unicodedata
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

BASE = Path.home() / ".claude" / "voz"
SONS = BASE / "sons"
NARRACOES = BASE / "narracoes"
ESTADO = BASE / "estado"
CONFIG = BASE / "config.json"
ESTILO = BASE / "estilo.json"
LOG = BASE / "gerar.log"
API = "https://api.elevenlabs.io"

# Tags de emoção que o ElevenLabs v3/v4 interpreta; são sugestões, não lista fechada.
TAGS = "[laughs], [chuckles], [excited], [sighs], [whispers], [sarcastic], [curious]"


def log(msg):
    print(time.strftime("%Y-%m-%d %H:%M:%S"), msg, flush=True)


def carregar_config():
    return json.loads(CONFIG.read_text(encoding="utf-8"))


def salvar_config(cfg):
    CONFIG.write_text(json.dumps(cfg, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def carregar_estilo():
    if ESTILO.exists():
        try:
            return json.loads(ESTILO.read_text(encoding="utf-8"))
        except ValueError:
            pass
    return {"tom": "", "expressoes": [], "atualizado_em": ""}


def salvar_estilo(estilo):
    ESTILO.write_text(json.dumps(estilo, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")


def chave_api(cfg):
    r = subprocess.run(
        ["security", "find-generic-password", "-s", cfg["keychain_service"], "-w"],
        capture_output=True,
        text=True,
    )
    return r.stdout.strip() if r.returncode == 0 else ""


def requisicao(chave, caminho, corpo=None, timeout=60):
    cabecalhos = {"xi-api-key": chave}
    dados = None
    if corpo is not None:
        dados = json.dumps(corpo).encode("utf-8")
        cabecalhos["Content-Type"] = "application/json"
    req = urllib.request.Request(API + caminho, data=dados, headers=cabecalhos)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.read()
    except urllib.error.HTTPError as erro:
        # O corpo traz o motivo (plano, cota, voz); sem ele o log só diria o código.
        motivo = erro.read().decode("utf-8", "replace")[:400]
        raise RuntimeError("ElevenLabs %d: %s" % (erro.code, motivo)) from None


BLOQUEIO = ESTADO / "api-bloqueada"


def api_bloqueada():
    # Recusa por plano ou cota não passa sozinha; tenta de novo só depois de um dia.
    return BLOQUEIO.exists() and time.time() - BLOQUEIO.stat().st_mtime < 86400


def sintetizar(cfg, chave, texto):
    try:
        return requisicao(
            chave,
            "/v1/text-to-speech/%s?output_format=mp3_44100_128" % cfg["voice_id"],
            {"text": texto, "model_id": cfg["model_id"]},
        )
    except RuntimeError as erro:
        if str(erro).startswith(("ElevenLabs 401", "ElevenLabs 402")):
            BLOQUEIO.write_text(str(erro), encoding="utf-8")
        raise


def chamar_claude(cfg, pedido, timeout=60):
    """Roda um `claude -p` sem ferramentas e devolve o JSON da resposta."""
    claude = shutil.which("claude") or str(Path.home() / ".local" / "bin" / "claude")
    # Sem as configurações do usuário, o `claude -p` não dispara os hooks
    # globais (nem os da voz, nem o do ai-memory); VOZ_GERANDO é a segunda trava.
    r = subprocess.run(
        [claude, "-p", "--model", cfg["modelo_claude"], "--setting-sources", "project,local",
         "--strict-mcp-config", "--no-session-persistence", "--output-format", "text",
         "--tools", ""],
        input=pedido,
        capture_output=True,
        text=True,
        timeout=timeout,
        cwd=str(ESTADO),
        env=dict(os.environ, VOZ_GERANDO="1"),
    )
    if r.returncode != 0:
        raise RuntimeError("claude -p falhou: " + (r.stderr or r.stdout).strip()[:300])
    achado = re.search(r"\{.*\}", r.stdout, re.S)
    if not achado:
        raise RuntimeError("resposta sem JSON: " + r.stdout.strip()[:300])
    return json.loads(achado.group(0))


def sem_tags(texto):
    return " ".join(re.sub(r"\[.*?\]", "", texto).split())


def normalizar(frase):
    # Compara frases ignorando caixa, pontuação, numeração e marcas como "(raiva)".
    frase = unicodedata.normalize("NFC", frase).lower()
    frase = re.sub(r"\(.*?\)|\[.*?\]", "", frase)
    return " ".join(re.sub(r"[\d\W_]+", " ", frase).split())


def registros(transcript):
    with open(transcript, encoding="utf-8") as f:
        for linha in f:
            try:
                yield json.loads(linha)
            except ValueError:
                continue


def texto_digitado(registro):
    """Devolve o que o usuário digitou, ou None se o registro não for isso."""
    if registro.get("type") == "USER_INPUT" and registro.get("source") == "USER_EXPLICIT":
        conteudo = registro.get("content")
        if not isinstance(conteudo, str):
            return None
        achado = re.search(r"<USER_REQUEST>\s*(.*?)\s*</USER_REQUEST>", conteudo, flags=re.S)
        conteudo = (
            achado.group(1).strip()
            if achado
            else re.sub(r"<([\w-]+)[^>]*>.*?</\1>", "", conteudo, flags=re.S).strip()
        )
        return conteudo if conteudo and not conteudo.startswith("<") else None
    if registro.get("type") != "user" or registro.get("isSidechain") or registro.get("isMeta"):
        return None
    conteudo = registro.get("message", {}).get("content")
    # Texto digitado chega como string; resultados de ferramenta, como lista.
    if not isinstance(conteudo, str):
        return None
    conteudo = re.sub(r"<([\w-]+)[^>]*>.*?</\1>", "", conteudo, flags=re.S).strip()
    if not conteudo or conteudo.startswith("<"):
        return None
    return conteudo


def mensagens_do_usuario(transcript, limite=30):
    mensagens = [t[:300] for t in map(texto_digitado, registros(transcript)) if t]
    return mensagens[-limite:]


def instante(registro):
    marca = registro.get("timestamp") or registro.get("created_at")
    if not marca:
        return None
    return datetime.fromisoformat(marca.replace("Z", "+00:00")).timestamp()


def agora():
    return datetime.now(timezone.utc).timestamp()


CAMINHOS_BANCO = ["sons", "narracoes", "estilo.json", "historico.tsv", "narracoes.tsv"]


def sincronizar_git(cfg, forcar=False):
    """Faz commit e push diário da base de vozes e estilo se houver alterações."""
    marca = ESTADO / "ultimo-sync"
    intervalo = cfg.get("intervalo_horas", 24) * 3600
    if not forcar and marca.exists() and time.time() - marca.stat().st_mtime < intervalo:
        return False
    marca.touch()

    alvos = [c for c in CAMINHOS_BANCO if (BASE / c).exists()]
    if not alvos:
        return False
    status = subprocess.run(
        ["git", "-C", str(BASE), "status", "--porcelain", "--", *alvos],
        capture_output=True,
        text=True,
    )
    if status.returncode != 0 or not status.stdout.strip():
        return False

    subprocess.run(["git", "-C", str(BASE), "add", "--", *alvos], check=True)
    msg = (
        "chore(banco): sincroniza base diária de vozes e perfil de estilo\n\n"
        "Registra no repositório os novos áudios sintetizados, narrações e a\n"
        "evolução do vocabulário acumulado nas sessões do dia, preservando o\n"
        "histórico completo sem sobrescrever alterações manuais de código."
    )
    r_commit = subprocess.run(
        ["git", "-C", str(BASE), "commit", "-m", msg],
        capture_output=True,
        text=True,
    )
    if r_commit.returncode != 0:
        log("git commit ignorado: " + (r_commit.stderr or r_commit.stdout).strip()[:200])
        return False

    r_push = subprocess.run(
        ["git", "-C", str(BASE), "push", "origin", "main"],
        capture_output=True,
        text=True,
        timeout=60,
    )
    if r_push.returncode != 0:
        log("git push falhou: " + (r_push.stderr or r_push.stdout).strip()[:200])
        return False
    log("git sync: alterações do banco enviadas para origin/main")
    return True
