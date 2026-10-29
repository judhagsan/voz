"""Peças compartilhadas pelo gerador de frases e pela narração."""

import fcntl
import hashlib
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

STOPWORDS = {
    "a", "o", "as", "os", "de", "do", "da", "dos", "das", "em", "no", "na",
    "nos", "nas", "um", "uma", "para", "pra", "pro", "por", "com", "sem",
    "que", "se", "ou", "e", "ja", "ta", "to", "aqui", "voce", "seu", "sua",
    "me", "eu", "ele", "ela", "isso", "isto", "esse", "essa", "este", "esta",
    "foi", "ser", "ter", "mais", "muito", "como", "quando", "onde", "pelo",
    "pela", "ao", "aos", "as",
}

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


def sintetizar(cfg=None, chave=None, texto=None):
    """Sintetiza áudio WAV nativo usando o motor local Chatterbox com o timbre da Laura."""
    if texto is None:
        texto = chave
    from motor_local import sintetizar as sintetizar_local
    texto_limpo = sem_tags(texto)
    return sintetizar_local(texto_limpo)


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
    # Compara frases ignorando caixa, pontuação e marcas como "(raiva)", preservando números.
    frase = unicodedata.normalize("NFC", frase).lower()
    frase = re.sub(r"\(.*?\)|\[.*?\]", "", frase)
    return " ".join(re.sub(r"[\W_]+", " ", frase).split())


def palavras_chave(texto):
    """Extrai palavras e números significativos normalizados (sem acento e sem stopwords)."""
    sem_acento = "".join(
        c for c in unicodedata.normalize("NFD", normalizar(texto))
        if unicodedata.category(c) != "Mn"
    )
    return [p for p in sem_acento.split() if (len(p) > 1 or p.isdigit()) and p not in STOPWORDS]


def sem_acento_str(texto):
    return "".join(
        c for c in unicodedata.normalize("NFD", normalizar(texto))
        if unicodedata.category(c) != "Mn"
    )


def id_hash_texto(texto):
    return hashlib.sha1(normalizar(texto).encode("utf-8")).hexdigest()[:10]


def classificar_intencao(texto):
    """Classifica a intenção comunicativa e interpretativa da fala."""
    t = sem_acento_str(texto)
    if any(p in t for p in ("proposta", "plano", "aguardo", "aprovacao", "confirmar", "opcao", "prefere", "posso")):
        return "proposta"
    if any(p in t for p in ("erro", "falhou", "bloqueou", "bloqueado", "problema", "pendente")):
        return "bloqueio"
    if any(p in t for p in ("diagnostico", "causa", "explicacao", "analise", "motivo")):
        return "diagnostico"
    return "conclusao"


def termos_sensiveis_locais():
    """Descobre nomes de repositórios locais, usuários e marcas de sessão que não devem ir para o banco."""
    proibidos = {
        "aracuan", "pindorama", "menuspoiler", "spoiler", "superpistaweb", "superpista",
        "grao", "barsan", "jlmoldes", "judhagsan", "antigravity", "claude", "elevenlabs",
        "callum", "mayla", "laura", "keychain", "zshrc",
    }
    user = os.environ.get("USER", "").strip().lower()
    if len(user) > 2:
        proibidos.add(user)
    home = Path.home()
    try:
        for item in home.iterdir():
            if item.name.startswith(".") or not item.is_dir():
                continue
            if (item / ".git").exists():
                nome = sem_acento_str(item.name)
                if len(nome) > 2 and nome not in {"voz", "menu"}:
                    proibidos.add(nome)
    except OSError:
        pass
    return proibidos


def contem_termo_sensivel(texto):
    """Retorna True se o texto mencionar repositórios locais, branches ou dados de sessão privada."""
    tokens = set(sem_acento_str(texto).split())
    if tokens & termos_sensiveis_locais():
        return True
    t = sem_acento_str(texto)
    padroes_sessao = (
        r"\bprojeto\s+d[aeo]\s+voz\b",
        r"\baqui\s+no\s+voz\b",
        r"\bno\s+voz\b",
        r"\bpr\s+\d+\b",
        r"\b(?:feat|fix|chore)/",
    )
    return any(re.search(p, t) for p in padroes_sessao)


def duracao_audio(caminho):
    """Mede a duração exata em segundos de um arquivo de áudio via afinfo nativo do macOS."""
    if not caminho or not Path(caminho).is_file():
        return 0.0
    try:
        r = subprocess.run(
            ["afinfo", str(caminho)],
            capture_output=True,
            text=True,
            timeout=3,
        )
        m = re.search(r"estimated duration:\s*([\d.]+)", r.stdout)
        if m:
            return round(float(m.group(1)), 2)
    except Exception:
        pass
    return 0.0


def perfil_diccao_interpretacao(texto, intencao, mp3_path=None):
    """Calcula metadados completos de dicção, prosódia e interpretação sem expor sessão."""
    cfg = carregar_config()
    vs = cfg.get("voice_settings") or {}
    txt = texto.strip()
    if txt.endswith("?"):
        entoacao = "interrogativa"
    elif txt.endswith("!"):
        entoacao = "exclamativa"
    else:
        entoacao = "declarativa"

    pausas_internas = sum(txt.count(p) for p in (",", ":", ";", "—"))
    frases = [f for f in re.split(r"(?<=[.!?])\s+", txt) if f.strip()]
    cadencia = "pausada" if (pausas_internas > 0 or len(frases) > 1) else "direta"

    mapa_energia = {
        "terminei": "alta",
        "conclusao": "firme",
        "proposta": "consultiva",
        "permissao": "consultiva",
        "diagnostico": "analitica",
        "bloqueio": "alerta",
        "espera": "calma",
    }
    energia = mapa_energia.get(intencao, "firme")
    vogais = re.findall(r"[aeiouáéíóúâêôãõ]+", txt.lower())

    return {
        "interpretacao": {
            "intencao": intencao,
            "entoacao": entoacao,
            "cadencia": cadencia,
            "energia": energia,
            "parametros_voz": {
                "tom": float(cfg.get("tom_voz", 0.90)),
                "velocidade": float(cfg.get("velocidade_voz", 0.91)),
                "stability": float(vs.get("stability", 0.30)),
                "similarity_boost": float(vs.get("similarity_boost", 0.75)),
                "style": float(vs.get("style", 0.38)),
            },
        },
        "diccao": {
            "silabas_estimadas": max(1, len(vogais)),
            "pausas_internas": pausas_internas,
            "sentencas": len(frases),
            "duracao_s": duracao_audio(mp3_path) if mp3_path else 0.0,
        },
    }


def sanitizar_banco():
    """Limpa histórico e expressões de estilo garantindo termos sensíveis filtrados."""
    if HISTORICO.exists():
        h_limpo = []
        for linha in HISTORICO.read_text(encoding="utf-8").splitlines():
            partes = linha.split("\t")
            if len(partes) == 4:
                _, cat, frase, arq = partes
            elif len(partes) == 3:
                cat, frase, arq = partes
            else:
                continue
            if not contem_termo_sensivel(frase) and (SONS / cat / arq).is_file():
                h_limpo.append("%s\t%s\t%s" % (cat, frase, arq))
        HISTORICO.write_text(("\n".join(h_limpo) + "\n") if h_limpo else "", encoding="utf-8")

    if ESTILO.exists():
        estilo = carregar_estilo()
        exp_limpas = [
            e for e in estilo.get("expressoes", [])
            if not contem_termo_sensivel(e)
        ]
        tom_limpo = estilo.get("tom", "")
        if contem_termo_sensivel(tom_limpo):
            tom_limpo = "Direto, técnico e objetivo, focado em qualidade de código e validação prática."
        ESTILO.write_text(
            json.dumps({"tom": tom_limpo, "expressoes": exp_limpas}, ensure_ascii=False, indent=2) + "\n",
            encoding="utf-8",
        )


def sincronizar_catalogo():
    """Sanitiza o banco, verifica os áudios em sons/ e grava catalogo.json com dicção e interpretação."""
    sanitizar_banco()
    itens = []
    vistos = set()

    if SONS.exists():
        for arq in sorted(list(SONS.glob("*/*.mp3")) + list(SONS.glob("*/*.wav"))):
            if not arq.is_file() or arq.stat().st_size == 0:
                continue
            rel = str(arq.relative_to(BASE))
            cat = arq.parent.name
            texto = unicodedata.normalize("NFC", arq.stem)
            perfil = perfil_diccao_interpretacao(texto, cat, arq)
            itens.append({
                "id": id_hash_texto(texto),
                "arquivo": rel,
                "tipo": "som",
                "escopo": cat,
                "intencao": cat,
                "texto": texto,
                "normalizado": normalizar(texto),
                "palavras": palavras_chave(texto),
                "interpretacao": perfil["interpretacao"],
                "diccao": perfil["diccao"],
                "bytes": arq.stat().st_size,
            })
            vistos.add(rel)

    dados = {
        "total": len(itens),
        "itens": itens,
    }
    CATALOGO.write_text(json.dumps(dados, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return itens

    dados = {
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
    nums_alvo = re.findall(r"\d+", texto)
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
        nums_item = re.findall(r"\d+", item.get("texto", ""))
        # Em modo de reutilização direta, números divergentes impedem reaproveitar o áudio.
        if limiar >= 0.65 and nums_alvo != nums_item:
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


def comando_autorizado_agy(cmd):
    """Verifica se todos os subcomandos do CommandLine já constam em permissions.allow da CLI."""
    cfg_agy = Path.home() / ".gemini" / "antigravity-cli" / "settings.json"
    if not cfg_agy.exists() or not cmd:
        return False
    try:
        allow = set(json.loads(cfg_agy.read_text(encoding="utf-8")).get("permissions", {}).get("allow", []))
    except (ValueError, OSError):
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


def agy_precisa_permissao(_token):
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
        time.sleep(1.0)
        return True

    if nome == "run_command":
        # Comandos em sandbox padrão rodam automaticamente e nunca pedem aprovação
        if args.get("BypassSandbox") is not True:
            return False
        # Apenas comandos que pedem para sair do sandbox solicitam aprovação manual
        cmd = str(args.get("CommandLine", "")).strip()
        if comando_autorizado_agy(cmd):
            return False
        return True

    return False


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
