"""Peças compartilhadas pelo gerador de frases e pela narração."""

import fcntl
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
CATALOGO = BASE / "catalogo.json"
HISTORICO = BASE / "historico.tsv"
INDICE_NARRACOES = BASE / "narracoes.tsv"
LOG = BASE / "gerar.log"
API = "https://api.elevenlabs.io"

STOPWORDS = {
    "a", "o", "as", "os", "de", "do", "da", "dos", "das", "em", "no", "na",
    "nos", "nas", "um", "uma", "para", "pra", "pro", "por", "com", "sem",
    "que", "se", "ou", "e", "ja", "ta", "to", "aqui", "voce", "seu", "sua",
    "me", "eu", "ele", "ela", "isso", "isto", "esse", "essa", "este", "esta",
    "foi", "ser", "ter", "mais", "muito", "como", "quando", "onde", "pelo",
    "pela", "ao", "aos", "as",
}

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


def ajustar_tom(audio, fator, velocidade=1.0):
    """Altera o tom (pitch) e a velocidade do MP3 via ffmpeg."""
    if (not fator or abs(fator - 1.0) < 0.01) and (not velocidade or abs(velocidade - 1.0) < 0.01):
        return audio
    ffmpeg = shutil.which("ffmpeg") or "/opt/homebrew/bin/ffmpeg"
    if not os.path.exists(ffmpeg):
        return audio
    atempo = max(0.5, min(2.0, float(velocidade) / float(fator)))
    filtro = "asetrate=44100*%.4f,aresample=44100,atempo=%.4f" % (fator, atempo)
    r = subprocess.run(
        [ffmpeg, "-hide_banner", "-loglevel", "error", "-f", "mp3", "-i", "pipe:0", "-af", filtro, "-b:a", "128k", "-f", "mp3", "pipe:1"],
        input=audio,
        capture_output=True,
    )
    return r.stdout if r.returncode == 0 and r.stdout else audio


def sintetizar(cfg, chave, texto):
    texto_limpo = sem_tags(texto)
    try:
        audio = requisicao(
            chave,
            "/v1/text-to-speech/%s?output_format=mp3_44100_128" % cfg["voice_id"],
            {
                "text": texto_limpo,
                "model_id": cfg["model_id"],
                "language_code": "pt",
                "voice_settings": {
                    "stability": 0.30,
                    "similarity_boost": 0.75,
                    "style": 0.38,
                },
            },
        )
        return ajustar_tom(
            audio,
            float(cfg.get("tom_voz", 1.0)),
            float(cfg.get("velocidade_voz", 1.0)),
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


def palavras_chave(texto):
    """Extrai palavras significativas normalizadas (sem acento e sem stopwords)."""
    sem_acento = "".join(
        c for c in unicodedata.normalize("NFD", normalizar(texto))
        if unicodedata.category(c) != "Mn"
    )
    return [p for p in sem_acento.split() if len(p) > 1 and p not in STOPWORDS]


def sincronizar_catalogo():
    """Verifica todos os .mp3 reais em sons/ e narracoes/ contra os índices e grava catalogo.json."""
    itens = []
    vistos = set()

    # 1. Sons curtos por categoria (sons/<categoria>/*.mp3)
    if SONS.exists():
        for mp3 in sorted(SONS.glob("*/*.mp3")):
            if not mp3.is_file() or mp3.stat().st_size == 0:
                continue
            rel = str(mp3.relative_to(BASE))
            cat = mp3.parent.name
            texto = unicodedata.normalize("NFC", mp3.stem)
            itens.append({
                "arquivo": rel,
                "tipo": "som",
                "escopo": cat,
                "texto": texto,
                "normalizado": normalizar(texto),
                "palavras": palavras_chave(texto),
                "bytes": mp3.stat().st_size,
            })
            vistos.add(rel)

    # 2. Narrações contextuais verificadas (narracoes/<projeto>/*.mp3 <-> narracoes.tsv)
    linhas_validas = []
    if INDICE_NARRACOES.exists():
        for linha in INDICE_NARRACOES.read_text(encoding="utf-8").splitlines():
            partes = linha.split("\t")
            if len(partes) < 4:
                continue
            data_hora, proj, nome_arq, fala = partes[0], partes[1], partes[2], partes[3]
            pasta_proj = re.sub(r"[^\w.-]+", "-", proj).strip("-")
            mp3 = NARRACOES / pasta_proj / nome_arq
            if not mp3.is_file() or mp3.stat().st_size == 0:
                continue
            rel = str(mp3.relative_to(BASE))
            if rel in vistos:
                continue
            vistos.add(rel)
            linhas_validas.append(linha)
            itens.append({
                "arquivo": rel,
                "tipo": "narracao",
                "escopo": proj,
                "texto": fala,
                "normalizado": normalizar(fala),
                "palavras": palavras_chave(fala),
                "bytes": mp3.stat().st_size,
                "criado_em": data_hora,
            })
        # Limpa entradas órfãs de arquivos que já foram removidos do disco.
        novo_tsv = ("\n".join(linhas_validas) + "\n") if linhas_validas else ""
        if INDICE_NARRACOES.read_text(encoding="utf-8") != novo_tsv:
            INDICE_NARRACOES.write_text(novo_tsv, encoding="utf-8")

    dados = {
        "atualizado_em": time.strftime("%Y-%m-%d %H:%M:%S"),
        "total": len(itens),
        "itens": itens,
    }
    CATALOGO.write_text(json.dumps(dados, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return itens


def buscar_audio_existente(texto, escopo=None, tipo=None, limiar=0.72):
    """Procura no catálogo verificado um .mp3 existente compatível por texto ou palavras-chave."""
    itens = sincronizar_catalogo()
    norm = normalizar(texto)
    p_alvo = set(palavras_chave(texto))
    if not norm:
        return None, None, 0.0

    melhor = None
    melhor_nota = 0.0
    for item in itens:
        if tipo and item.get("tipo") != tipo:
            continue
        caminho = BASE / item["arquivo"]
        if not caminho.is_file() or caminho.stat().st_size == 0:
            continue
        if item["normalizado"] == norm:
            if not escopo or item.get("escopo") == escopo:
                return caminho, item["texto"], 1.0
        p_item = set(item.get("palavras") or [])
        if not p_alvo or not p_item:
            continue
        comuns = len(p_alvo & p_item)
        if comuns == 0:
            continue
        # Combina cobertura das palavras do áudio e das palavras buscadas.
        cobertura_item = comuns / len(p_item)
        cobertura_alvo = comuns / len(p_alvo)
        nota = (cobertura_item * 0.65) + (cobertura_alvo * 0.35)
        if escopo and item.get("escopo") and item["escopo"].lower() == escopo.lower():
            nota += 0.08
        if nota > melhor_nota:
            melhor_nota = nota
            melhor = item

    if melhor and melhor_nota >= limiar:
        return BASE / melhor["arquivo"], melhor["texto"], min(1.0, melhor_nota)
    return None, None, 0.0


def tocar_enfileirado(arquivo=None, reserva=""):
    """Reproduz o áudio com trava exclusiva (flock) para nunca sobrepor falas simultâneas."""
    ESTADO.mkdir(parents=True, exist_ok=True)
    trava = ESTADO / "reproducao.lock"
    with open(trava, "w", encoding="utf-8") as f:
        fcntl.flock(f, fcntl.LOCK_EX)
        try:
            tocou = False
            if arquivo and os.path.exists(str(arquivo)):
                r = subprocess.run(["afplay", str(arquivo)], stderr=subprocess.DEVNULL)
                tocou = r.returncode == 0
            if not tocou and reserva:
                r_say = subprocess.run(["say", "-v", "Luciana (Enhanced)", reserva], stderr=subprocess.DEVNULL)
                if r_say.returncode != 0:
                    subprocess.run(["say", reserva], stderr=subprocess.DEVNULL)
            time.sleep(0.35)
        finally:
            fcntl.flock(f, fcntl.LOCK_UN)


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
        if achado:
            texto = achado.group(1).strip()
            return texto if texto else None
        # Quando a sessão compacta a janela (<CONTEXT_SUMMARY>), extrai o último pedido listado.
        if "<CONTEXT_SUMMARY>" in conteudo:
            pedidos = re.findall(r"^\d+\.\s+(.+)$", conteudo, flags=re.M)
            if pedidos:
                return pedidos[-1].strip()
        conteudo = re.sub(r"<([\w-]+)[^>]*>.*?</\1>", "", conteudo, flags=re.S).strip()
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


CAMINHOS_BANCO = ["sons", "narracoes", "estilo.json", "catalogo.json", "historico.tsv", "narracoes.tsv"]


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


def comando_autorizado_agy(cmd):
    """Verifica se todos os subcomandos do CommandLine já constam em permissions.allow da CLI."""
    cfg_agy = Path.home() / ".gemini" / "antigravity-cli" / "settings.json"
    if not cfg_agy.exists() or not cmd:
        return False
    try:
        allow = set(json.loads(cfg_agy.read_text(encoding="utf-8")).get("permissions", {}).get("allow", []))
    except ValueError:
        return False
    if f"command({cmd})" in allow:
        return True

    builtins_livres = {"cd", "true", "false", ":", "echo", "printf", "sleep", "for", "do", "done", "test", "[", "setopt"}
    subcomandos = [s.strip() for s in re.split(r"\s*(?:&&|\|\||;|\||\n)\s*", cmd) if s.strip()]
    if not subcomandos:
        return False

    for sub in subcomandos:
        if f"command({sub})" in allow:
            continue
        # Remove atribuições iniciais de variáveis (ex: FOO=bar cmd ...)
        tokens = sub.split()
        while tokens and re.match(r"^[A-Za-z_][A-Za-z0-9_]*=", tokens[0]):
            tokens.pop(0)
        if not tokens:
            continue
        prim = tokens[0]
        if prim in builtins_livres:
            continue
        if f"command({prim})" in allow:
            continue
        if len(tokens) >= 2 and f"command({prim} {tokens[1]})" in allow:
            continue
        if len(tokens) >= 3 and f"command({prim} {tokens[1]} {tokens[2]})" in allow:
            continue
        return False
    return True


def comando_em_execucao(cmd):
    """Retorna True se o comando já foi aprovado pela CLI e está rodando em processo filho."""
    trecho = cmd.strip().splitlines()[0][:40].strip()
    if len(trecho) < 2:
        return False
    try:
        saida = subprocess.run(
            ["ps", "-axo", "args="],
            capture_output=True,
            text=True,
            timeout=2,
        ).stdout
    except (subprocess.SubprocessError, OSError):
        return False
    for linha in saida.splitlines():
        if "comum.py agy-permissao" in linha or "voz.sh" in linha:
            continue
        if trecho in linha:
            return True
    return False


def agy_precisa_permissao(token):
    """Retorna True se a ferramenta na CLI parou aguardando aprovação do usuário."""
    import sys

    try:
        entrada = json.loads(sys.stdin.read() or "{}")
    except ValueError:
        return False
    tc = entrada.get("toolCall") or {}
    nome = tc.get("name", "")
    args = tc.get("args") or {}

    if nome == "ask_question":
        return True

    cmd = str(args.get("CommandLine", "")).strip()
    if nome == "run_command" and comando_autorizado_agy(cmd):
        return False

    pendente = ESTADO / "agy-pendente"
    time.sleep(2.0)
    # Se PostToolUse já limpou ou outro tool call entrou no lugar, já executou sem parar.
    if not pendente.exists() or pendente.read_text(encoding="utf-8").strip() != token:
        return False
    # Se o comando já está em execução no sistema operacional, a CLI já o aprovou.
    if nome == "run_command" and cmd and comando_em_execucao(cmd):
        return False
    return True


if __name__ == "__main__":
    import sys

    acao = sys.argv[1] if len(sys.argv) > 1 else ""
    if acao == "agy-permissao" and len(sys.argv) > 2:
        sys.exit(0 if agy_precisa_permissao(sys.argv[2]) else 1)
    elif acao == "tocar":
        arq = sys.argv[2] if len(sys.argv) > 2 else ""
        res = sys.argv[3] if len(sys.argv) > 3 else ""
        tocar_enfileirado(arq, res)
    elif acao == "catalogo":
        itens = sincronizar_catalogo()
        print("catalogo sincronizado: %d itens verificados" % len(itens))
