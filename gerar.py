"""Gera frases novas para o sorteio dos avisos de voz do Claude Code.

Lê o que o usuário escreveu na sessão, pede ao Claude frases curtas no mesmo
jeito de falar e sintetiza cada uma com o motor local (timbre da Laura).

Uso:
  gerar.py automatico           chamado pelo voz.sh, com o JSON do hook na entrada
  gerar.py configurar           mostra o status do motor de voz local e condicionais
  gerar.py simular <transcript> mostra as frases propostas, sem sintetizar
  gerar.py sincronizar          faz commit e push imediato da base se houver mudanças
"""

import json
import os
import re
import sys
import time
import unicodedata
import urllib.parse
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from comum import (  # noqa: E402
    ESTADO, LOG, SONS, TAGS, BASE, buscar_audio_existente, carregar_config,
    carregar_estilo, chamar_claude, contem_termo_sensivel, log, mensagens_do_usuario,
    normalizar, salvar_config, salvar_estilo, sem_tags, sincronizar_catalogo,
    sincronizar_git, sintetizar,
)

HISTORICO = BASE / "historico.tsv"

CATEGORIAS = {
    "terminei": "a tarefa pedida acabou. Pode ter dado certo ou não, então a "
    "frase não pode afirmar sucesso, testes passando nem nada específico",
    "permissao": "a assistente precisa que o usuário aprove uma ação antes de seguir",
    "espera": "a assistente está parada esperando o usuário responder",
}


def frases_existentes():
    existentes = {c: set() for c in CATEGORIAS}
    for c in CATEGORIAS:
        for arquivo in (SONS / c).glob("*.mp3"):
            existentes[c].add(unicodedata.normalize("NFC", arquivo.stem))
    # O histórico guarda também as frases cujo áudio o usuário apagou, para
    # que não voltem a ser geradas.
    if HISTORICO.exists():
        for linha in HISTORICO.read_text(encoding="utf-8").splitlines():
            partes = linha.split("\t")
            if len(partes) == 3 and partes[0] in existentes:
                existentes[partes[0]].add(partes[1])
            elif len(partes) >= 4 and partes[1] in existentes:
                existentes[partes[1]].add(partes[2])
    return existentes


def montar_pedido(mensagens, existentes, categorias, estilo):
    momentos = "\n".join("- %s: %s." % (c, CATEGORIAS[c]) for c in categorias)
    ja_existem = "\n".join("- %s: %s" % (c, "; ".join(sorted(existentes[c]))) for c in categorias)
    exemplo = ", ".join('"%s": "..."' % c for c in categorias) + ', "tom": "...", "novas_expressoes": ["..."]'
    conversa = "\n".join("> " + m.replace("\n", " ") for m in mensagens)
    perfil = f"Tom observado até aqui: {estilo.get('tom') or '(ainda em formação)'}\nExpressões acumuladas: {', '.join(estilo.get('expressoes', [])) or '(nenhuma ainda)'}"
    return f"""Você escreve frases curtas que uma assistente de programação fala em voz alta para o usuário, em momentos diferentes:
{momentos}

Abaixo estão o perfil de estilo acumulado do usuário e mensagens recentes dele. Use-os para captar o vocabulário técnico e a objetividade dele. Não copie trechos literais longos e não cite nomes de pessoas, projetos, arquivos ou segredos.

Regras obrigatórias para cada frase:
- português do Brasil falado e fluido, de 3 a 8 palavras, no máximo 50 caracteres;
- tom conversado, calmo, expressivo e próximo (como uma parceira de programação com boa entonação e espaço para gracejos sutis, bom humor leve e tiradas discretas, usando vírgulas, interrogações ou exclamações suaves para dar cadência humana à fala);
- NUNCA escreva frases burocráticas, robóticas ou engessadas de telemarketing como "Concluí a tarefa solicitada" ou "Aguardo sua resposta para continuar"; prefira falas naturais e com charme discreto como "Pronto por aqui, já pode conferir!", "Fechei essa parte, dá uma olhada.", "Preciso que você aprove aqui pra seguir.", "Parada aqui esperando você me dizer.";
- PROIBIDO usar tags entre colchetes (nada de [laughs], [chuckles], [sighs], etc.), risadinhas soltas ou onomatopeias ("psiu", "shh", "hm", "haha", "rs", "kk"); o gracejo deve vir na escolha espirituosa e controlada das palavras, nunca em risadas forçadas;
- NUNCA use palavras com erro de digitação ou grafia errada: revise e corrija 100% da ortografia e acentuação padrão do português antes de devolver (contrações faladas naturais como "tô", "tá" e "pra" são bem-vindas);
- escrita como deve ser falada, sem abreviações ("vc", "n", "pq", "tb", "blz") e sem "ok";
- diferente das frases que já existem;
- precisa fazer sentido em qualquer situação daquele momento.

Além das frases, devolva também:
- "tom": resumo curto atualizado (até 25 palavras) do estilo de trabalho e conversa do usuário (com ortografia 100% revisada);
- "novas_expressoes": lista de até 5 expressões faladas naturais e com personalidade em português correto (sem onomatopeias, sem "psiu" e sem erros de digitação).

Perfil acumulado do usuário:
{perfil}

Frases que já existem:
{ja_existem}

Mensagens recentes do usuário:
{conversa}

Responda só com um JSON neste formato, sem mais nada: {{{exemplo}}}"""


PROIBIDAS = {
    "psiu", "vc", "pq", "tb", "blz", "vlw", "flw", "shh", "hm",
    "haha", "hehe", "kkk", "rs", "ok",
}


def tem_palavra_proibida(texto):
    palavras = set(normalizar(texto).split())
    return bool(palavras & PROIBIDAS) or contem_termo_sensivel(texto)


def atualizar_estilo(estilo, propostas):
    tom = sem_tags(str(propostas.get("tom", ""))).strip()
    if tom and not contem_termo_sensivel(tom):
        estilo["tom"] = tom[:200]
    atuais = [e for e in estilo.get("expressoes", []) if not tem_palavra_proibida(e)]
    vistas = {normalizar(e) for e in atuais}
    for exp in propostas.get("novas_expressoes") or []:
        exp = sem_tags(str(exp)).strip()
        if 1 < len(exp) <= 40 and not tem_palavra_proibida(exp) and normalizar(exp) not in vistas:
            atuais.append(exp)
            vistas.add(normalizar(exp))
    estilo.pop("atualizado_em", None)
    estilo["expressoes"] = atuais[-100:]
    salvar_estilo(estilo)


def pedir_frases(cfg, mensagens, existentes, categorias):
    estilo = carregar_estilo()
    propostas = chamar_claude(cfg, montar_pedido(mensagens, existentes, categorias, estilo), timeout=180)
    atualizar_estilo(estilo, propostas)
    aprovadas = {}
    for c in categorias:
        falada = sem_tags(str(propostas.get(c, ""))).strip()
        vistas = {normalizar(f) for f in existentes[c]}
        if (
            0 < len(falada) <= 50
            and len(falada.split()) <= 8
            and not tem_palavra_proibida(falada)
            and normalizar(falada) not in vistas
        ):
            aprovadas[c] = falada
        elif falada:
            log("descartada (%s): %s" % (c, falada))
    return aprovadas


def salvar_frase(cfg, chave, categoria, frase):
    arq_existente, txt_existente, nota = buscar_audio_existente(
        frase, escopo=categoria, tipo="som", limiar=0.80
    )
    if arq_existente:
        log("frase já coberta pelo catálogo (%s, nota %.2f): %s -> %s" % (categoria, nota, frase, txt_existente))
        return
    audio = sintetizar(cfg, chave, frase)
    nome = re.sub(r'[/:\\"]', "-", frase)
    destino = SONS / categoria / (nome + ".mp3")
    n = 2
    while destino.exists():
        destino = SONS / categoria / ("%s%02d.mp3" % (nome, n))
        n += 1
    destino.write_bytes(audio)
    with HISTORICO.open("a", encoding="utf-8") as h:
        h.write("%s\t%s\t%s\n" % (categoria, frase, destino.name))
    sincronizar_catalogo()
    log("nova (%s): %s" % (categoria, frase))


def categorias_abertas(cfg):
    limite = cfg.get("max_por_categoria", 0)
    if limite <= 0:
        return list(CATEGORIAS)
    return [c for c in CATEGORIAS if len(list((SONS / c).glob("*.mp3"))) < limite]


def automatico():
    if os.environ.get("VOZ_GERANDO"):
        return
    try:
        entrada = json.loads(sys.stdin.read() or "{}")
    except ValueError:
        return
    transcript = entrada.get("transcript_path") or entrada.get("transcriptPath")
    cfg = carregar_config()
    if not transcript or not os.path.exists(transcript):
        return

    marca = ESTADO / "ultima-geracao"
    if marca.exists() and time.time() - marca.stat().st_mtime < cfg["intervalo_horas"] * 3600:
        sincronizar_git(cfg)
        return
    categorias = categorias_abertas(cfg)
    if not categorias:
        sincronizar_git(cfg)
        return
    mensagens = mensagens_do_usuario(transcript)
    if len(mensagens) < cfg["min_mensagens"]:
        sincronizar_git(cfg)
        return

    trava = ESTADO / "gerando.lock"
    try:
        trava.mkdir()
    except FileExistsError:
        # Trava de uma execução que morreu no meio é descartada depois de 10 min.
        if time.time() - trava.stat().st_mtime < 600:
            return
    try:
        # Marca antes de tentar, para que uma falha não se repita a cada aviso.
        marca.touch()
        aprovadas = pedir_frases(cfg, mensagens, frases_existentes(), categorias)
        for categoria, frase in aprovadas.items():
            salvar_frase(cfg, None, categoria, frase)
        sincronizar_git(cfg, forcar=True)
    except Exception as erro:  # noqa: BLE001 - roda em segundo plano, só registra
        log("erro: %s" % erro)
    finally:
        trava.rmdir()
        if LOG.exists() and LOG.stat().st_size > 200_000:
            linhas = LOG.read_text(encoding="utf-8").splitlines()[-500:]
            LOG.write_text("\n".join(linhas) + "\n", encoding="utf-8")


def configurar():
    cfg = carregar_config()
    from motor_local import CKPT_DIR, CONDS_FILE, daemon_esta_rodando
    print("motor:  %s (voz %s)" % (cfg.get("motor", "local"), cfg.get("voz", "Laura")))
    print("tom:    %.2f, velocidade: %.2f" % (cfg.get("tom_voz", 0.90), cfg.get("velocidade_voz", 0.91)))
    print("pesos:  %s" % ("presentes" if CKPT_DIR.is_dir() else "não encontrados"))
    print("conds:  %s" % ("prontas" if CONDS_FILE.is_file() else "não geradas"))
    print("daemon: %s" % ("rodando" if daemon_esta_rodando() else "parado"))


def simular(transcript):
    cfg = carregar_config()
    mensagens = mensagens_do_usuario(transcript)
    print("%d mensagens lidas" % len(mensagens))
    propostas = pedir_frases(cfg, mensagens, frases_existentes(), list(CATEGORIAS))
    print(json.dumps(propostas, ensure_ascii=False, indent=2))


if __name__ == "__main__":
    ESTADO.mkdir(parents=True, exist_ok=True)
    comando = sys.argv[1] if len(sys.argv) > 1 else ""
    if comando == "automatico":
        automatico()
    elif comando == "configurar":
        configurar()
    elif comando == "sincronizar":
        sincronizar_git(carregar_config(), forcar=True)
    elif comando == "simular" and len(sys.argv) > 2:
        simular(sys.argv[2])
    else:
        sys.exit(__doc__)
