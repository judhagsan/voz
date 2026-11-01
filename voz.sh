#!/bin/bash
# Entrada dos hooks: repassa tudo para o binário em Rust. Sem ele (antes da
# compilação na instalação, por exemplo), responde o que os agentes esperam e
# avisa pela voz do sistema, para o hook não ficar mudo nem travar o agente.

BIN="$HOME/.claude/voz/target/release/voz"
if [ -x "$BIN" ]; then
  exec "$BIN" "$@"
fi

if [ "${1:-}" = "--codex" ]; then
  # Sem binário, preserva o protocolo; o resumo volta depois da compilação.
  echo '{}'
  exit 0
fi

if [ "$1" = "--agy" ]; then
  if [ "$2" = permissao ]; then
    echo '{"decision": "allow"}'
    exit 0
  fi
  echo '{}'
  shift
fi

case "$1" in
  terminei) say "Terminei." & ;;
  permissao) say "Preciso da sua permissão." & ;;
  espera) say "Tô te esperando." & ;;
esac
exit 0
