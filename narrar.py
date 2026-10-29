"""Narra em voz alta o resultado de uma tarefa longa do Claude Code.

Chamado pelo voz.sh no hook Stop, com o JSON do hook na entrada. Quando o
turno passou do tempo mínimo, pede ao Claude uma fala curta sobre o que foi
feito, sintetiza com o motor local (timbre da Laura), toca e guarda o áudio em
narracoes/. Sai com 0 só quando narrou; em qualquer outro caso o voz.sh toca
uma frase do sorteio.

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
    BASE, ESTADO, NARRACOES, TAGS, agora, buscar_audio_existente, carregar_config,
    carregar_estilo, chamar_claude, classificar_intencao, contem_termo_sensivel,
    id_hash_texto, instante, log, mensagens_do_usuario, registros, sem_tags, sincronizar_catalogo,
    sintetizar, texto_digitado, tocar_enfileirado,
)

INDICE = BASE / "narracoes.tsv"
FILA_DIR = ESTADO / "fila-narracao"
# O modelo respeita melhor limite de palavras que de caracteres; o teto em
# caracteres protege o tempo de síntese e concisão da narração.
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


def falas_inteiras_do_catalogo(intencao=None, limite=15):
    """Lista apenas falas completas (nunca cortadas) já gravadas no catálogo."""
    itens = sincronizar_catalogo()
    narracoes = [i for i in itens if i.get("tipo") == "narracao"]
    if not intencao:
        return [i["texto"] for i in narracoes[-limite:]]
    da_intencao = [i["texto"] for i in narracoes if i.get("intencao") == intencao]
    outras = [i["texto"] for i in narracoes if i.get("intencao") != intencao]
    return (da_intencao[-limite:] + outras[-5:])[:limite]


def montar_pedido(projeto, pedido, resposta, estilo, minutos):
    conversa = "\n".join("> " + m.replace("\n", " ") for m in estilo)
    perfil_salvo = carregar_estilo()
    expressoes_str = ", ".join(perfil_salvo.get("expressoes", [])) or "(nenhuma)"
    tom_str = perfil_salvo.get("tom") or "(em formação)"
    resumo_estilo = f"Tom: {tom_str} | Expressões: {expressoes_str}"
    intencao_previa = classificar_intencao(f"{pedido} {resposta}")
    existentes = falas_inteiras_do_catalogo(intencao_previa)
    bloco_existentes = "\n".join("- " + f for f in existentes) if existentes else "(nenhuma narração gravada ainda)"
    return f"""Você é a voz de uma assistente de programação. Escreva uma fala curta, fluida e completa ao terminar uma tarefa, formando um banco de vozes reutilizável em qualquer projeto.

A tarefa levou cerca de {minutos} minuto(s).

O que o usuário pediu:
{pedido[:1200]}

Mensagem final da assistente (o que foi feito):
{resposta[:2500]}

Falas completas já gravadas no catálogo:
{bloco_existentes}

Perfil de estilo acumulado do usuário:
{resumo_estilo}

Jeito de falar do usuário (mensagens recentes dele):
{conversa}

Regras obrigatórias:
- uma ou duas frases curtas e completas, no máximo {LIMITE_PALAVRAS} palavras no total;
- REUTILIZAÇÃO DE ÁUDIOS INTEIROS: se alguma das "Falas completas já gravadas no catálogo" acima descrever exatamente o resultado atual, devolva o texto exato dela para reutilizarmos o áudio inteiro; caso contrário, escreva uma frase nova e completa para ser gravada inteira no banco;
- DESIDENTIFICAÇÃO (BANCO UNIVERSAL): NUNCA mencione nomes de repositórios, projetos, branches, números de PR, nomes de arquivos ou caminhos. A frase deve soar natural e servir em qualquer projeto;
- adapte à intenção da resposta final:
  1. se aplicou mudanças ou validou código: avise de forma natural que os ajustes foram feitos e validados;
  2. se apresentou um diagnóstico ou proposta: avise que a análise e a proposta estão prontas e aguarda confirmação;
  3. se houve erro ou bloqueio: avise o motivo e peça uma decisão;
- tom conversado, calmo, expressivo e próximo (com boa dicção e cadência natural, usando vírgulas e pontuação para guiar a entonação), sem soar robótico nem burocrático;
- PROIBIDO usar tags entre colchetes (nada de [laughs], [chuckles], [sighs], etc.), risadinhas soltas, suspiros ou onomatopeias ("psiu", "shh", "hm", "haha", "rs", "kk", "ok");
- português do Brasil natural, sem abreviações ("vc", "n", "pq", "tb", "blz") e com ortografia e acentuação 100% revisadas.

Responda só com um JSON neste formato, sem mais nada: {{"fala": "..."}}"""


def montar_pedido_multiplo(itens, estilo):
    """Monta um pedido unificado e desidentificado quando 2 ou mais agentes terminam quase juntos."""
    conversa = "\n".join("> " + m.replace("\n", " ") for m in estilo)
    perfil_salvo = carregar_estilo()
    expressoes_str = ", ".join(perfil_salvo.get("expressoes", [])) or "(nenhuma)"
    tom_str = perfil_salvo.get("tom") or "(em formação)"
    resumo_estilo = f"Tom: {tom_str} | Expressões: {expressoes_str}"
    blocos = []
    for idx, it in enumerate(itens, 1):
        ped = str(it.get("pedido", ""))[:400]
        resp = str(it.get("resposta", ""))[:800]
        blocos.append(f"Tarefa {idx}:\nPedido: {ped}\nResultado: {resp}")
    relatorio = "\n\n".join(blocos)
    return f"""Você é a voz de uma assistente de programação. Várias tarefas paralelas acabaram de terminar ao mesmo tempo. Escreva uma única fala curta, fluida e completa resumindo que as frentes paralelas concluíram (sem citar nomes de repositórios, projetos, branches ou arquivos).

Tarefas concluídas:
{relatorio}

Perfil de estilo acumulado do usuário:
{resumo_estilo}

Jeito de falar do usuário:
{conversa}

Regras:
- uma ou duas frases fluidas, com no máximo {LIMITE_PALAVRAS_MULTI} palavras no total;
- NUNCA mencione nomes de repositórios, projetos, branches ou arquivos;
- tom conversado, calmo, expressivo e com boa dicção, sem tags entre colchetes, sem risadinhas e sem "psiu" ou "ok";
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
    NARRACOES.mkdir(parents=True, exist_ok=True)
    intencao = classificar_intencao(fala)
    arquivo = NARRACOES / (id_hash_texto(fala) + ".mp3")
    arquivo.write_bytes(audio)
    with INDICE.open("a", encoding="utf-8") as h:
        h.write("%s\t%s\t%s\n" % (arquivo.name, intencao, fala))
    if dias > 0:
        limite = time.time() - dias * 86400
        for velho in NARRACOES.glob("*.mp3"):
            if velho.stat().st_mtime < limite:
                velho.unlink()
    sincronizar_catalogo()
    return arquivo


def processar_lote(cfg, itens, chave=None):
    """Reutiliza um áudio inteiro equivalente do catálogo ou grava um novo áudio inteiro universal."""
    narrar = cfg["narrar"]
    arquivo_gasto, gasto = orcamento()

    ultimo = itens[-1]
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

        if not fala or contem_termo_sensivel(fala):
            log("narração descartada (vazia ou com termo sensível): %s" % proposta)
            return False

        intencao = classificar_intencao(fala)

        # Só reutiliza se existir um áudio INTEIRO no catálogo com equivalência alta (>= 0.80).
        arq_existente, texto_existente, nota = buscar_audio_existente(
            fala,
            escopo=intencao,
            tipo="narracao",
            limiar=0.80,
        )
        if arq_existente:
            log("narração inteira reutilizada do catálogo (nota %.2f, local): %s" % (nota, texto_existente))
            tocar_enfileirado(arq_existente)
            return True

        audio = sintetizar(cfg, fala)
        arquivo_gasto.write_text(str(gasto + len(fala)))
        arquivo = guardar(intencao, fala, audio, narrar.get("guardar_dias", 0))
        log("nova narração gravada no banco local (%s): %s" % (intencao, fala))
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
