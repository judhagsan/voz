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

echo "==> Verificando e atualizando dependências para as versões mais recentes (tmux, gh)..."
brew update --quiet || true

for pacote in tmux gh; do
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

# O cargo precisa do linker das Command Line Tools da Apple.
if ! xcode-select -p >/dev/null 2>&1; then
  echo "ERRO: faltam as Command Line Tools da Apple."
  echo "Rode 'xcode-select --install' e depois o instalador de novo."
  exit 1
fi

# O binário é compilado com o cargo; sem Rust instalado, vem pelo Homebrew.
if ! command -v cargo >/dev/null 2>&1; then
  echo "==> Instalando Rust (cargo)..."
  brew install rust
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

# Versões antigas usavam um motor de voz local em Python e um banco de áudios.
# O que sobrou delas sai aqui: o processo do motor, o ambiente Python e o gancho
# de pre-commit que chamava o comum.py (sem ele, todo commit falharia).
pkill -f "$DESTINO/motor_local.py" 2>/dev/null || true
if [ -d "$DESTINO/.venv" ]; then
  echo "==> Removendo o ambiente Python da versão antiga..."
  rm -rf "$DESTINO/.venv"
fi
if grep -qs 'comum.py' "$DESTINO/.git/hooks/pre-commit"; then
  echo "==> Removendo o gancho de pre-commit da versão antiga..."
  rm -f "$DESTINO/.git/hooks/pre-commit"
fi

chmod +x "$DESTINO/voz.sh" "$DESTINO/instalar.sh"

echo "==> Compilando o binário voz (painel, hooks e voz)..."
(cd "$DESTINO" && cargo build --release --quiet)
ln -sf "$DESTINO/target/release/voz" "$BIN_DIR/voz"
echo "==> Binário vinculado em ~/.local/bin/voz."

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
cfg1.parent.mkdir(parents=True, exist_ok=True)
dados1 = {}
if cfg1.exists():
    try:
        dados1 = json.loads(cfg1.read_text(encoding="utf-8"))
    except ValueError:
        dados1 = {}

hooks1 = dados1.setdefault("hooks", {})

def garantir_hook_lista(lista, cmd, matcher=None, async_flag=True, timeout=None):
    # Identifica pelo comando exato: permissao e espera ficam na mesma lista e
    # um não pode sobrescrever o outro. Hook que já existe recebe os valores
    # atuais de matcher, async e timeout.
    for item in lista:
        for h in item.get("hooks", []):
            if h.get("command") == cmd:
                h["async"] = async_flag
                if timeout:
                    h["timeout"] = timeout
                if matcher is not None:
                    item["matcher"] = matcher
                return
    bloco_h = {"type": "command", "command": cmd, "async": async_flag}
    if timeout:
        bloco_h["timeout"] = timeout
    novo = {"hooks": [bloco_h]}
    if matcher is not None:
        novo["matcher"] = matcher
    lista.append(novo)

garantir_hook_lista(hooks1.setdefault("Stop", []), f"{voz_sh} terminei", timeout=600)
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

if ! security find-generic-password -s gemini-api-key >/dev/null 2>&1; then
  echo ""
  echo "==> Falta a chave da voz (Gemini). Crie em https://aistudio.google.com/apikey e salve:"
  echo "    security add-generic-password -a \"\$USER\" -s gemini-api-key -w"
  echo "    Sem a chave, os avisos saem pela voz do sistema (say)."
fi

echo ""
echo "==> Instalação concluída! Abra um novo terminal (ou rode 'source ~/.zshrc') e teste:"
echo "    voz                                  # painel de repositórios e cotas"
echo "    voz sintetizar \"Olá, tudo certo.\"     # voz"
