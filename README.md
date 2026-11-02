# Voz

Assistente sonoro e painel de controle para o terminal (macOS, Linux e Windows).

O **Voz** monitora a execução dos seus agentes de programação (Claude Code, Antigravity e Codex). Quando uma tarefa é concluída, quando uma permissão é necessária ou quando o agente aguarda sua resposta, o Voz avisa em voz alta e fala um resumo objetivo do que foi feito, liberando você de vigiar telas de terminal.

No terminal, o comando `voz` abre um painel ao vivo com o andamento dos seus repositórios Git e as cotas de uso dos agentes.

---

## Recursos principais

- **Avisos e resumos em voz alta:** fala quando o agente termina, pede permissão ou faz uma pergunta, com resumo falado de até 40 palavras.
- **Funciona offline ou na nuvem:** síntese local nativa sem precisar de internet ou chaves de API (Siri no macOS, `speech-dispatcher` / `espeak-ng` no Linux e `System.Speech` no Windows), com suporte opcional a vozes em alta fidelidade via Gemini, ElevenLabs e OpenAI.
- **Painel no terminal:** tabela com branches ativas, arquivos alterados, commits pendentes, Pull Requests e cotas de uso do Claude Code, Antigravity e ChatGPT / Codex.
- **Fila inteligente:** se vários agentes terminarem juntos, as mensagens tocam em sequência sem se atropelar.
- **Dicionário de pronúncia:** acerta nomes, siglas e termos técnicos em português.

---

## Instalação

### macOS e Linux

Abra o terminal e execute:

```bash
curl -fsSL https://raw.githubusercontent.com/judhagsan/voz/main/instalar.sh | bash
```

O instalador cuida de tudo: dependências do sistema (Homebrew no macOS; apt/dnf/pacman no Linux), compilação em Rust, configuração no shell e integração com os agentes.

Se você já clonou o repositório na sua máquina (por exemplo, em `~/voz`):

```bash
cd ~/voz && ./instalar.sh
```

Depois de instalado, recarregue o terminal:

```bash
source ~/.zshrc
```
Caso utilize bash, substitua por `source ~/.bashrc`.

### Windows

Abra o PowerShell e execute:

```powershell
irm https://raw.githubusercontent.com/judhagsan/voz/main/instalar.ps1 | iex
```

Se você já clonou o repositório na sua máquina (por exemplo, em `~/voz`):

```powershell
cd $HOME\voz
.\instalar.ps1
```

O instalador compila o executável com `cargo`, instala em `~/.local/bin` (com o executável `voz.exe` e o wrapper `voz.cmd`) e registra os hooks dos agentes.

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

A síntese local do sistema operacional já vem pronta para uso 100% offline, sem nenhuma configuração:
- **macOS:** Siri (`Voz 2` - Sandra)
- **Linux:** `speech-dispatcher` (`spd-say`) ou `espeak-ng` / `espeak`
- **Windows:** síntese nativa do sistema (`System.Speech` via PowerShell)

Se desejar utilizar síntese neural em nuvem (Gemini, ElevenLabs ou OpenAI), configure as chaves de API via variáveis de ambiente (suportadas em todos os sistemas) ou pelos cofres e arquivos de credenciais nativos.

### 1. Variáveis de Ambiente (Recomendado / Multiplataforma)

As variáveis de ambiente são lidas diretamente no macOS, Linux e Windows.

| Provedor | Variável Principal | Rotação Automática (Cotas Extras) |
|---|---|---|
| **Gemini** (Recomendado, gratuito) | `GEMINI_API_KEY` | `GEMINI_API_KEY_2`, `GEMINI_API_KEY_3`, ... |
| **ElevenLabs** (Opcional) | `ELEVENLABS_API_KEY` | `ELEVENLABS_API_KEY_2`, ... |
| **OpenAI** (Opcional) | `OPENAI_API_KEY` | `OPENAI_API_KEY_2`, ... |

#### macOS e Linux

Obtenha a chave gratuita no Google AI Studio (aistudio.google.com) e adicione ao seu arquivo de configuração do shell (`~/.zshrc` ou `~/.bashrc`):

```bash
export GEMINI_API_KEY="sua_chave_aqui"
```

Para multiplicar sua cota diária gratuita com rotação automática entre múltiplas chaves:

```bash
export GEMINI_API_KEY="primeira_chave"
export GEMINI_API_KEY_2="segunda_chave"
```

Para ElevenLabs ou OpenAI:

```bash
export ELEVENLABS_API_KEY="sua_chave_aqui"
export OPENAI_API_KEY="sua_chave_aqui"
```

#### Windows

No PowerShell, defina a chave permanentemente para o seu usuário:

```powershell
[System.Environment]::SetEnvironmentVariable("GEMINI_API_KEY", "sua_chave_aqui", "User")
```

Para adicionar chaves adicionais para rotação de cota no PowerShell:

```powershell
[System.Environment]::SetEnvironmentVariable("GEMINI_API_KEY_2", "segunda_chave", "User")
```

No Prompt de Comando (CMD clássico):

```cmd
setx GEMINI_API_KEY "sua_chave_aqui"
```

---

### 2. Cofre do Sistema e Arquivos de Credenciais

Se preferir armazenar suas chaves nos cofres de credenciais do sistema:

#### macOS (Keychain)

Adicione a chave ao cofre seguro do sistema:

```bash
security add-generic-password -a "$USER" -s gemini-api-key -w
```

Para rotação de cotas no Keychain, cadastre também `gemini-api-key-2`, etc.

Para outros provedores:

```bash
security add-generic-password -a "$USER" -s elevenlabs-api-key -w
security add-generic-password -a "$USER" -s openai-api-key -w
```

#### Linux (Secret Service ou Arquivo)

Via GNOME Keyring / KWallet (`secret-tool`):

```bash
secret-tool store --label="Gemini API Key" service gemini-api-key
```

Ou em arquivo local `~/.config/voz/credentials` (permissão `chmod 600`):

```text
gemini-api-key=sua_chave_aqui
gemini-api-key-2=segunda_chave
elevenlabs-api-key=sua_chave_aqui
openai-api-key=sua_chave_aqui
```

#### Windows (Arquivo de Credenciais)

No arquivo `%APPDATA%\voz\credentials` ou `%USERPROFILE%\.config\voz\credentials`:

```text
gemini-api-key=sua_chave_aqui
gemini-api-key-2=segunda_chave
elevenlabs-api-key=sua_chave_aqui
openai-api-key=sua_chave_aqui
```

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
