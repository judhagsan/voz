#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BIN_DIR="$HOME/.local/bin"
REPO_PADRAO="https://github.com/judhagsan/voz.git"

if [ -d "$SCRIPT_DIR/.git" ] && [ -f "$SCRIPT_DIR/Cargo.toml" ]; then
  DESTINO="$SCRIPT_DIR"
elif [ -n "${VOZ_DIR:-}" ]; then
  DESTINO="$VOZ_DIR"
elif [ -t 0 ] || [ -e /dev/tty ]; then
  echo "==> Onde deseja instalar o repositório do Voz?"
  echo "    1) $HOME/voz (junto dos seus repositórios) [padrão]"
  echo "    2) $HOME/.config/voz (configurações do sistema)"
  echo "    3) Outro caminho customizado"
  printf "Escolha [1/2/3] (Enter para 1): "
  if [ -t 0 ]; then
    read -r opcao || opcao="1"
  else
    read -r opcao </dev/tty || opcao="1"
  fi
  case "${opcao:-1}" in
    1) DESTINO="$HOME/voz" ;;
    2) DESTINO="$HOME/.config/voz" ;;
    3)
      printf "Digite o caminho completo: "
      if [ -t 0 ]; then
        read -r custom || custom="$HOME/voz"
      else
        read -r custom </dev/tty || custom="$HOME/voz"
      fi
      DESTINO="${custom/#\~/$HOME}"
      ;;
    *) DESTINO="$HOME/voz" ;;
  esac
else
  DESTINO="$HOME/voz"
fi

echo "==> Instalando Voz em $DESTINO..."

SO="$(uname -s)"
if [ "$SO" = "Darwin" ]; then
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
elif [ "$SO" = "Linux" ]; then
  echo "==> Verificando dependências no Linux..."
  if command -v apt-get >/dev/null 2>&1; then
    sudo apt-get update -qq || true
    sudo apt-get install -y -qq tmux gh libasound2-dev build-essential speech-dispatcher || true
  elif command -v dnf >/dev/null 2>&1; then
    sudo dnf install -y tmux gh alsa-lib-devel gcc speech-dispatcher || true
  elif command -v pacman >/dev/null 2>&1; then
    sudo pacman -Sy --noconfirm tmux github-cli alsa-lib base-devel speech-dispatcher || true
  fi

  if ! command -v cargo >/dev/null 2>&1; then
    echo "==> Rust não encontrado. Instalando via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
    source "$HOME/.cargo/env" 2>/dev/null || true
  fi
fi

mkdir -p "$BIN_DIR"

if [ ! -d "$DESTINO/.git" ]; then
  REPO_URL="${VOZ_REPO_URL:-$REPO_PADRAO}"
  echo "==> Clonando $REPO_URL em $DESTINO..."
  mkdir -p "$(dirname "$DESTINO")"
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

/usr/bin/python3 "$DESTINO/scripts/instalar_hooks.py" --destino "$DESTINO"

echo ""
echo "==> Configuração de Chaves de API (Keychain do macOS):"
if security find-generic-password -s gemini-api-key >/dev/null 2>&1; then
  echo "    [✓] Gemini: chave principal configurada."
else
  echo "    [ ] Gemini (gratuito): https://aistudio.google.com/apikey"
  echo "        security add-generic-password -a \"$USER\" -s gemini-api-key -w \"SUA_CHAVE\""
  echo "        Dica: adicione gemini-api-key-2 para multiplicar a cota diária!"
fi

if security find-generic-password -s elevenlabs-api-key >/dev/null 2>&1; then
  echo "    [✓] ElevenLabs: chave configurada."
else
  echo "    [-] ElevenLabs (opcional): https://elevenlabs.io"
  echo "        security add-generic-password -a \"$USER\" -s elevenlabs-api-key -w \"SUA_CHAVE\""
fi

if security find-generic-password -s openai-api-key >/dev/null 2>&1; then
  echo "    [✓] OpenAI: chave configurada."
else
  echo "    [-] OpenAI (opcional): https://platform.openai.com/api-keys"
  echo "        security add-generic-password -a \"$USER\" -s openai-api-key -w \"SUA_CHAVE\""
fi

echo "    [✓] Siri Natural (macOS 27 - Voz 2): pronta para uso offline sem chaves."

echo ""
echo "==> Instalação concluída! Abra um novo terminal (ou rode 'source ~/.zshrc') e teste:"
echo "    voz                                  # painel de repositórios e cotas"
echo "    voz sintetizar \"Olá, tudo certo.\"     # voz"
