# Voz

Sistema de avisos falados, narração contextual por projeto, painel visual de repositórios no terminal e aprendizado contínuo de vocabulário no macOS, usando o motor de voz neural local **Chatterbox** com clonagem do timbre da **Laura** e ajuste de tom/cadência via **ffmpeg**.

100% local, offline, sem envio de dados para serviços externos e sem custos de API ou limites de créditos.

---

## O que o projeto faz

1. **Painel visual de repositórios no terminal (`voz` / `painel.py`):**
   - Mostra em uma tabela rápida todos os repositórios locais e o estado remoto no GitHub:
     - **Andamento local:** branch ativa e quantidade de arquivos modificados;
     - **Sem push:** quantos commits locais ainda não foram enviados ao remoto (`↑`) ou faltam puxar (`↓`);
     - **PR aberto:** número dos Pull Requests abertos no repositório (`PR #N`).
   - No modo contínuo (`voz -w`), atualiza a tabela automaticamente na tela e fala um resumo inicial das pendências uma única vez ao abrir.

2. **Narração contextual e fila sem sobreposição (`narrar.py` + `comum.py`):**
   - Ao concluir uma tarefa, identifica o repositório/projeto e resume em voz alta o que foi feito, proposto ou explicado.
   - Possui **fila global com trava (`flock`)**: quando vários agentes rodam em paralelo em projetos diferentes, as vozes nunca se atropelam — ou tocam em fila ordenada, ou juntam os resultados em uma única fala identificando cada repositório.

3. **Catálogo verificado e síntese local rápida (`catalogo.json` + `motor_local.py`):**
   - Cruza todos os arquivos `.mp3` existentes em `sons/` e `narracoes/` com seus textos e palavras-chave.
   - Antes de sintetizar qualquer áudio novo, verifica se já existe uma gravação compatível no catálogo para reutilizá-la de imediato.
   - Quando precisa sintetizar, o motor residente local (`motor_local.py`) mantém os pesos e as condicionais pré-calculadas da Laura em memória, gerando o áudio em poucos segundos via CPU multicore.

4. **Avisos rápidos e aprendizado de estilo (`voz.sh` + `gerar.py` + `estilo.json`):**
   - Avisos para `terminei`, `permissao` (apenas quando um comando realmente aguarda aprovação manual) e `espera`.
   - Aprende expressões e estilo de comunicação ao longo do uso e sincroniza automaticamente o banco de áudios com o repositório.

---

## Pré-requisitos

O instalador verifica e atualiza automaticamente para as versões mais recentes via Homebrew:

- **macOS** (usa `afplay` nativo do sistema)
- **ffmpeg** (ajuste de tom `0.90` e cadência `0.91` da voz)
- **tmux** (multiplexador de terminal configurado com tema Dracula, cópia automática por seleção de mouse e etiqueta `✦ pasta` no canto direito de cada painel)
- **GitHub CLI (`gh`)** (para consultar Pull Requests abertos e sincronizar seu fork)
- **uv** e **espeak-ng** (gerenciamento do ambiente Python 3.11 e fonemizador para o Chatterbox)

---

## 1. Instalação via Terminal

Para instalar direto no terminal:

```zsh
curl -fsSL https://raw.githubusercontent.com/judhagsan/voz/main/instalar.sh | bash
```

Ou, se já tiver clonado o repositório manualmente:

```zsh
chmod +x ~/.claude/voz/instalar.sh && ~/.claude/voz/instalar.sh
```

Depois de instalar, recarregue o shell:

```zsh
source ~/.zshrc
```

---

## 2. Como conectar o GitHub

### Caso 1: Instalando na sua própria máquina (dono do repositório)

Se você é o dono do repositório, não precisa criar fork nem branch separada. Basta autenticar o GitHub CLI na máquina:

```zsh
gh auth login
```

Ao rodar o `instalar.sh`, ele reconhece automaticamente que a conta logada no `gh` é a dona do repositório e já configura o `origin` para fazer os commits e pushes diários direto na sua `main`.

### Caso 2: Outros usuários (Fork ou Branch própria)

O repositório principal aceita `push` apenas do dono original, evitando mistura de vozes e perfis na `main`.

Para que o seu ambiente faça o salvamento e `push` diário automático dos seus próprios áudios (`sons/`, `narracoes/`, `catalogo.json` e `estilo.json`), autentique o `gh` e crie seu fork:

```zsh
gh auth login
```

Em seguida, crie o fork na sua conta do GitHub apontando o `origin` local para ele:

```zsh
cd ~/.claude/voz && gh repo fork judhagsan/voz --remote=true --remote-name=origin
```

Se quiser zerar as narrações antigas para começar o seu acervo do zero no seu fork:

```zsh
rm -rf ~/.claude/voz/narracoes/* && : > ~/.claude/voz/narracoes.tsv && /usr/bin/python3 -I ~/.claude/voz/comum.py catalogo
```

Se preferir usar uma branch específica dentro do seu fork (em vez da `main`):

```zsh
cd ~/.claude/voz && git checkout -b meu-perfil && git push -u origin meu-perfil
```

O script detecta automaticamente a branch ativa (`HEAD`) e envia todas as atualizações automáticas do banco de vozes para `origin/<sua-branch>`.

---

## 3. Como usar no terminal

Mostrar o painel com todos os repositórios (destacando os que estão em andamento no topo):

```zsh
voz
```

Mostrar **apenas** os repositórios que têm trabalho em andamento (branch ativa, arquivos modificados, commits sem push ou PR aberto):

```zsh
voz -a
```

Abrir o painel em **modo contínuo** (atualiza sozinho a cada 5 segundos e fala o resumo inicial em voz alta ao abrir):

```zsh
voz -w
```

Definir um intervalo personalizado em segundos no modo contínuo (exemplo: 10 segundos):

```zsh
voz -w 10
```

---

## Estrutura de arquivos

| Arquivo / Pasta | Função |
| :--- | :--- |
| `motor_local.py` | Motor de síntese neural local Chatterbox com timbre clonado da Laura, fila assíncrona IPC e trava `flock`. |
| `painel.py` | Painel visual de repositórios no terminal (executado pelo comando `voz`). |
| `voz.sh` | Entrada dos hooks de eventos (`terminei`, `permissao`, `espera` e `pos-tool`). |
| `comum.py` | Ajuste de tom/velocidade via `ffmpeg`, fila `flock`, catálogo verificado (`catalogo.json`) e sincronização Git. |
| `narrar.py` | Narração contextual com fila de múltiplos agentes e reaproveitamento de áudios existentes. |
| `gerar.py` | Geração periódica de novas frases curtas e evolução do perfil em `estilo.json`. |
| `instalar.sh` | Instalador automático para macOS via terminal (gerencia `ffmpeg`, `tmux`, `gh`, `uv`, `espeak-ng` e `.venv`). |
| `tmux.conf` | Configuração versionada do `tmux` (vinculada em `~/.tmux.conf`). |
| `config.json` | Configurações da voz (`Laura`, `tom_voz: 0.90`, `velocidade_voz: 0.91`, motor local). |
| `catalogo.json` | Relação verificada entre cada arquivo `.mp3` existente no disco, seu texto e suas palavras-chave. |
| `sons/` | Áudios curtos divididos em `terminei/`, `permissao/` e `espera/`. |
| `narracoes/` | Áudios de narração contextual organizados por repositório/projeto. |
