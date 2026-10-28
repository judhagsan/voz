# Aracuan

Sistema de avisos de voz, narração de tarefas longas e aprendizado contínuo de vocabulário para agentes de código (**Claude Code** e **Antigravity**) no macOS, usando síntese neural da **ElevenLabs**.

---

## Como funciona

O **Aracuan** atua por eventos nos hooks dos agentes (zero consumo em segundo plano enquanto você programa) e opera em 3 frentes:

1. **Sorteio inteligente de avisos (`voz.sh`):**
   - **`terminei`**: quando o agente conclui uma tarefa.
   - **`permissao`**: quando o agente pede aprovação para rodar uma ferramenta ou comando.
   - **`espera`**: quando o agente aguarda uma resposta ou decisão sua.
   - Evita repetir a última frase tocada e possui fallback automático: `.mp3` locais $\rightarrow$ voz offline do macOS (`say`).

2. **Narração de tarefas longas (`narrar.py`):**
   - Quando uma tarefa passa do tempo mínimo configurado (`45s`), lê o pedido e o resultado final da sessão, gera uma fala curta e bem-humorada contando o que realmente ficou pronto e sintetiza o áudio na hora (guardando tudo catalogado em `narracoes/` e `narracoes.tsv`).
   - Se a API estiver sem cota, offline ou em pausa, cai imediatamente para os áudios gravados em `sons/`.

3. **Enriquecimento contínuo de vocabulário e estilo (`gerar.py` + `estilo.json`):**
   - Aprende o jeito de falar, humor, gírias e expressões do usuário a partir das conversas nos agentes, acumulando o perfil em `estilo.json`.
   - Gera periodicamente novas frases curtas com tags de entonação (`[laughs]`, `[sighs]`, `[excited]`, etc.), sintetiza na ElevenLabs e adiciona ao banco permanente (`sons/` e `historico.tsv`).

---

## Estrutura do projeto

| Arquivo / Pasta | Descrição |
|---|---|
| `voz.sh` | Script principal chamado pelos hooks do Claude Code e do Antigravity (`--agy`). |
| `comum.py` | Funções compartilhadas: cliente ElevenLabs, acesso ao Keychain, leitura de transcripts e perfil de estilo. |
| `gerar.py` | Gerador de novas frases de sorteio e atualizador do perfil de estilo (`estilo.json`). |
| `narrar.py` | Narrador contextual para tarefas longas. |
| `config.json` | Configurações de voz, modelo (`eleven_v4`), intervalos e limites de orçamento. |
| `estilo.json` | Perfil persistente de tom e expressões aprendidas do usuário. |
| `historico.tsv` | Base catalogada de todas as frases geradas por categoria. |
| `narracoes.tsv` | Índice histórico das narrações de tarefas longas por projeto. |
| `sons/` | Banco de áudios `.mp3` separados em `terminei/`, `permissao/` e `espera/`. |

---

## Configuração da chave da ElevenLabs (Keychain do macOS)

A chave da API nunca fica salva em texto puro no repositório. Ela é lida diretamente do Keychain do macOS:

```bash
security add-generic-password -U -a "$USER" -s "elevenlabs-api-key" -w "SUA_CHAVE_AQUI"
```

Para validar a voz, o modelo e o orçamento da conta:

```bash
python3 ~/.claude/voz/gerar.py configurar
```

---

## Integração nos agentes

### Claude Code (`~/.claude/settings.json`)

```json
{
  "hooks": {
    "Stop": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "/Users/seu-usuario/.claude/voz/voz.sh terminei",
            "async": true,
            "timeout": 90
          }
        ]
      }
    ],
    "Notification": [
      {
        "matcher": "permission_prompt|elicitation_dialog",
        "hooks": [
          {
            "type": "command",
            "command": "/Users/seu-usuario/.claude/voz/voz.sh permissao",
            "async": true
          }
        ]
      },
      {
        "matcher": "idle_prompt",
        "hooks": [
          {
            "type": "command",
            "command": "/Users/seu-usuario/.claude/voz/voz.sh espera",
            "async": true
          }
        ]
      }
    ]
  }
}
```

### Antigravity (`~/.gemini/config/hooks.json`)

```json
{
  "voz": {
    "PreToolUse": [
      {
        "matcher": "ask_question",
        "hooks": [
          {
            "type": "command",
            "command": "/Users/seu-usuario/.claude/voz/voz.sh --agy permissao"
          }
        ]
      }
    ],
    "Stop": [
      {
        "type": "command",
        "command": "/Users/seu-usuario/.claude/voz/voz.sh --agy terminei"
      }
    ]
  }
}
```

---

## Comandos úteis

```bash
# Testar os avisos manualmente
~/.claude/voz/voz.sh terminei
~/.claude/voz/voz.sh permissao
~/.claude/voz/voz.sh espera

# Simular novas frases a partir de um transcript sem gastar créditos da ElevenLabs
python3 ~/.claude/voz/gerar.py simular /caminho/do/transcript.jsonl

# Simular a narração do último turno de um transcript
python3 ~/.claude/voz/narrar.py simular /caminho/do/transcript.jsonl
```
