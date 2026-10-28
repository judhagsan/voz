"""Narra em voz alta o resultado de uma tarefa longa do Claude Code.

Chamado pelo voz.sh no hook Stop, com o JSON do hook na entrada. Quando o
turno passou do tempo mínimo, pede ao Claude uma fala curta sobre o que foi
feito, sintetiza com o ElevenLabs, toca e guarda o áudio em narracoes/. Sai
com 0 só quando narrou; em qualquer outro caso o voz.sh toca uma frase do
sorteio.

Uso:
  narrar.py                              modo hook
  narrar.py simular <transcript> [cwd]   mostra a fala, sem sintetizar
"""

import json
import os
import re
import subprocess
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from comum import (  # noqa: E402
    BASE, ESTADO, NARRACOES, TAGS, agora, api_bloqueada, carregar_config, carregar_estilo, chamar_claude, chave_api,
    instante, log, mensagens_do_usuario, registros, sem_tags, sintetizar, texto_digitado,
)

INDICE = BASE / "narracoes.tsv"
# O modelo respeita melhor limite de palavras que de caracteres; o teto em
# caracteres protege o orçamento do ElevenLabs.
LIMITE_PALAVRAS = 25
LIMITE_FALA = 220


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
        return "nenhum projeto específico"
    r = subprocess.run(["git", "-C", cwd, "rev-parse", "--show-toplevel"], capture_output=True, text=True)
    raiz = Path(r.stdout.strip() if r.returncode == 0 else cwd)
    if raiz == Path.home():
        return "nenhum projeto específico (pasta pessoal)"
    return raiz.name


def montar_pedido(projeto, pedido, resposta, estilo, minutos):
    conversa = "\n".join("> " + m.replace("\n", " ") for m in estilo)
    perfil_salvo = carregar_estilo()
    resumo_estilo = f"Tom: {perfil_salvo.get('tom') or '(em formação)'} | Expressões: {', '.join(perfil_salvo.get('expressoes', [])) or '(nenhuma)'}"
    return f"""Você é a voz de uma assistente de programação. Escreva o que ela fala em voz alta ao terminar uma tarefa, para um usuário que talvez esteja longe da tela.

Projeto: {projeto}
A tarefa levou cerca de {minutos} minuto(s).

O que o usuário pediu:
{pedido[:1200]}

Mensagem final da assistente (o que foi feito):
{resposta[:2500]}

Perfil de estilo acumulado do usuário:
{resumo_estilo}

Jeito de falar do usuário (mensagens recentes dele):
{conversa}

Regras:
- uma ou duas frases curtas, no máximo {LIMITE_PALAVRAS} palavras no total, sem contar as tags;
- diga o resultado de verdade: o que ficou pronto, o que falhou ou ficou pendente, e se a assistente espera uma resposta ou decisão dele. Não invente nada que não esteja na mensagem final e, se não tiver certeza de um detalhe, deixe-o de fora;
- a informação vem primeiro; depois, quando couber, uma piada leve ou um comentário sobre o projeto;
- tom informal, com personalidade, no jeito de falar do usuário;
- pode usar até duas tags de emoção do ElevenLabs entre colchetes, em inglês, como {TAGS};
- escreva como se fala: sem caminhos de arquivo, código, comandos, nomes de função ou símbolos; descreva em palavras;
- nunca inclua senhas, chaves, tokens, e-mails ou dados pessoais;
- português do Brasil, sem abreviações que a voz leria errado (nada de "vc", "n", "pq");
- ortografia e acentuação sempre corretas, mesmo que as mensagens do usuário tenham erros de digitação: copie o tom dele, não os erros (formas faladas como "tô", "tá" e "pra" podem).

Responda só com um JSON neste formato, sem mais nada: {{"fala": "..."}}"""


def pedir_fala(cfg, transcript, cwd, resposta=None):
    inicio, pedido, final = ultimo_turno(transcript)
    resposta = resposta or final
    if not inicio or not pedido or not resposta:
        return None, 0
    segundos = agora() - inicio
    minutos = max(1, round(segundos / 60))
    pedido_claude = montar_pedido(nome_do_projeto(cwd), pedido, resposta, mensagens_do_usuario(transcript, 10), minutos)
    proposta = str(chamar_claude(cfg, pedido_claude, timeout=45).get("fala", "")).strip()
    fala = encurtar(proposta)
    if not fala:
        log("narração descartada: %s" % proposta)
    elif fala != proposta:
        log("narração encurtada: %s" % proposta)
    return fala, segundos


def encurtar(fala):
    """Mantém as frases inteiras que cabem no teto; None se nem a primeira cabe."""
    frases = re.findall(r".+?(?:[.!?…]+(?:\s*\[[^\]]*\])*|$)", fala.strip())
    curta = ""
    for frase in (f.strip() for f in frases if f.strip()):
        candidata = (curta + " " + frase).strip()
        if len(sem_tags(candidata)) > LIMITE_FALA:
            break
        curta = candidata
    return curta or None


def orcamento():
    """Devolve o arquivo do gasto de hoje e quantos caracteres já foram usados."""
    hoje = ESTADO / ("narracao-" + time.strftime("%Y-%m-%d"))
    for antigo in ESTADO.glob("narracao-*"):
        if antigo != hoje:
            antigo.unlink()
    return hoje, int(hoje.read_text()) if hoje.exists() else 0


def guardar(projeto, fala, audio, dias):
    pasta = NARRACOES / re.sub(r"[^\w.-]+", "-", projeto).strip("-")
    pasta.mkdir(parents=True, exist_ok=True)
    arquivo = pasta / (time.strftime("%Y-%m-%d_%H%M%S") + ".mp3")
    arquivo.write_bytes(audio)
    with INDICE.open("a", encoding="utf-8") as h:
        h.write("%s\t%s\t%s\t%s\n" % (time.strftime("%Y-%m-%d %H:%M"), projeto, arquivo.name, fala))
    if dias > 0:
        limite = time.time() - dias * 86400
        for velho in NARRACOES.glob("*/*.mp3"):
            if velho.stat().st_mtime < limite:
                velho.unlink()
    return arquivo


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
    if not narrar["ativo"] or not cfg.get("voice_id") or entrada.get("stop_hook_active"):
        return False
    if api_bloqueada():
        return False
    if not transcript or not os.path.exists(transcript):
        return False

    inicio, _, final = ultimo_turno(transcript)
    if not inicio or agora() - inicio < narrar["min_segundos"]:
        return False
    arquivo_gasto, gasto = orcamento()
    if gasto + LIMITE_FALA > narrar["max_caracteres_dia"]:
        return False
    chave = chave_api(cfg)
    if not chave:
        return False

    resposta = entrada.get("last_assistant_message")
    if not resposta and not final:
        # A resposta final pode ainda não ter sido gravada no transcript.
        time.sleep(1)
    try:
        fala, _ = pedir_fala(cfg, transcript, cwd, resposta)
        if not fala:
            return False
        audio = sintetizar(cfg, chave, fala)
    except Exception as erro:  # noqa: BLE001 - roda dentro do hook, só registra
        log("erro na narração: %s" % erro)
        return False
    arquivo_gasto.write_text(str(gasto + len(fala)))
    projeto = nome_do_projeto(cwd)
    arquivo = guardar(projeto, fala, audio, narrar["guardar_dias"])
    log("narração (%s): %s" % (projeto, fala))
    subprocess.run(["afplay", str(arquivo)])
    return True


def simular(transcript, cwd):
    cfg = carregar_config()
    fala, segundos = pedir_fala(cfg, transcript, cwd)
    print("turno de %d s, projeto: %s" % (segundos, nome_do_projeto(cwd)))
    print(fala or "(sem fala)")
    if fala:
        print("%d caracteres enviados ao ElevenLabs" % len(fala))


if __name__ == "__main__":
    ESTADO.mkdir(parents=True, exist_ok=True)
    if len(sys.argv) > 2 and sys.argv[1] == "simular":
        simular(sys.argv[2], sys.argv[3] if len(sys.argv) > 3 else os.getcwd())
    else:
        sys.exit(0 if hook() else 1)
