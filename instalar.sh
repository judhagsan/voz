#!/usr/bin/env bash
set -euo pipefail

DESTINO="$HOME/.claude/voz"
BIN_DIR="$HOME/.local/bin"
REPO_PADRAO="https://github.com/judhagsan/voz.git"

echo "==> Instalando Voz em $DESTINO..."

if ! command -v brew >/dev/null 2>&1 && [ -x /opt/homebrew/bin/brew ]; then
  eval "$(/opt/homebrew/bin/brew shellenv)"
fi

if ! command -v brew >/dev/null 2>&1; then
  echo "ERRO: Homebrew não encontrado."
  echo "Instale o Homebrew (https://brew.sh) e rode o instalador novamente."
  exit 1
fi

echo "==> Verificando e atualizando dependências para as versões mais recentes (tmux, gh, uv, espeak-ng)..."
brew update --quiet || true

for pacote in tmux gh uv espeak-ng; do
  if ! brew list --formula "$pacote" >/dev/null 2>&1; then
    echo "==> Instalando $pacote (versão mais recente)..."
    brew install "$pacote"
  elif [ -n "$(brew outdated --formula "$pacote" 2>/dev/null)" ]; then
    echo "==> Atualizando $pacote para a versão mais recente..."
    brew upgrade "$pacote"
  else
    echo "==> $pacote já está na versão mais recente."
  fi
done

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

chmod +x "$DESTINO/voz.sh" "$DESTINO/painel.py" "$DESTINO/motor_local.py" "$DESTINO/instalar.sh"
ln -sf "$DESTINO/painel.py" "$BIN_DIR/voz"

if [ ! -d "$DESTINO/.venv" ]; then
  echo "==> Criando ambiente virtual Python 3.11 para o motor de voz local..."
  uv venv "$DESTINO/.venv" --python 3.11
fi

echo "==> Verificando dependências do motor de voz local (chatterbox-tts, soundfile)..."
"$DESTINO/.venv/bin/pip" install --quiet --upgrade chatterbox-tts soundfile torch

if [ -f "$DESTINO/tmux.conf" ]; then
  ln -sf "$DESTINO/tmux.conf" "$HOME/.tmux.conf"
  echo "==> Configuração do tmux vinculada em ~/.tmux.conf."
  if command -v tmux >/dev/null 2>&1 && tmux list-sessions >/dev/null 2>&1; then
    tmux source-file "$HOME/.tmux.conf" >/dev/null 2>&1 || true
  fi
fi

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

mkdir -p "$DESTINO/.git/hooks"
cat > "$DESTINO/.git/hooks/pre-commit" <<'HOOK'
#!/usr/bin/env bash
set -euo pipefail
REPO_DIR="$(git rev-parse --show-toplevel)"
/usr/bin/python3 -I "$REPO_DIR/comum.py" catalogo >/dev/null
git -C "$REPO_DIR" add -- sons estilo.json catalogo.json historico.tsv 2>/dev/null || true
HOOK
chmod +x "$DESTINO/.git/hooks/pre-commit"

echo ""
echo "==> Instalação concluída! Abra um novo terminal (ou rode 'source ~/.zshrc') e digite:"
echo "    voz"
