# Voz

Assistente sonoro e painel de controle para o terminal do macOS.

O **Voz** monitora a execução dos seus agentes de programação (Claude Code, Antigravity e Codex). Quando uma tarefa é concluída, quando uma permissão é necessária ou quando o agente aguarda sua resposta, o Voz avisa em voz alta e fala um resumo objetivo do que foi feito, liberando você de vigiar telas de terminal.

No terminal, o comando `voz` abre um painel ao vivo com o andamento dos seus repositórios Git e as cotas de uso dos agentes.

---

## Recursos principais

- **Avisos e resumos em voz alta:** fala quando o agente termina, pede permissão ou faz uma pergunta, com resumo falado de até 40 palavras.
- **Funciona offline ou na nuvem:** usa a voz neural local da Siri sem precisar de internet ou chaves de API, com suporte opcional a vozes em alta fidelidade via Gemini, ElevenLabs e OpenAI.
- **Painel no terminal:** tabela com branches ativas, arquivos alterados, commits pendentes, Pull Requests e cotas de uso do Claude Code, Antigravity e ChatGPT / Codex.
- **Fila inteligente:** se vários agentes terminarem juntos, as mensagens tocam em sequência sem se atropelar.
- **Dicionário de pronúncia:** acerta nomes, siglas e termos técnicos em português.

---

## Instalação

Abra o terminal e execute:

```zsh
curl -fsSL https://raw.githubusercontent.com/judhagsan/voz/main/instalar.sh | bash
```

O instalador cuida de tudo: dependências do Homebrew, compilação em Rust, configuração no shell e integração com os agentes.

Se você já clonou o repositório na sua máquina (por exemplo, em `~/voz`):

```zsh
cd ~/voz && ./instalar.sh
```

Depois de instalado, recarregue o terminal:

```zsh
source ~/.zshrc
```

---

## Como usar

### Painel no terminal

Exibir o painel de repositórios e cotas:

```zsh
voz
```

Exibir apenas repositórios com alterações ou trabalho em andamento:

```zsh
voz -a
```

Modo contínuo ao vivo (atualiza a cada segundo, ideal para rodar no tmux):

```zsh
voz -w
```

### Testar a fala

Falar qualquer frase:

```zsh
voz sintetizar "A voz está funcionando perfeitamente."
```

Conferir como o dicionário pronuncia um termo técnico ou nome próprio:

```zsh
voz fonetica "Abri o PR da API no GitHub"
```

---

## Provedores de Voz (Opcional)

A voz neural da Siri (`Voz 2` - Sandra) já vem pronta para uso 100% offline, sem nenhuma configuração.

Se desejar utilizar síntese em nuvem, adicione as chaves no Keychain do macOS:

### Gemini (Recomendado, gratuito)

Crie sua chave no Google AI Studio (aistudio.google.com) e salve:

```zsh
security add-generic-password -a "$USER" -s gemini-api-key -w
```

### ElevenLabs (Opcional)

```zsh
security add-generic-password -a "$USER" -s elevenlabs-api-key -w
```

### OpenAI (Opcional)

```zsh
security add-generic-password -a "$USER" -s openai-api-key -w
```

As chaves ficam salvas com segurança no cofre do sistema, sem nunca ficarem expostas em arquivos de texto.

---

## Ativação nos Agentes

- **Claude Code e Antigravity:** configurados automaticamente durante a instalação.
- **Codex:** ao abrir o Codex pela primeira vez, execute `/hooks` no chat e confirme os três ganchos do Voz para autorizar a execução.

---

## Personalização

Os arquivos de configuração ficam na raiz do repositório:

- `config.json`: ordem dos provedores de voz, estilo da fala e frases de aviso.
- `fonetica.json`: pronúncia de siglas e termos customizados.
- `tmux.conf`: tema escuro Dracula e atalhos de divisão de painéis.
