#!/usr/bin/env python3
"""Painel visual de terminal para acompanhar o andamento dos repositórios locais e remotos.

Uso:
  voz            mostra o quadro de todos os projetos (ativos no topo)
  voz -a         mostra apenas os projetos com trabalho em andamento
  voz -w [seg]   atualiza o painel continuamente (padrão: 1s) e narra o resumo na abertura
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
    BASE, ESTADO, carregar_config, id_hash_texto,
    sintetizar, tocar_enfileirado,
)

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

ARQUIVO_CACHE_PRS = ESTADO / "cache-prs.json"
_LOCK_PRS = threading.Lock()
_EVENTO_PRS_INICIAL = threading.Event()
_CACHE_PRS = {}
_CACHE_REMOTO = {}
_CACHE_REPOS = {"instante": 0.0, "lista": []}
_POOL_GIT = concurrent.futures.ThreadPoolExecutor(max_workers=12)


def largura_visivel(texto):
    return len(RE_ANSI.sub("", texto))


def alinhar(texto, largura):
    falta = max(0, largura - largura_visivel(texto))
    return texto + (" " * falta)


def carregar_cache_prs_disco():
    if not ARQUIVO_CACHE_PRS.is_file():
        return
    try:
        dados = json.loads(ARQUIVO_CACHE_PRS.read_text(encoding="utf-8"))
        if isinstance(dados, dict):
            with _LOCK_PRS:
                _CACHE_PRS.update(dados)
    except (OSError, ValueError):
        pass


def salvar_cache_prs_disco():
    try:
        ESTADO.mkdir(parents=True, exist_ok=True)
        with _LOCK_PRS:
            conteudo = json.dumps(_CACHE_PRS, ensure_ascii=False)
        ARQUIVO_CACHE_PRS.write_text(conteudo, encoding="utf-8")
    except OSError:
        pass


def descobrir_repos(forcar=False):
    """Encontra os repositórios git do usuário com cache de 30s para não varrer o disco a cada ciclo."""
    agora = time.monotonic()
    if not forcar and _CACHE_REPOS["lista"] and (agora - _CACHE_REPOS["instante"]) < 30.0:
        return _CACHE_REPOS["lista"]

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

    _CACHE_REPOS["instante"] = agora
    _CACHE_REPOS["lista"] = candidatos
    return candidatos


def rodar_git(repo, *args, timeout=3):
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


def info_remoto_repo(repo):
    chave = str(repo)
    if chave in _CACHE_REMOTO:
        return _CACHE_REMOTO[chave]
    remoto = rodar_git(repo, "config", "--get", "remote.origin.url", timeout=2)
    slug = extrair_slug_github(remoto)
    nome = slug.split("/")[-1] if slug else repo.name
    info = (slug, nome)
    _CACHE_REMOTO[chave] = info
    return info


def consultar_prs_abertos(slug):
    if not slug:
        return []
    gh = shutil.which("gh") or "/opt/homebrew/bin/gh"
    if not os.path.exists(gh):
        return []
    try:
        r = subprocess.run(
            [gh, "pr", "list", "-R", slug, "--state", "open", "--limit", "3", "--json", "number,headRefName,commits"],
            capture_output=True,
            text=True,
            timeout=6,
        )
        if r.returncode == 0 and r.stdout.strip():
            return json.loads(r.stdout)
    except (subprocess.SubprocessError, OSError, ValueError):
        pass
    return None


def atualizar_prs_em_paralelo(repos):
    """Consulta PRs abertos de todos os repositórios simultaneamente em pool de threads."""
    slugs = []
    for repo in repos:
        slug, _ = info_remoto_repo(repo)
        if slug:
            slugs.append(slug)
    if not slugs:
        _EVENTO_PRS_INICIAL.set()
        return

    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        futuros = {pool.submit(consultar_prs_abertos, s): s for s in slugs}
        mudou = False
        for fut in concurrent.futures.as_completed(futuros):
            slug = futuros[fut]
            res = fut.result()
            if res is not None:
                with _LOCK_PRS:
                    if _CACHE_PRS.get(slug) != res:
                        _CACHE_PRS[slug] = res
                        mudou = True
        if mudou:
            salvar_cache_prs_disco()
    _EVENTO_PRS_INICIAL.set()


def iniciar_worker_prs(intervalo_prs=15.0):
    """Thread daemon que mantém o cache de PRs atualizado sem bloquear o painel local."""
    def _loop():
        while True:
            try:
                repos = descobrir_repos()
                atualizar_prs_em_paralelo(repos)
            except Exception:  # noqa: BLE001
                _EVENTO_PRS_INICIAL.set()
            time.sleep(intervalo_prs)

    t = threading.Thread(target=_loop, daemon=True)
    t.start()
    return t


def inspecionar_repo_local(repo):
    """Lê branch, modificados e ahead/behind em um único subprocesso git (~5ms)."""
    slug, nome = info_remoto_repo(repo)
    saida = rodar_git(repo, "status", "--porcelain=v2", "--branch", timeout=2)

    branch = "—"
    oid_curto = ""
    ahead, behind = 0, 0
    tem_ab = False
    modificados = 0

    for linha in saida.splitlines():
        if linha.startswith("# branch.head "):
            head = linha[14:].strip()
            branch = head if head != "(detached)" else "—"
        elif linha.startswith("# branch.oid "):
            oid_curto = linha[13:].strip()[:7]
        elif linha.startswith("# branch.ab "):
            tem_ab = True
            partes = linha[12:].strip().split()
            for p in partes:
                if p.startswith("+"):
                    try:
                        ahead = int(p[1:])
                    except ValueError:
                        pass
                elif p.startswith("-"):
                    try:
                        behind = abs(int(p[1:]))
                    except ValueError:
                        pass
        elif linha and not linha.startswith("#"):
            modificados += 1

    if branch == "—" and oid_curto:
        branch = oid_curto

    if not tem_ab and branch not in ("—", ""):
        for base_remota in (f"origin/{branch}", "origin/main", "origin/master"):
            contagem = rodar_git(repo, "rev-list", "--left-right", "--count", f"{base_remota}...HEAD", timeout=2)
            if contagem and len(contagem.split()) == 2:
                try:
                    behind, ahead = (int(x) for x in contagem.split())
                except ValueError:
                    behind, ahead = 0, 0
                break

    with _LOCK_PRS:
        prs = list(_CACHE_PRS.get(slug, []))

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


def coletar_status(sincronizar_prs=False):
    repos = descobrir_repos()
    if sincronizar_prs:
        atualizar_prs_em_paralelo(repos)
    dados = list(_POOL_GIT.map(inspecionar_repo_local, repos))
    dados.sort(key=lambda d: (not d["em_andamento"], d["nome"].lower()))
    return dados


def formatar_andamento(d):
    b = d["branch"]
    cor_b = MAGENTA if b not in ("main", "master") else ""
    fim_cor = RESET if cor_b else ""
    if d["modificados"] > 0:
        return f"{cor_b}{b}{fim_cor} {AMARELO}({d['modificados']} modif.){RESET}"
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
    partes = []
    for p in d["prs"]:
        qtd = len(p.get("commits") or [])
        termo = "commit" if qtd == 1 else "commits"
        partes.append(f"PR #{p['number']} · {qtd} {termo}")
    return f"{AZUL}" + ", ".join(partes) + f"{RESET}"


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


def obter_ou_gerar_trecho(cfg, escopo, texto):
    """Usa cache local (estado/cache-painel) para trechos com nomes de repositórios locais."""
    pasta_cache = ESTADO / "cache-painel"
    pasta_cache.mkdir(parents=True, exist_ok=True)
    arq = pasta_cache / (id_hash_texto(texto) + ".mp3")
    if arq.is_file() and arq.stat().st_size > 0:
        return arq
    try:
        audio = sintetizar(cfg, texto)
        arq.write_bytes(audio)
        return arq
    except Exception:  # noqa: BLE001
        return None


def montar_trechos_resumo(dados):
    ativos = [d for d in dados if d["em_andamento"]]
    if not ativos:
        return [("painel", "Todos os repositórios estão limpos, em dia com o remoto e sem PR aberto.")]

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
            for p in d["prs"]:
                qtd = len(p.get("commits") or [])
                termo = "commit enviado" if qtd == 1 else "commits enviados"
                detalhes.append(f"PR {p['number']} aberto com {qtd} {termo}")
        if detalhes:
            trechos.append((d["nome"], f"No {d['nome']}, " + ", ".join(detalhes) + "."))
    return trechos


def falar_resumo():
    """Aguarda brevemente o 1º ciclo de PRs, pré-gera todos os trechos em paralelo e toca sem pausas."""
    _EVENTO_PRS_INICIAL.wait(timeout=1.5)
    dados = coletar_status(sincronizar_prs=False)
    trechos = montar_trechos_resumo(dados)
    if not trechos:
        return

    cfg = carregar_config()
    with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
        futuros = [
            (texto, pool.submit(obter_ou_gerar_trecho, cfg, escopo, texto))
            for escopo, texto in trechos
        ]
        for texto, fut in futuros:
            arq = fut.result()
            tocar_enfileirado(arq, texto)


def desenhar_sem_piscar(quadro):
    linhas = quadro.splitlines()
    corpo = "\n".join(l + "\033[K" for l in linhas)
    sys.stdout.write("\033[H" + corpo + "\n\033[J")
    sys.stdout.flush()


def main():
    parser = argparse.ArgumentParser(description="Painel visual de repositórios do Voz.")
    parser.add_argument("-a", "--ativos", action="store_true", help="Mostra apenas projetos em andamento")
    parser.add_argument("-w", "--watch", nargs="?", const=1, type=float, metavar="SEG", help="Atualiza continuamente (padrão: 1s) e narra o resumo ao abrir")
    args = parser.parse_args()

    carregar_cache_prs_disco()

    if args.watch is not None:
        iniciar_worker_prs(intervalo_prs=15.0)
        threading.Thread(target=falar_resumo, daemon=True).start()
        intervalo = max(0.5, float(args.watch))
        ultimo_quadro = ""
        sys.stdout.write("\033[2J\033[H\033[?25l")
        sys.stdout.flush()
        try:
            while True:
                dados = coletar_status(sincronizar_prs=False)
                quadro = renderizar_quadro(dados, apenas_ativos=args.ativos)
                if quadro != ultimo_quadro:
                    desenhar_sem_piscar(quadro)
                    ultimo_quadro = quadro
                time.sleep(intervalo)
        except KeyboardInterrupt:
            pass
        finally:
            sys.stdout.write("\033[?25h\n")
            sys.stdout.flush()
        return

    # Execução única (`voz` ou `voz -a`): se o cache de PRs tiver menos de 20s, usa na hora; senão atualiza em paralelo.
    precisa_rede = not ARQUIVO_CACHE_PRS.is_file() or (time.time() - ARQUIVO_CACHE_PRS.stat().st_mtime) > 20.0
    dados = coletar_status(sincronizar_prs=precisa_rede)
    print(renderizar_quadro(dados, apenas_ativos=args.ativos))


if __name__ == "__main__":
    main()
