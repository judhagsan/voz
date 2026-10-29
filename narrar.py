"""Narra em voz alta o resultado de uma tarefa longa.

Chamado pelo voz.sh no hook Stop, com o JSON do hook na entrada. Quando o
turno passou do tempo mínimo, extrai um resumo direto da resposta final,
sintetiza com o motor local Chatterbox (timbre da Laura) e toca via fila.
Sai com 0 só quando narrou; em qualquer outro caso o voz.sh toca uma frase
do sorteio.

Uso:
  narrar.py                              modo hook
  narrar.py simular <transcript> [cwd]   mostra a fala, sem sintetizar
"""

import fcntl
import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from comum import (  # noqa: E402
    BASE, ESTADO, agora, carregar_config,
    classificar_intencao, instante, log,
    registros, sem_tags, sintetizar, texto_digitado, tocar_enfileirado,
)

FILA_DIR = ESTADO / "fila-narracao"
LIMITE_PALAVRAS = 25
LIMITE_FALA = 220
LIMITE_PALAVRAS_MULTI = 45
LIMITE_FALA_MULTI = 340


def ultimo_turno(transcript):
    """Devolve (início, pedido, resposta final) do último pedido do usuário."""
    inicio, pedido, textos = None, "", []
    for r in registros(transcript):
        digitado = texto_digitado(r)
        if digitado:
            inicio, pedido, textos = instante(r), digitado, []
            continue
        if r.get("type") == "PLANNER_RESPONSE":
            if r.get("tool_calls"):
                textos = []
            elif isinstance(r.get("content"), str) and r.get("content").strip():
                textos.append(r["content"])
            continue
        if r.get("isSidechain"):
            continue
        conteudo = r.get("message", {}).get("content")
        if not isinstance(conteudo, list):
            continue
        if r.get("type") == "user" and any(i.get("type") == "tool_result" for i in conteudo):
            # Texto anterior a uma ferramenta era só andamento, não a resposta final.
            textos = []
        elif r.get("type") == "assistant":
            textos += [i["text"] for i in conteudo if i.get("type") == "text" and i.get("text")]
    return inicio, pedido, "\n".join(textos).strip()


def nome_do_projeto(cwd):
    if not cwd:
        return "geral"
    remoto = subprocess.run(["git", "-C", cwd, "remote", "get-url", "origin"], capture_output=True, text=True)
    if remoto.returncode == 0 and remoto.stdout.strip():
        nome_remoto = Path(remoto.stdout.strip().rstrip("/")).stem
        if nome_remoto:
            return nome_remoto
    r = subprocess.run(["git", "-C", cwd, "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    raiz = Path(r.stdout.strip() if r.returncode == 0 else cwd)
    if raiz == Path.home():
        return "geral"
    return raiz.name


def limpar_linha(l):
    """Remove marcações de markdown e pontuação que atrapalham a fala."""
    l = re.sub(r"\[([^\]]+)\]\([^\)]+\)", r"\1", l)
    l = re.sub(r"[*_`#~]", "", l)
    l = re.sub(r"^[0-9]+[.)]\s*", "", l)
    l = re.sub(r"^[-*•]\s*", "", l)
    return l.strip().rstrip(":")


def extrair_resumo_direto(resposta, projeto="geral"):
    """Extrai a primeira frase conclusiva e objetiva da resposta final da assistente."""
    linhas = [l.strip() for l in (resposta or "").splitlines() if l.strip()]
    candidatos = []
    for l in linhas:
        if l.startswith(("#", "```", "|", ">", "<!--", "http")):
            continue
        l_limpo = limpar_linha(l)
        if len(l_limpo.split()) >= 4:
            candidatos.append(l_limpo)
            if len(candidatos) >= 2:
                break
    if not candidatos:
        return f"Tudo pronto por aqui no {projeto}." if projeto and projeto != "geral" else "Tudo pronto por aqui."
    frase = " ".join(candidatos)
    sentencas = [s.strip() for s in re.split(r"(?<=[.!?])\s+", frase) if s.strip()]
    escolhida = sentencas[0] if sentencas else candidatos[0]
    if len(sentencas) > 1 and len((escolhida + " " + sentencas[1]).split()) <= 22:
        escolhida += " " + sentencas[1]
    palavras = escolhida.split()
    if len(palavras) > 25:
        escolhida = " ".join(palavras[:25]) + "..."
    if not escolhida.endswith((".", "!", "?", "…")):
        escolhida += "."
    if projeto and projeto != "geral" and not any(escolhida.lower().startswith(p) for p in ("no ", "na ", "em ", "aqui no ")):
        escolhida = f"No {projeto}, {escolhida[:1].lower() + escolhida[1:]}"
    return escolhida


def extrair_resumo_multiplo(itens):
    """Gera resumo combinado quando 2 ou mais frentes paralelas terminam juntas."""
    total = len(itens)
    projetos = [it.get("projeto", "geral") for it in itens]
    if total == 2:
        return f"Concluí duas frentes em paralelo: no {projetos[0]} e no {projetos[1]}."
    projs_str = ", ".join(p for p in projetos[:-1] if p != "geral") + " e " + (projetos[-1] if projetos[-1] != "geral" else "outras")
    return f"Concluí {total} frentes de trabalho em paralelo: {projs_str}."


def pedir_fala(cfg, transcript, cwd, resposta=None):
    inicio, pedido, final = ultimo_turno(transcript)
    resposta = resposta or final
    if not inicio or not resposta:
        return None, 0
    segundos = agora() - inicio
    projeto = nome_do_projeto(cwd)
    proposta = extrair_resumo_direto(resposta, projeto)
    fala = encurtar(proposta, LIMITE_FALA)
    if not fala:
        log("narração descartada (vazia): %s" % proposta)
        return None, segundos
    return fala, segundos


def encurtar(fala, limite=LIMITE_FALA):
    """Mantém as frases inteiras que cabem no teto; None se nem a primeira cabe."""
    frases = re.findall(r".+?(?:[.!?…]+(?:\s*\[[^\]]*\])*|$)", fala.strip())
    curta = ""
    for frase in (f.strip() for f in frases if f.strip()):
        candidata = (curta + " " + frase).strip()
        if len(sem_tags(candidata)) > limite:
            break
        curta = candidata
    return curta or None


def guardar_efemero(fala, audio):
    """Guarda a narração apenas em arquivo efêmero em estado/ para reprodução imediata."""
    arquivo = ESTADO / "narracao_temp.wav"
    arquivo.write_bytes(audio)
    return arquivo


def processar_lote(cfg, itens, chave=None):
    """Gera áudio contextual sob demanda e reproduz via fila sem gravar no Git."""
    try:
        if len(itens) == 1:
            it = itens[0]
            fala = extrair_resumo_direto(it["resposta"], it["projeto"])
            fala = encurtar(fala, LIMITE_FALA)
        else:
            fala = extrair_resumo_multiplo(itens)
            fala = encurtar(fala, LIMITE_FALA_MULTI)

        if not fala:
            log("narração descartada (vazia)")
            return False

        intencao = classificar_intencao(fala)
        audio = sintetizar(cfg, fala)
        arquivo = guardar_efemero(fala, audio)
        log("narração sintetizada no motor local (%s): %s" % (intencao, fala))
        tocar_enfileirado(arquivo)
        return True
    except Exception as erro:  # noqa: BLE001 - falha cai para o aviso curto inteiro em voz.sh
        log("erro na narração (%s); usando aviso curto inteiro de sons/" % erro)
        return False


def hook():
    if os.environ.get("VOZ_GERANDO"):
        return False
    try:
        entrada = json.loads(sys.stdin.read() or "{}")
    except ValueError:
        return False
    cfg = carregar_config()
    narrar = cfg["narrar"]
    transcript = entrada.get("transcript_path") or entrada.get("transcriptPath")
    cwd = entrada.get("cwd") or (entrada.get("workspacePaths") or [None])[0]
    if not narrar["ativo"] or entrada.get("stop_hook_active"):
        return False
    if not transcript or not os.path.exists(transcript):
        return False

    inicio, pedido, final = ultimo_turno(transcript)
    segundos = (agora() - inicio) if inicio else 0
    if not inicio or segundos < narrar.get("min_segundos", 0):
        log("narração ignorada: duração %.1fs menor que min_segundos" % segundos)
        return False

    resposta = entrada.get("last_assistant_message")
    for _ in range(5):
        if resposta or final:
            break
        time.sleep(0.3)
        _, pedido, final = ultimo_turno(transcript)
    resposta = resposta or final
    if not pedido or not resposta:
        log("narração ignorada: pedido ou resposta vazia no transcript")
        return False

    projeto = nome_do_projeto(cwd)
    FILA_DIR.mkdir(parents=True, exist_ok=True)
    item_arq = FILA_DIR / ("%.6f-%d.json" % (time.time(), os.getpid()))
    item_arq.write_text(
        json.dumps(
            {
                "projeto": projeto,
                "pedido": pedido,
                "resposta": resposta,
                "transcript": str(transcript),
                "cwd": str(cwd or ""),
                "segundos": segundos,
            },
            ensure_ascii=False,
        ),
        encoding="utf-8",
    )

    # Janela curta para agrupar outros agentes que terminem quase no mesmo instante.
    time.sleep(1.2)

    trava = ESTADO / "narrando.lock"
    with open(trava, "w", encoding="utf-8") as f_lock:
        fcntl.flock(f_lock, fcntl.LOCK_EX)
        try:
            pendentes = sorted(FILA_DIR.glob("*.json"))
            if not pendentes:
                # Já foi agrupado e narrado pelo lote anterior na fila.
                return True
            itens = []
            for p in pendentes:
                try:
                    itens.append(json.loads(p.read_text(encoding="utf-8")))
                except ValueError:
                    pass
                p.unlink(missing_ok=True)
            if not itens:
                return False
            return processar_lote(cfg, itens)
        finally:
            fcntl.flock(f_lock, fcntl.LOCK_UN)


def simular(transcript, cwd):
    cfg = carregar_config()
    fala, segundos = pedir_fala(cfg, transcript, cwd)
    print("turno de %d s, projeto: %s" % (segundos, nome_do_projeto(cwd)))
    print(fala or "(sem fala)")
    if fala:
        print("%d caracteres para síntese local (Chatterbox Laura)" % len(fala))


if __name__ == "__main__":
    ESTADO.mkdir(parents=True, exist_ok=True)
    if len(sys.argv) > 2 and sys.argv[1] == "simular":
        simular(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else os.getcwd())
    else:
        sys.exit(0 if hook() else 1)
