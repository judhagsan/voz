#!/usr/bin/env bash
set -euo pipefail

DESTINO="$HOME/.claude/voz"
BIN_DIR="$HOME/.local/bin"
REPO_PADRAO="https://github.com/judhagsan/voz.git"

echo "==> Instalando Voz em $DESTINO..."

if ! command -v brew >/dev/null 2>&1 && [ -x /opt/homebrew/bin/brew ]; then
  eval "$(/opt/homebrew/bin/brew shellenv)"
fi

if ! command -v ffmpeg >/dev/null 2>&1 && [ ! -x /opt/homebrew/bin/ffmpeg ]; then
  if command -v brew >/dev/null 2>&1; then
    echo "==> Instalando ffmpeg (necessário para manter o tom e a cadência da voz)..."
    brew install ffmpeg
  else
    echo "ERRO: ffmpeg não encontrado e Homebrew não está instalado."
    echo "Instale o Homebrew (https://brew.sh) e rode: brew install ffmpeg"
    exit 1
  fi
fi

if ! command -v gh >/dev/null 2>&1 && [ ! -x /opt/homebrew/bin/gh ]; then
  if command -v brew >/dev/null 2>&1; then
    echo "==> Instalando GitHub CLI (gh)..."
    brew install gh
  fi
fi

mkdir -p "$HOME/.claude" "$BIN_DIR"

if [ ! -d "$DESTINO/.git" ]; then
  REPO_URL="${VOZ_REPO_URL:-$REPO_PADRAO}"
  echo "==> Clonando $REPO_URL em $DESTINO..."
  git clone "$REPO_URL" "$DESTINO"
else
  echo "==> Repositório já presente em $DESTINO."
fi

if command -v gh >/dev/null 2>&1; then
  USUARIO_GH=$(gh api user -q .login 2>/dev/null || true)
  REMOTO_ATUAL=$(git -C "$DESTINO" remote get-url origin 2>/dev/null || true)
  if [ -n "$USUARIO_GH" ] && [[ "$REMOTO_ATUAL" == *"/$USUARIO_GH/voz"* ]]; then
    gh auth setup-git >/dev/null 2>&1 || true
    if ssh -o BatchMode=yes -o ConnectTimeout=3 -T git@github.com 2>&1 | grep -qi "$USUARIO_GH"; then
      git -C "$DESTINO" remote set-url origin "git@github.com:$USUARIO_GH/voz.git"
    fi
  fi
fi

chmod +x "$DESTINO/voz.sh" "$DESTINO/painel.py" "$DESTINO/instalar.sh"
ln -sf "$DESTINO/painel.py" "$BIN_DIR/voz"

if ! grep -qs '\.local/bin' "$HOME/.zshrc" 2>/dev/null; then
  printf '\nexport PATH="$HOME/.local/bin:$PATH"\n' >>"$HOME/.zshrc"
  echo "==> Adicionado ~/.local/bin ao PATH no ~/.zshrc."
fi

/usr/bin/python3 - "$DESTINO" <<'PY'
import json
import sys
from pathlib import Path

destino = Path(sys.argv[1])
voz_sh = str(destino / "voz.sh")

cfg1 = Path.home() / ".claude" / "settings.json"
dados1 = {}
if cfg1.exists():
    try:
        dados1 = json.loads(cfg1.read_text(encoding="utf-8"))
    except ValueError:
        dados1 = {}

hooks1 = dados1.setdefault("hooks", {})

def garantir_hook_lista(lista, cmd, matcher=None, async_flag=True, timeout=None):
    for item in lista:
        for h in item.get("hooks", []):
            if voz_sh in h.get("command", ""):
                h["command"] = cmd
                return
    bloco_h = {"type": "command", "command": cmd, "async": async_flag}
    if timeout:
        bloco_h["timeout"] = timeout
    novo = {"hooks": [bloco_h]}
    if matcher is not None:
        novo["matcher"] = matcher
    lista.append(novo)

garantir_hook_lista(hooks1.setdefault("Stop", []), f"{voz_sh} terminei", timeout=90)
garantir_hook_lista(
    hooks1.setdefault("Notification", []),
    f"{voz_sh} permissao",
    matcher="permission_prompt|elicitation_dialog",
)
garantir_hook_lista(
    hooks1.setdefault("Notification", []),
    f"{voz_sh} espera",
    matcher="idle_prompt",
)
cfg1.write_text(json.dumps(dados1, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")

cfg2 = Path.home() / ".gemini" / "config" / "hooks.json"
cfg2.parent.mkdir(parents=True, exist_ok=True)
dados2 = {}
if cfg2.exists():
    try:
        dados2 = json.loads(cfg2.read_text(encoding="utf-8"))
    except ValueError:
        dados2 = {}

bloco_voz = {
    "PreToolUse": [
        {
            "matcher": "run_command|ask_question",
            "hooks": [{"type": "command", "command": f"{voz_sh} --agy permissao"}],
        }
    ],
    "PostToolUse": [
        {
            "matcher": "",
            "hooks": [{"type": "command", "command": f"{voz_sh} --agy pos-tool"}],
        }
    ],
    "Stop": [{"type": "command", "command": f"{voz_sh} --agy terminei"}],
}

novo_dados2 = {"voz": bloco_voz}
for k, v in dados2.items():
    if k != "voz":
        novo_dados2[k] = v
cfg2.write_text(json.dumps(novo_dados2, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
PY

/usr/bin/python3 -I "$DESTINO/comum.py" catalogo

if ! security find-generic-password -s "elevenlabs-api-key" >/dev/null 2>&1; then
  echo ""
  echo "==> Chave da ElevenLabs ainda não cadastrada no Keychain."
  echo "    Para cadastrar, rode no terminal:"
  echo "    security add-generic-password -U -a \"\$USER\" -s \"elevenlabs-api-key\" -w \"SUA_CHAVE_AQUI\""
fi

echo ""
echo "==> Instalação concluída! Abra um novo terminal (ou rode 'source ~/.zshrc') e digite:"
echo "    voz"
