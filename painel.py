#!/usr/bin/env python3
"""Painel visual de terminal para acompanhar o andamento dos repositórios locais e remotos.

Uso:
  voz            mostra o quadro de todos os projetos (ativos no topo)
  voz -a         mostra apenas os projetos com trabalho em andamento
  voz -w [seg]   atualiza o painel continuamente (padrão: 5s) e narra o resumo na abertura
"""

import argparse
import concurrent.futures
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from comum import (  # noqa: E402
    BASE, buscar_audio_existente, carregar_config, chave_api,
    sincronizar_catalogo, sintetizar, tocar_enfileirado,
)
from narrar import guardar  # noqa: E402

PASTAS_IGNORADAS = {
    "Library", "Pictures", "Music", "Movies", "Public", "Downloads",
    "Applications", "node_modules", "target", "dist", "build", "venv",
}

RESET = "\033[0m"
NEGRITO = "\033[1m"
FRACO = "\033[2m"
VERDE = "\033[32m"
AMARELO = "\033[33m"
AZUL = "\033[36m"
MAGENTA = "\033[35m"

RE_ANSI = re.compile(r"\033\[[0-9;]*m")


def largura_visivel(texto):
    return len(RE_ANSI.sub("", texto))


def alinhar(texto, largura):
    falta = max(0, largura - largura_visivel(texto))
    return texto + (" " * falta)


def descobrir_repos():
    """Encontra os repositórios git do usuário na pasta pessoal e em ~/.claude/voz."""
    home = Path.home()
    candidatos = []
    vistos = set()

    if (BASE / ".git").exists():
        vistos.add(BASE.resolve())
        candidatos.append(BASE)

    for item in sorted(home.iterdir()):
        if item.name.startswith(".") or item.name in PASTAS_IGNORADAS or not item.is_dir():
            continue
        try:
            if (item / ".git").exists():
                r = item.resolve()
                if r not in vistos:
                    vistos.add(r)
                    candidatos.append(item)
                continue
            for sub in sorted(item.iterdir()):
                if sub.name.startswith(".") or sub.name in PASTAS_IGNORADAS or not sub.is_dir():
                    continue
                if (sub / ".git").exists():
                    r = sub.resolve()
                    if r not in vistos:
                        vistos.add(r)
                        candidatos.append(sub)
        except PermissionError:
            continue
    return candidatos


def rodar_git(repo, *args, timeout=5):
    try:
        r = subprocess.run(
            ["git", "-C", str(repo), *args],
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        return r.stdout.strip() if r.returncode == 0 else ""
    except (subprocess.SubprocessError, OSError):
        return ""


def extrair_slug_github(remoto):
    if not remoto:
        return ""
    achado = re.search(r"github\.com[:/]([^/]+/[^/.]+?)(?:\.git)?$", remoto.strip())
    return achado.group(1) if achado else ""


def consultar_prs_abertos(slug):
    if not slug:
        return []
    gh = shutil.which("gh") or "/opt/homebrew/bin/gh"
    if not os.path.exists(gh):
        return []
    try:
        r = subprocess.run(
            [gh, "pr", "list", "-R", slug, "--state", "open", "--limit", "3", "--json", "number,headRefName"],
            capture_output=True,
            text=True,
            timeout=6,
        )
        if r.returncode == 0 and r.stdout.strip():
            return json.loads(r.stdout)
    except (subprocess.SubprocessError, OSError, ValueError):
        pass
    return []


def inspecionar_repo(repo):
    remoto = rodar_git(repo, "remote", "get-url", "origin")
    slug = extrair_slug_github(remoto)
    nome = slug.split("/")[-1] if slug else repo.name

    branch = rodar_git(repo, "symbolic-ref", "--short", "HEAD") or rodar_git(repo, "rev-parse", "--short", "HEAD") or "—"
    porcelain = rodar_git(repo, "status", "--porcelain")
    modificados = len([l for l in porcelain.splitlines() if l.strip()]) if porcelain else 0

    # Conta commits locais ainda não enviados ao remoto (ahead) e pendentes de pull (behind).
    contagem = rodar_git(repo, "rev-list", "--left-right", "--count", "@{upstream}...HEAD")
    if not contagem:
        for base_remota in (f"origin/{branch}", "origin/main", "origin/master"):
            contagem = rodar_git(repo, "rev-list", "--left-right", "--count", f"{base_remota}...HEAD")
            if contagem:
                break
    behind, ahead = 0, 0
    if contagem and len(contagem.split()) == 2:
        try:
            behind, ahead = (int(x) for x in contagem.split())
        except ValueError:
            behind, ahead = 0, 0

    prs = consultar_prs_abertos(slug)
    em_andamento = bool(
        modificados > 0
        or ahead > 0
        or behind > 0
        or prs
        or branch not in ("main", "master", "—")
    )

    return {
        "nome": nome,
        "caminho": str(repo),
        "branch": branch,
        "modificados": modificados,
        "ahead": ahead,
        "behind": behind,
        "prs": prs,
        "em_andamento": em_andamento,
    }


def coletar_status():
    repos = descobrir_repos()
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        dados = list(pool.map(inspecionar_repo, repos))
    # Ordena colocando os que estão em andamento primeiro, e depois por nome.
    dados.sort(key=lambda d: (not d["em_andamento"], d["nome"].lower()))
    return dados


def formatar_andamento(d):
    b = d["branch"]
    cor_b = MAGENTA if b not in ("main", "master") else ""
    fim_cor = RESET if cor_b else ""
    if d["modificados"] > 0:
        plural = "modif." if d["modificados"] > 1 else "modif."
        return f"{cor_b}{b}{fim_cor} {AMARELO}({d['modificados']} {plural}){RESET}"
    return f"{cor_b}{b}{fim_cor} {FRACO}(limpo){RESET}"


def formatar_push(d):
    partes = []
    if d["ahead"] > 0:
        termo = "commit" if d["ahead"] == 1 else "commits"
        partes.append(f"{AMARELO}↑ {d['ahead']} {termo}{RESET}")
    if d["behind"] > 0:
        partes.append(f"{AZUL}↓ {d['behind']} pull{RESET}")
    if not partes:
        return f"{VERDE}✓ em dia{RESET}"
    return " ".join(partes)


def formatar_pr(d):
    if not d["prs"]:
        return f"{FRACO}—{RESET}"
    numeros = ", ".join(f"#{p['number']}" for p in d["prs"])
    return f"{AZUL}PR {numeros}{RESET}"


def renderizar_quadro(dados, apenas_ativos=False):
    exibidos = [d for d in dados if d["em_andamento"]] if apenas_ativos else dados
    linhas_formatadas = []
    for d in exibidos:
        icone = f"{VERDE}●{RESET}" if d["em_andamento"] else f"{FRACO}○{RESET}"
        nome_proj = f"{NEGRITO}{d['nome']}{RESET}" if d["em_andamento"] else f"{FRACO}{d['nome']}{RESET}"
        linhas_formatadas.append((
            f"{icone} {nome_proj}",
            formatar_andamento(d),
            formatar_push(d),
            formatar_pr(d),
        ))

    cabecalhos = ("Projeto", "Andamento (local)", "Sem push", "PR aberto")
    larguras = [len(c) for c in cabecalhos]
    for l in linhas_formatadas:
        for idx, col in enumerate(l):
            larguras[idx] = max(larguras[idx], largura_visivel(col))

    topo = "╭─" + "─┬─".join("─" * w for w in larguras) + "─╮"
    meio_titulo = "├─" + "─┬─".join("─" * w for w in larguras) + "─┤"
    sep = "├─" + "─┼─".join("─" * w for w in larguras) + "─┤"
    base = "╰─" + "─┴─".join("─" * w for w in larguras) + "─╯"

    largura_interna = sum(larguras) + 3 * (len(larguras) - 1)
    ativos = sum(1 for d in dados if d["em_andamento"])
    titulo_esq = f"{NEGRITO}VOZ{RESET} · {ativos} em andamento de {len(dados)} repositórios"
    linha_titulo = "│ " + alinhar(titulo_esq, largura_interna) + " │"
    topo_caixa = "╭─" + ("─" * largura_interna) + "─╮"

    linha_cab = "│ " + " │ ".join(
        alinhar(f"{NEGRITO}{c}{RESET}", larguras[i]) for i, c in enumerate(cabecalhos)
    ) + " │"

    saida = [topo_caixa, linha_titulo, meio_titulo, linha_cab, sep]
    if not linhas_formatadas:
        vazio = alinhar(f"{FRACO}Nenhum projeto com pendências no momento.{RESET}", largura_interna)
        saida.append(f"│ {vazio} │")
        saida.append("╰─" + ("─" * largura_interna) + "─╯")
        return "\n".join(saida)

    for l in linhas_formatadas:
        saida.append("│ " + " │ ".join(alinhar(l[i], larguras[i]) for i in range(len(larguras))) + " │")
    saida.append(base)
    return "\n".join(saida)


def falar_resumo(dados):
    ativos = [d for d in dados if d["em_andamento"]]
    if not ativos:
        fala = "Todos os repositórios estão limpos, em dia com o remoto e sem PR aberto."
    else:
        trechos = []
        for d in ativos:
            detalhes = []
            if d["branch"] not in ("main", "master"):
                detalhes.append(f"na branch {d['branch']}")
            if d["modificados"] == 1:
                detalhes.append("com um arquivo modificado")
            elif d["modificados"] > 1:
                detalhes.append(f"com {d['modificados']} arquivos modificados")
            if d["ahead"] == 1:
                detalhes.append("um commit sem push")
            elif d["ahead"] > 1:
                detalhes.append(f"{d['ahead']} commits sem push")
            if d["prs"]:
                nums = " e ".join(str(p["number"]) for p in d["prs"])
                detalhes.append(f"PR {nums} aberto")
            if detalhes:
                trechos.append(f"No {d['nome']}, " + ", ".join(detalhes) + ".")
        fala = " ".join(trechos)

    arq_existente, _, _ = buscar_audio_existente(fala, escopo="painel", tipo="narracao", limiar=0.92)
    if arq_existente:
        tocar_enfileirado(arq_existente, fala)
        return
    cfg = carregar_config()
    chave = chave_api(cfg)
    if not chave:
        tocar_enfileirado(None, fala)
        return
    try:
        audio = sintetizar(cfg, chave, fala)
        arq = guardar("painel", fala, audio, cfg["narrar"].get("guardar_dias", 0))
        sincronizar_catalogo()
        tocar_enfileirado(arq, fala)
    except Exception:  # noqa: BLE001
        tocar_enfileirado(None, fala)


def main():
    parser = argparse.ArgumentParser(description="Painel visual de repositórios do Voz.")
    parser.add_argument("-a", "--ativos", action="store_true", help="Mostra apenas projetos em andamento")
    parser.add_argument("-w", "--watch", nargs="?", const=5, type=int, metavar="SEG", help="Atualiza continuamente e narra o resumo ao abrir")
    args = parser.parse_args()

    if args.watch:
        falou = False
        try:
            while True:
                dados = coletar_status()
                quadro = renderizar_quadro(dados, apenas_ativos=args.ativos)
                sys.stdout.write("\033[2J\033[H" + quadro + "\n")
                sys.stdout.flush()
                if not falou:
                    falou = True
                    threading.Thread(target=falar_resumo, args=(dados,), daemon=True).start()
                time.sleep(max(2, args.watch))
        except KeyboardInterrupt:
            print()
            return

    dados = coletar_status()
    print(renderizar_quadro(dados, apenas_ativos=args.ativos))


if __name__ == "__main__":
    main()
