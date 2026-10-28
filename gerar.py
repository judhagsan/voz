"""Gera frases novas para o sorteio dos avisos de voz do Claude Code.

Lê o que o usuário escreveu na sessão, pede ao Claude frases curtas no mesmo
jeito de falar e sintetiza cada uma com a voz configurada no ElevenLabs.

Uso:
  gerar.py automatico           chamado pelo voz.sh, com o JSON do hook na entrada
  gerar.py configurar           acha a voz, o modelo e o orçamento no ElevenLabs
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
    BLOQUEIO, ESTADO, LOG, SONS, TAGS, BASE, api_bloqueada, carregar_config, carregar_estilo, chamar_claude,
    chave_api, log, mensagens_do_usuario, normalizar, requisicao, salvar_config, salvar_estilo, sem_tags,
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
            if len(partes) >= 3 and partes[1] in existentes:
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

Abaixo estão o perfil de estilo acumulado do usuário e mensagens recentes dele. Use-os para captar e evoluir o jeito dele de falar (gírias, tom, humor) e o universo de programação dele. Não copie trechos literais longos e não cite nomes de pessoas, projetos, arquivos ou segredos.

Regras para cada frase:
- português do Brasil falado, de 1 a 8 palavras, no máximo 50 caracteres sem contar as tags;
- informal e com personalidade, no tom das frases que já existem; pode ser uma piada leve; nada formal ou genérico como "Acabei o que você pediu";
- pode ter no máximo uma tag de emoção do ElevenLabs entre colchetes, em inglês, como {TAGS};
- escrita como deve ser falada, sem abreviações que a voz leria errado (nada de "vc", "n", "pq");
- ortografia e acentuação sempre corretas, mesmo que as mensagens do usuário tenham erros de digitação: copie o tom dele, não os erros (formas faladas como "tô", "tá" e "pra" podem);
- nada ofensivo;
- diferente das frases que já existem;
- precisa fazer sentido em qualquer situação daquele momento.

Além das frases, devolva também:
- "tom": resumo curto atualizado (até 25 palavras) do estilo, humor e jeito de conversar do usuário;
- "novas_expressoes": lista de até 5 gírias, interjeições ou termos recorrentes do jeito dele (já escritos por extenso como devem ser falados).

Perfil acumulado do usuário:
{perfil}

Frases que já existem:
{ja_existem}

Mensagens recentes do usuário:
{conversa}

Responda só com um JSON neste formato, sem mais nada: {{{exemplo}}}"""


def atualizar_estilo(estilo, propostas):
    tom = str(propostas.get("tom", "")).strip()
    if tom:
        estilo["tom"] = tom[:200]
    atuais = list(estilo.get("expressoes", []))
    vistas = {normalizar(e) for e in atuais}
    for exp in propostas.get("novas_expressoes") or []:
        exp = str(exp).strip()
        if 1 < len(exp) <= 40 and normalizar(exp) not in vistas:
            atuais.append(exp)
            vistas.add(normalizar(exp))
    estilo["expressoes"] = atuais[-100:]
    estilo["atualizado_em"] = time.strftime("%Y-%m-%d %H:%M:%S")
    salvar_estilo(estilo)


def pedir_frases(cfg, mensagens, existentes, categorias):
    estilo = carregar_estilo()
    propostas = chamar_claude(cfg, montar_pedido(mensagens, existentes, categorias, estilo), timeout=180)
    atualizar_estilo(estilo, propostas)
    aprovadas = {}
    for c in categorias:
        frase = str(propostas.get(c, "")).strip()
        falada = sem_tags(frase)
        vistas = {normalizar(f) for f in existentes[c]}
        if 0 < len(falada) <= 50 and len(falada.split()) <= 8 and normalizar(frase) not in vistas:
            aprovadas[c] = frase
        elif frase:
            log("descartada (%s): %s" % (c, frase))
    return aprovadas


def salvar_frase(cfg, chave, categoria, frase):
    audio = sintetizar(cfg, chave, frase)
    nome = re.sub(r'[/:\\"]', "-", frase)
    destino = SONS / categoria / (nome + ".mp3")
    n = 2
    while destino.exists():
        destino = SONS / categoria / ("%s%02d.mp3" % (nome, n))
        n += 1
    destino.write_bytes(audio)
    with HISTORICO.open("a", encoding="utf-8") as h:
        h.write("%s\t%s\t%s\t%s\n" % (time.strftime("%Y-%m-%d"), categoria, frase, destino.name))
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
    if not transcript or not os.path.exists(transcript) or not cfg.get("voice_id"):
        return
    if api_bloqueada():
        sincronizar_git(cfg)
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
    chave = chave_api(cfg)
    if not chave:
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
            salvar_frase(cfg, chave, categoria, frase)
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
    chave = chave_api(cfg)
    if not chave:
        sys.exit("Chave não encontrada no Keychain (serviço %s)." % cfg["keychain_service"])

    busca = urllib.parse.quote(cfg["voice_name"])
    vozes = json.loads(requisicao(chave, "/v2/voices?search=" + busca)).get("voices", [])
    origem = "suas vozes"
    if not vozes:
        vozes = json.loads(requisicao(chave, "/v1/shared-voices?search=" + busca)).get("voices", [])
        origem = "biblioteca de vozes"
    if not vozes:
        sys.exit("Nenhuma voz chamada %s encontrada." % cfg["voice_name"])
    for v in vozes:
        print("voz:    %s  %s  (%s)" % (v.get("voice_id"), v.get("name"), origem))
    cfg["voice_id"] = vozes[0]["voice_id"]

    modelos = json.loads(requisicao(chave, "/v1/models"))
    tts = [m["model_id"] for m in modelos if m.get("can_do_text_to_speech")]
    print("modelos:", ", ".join(tts))
    # O v4 "cheio" é o mais expressivo; o Turbo troca expressividade por latência.
    v4 = [m for m in tts if "v4" in m and "turbo" not in m and "flash" not in m]
    if v4:
        cfg["model_id"] = v4[0]
    else:
        print("Nenhum modelo v4 na lista; mantido", cfg["model_id"])

    try:
        assinatura = json.loads(requisicao(chave, "/v1/user/subscription"))
        limite = int(assinatura["character_limit"])
        # 70% do limite mensal dividido por 30 dias fica para a narração.
        cfg["narrar"]["max_caracteres_dia"] = max(200, int(limite * 0.7 / 30))
        print("plano:  %s, %d créditos/mês, %d usados" % (
            assinatura.get("tier"), limite, assinatura.get("character_count", 0)))
    except Exception as erro:  # noqa: BLE001 - a chave pode não ter permissão de leitura
        print("Sem acesso à assinatura (%s); orçamento diário mantido." % erro)

    salvar_config(cfg)
    # Configurar de novo (outra voz, outro plano) libera uma pausa por recusa.
    if BLOQUEIO.exists():
        BLOQUEIO.unlink()
    print("salvo: voz %s, modelo %s, %d caracteres/dia para narração" % (
        cfg["voice_id"], cfg["model_id"], cfg["narrar"]["max_caracteres_dia"]))


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
