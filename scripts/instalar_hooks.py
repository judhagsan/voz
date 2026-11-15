#!/usr/bin/env python3
"""Registra os hooks do Voz preservando as integrações já instaladas."""

import argparse
import json
import os
import shlex
import shutil
import tempfile
from pathlib import Path


MARCA_INICIO = "<!-- voz:inicio -->"
MARCA_FIM = "<!-- voz:fim -->"
BLOCO_INSTRUCAO = (
    f"{MARCA_INICIO}\n"
    "## Resposta objetiva para leitura em voz alta\n\n"
    "- Ao concluir alterações ou diagnóstico técnico, abra a resposta final com uma frase curta e natural em português (até 35 palavras) resumindo o que foi feito e o resultado principal, sem blocos de código, links ou caminhos de arquivo.\n"
    "- Se precisar de confirmação ou decisão do usuário para seguir, encerre a resposta com essa pergunta de forma direta.\n"
    f"{MARCA_FIM}\n"
)


def carregar(caminho):
    if not caminho.exists():
        return {}
    dados = json.loads(caminho.read_text(encoding="utf-8"))
    if not isinstance(dados, dict):
        raise ValueError(f"{caminho}: esperado um objeto JSON")
    return dados


def escrever_texto(caminho, texto):
    if caminho.exists() and caminho.read_text(encoding="utf-8") == texto:
        return
    caminho.parent.mkdir(parents=True, exist_ok=True)
    if caminho.exists():
        backup = caminho.with_name(caminho.name + ".voz.bak")
        if not backup.exists():
            shutil.copy2(caminho, backup)
    temporario = None
    try:
        with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=caminho.parent, delete=False) as arquivo:
            temporario = Path(arquivo.name)
            arquivo.write(texto)
        if caminho.exists():
            temporario.chmod(caminho.stat().st_mode & 0o777)
        os.replace(temporario, caminho)
    finally:
        if temporario is not None:
            temporario.unlink(missing_ok=True)


def salvar(caminho, dados):
    texto = json.dumps(dados, ensure_ascii=False, indent=2) + "\n"
    escrever_texto(caminho, texto)


def aplicar_bloco_instrucao(conteudo, bloco=BLOCO_INSTRUCAO):
    linhas = conteudo.splitlines(keepends=True)
    inicio = None
    fim = None
    for idx, linha in enumerate(linhas):
        marcador = linha.strip()
        if inicio is None and marcador == MARCA_INICIO:
            inicio = idx
        elif inicio is not None and marcador == MARCA_FIM:
            fim = idx
            break

    if inicio is not None and fim is not None:
        antes = "".join(linhas[:inicio])
        depois = "".join(linhas[fim + 1:])
        bloco_norm = bloco if depois else bloco.rstrip("\n") + "\n"
        return antes + bloco_norm + depois

    if not conteudo.strip():
        return bloco
    return conteudo.rstrip("\n") + "\n\n" + bloco


def configurar_instrucao(caminho):
    atual = caminho.read_text(encoding="utf-8") if caminho.exists() else ""
    novo = aplicar_bloco_instrucao(atual)
    escrever_texto(caminho, novo)


def eh_comando_voz(cmd_existente, cmd_novo):
    if cmd_existente == cmd_novo:
        return True
    try:
        partes_antigas = shlex.split(cmd_existente)
        partes_novas = shlex.split(cmd_novo)
    except ValueError:
        return False
    if not partes_antigas or not partes_novas:
        return False
    prog_antigo = Path(partes_antigas[0]).name
    prog_novo = Path(partes_novas[0]).name
    if prog_antigo == "voz.sh" and prog_novo == "voz.sh":
        return partes_antigas[1:] == partes_novas[1:]
    return False


def garantir_hook(lista, comando, matcher=None, timeout=600):
    # Acrescenta ao fim para preservar também os índices dos hooks confiados
    # pelo Codex. Só atualiza o handler que corresponde ao comando do Voz.
    for grupo in lista:
        if grupo.get("matcher") != matcher:
            continue
        for hook in grupo.get("hooks", []):
            if eh_comando_voz(hook.get("command", ""), comando):
                hook.update(type="command", command=comando, **{"async": True, "timeout": timeout})
                return
    grupo = {"hooks": [{"type": "command", "command": comando, "async": True, "timeout": timeout}]}
    if matcher is not None:
        grupo["matcher"] = matcher
    lista.append(grupo)


def configurar_codex(dados, voz_sh):
    hooks = dados.setdefault("hooks", {})
    comando = f"{shlex.quote(str(voz_sh))} --codex"
    garantir_hook(hooks.setdefault("Stop", []), comando)
    garantir_hook(hooks.setdefault("PermissionRequest", []), comando, timeout=120)
    garantir_hook(
        hooks.setdefault("PreToolUse", []), comando,
        matcher=r"^(functions\.)?request_user_input(_async)?$", timeout=120,
    )


def configurar_claude(dados, voz_sh):
    hooks = dados.setdefault("hooks", {})
    comando = shlex.quote(str(voz_sh))
    garantir_hook(hooks.setdefault("Stop", []), f"{comando} terminei")
    garantir_hook(hooks.setdefault("Notification", []), f"{comando} permissao", matcher="permission_prompt|elicitation_dialog")
    garantir_hook(hooks.setdefault("Notification", []), f"{comando} espera", matcher="idle_prompt")


def configurar_antigravity(dados, voz_sh):
    comando = shlex.quote(str(voz_sh))
    dados["voz"] = {
        "PreInvocation": [{"type": "command", "command": f"{comando} --agy pre-invocation"}],
        "PreToolUse": [{"matcher": "run_command|write_file|edit_file|ask_question", "hooks": [{"type": "command", "command": f"{comando} --agy permissao"}]}],
        "PostToolUse": [{"matcher": "", "hooks": [{"type": "command", "command": f"{comando} --agy pos-tool"}]}],
        "Stop": [{"type": "command", "command": f"{comando} --agy terminei"}],
    }
    # O Voz deve manter a posição que tinha no instalador do Antigravity.
    voz = dados.pop("voz")
    outros = dict(dados)
    dados.clear()
    dados.update(voz=voz, **outros)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agente", choices=["todos", "codex", "claude", "antigravity"], default="todos")
    parser.add_argument("--destino", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument(
        "--sem-instrucoes",
        action="store_true",
        help="Não adiciona o bloco de resumo em voz alta nos arquivos globais dos agentes.",
    )
    args = parser.parse_args()
    voz_sh = args.destino.resolve() / "voz.sh"
    if not voz_sh.is_file():
        parser.error(f"voz.sh não encontrado em {args.destino}")
    codex_home = Path(os.environ.get("CODEX_HOME") or Path.home() / ".codex")
    alvos = [
        ("claude", Path.home() / ".claude/settings.json", configurar_claude, Path.home() / ".claude/CLAUDE.md"),
        ("antigravity", Path.home() / ".gemini/config/hooks.json", configurar_antigravity, Path.home() / ".gemini/GEMINI.md"),
        ("codex", codex_home / "hooks.json", configurar_codex, codex_home / "AGENTS.md"),
    ]
    try:
        for agente, caminho, configurar, caminho_instrucao in alvos:
            if args.agente not in ("todos", agente):
                continue
            dados = carregar(caminho)
            configurar(dados, voz_sh)
            salvar(caminho, dados)
            print(f"Hooks do Voz registrados para {agente}: {caminho}")
            if not args.sem_instrucoes:
                configurar_instrucao(caminho_instrucao)
                print(f"Instruções de resumo do Voz atualizadas para {agente}: {caminho_instrucao}")
    except (OSError, ValueError, TypeError, AttributeError) as erro:
        parser.exit(1, f"Não foi possível registrar os hooks: {erro}\n")
    if args.agente in ("todos", "codex"):
        print("No Codex, abra /hooks e revise/confie nos três hooks novos do Voz.")


if __name__ == "__main__":
    main()
