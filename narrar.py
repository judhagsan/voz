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
    BASE, ESTADO, NARRACOES, TAGS, agora, api_bloqueada, buscar_audio_existente, carregar_config,
    carregar_estilo, chamar_claude, chave_api, instante, log, mensagens_do_usuario, registros,
    sem_tags, sincronizar_catalogo, sintetizar, texto_digitado, tocar_enfileirado,
)

INDICE = BASE / "narracoes.tsv"
FILA_DIR = ESTADO / "fila-narracao"
# O modelo respeita melhor limite de palavras que de caracteres; o teto em
# caracteres protege o orçamento do ElevenLabs.
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


def falas_do_catalogo(projeto, limite=15):
    """Lista falas já gravadas e verificadas no disco para reaproveitamento."""
    itens = sincronizar_catalogo()
    do_projeto = [
        i["texto"] for i in itens
        if i.get("tipo") == "narracao" and i.get("escopo", "").lower() == projeto.lower()
    ]
    gerais = [
        i["texto"] for i in itens
        if i.get("tipo") == "narracao" and i.get("escopo", "").lower() != projeto.lower()
    ]
    return (do_projeto[-limite:] + gerais[-5:])[:limite]


def montar_pedido(projeto, pedido, resposta, estilo, minutos):
    conversa = "\n".join("> " + m.replace("\n", " ") for m in estilo)
    perfil_salvo = carregar_estilo()
    expressoes_str = ", ".join(perfil_salvo.get("expressoes", [])) or "(nenhuma)"
    tom_str = perfil_salvo.get("tom") or "(em formação)"
    resumo_estilo = f"Tom: {tom_str} | Expressões: {expressoes_str}"
    existentes = falas_do_catalogo(projeto)
    bloco_existentes = "\n".join("- " + f for f in existentes) if existentes else "(nenhum áudio gravado ainda para este projeto)"
    desc_projeto = (
        "sessão na pasta pessoal (identifique o nome real do projeto pelo texto da resposta, ex: Aracuan, Pindorama, MenuSpoiler, tmux)"
        if projeto == "geral"
        else projeto
    )
    return f"""Você é a voz de uma assistente de programação. Escreva o que ela fala em voz alta ao terminar uma tarefa, para um usuário que talvez esteja longe da tela ou com vários projetos abertos.

Projeto da pasta atual: {desc_projeto}
A tarefa levou cerca de {minutos} minuto(s).

O que o usuário pediu:
{pedido[:1200]}

Mensagem final da assistente (o que foi feito):
{resposta[:2500]}

Áudios já gravados e verificados no catálogo para reaproveitamento:
{bloco_existentes}

Perfil de estilo acumulado do usuário:
{resumo_estilo}

Jeito de falar do usuário (mensagens recentes dele):
{conversa}

Regras:
- uma ou duas frases curtas e fluidas, no máximo {LIMITE_PALAVRAS} palavras no total;
- REUTILIZAÇÃO INTELIGENTE (ECONOMIA DE CRÉDITOS): se algum dos "Áudios já gravados" acima servir bem e com precisão para avisar o resultado atual (ex: proposta pronta aguardando aprovação no mesmo projeto), devolva EXATAMENTE o texto dele para reaproveitarmos o arquivo sem gastar créditos. Se o resultado tiver detalhes novos importantes, escreva uma fala nova;
- SEMPRE mencione de forma natural de qual projeto ou ferramenta está falando (ex: "Aqui no Aracuan...", "No Pindorama...", "No Menu Spoiler..."). Se o campo Projeto indicar a pasta pessoal, identifique o nome do projeto pelo assunto do pedido/resposta;
- adapte ao contexto da resposta final:
  1. se executou código/mudanças: conte em poucas palavras o que ficou pronto (ou se algo falhou);
  2. se apresentou um plano ou proposta: resuma a ideia principal em uma frase e avise que aguarda aprovação;
  3. se respondeu uma dúvida ou diagnóstico: resuma a conclusão principal de forma direta;
- tom conversado, calmo, expressivo e próximo (como uma parceira de programação falando com boa entonação, cadência tranquila e espaço para um gracejo sutil ou leveza espirituosa quando couber, sem exagero), sem soar robótico nem burocrático;
- PROIBIDO usar tags entre colchetes (nada de [laughs], [chuckles], [sighs], etc.), risadinhas soltas, suspiros ou onomatopeias ("psiu", "shh", "hm", "haha", "rs", "kk", "ok"); o humor deve ser sutil e controlado nas palavras;
- escreva como se fala: sem caminhos de arquivo, código, comandos, nomes de função ou símbolos; descreva em palavras;
- nunca inclua senhas, chaves, tokens, e-mails ou dados pessoais;
- português do Brasil natural, sem abreviações ("vc", "n", "pq", "tb", "blz");
- revise e corrija 100% da ortografia e acentuação padrão do português antes de devolver (contrações faladas naturais como "tô", "tá" e "pra" são permitidas).

Responda só com um JSON neste formato, sem mais nada: {{"fala": "..."}}"""


def montar_pedido_multiplo(itens, estilo):
    """Monta um pedido unificado quando 2 ou mais agentes/repositórios terminam juntos."""
    conversa = "\n".join("> " + m.replace("\n", " ") for m in estilo)
    perfil_salvo = carregar_estilo()
    expressoes_str = ", ".join(perfil_salvo.get("expressoes", [])) or "(nenhuma)"
    tom_str = perfil_salvo.get("tom") or "(em formação)"
    resumo_estilo = f"Tom: {tom_str} | Expressões: {expressoes_str}"
    blocos = []
    for idx, it in enumerate(itens, 1):
        proj = it.get("projeto", "geral")
        ped = str(it.get("pedido", ""))[:500]
        resp = str(it.get("resposta", ""))[:1000]
        blocos.append(f"Agente {idx} — Repositório/Projeto: {proj}\nPedido: {ped}\nResultado: {resp}")
    relatorio = "\n\n".join(blocos)
    return f"""Você é a voz de uma assistente de programação. Vários agentes acabaram de concluir tarefas ao mesmo tempo em projetos/repositórios diferentes. Faça uma única fala fluida juntando os resultados, identificando claramente quando está falando de cada repositório (ex: "Aqui no repositório X fiz tal coisa; e agora falando do repositório Y...").

 Tarefas concluídas:
{relatorio}

Perfil de estilo acumulado do usuário:
{resumo_estilo}

Jeito de falar do usuário:
{conversa}

Regras:
- junte todos os itens de forma fluida, com no máximo {LIMITE_PALAVRAS_MULTI} palavras no total;
- identifique claramente cada repositório/projeto antes do respectivo resumo;
- tom conversado, calmo, expressivo e próximo, com espaço para leveza discreta, sem tags entre colchetes, sem risadinhas e sem "psiu" ou "ok";
- português do Brasil falado com ortografia e acentuação 100% revisadas.

Responda só com um JSON neste formato, sem mais nada: {{"fala": "..."}}"""


def pedir_fala(cfg, transcript, cwd, resposta=None):
    inicio, pedido, final = ultimo_turno(transcript)
    resposta = resposta or final
    if not inicio or not pedido or not resposta:
        return None, 0
    segundos = agora() - inicio
    minutos = max(1, round(segundos / 60))
    pedido_claude = montar_pedido(nome_do_projeto(cwd), pedido, resposta, mensagens_do_usuario(transcript, 10), minutos)
    proposta = sem_tags(str(chamar_claude(cfg, pedido_claude, timeout=45).get("fala", ""))).strip()
    fala = encurtar(proposta, LIMITE_FALA)
    if not fala:
        log("narração descartada: %s" % proposta)
    elif fala != proposta:
        log("narração encurtada: %s" % proposta)
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
    sincronizar_catalogo()
    return arquivo


def processar_lote(cfg, chave, itens):
    """Processa 1 item ou junta múltiplos itens da fila em uma única fala, reutilizando áudios quando possível."""
    narrar = cfg["narrar"]
    teto_dia = int(narrar.get("max_caracteres_dia", 0))
    arquivo_gasto, gasto = orcamento()

    ultimo = itens[-1]
    projeto = ultimo["projeto"]
    transcript = ultimo["transcript"]
    msgs = mensagens_do_usuario(transcript, 10) if transcript and os.path.exists(transcript) else []

    try:
        if len(itens) == 1:
            it = itens[0]
            minutos = max(1, round(it.get("segundos", 10) / 60))
            pedido_claude = montar_pedido(it["projeto"], it["pedido"], it["resposta"], msgs, minutos)
            proposta = sem_tags(str(chamar_claude(cfg, pedido_claude, timeout=45).get("fala", ""))).strip()
            fala = encurtar(proposta, LIMITE_FALA)
        else:
            pedido_claude = montar_pedido_multiplo(itens, msgs)
            proposta = sem_tags(str(chamar_claude(cfg, pedido_claude, timeout=45).get("fala", ""))).strip()
            fala = encurtar(proposta, LIMITE_FALA_MULTI)
            projeto = "multiplos"

        if not fala:
            log("narração descartada: %s" % proposta)
            return False

        # Verifica no catálogo se já existe áudio equivalente gravado (0 créditos gastos).
        arq_existente, texto_existente, nota = buscar_audio_existente(
            fala,
            escopo=projeto if projeto != "multiplos" else None,
            tipo="narracao",
            limiar=0.75,
        )
        if arq_existente:
            log("narração reutilizada do catálogo (nota %.2f, 0 créditos): %s" % (nota, texto_existente))
            tocar_enfileirado(arq_existente)
            return True

        if teto_dia > 0 and gasto + len(fala) > teto_dia:
            log("orçamento diário atingido (%d/%d); buscando áudio existente no catálogo" % (gasto, teto_dia))
            arq_fb, txt_fb, nota_fb = buscar_audio_existente(fala, escopo=projeto, limiar=0.25)
            if arq_fb:
                log("fallback do catálogo (nota %.2f): %s" % (nota_fb, txt_fb))
                tocar_enfileirado(arq_fb)
                return True
            return False

        audio = sintetizar(cfg, chave, fala)
        arquivo_gasto.write_text(str(gasto + len(fala)))
        arquivo = guardar(projeto, fala, audio, narrar["guardar_dias"])
        log("narração (%s): %s" % (projeto, fala))
        tocar_enfileirado(arquivo)
        return True
    except Exception as erro:  # noqa: BLE001 - em falha da API, tenta responder com áudio já existente no catálogo
        log("erro na narração (%s); tentando catálogo existente" % erro)
        contexto_busca = "%s %s %s" % (projeto, ultimo.get("pedido", ""), ultimo.get("resposta", ""))
        arq_fb, txt_fb, nota_fb = buscar_audio_existente(contexto_busca, escopo=projeto, limiar=0.22)
        if arq_fb:
            log("narração respondida via catálogo existente (nota %.2f): %s" % (nota_fb, txt_fb))
            tocar_enfileirado(arq_fb)
            return True
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
    if not narrar["ativo"] or not cfg.get("voice_id") or entrada.get("stop_hook_active"):
        return False
    if not transcript or not os.path.exists(transcript):
        return False

    inicio, pedido, final = ultimo_turno(transcript)
    segundos = (agora() - inicio) if inicio else 0
    if not inicio or segundos < narrar.get("min_segundos", 0):
        log("narração ignorada: duração %.1fs menor que min_segundos" % segundos)
        return False

    chave = chave_api(cfg)
    if not chave:
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
            return processar_lote(cfg, chave, itens)
        finally:
            fcntl.flock(f_lock, fcntl.LOCK_UN)


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
