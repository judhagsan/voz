#!/bin/bash
# Toca uma frase sorteada da categoria pedida (terminei, permissao, espera).
# Uso nos hooks do Claude Code: voz.sh <categoria>. O JSON do hook chega pela
# entrada padrão e, em "terminei", segue para o gerador de frases novas.

BASE="$HOME/.claude/voz"

if [ "$1" = "--agy" ]; then
  shift
  categoria="$1"
  entrada=$(cat)
  if [ "$categoria" = permissao ]; then
    echo '{"decision": "allow"}'
  else
    echo '{}'
    case "$entrada" in
      *'"terminationReason":"USER_CANCELED"'*|*'"fullyIdle":false'*) exit 0 ;;
    esac
  fi
  nohup "$0" "$categoria" >/dev/null 2>&1 <<<"$entrada" &
  exit 0
fi

categoria="$1"

# O gerador roda um `claude -p` por baixo; dentro dele não se fala nada.
[ -n "$VOZ_GERANDO" ] && exit 0

if [ -t 0 ]; then
  entrada=""
else
  entrada=$(cat)
fi

case "$categoria" in
  terminei)  reserva="Terminei" ;;
  permissao) reserva="Preciso da sua permissão" ;;
  espera)    reserva="Tô te esperando" ;;
  *) exit 0 ;;
esac

if [ "$categoria" = terminei ] && [ -z "$VOZ_SIMULAR" ]; then
  printf '%s' "$entrada" |
    nohup /usr/bin/python3 -I "$BASE/gerar.py" automatico >>"$BASE/gerar.log" 2>&1 &
  # Em tarefa longa, a narração conta o resultado; se ela não falar, vale o sorteio.
  printf '%s' "$entrada" |
    /usr/bin/python3 -I "$BASE/narrar.py" >>"$BASE/gerar.log" 2>&1 && exit 0
fi

shopt -s nullglob
arquivos=("$BASE/sons/$categoria"/*.mp3)
shopt -u nullglob

escolhido=""
total=${#arquivos[@]}
if [ "$total" -gt 0 ]; then
  estado="$BASE/estado/ultimo-$categoria"
  ultimo=$(cat "$estado" 2>/dev/null)
  # Sorteia; se cair na última frase tocada, passa para a seguinte da lista.
  i=$((RANDOM % total))
  [ "${arquivos[i]}" = "$ultimo" ] && i=$(((i + 1) % total))
  escolhido="${arquivos[i]}"
  printf '%s' "$escolhido" >"$estado"
fi

if [ -n "$VOZ_SIMULAR" ]; then
  echo "${escolhido:-say: $reserva}"
elif [ -z "$escolhido" ] || ! afplay "$escolhido" 2>/dev/null; then
  say -v "Luciana (Enhanced)" "$reserva" 2>/dev/null || say "$reserva" 2>/dev/null
fi
exit 0
