#!/usr/bin/env python3
"""Registra os hooks do Voz preservando as integrações já instaladas."""

import argparse
import json
import os
import shlex
import shutil
import tempfile
from pathlib import Path


def carregar(caminho):
    if not caminho.exists():
        return {}
    dados = json.loads(caminho.read_text(encoding="utf-8"))
    if not isinstance(dados, dict):
        raise ValueError(f"{caminho}: esperado um objeto JSON")
    return dados


def salvar(caminho, dados):
    texto = json.dumps(dados, ensure_ascii=False, indent=2) + "\n"
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
        "PreToolUse": [{"matcher": "run_command|ask_question", "hooks": [{"type": "command", "command": f"{comando} --agy permissao"}]}],
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
    args = parser.parse_args()
    voz_sh = args.destino.resolve() / "voz.sh"
    if not voz_sh.is_file():
        parser.error(f"voz.sh não encontrado em {args.destino}")
    codex_home = Path(os.environ.get("CODEX_HOME") or Path.home() / ".codex")
    alvos = [
        ("claude", Path.home() / ".claude/settings.json", configurar_claude),
        ("antigravity", Path.home() / ".gemini/config/hooks.json", configurar_antigravity),
        ("codex", codex_home / "hooks.json", configurar_codex),
    ]
    try:
        for agente, caminho, configurar in alvos:
            if args.agente not in ("todos", agente):
                continue
            dados = carregar(caminho)
            configurar(dados, voz_sh)
            salvar(caminho, dados)
            print(f"Hooks do Voz registrados para {agente}: {caminho}")
    except (OSError, ValueError, TypeError, AttributeError) as erro:
        parser.exit(1, f"Não foi possível registrar os hooks: {erro}\n")
    if args.agente in ("todos", "codex"):
        print("No Codex, abra /hooks e revise/confie nos três hooks novos do Voz.")


if __name__ == "__main__":
    main()
