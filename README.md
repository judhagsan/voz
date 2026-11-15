# Voz

Assistente sonoro e painel de controle para o terminal (macOS, Linux e Windows).

O **Voz** monitora a execução dos seus agentes de programação no terminal. Quando uma tarefa é concluída, quando uma permissão é necessária ou quando o agente aguarda sua resposta, o Voz avisa em voz alta e fala um resumo objetivo do que foi feito, liberando você de vigiar telas de terminal.

No terminal, o comando `voz` abre um painel ao vivo com o andamento dos seus repositórios Git e as cotas de uso dos agentes.

---

## Recursos principais

- **Avisos e resumos em voz alta:** fala quando o agente termina, pede permissão ou faz uma pergunta, com resumo falado de até 40 palavras.
- **Voz neural de alta fidelidade sem chaves de API:** síntese integrada por padrão com a voz feminina **Thalita** (`pt-BR-ThalitaMultilingualNeural`), sem necessidade de cadastro, cartão ou credenciais.
- **Funciona offline ou na nuvem:** síntese padrão com a voz **Thalita** via Edge-TTS e reserva local nativa sem precisar de internet (`Luciana` no macOS, `speech-dispatcher` / `espeak-ng` no Linux e `System.Speech` no Windows).
- **Painel no terminal:** tabela com branches ativas, arquivos alterados, commits pendentes, Pull Requests e cotas de uso dos agentes.
- **Fila inteligente:** se vários agentes terminarem juntos, as mensagens tocam em sequência sem se atropelar.
- **Dicionário de pronúncia:** acerta nomes, siglas e termos técnicos em português.

---

## Instalação

### macOS e Linux

Abra o terminal e execute:

```bash
curl -fsSL https://raw.githubusercontent.com/judhagsan/voz/main/instalar.sh | bash
```

O instalador cuida de tudo: dependências do sistema (Homebrew no macOS; apt/dnf/pacman no Linux), dependência de voz (`edge-tts`), compilação em Rust, configuração no shell e integração com os agentes.

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

O instalador compila o executável com `cargo`, instala em `~/.local/bin` (com o executável `voz.exe` e o wrapper `voz.cmd`) e registra os ganchos dos agentes.

---

## Como usar

### Painel no terminal

Exibir o painel com os repositórios em andamento (alterações, commits pendentes, branch de trabalho ou PR aberto) e os serviços com cota ativa:

```zsh
voz
```

Exibir também os repositórios sem pendências no momento:

```zsh
voz -t
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

## Provedores de Voz

### 1. Voz Padrão (Sem chaves de API)

O Voz já vem configurado para utilizar a rede neural da **Thalita** (`pt-BR-ThalitaMultilingualNeural`) como voz padrão:
- Não exige cadastro, cartão de crédito nem configuração de credenciais.
- Requer apenas o pacote `edge-tts` instalado no Python (`pip install edge-tts`).
- A reprodução é protegida por trava de processo no sistema operacional, garantindo som cristalino sem cortes.

### 2. Voz de Reserva (100% Offline)

Caso o computador esteja sem conexão com a internet, o Voz recai automaticamente para o motor offline nativo do sistema operacional, sem interrupções:
- **macOS:** `Luciana` (ou qualquer voz ativada em *Ajustes do Sistema > Acessibilidade > Conteúdo Falado*, como vozes da Siri ou vozes aprimoradas)
- **Linux:** `speech-dispatcher` (`spd-say`) ou `espeak-ng` / `espeak`
- **Windows:** síntese nativa do sistema (`System.Speech` via PowerShell)

---

## Ativação nos Agentes

- **Ganchos de monitoramento:** configurados automaticamente durante a execução do instalador.
- **Codex:** ao abrir o chat, execute `/hooks` e confirme os ganchos do Voz para autorizar a execução.

---

## Personalização

Os arquivos de configuração ficam na raiz do repositório:

- `config.json`: ordem dos provedores de voz, voz selecionada, velocidade e frases de aviso.
- `fonetica.json`: pronúncia de siglas e termos customizados.
- `tmux.conf`: tema escuro Dracula e atalhos de divisão de painéis.
