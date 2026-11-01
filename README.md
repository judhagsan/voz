# Voz

Assistente sonoro de ambiente e monitor de produtividade para o terminal do macOS. O Voz acompanha a execução dos seus agentes de programação (Claude Code, Antigravity e Codex): quando uma tarefa é concluída, quando uma permissão é necessária ou quando o agente aguarda sua resposta, o Voz avisa em voz alta e sintetiza um resumo falado de até 40 palavras com o que foi feito. No terminal, o comando `voz` oferece um painel ao vivo para `tmux` com o status de todos os seus repositórios Git e as janelas de cota de cada agente.

A voz é gerada em streaming PCM de 24 kHz direto para o CoreAudio, com latência imperceptível, cascata automática entre múltiplos provedores (Gemini, ElevenLabs e OpenAI) e rotação sequencial de chaves pelo Keychain, mantendo consistência de timbre e voz feminina em todos os níveis. Nenhum arquivo de áudio é salvo em disco ou Git. Em caso de perda de conexão, chave ausente ou cota de nuvem esgotada, a reprodução recorre imediatamente à voz neural local do macOS (Siri Natural `Voz 2` - Sandra via Apple Neural Engine), com fallback para o `say` do sistema.

---

## Como funciona

Os agentes chamam o `voz.sh` nos eventos de fim de tarefa, pedido de permissão e espera. Ele repassa tudo para o binário `voz`, escrito em Rust:

```
evento do agente ─▶ voz.sh ─▶ voz
                               ├─ terminei ──────────▶ resumo da resposta ─┐
                               ├─ permissao, espera ─▶ frase do config ────┤
                               │                                           ▼
                               │   cascata de provedores (streaming 24 kHz) ─▶ alto-falante
                               │   ├─ Gemini (chaves sequenciais + modelos em cascata)
                               │   ├─ ElevenLabs (chaves sequenciais + streaming PCM)
                               │   ├─ OpenAI (chaves sequenciais + streaming PCM)
                               │   ▼
                               └─ sem chave ou offline ─▶ Siri Natural (Voz 2) / say do macOS
```

1. **Fim de tarefa (`terminei`):** o `voz` intercepta o evento de término do Claude, Antigravity ou Codex, localiza o turno correspondente no histórico e extrai a resposta final do assistente, descartando raciocínio interno, chamadas de ferramentas e comentários intermediários. Um modelo de texto (`gemini-3.5-flash-lite`) gera um resumo falado de até 40 palavras contendo o projeto, o resultado objetivo e eventuais decisões pendentes, em cerca de 1 segundo.
2. **Permissão e espera:** avisa com frases curtas sorteadas do `config.json` (sem repetições imediatas). No Antigravity e no Codex, o sistema avalia o contexto do comando: solicitações de permissão só soam quando há espera real por aprovação manual, e chamadas de perguntas ao usuário (`request_user_input` no Codex ou `ask_question` no Antigravity) acionam imediatamente a fala de espera.
3. **Pronúncia:** antes da voz, o texto passa pelo dicionário `fonetica.json`, para nomes que nenhuma voz acerta sozinha (como `judhagsan`).
4. **Voz e Provedores em Cascata:** o `voz` percorre a lista de provedores configurada (`provedores`). Para cada provedor, percorre todas as chaves cadastradas no Keychain (`servico`, `servico-2`, `servico-3`...). Se um provedor ou chave sofrer erro ou esgotar a cota (HTTP 429), ele passa instantaneamente para a próxima chave ou próximo provedor. Todos os provedores de nuvem devolvem PCM linear de 16-bit a 24 kHz sem arquivos intermediários, tocando direto no CoreAudio assim que o primeiro pedaço chega.
5. **Fila:** quando vários agentes terminam ao mesmo tempo, as falas tocam uma depois da outra, sem se atropelar.
6. **Reserva Local (Siri Natural AI):** se as chaves faltarem, a internet cair ou as cotas de nuvem acabarem, a fala sai pela voz neural local feminina do macOS (`Voz 2` - Sandra), com fallback para o `say` padrão, com o dicionário completo de pronúncia. Sem o resumo por IA, a narração usa o começo da resposta, cortado no mesmo tamanho.

---

## Painel no terminal

```zsh
voz
voz -a
voz -w
voz -w 10
```

- `voz`: todos os repositórios, com os que estão em andamento no topo.
- `voz -a`: só os repositórios que estão em andamento.
- `voz -w`: modo contínuo, atualizando a cada 1 segundo.
- `voz -w 10`: modo contínuo com intervalo customizado em segundos.
- **Tabela de repositórios:** para cada repositório na sua pasta pessoal, mostra a branch ativa e os arquivos modificados, quantos commits faltam enviar (`↑`) ou puxar (`↓`) e os Pull Requests abertos, com quantos commits cada um tem.
- **Cotas dos agentes:** ao lado da tabela, exibe quanto foi consumido de cada janela de uso e o horário exato de renovação para Claude Code, Antigravity e a conta conectada ao Codex (**ChatGPT / Codex**). As leituras são obtidas diretamente dos serviços locais a cada 2 minutos, sem consumir cotas de inferência.
- **Monitoramento do Codex / ChatGPT:** consulta [`account/rateLimits/read`](https://learn.chatgpt.com/docs/app-server#6-rate-limits-chatgpt) pela CLI autenticada, abrangendo limites primários e secundários (como cotas semanais e de janela curta). Se a CLI estiver indisponível ou offline, o Voz busca automaticamente a última cota registrada nas sessões em `$CODEX_HOME/sessions` (por padrão `~/.codex/sessions`) e indica **Histórico de dia/mês hora:minuto**, funcionando em qualquer diretório de trabalho com ou sem `ai-memory`.
- **Modo contínuo (`-w`):** fala um resumo sonoro das pendências ao inicializar e adapta o layout dinamicamente conforme as dimensões do terminal ou painel do `tmux`. Caso a largura seja insuficiente para exibição lado a lado, os blocos de cota são reorganizados abaixo da tabela de repositórios.

---

## Pré-requisitos

O instalador verifica e instala, via Homebrew:

- **macOS** (usa a saída de som do sistema, `afplay` e `say`)
- **tmux** (configuração versionada em `tmux.conf`, descrita abaixo)
- **GitHub CLI (`gh`)** (para consultar Pull Requests e configurar seu fork)
- **Rust (`cargo`)** (para compilar o binário `voz`)
- **Command Line Tools da Apple** (o `cargo` precisa delas; se faltarem, o instalador avisa para rodar `xcode-select --install`)

Chaves de API são opcionais: sem nenhuma configurada, a voz neural local do macOS (Siri Natural) funciona offline.

---

## Instalação

```zsh
curl -fsSL https://raw.githubusercontent.com/judhagsan/voz/main/instalar.sh | bash
```

Ou, com o repositório já clonado:

```zsh
chmod +x ~/.claude/voz/instalar.sh && ~/.claude/voz/instalar.sh
```

O instalador compila o `voz`, cria o atalho em `~/.local/bin/voz`, registra os hooks nos agentes (rodar de novo só atualiza, sem duplicar), vincula o `tmux.conf` e verifica as chaves salvas no Keychain. Se encontrar restos de uma versão antiga do Voz (o motor de voz em Python, o ambiente `.venv` e o gancho de pre-commit que ela criava), ele os remove. Depois, recarregue o shell:

```zsh
source ~/.zshrc
```

### Codex

O instalador acrescenta três [hooks do Codex](https://learn.chatgpt.com/docs/hooks) em `$CODEX_HOME/hooks.json` (por padrão, `~/.codex/hooks.json`), preservando os hooks existentes, inclusive os do `ai-memory`:

| Evento | Aviso do Voz |
| :--- | :--- |
| `Stop` | Resume e lê a resposta final da tarefa. |
| `PermissionRequest` | Avisa quando o Codex precisa de permissão para uma ação. |
| `PreToolUse` de `request_user_input` ou `request_user_input_async` | Avisa quando há uma pergunta esperando sua resposta. |

Todos rodam em segundo plano e devolvem apenas `{}` ao Codex. O Voz usa a mesma configuração de resumo, pronúncia, provedores e fila de áudio dos outros agentes. Para o resumo, prioriza `last_assistant_message` do evento e busca o pedido no turno correspondente do histórico; comentários intermediários, raciocínio e saídas de ferramentas não entram na leitura.

**Ativação:** reabra o Codex, execute `/hooks` e revise/confie nos três hooks que chamam `voz.sh --codex`. O Codex exige essa revisão para executar hooks novos ou alterados; o instalador não modifica as configurações de confiança.

Se o Voz já estiver compilado, registre somente a integração do Codex com:

```zsh
python3 ~/.claude/voz/scripts/instalar_hooks.py --agente codex
```

O script guarda uma cópia inicial em `hooks.json.voz.bak`. Se o JSON existente estiver inválido, interrompe a instalação sem substituir o arquivo.

---

## Como conectar a voz (Provedores e Chaves)

As chaves são salvas com segurança no Keychain do macOS, sem nunca ficar registradas em arquivos do repositório ou do disco.

O `voz` suporta **múltiplas chaves** para o mesmo serviço: ao atingir limite diário ou cota (429), o sistema tenta a chave `-2`, depois `-3`, e assim sucessivamente antes de pular para o próximo provedor.

### 1. Gemini (Principal gratuito / pago)

Crie chaves em [Google AI Studio](https://aistudio.google.com/apikey).

Salvar chave principal:
```zsh
security add-generic-password -a "$USER" -s gemini-api-key -w
```

Salvar chave secundária para contornar limite diário de requisições:
```zsh
security add-generic-password -a "$USER" -s gemini-api-key-2 -w
```

### 2. ElevenLabs (Opcional)

Crie sua chave em [ElevenLabs](https://elevenlabs.io).

Salvar chave principal:
```zsh
security add-generic-password -a "$USER" -s elevenlabs-api-key -w
```

Salvar chave secundária:
```zsh
security add-generic-password -a "$USER" -s elevenlabs-api-key-2 -w
```

### 3. OpenAI (Opcional)

Crie sua chave em [OpenAI Platform](https://platform.openai.com/api-keys).

Salvar chave principal:
```zsh
security add-generic-password -a "$USER" -s openai-api-key -w
```

Salvar chave secundária:
```zsh
security add-generic-password -a "$USER" -s openai-api-key-2 -w
```

### 4. Siri Natural AI (macOS 27 - Local)

A voz local neural da Siri (`Voz 2` - Sandra pt-BR) já vem integrada no sistema, funciona 100% offline via Apple Neural Engine (ANE) e não requer chave nem internet. Ela atua como reserva imediata se nenhuma chave de nuvem responder.

### Testar a síntese

```zsh
voz sintetizar "Pronto, a voz está funcionando perfeitamente."
```

Para atualizar qualquer chave existente no Keychain, basta adicionar `-U` ao comando `security add-generic-password`.

**Privacidade:** na narração, o pedido e a resposta final do agente vão para o modelo que escreve o resumo, e o resumo vai para a voz. No plano gratuito da API Gemini, o Google pode usar esses textos e áudios para melhorar produtos; habilitar o billing no Google AI Studio remove essa retenção.

---

## Configuração

### `config.json`

| Chave | O que faz | Padrão |
| :--- | :--- | :--- |
| `provedores` | Lista com a ordem dos provedores para síntese de voz. | `["gemini", "elevenlabs", "openai"]` |
| `gemini.modelos` | Modelos de voz do Gemini em cascata de cota (TTS dedicados priorizados). | `["gemini-3.8-flash-tts", "gemini-3.8-flash-lite-tts", "gemini-3.1-flash-tts-preview", "gemini-2.5-flash-preview-tts", "gemini-2.5-pro-preview-tts"]` |
| `gemini.voz` | Nome da voz do Gemini. | `Vindemiatrix` |
| `gemini.estilo` | Sotaque e tom em linguagem natural. | português do Brasil, calma, natural, profissional |
| `gemini.modelo_resumo` | Modelo de texto que escreve o resumo da narração. | `gemini-3.5-flash-lite` |
| `elevenlabs.modelo` | Modelo do ElevenLabs para geração de áudio. | `eleven_turbo_v2_5` |
| `elevenlabs.voz_id` | ID da voz no ElevenLabs. | `21m00Tcm4TlvDq8ikWAM` |
| `openai.modelo` | Modelo de voz da OpenAI. | `tts-1` |
| `openai.voz` | Voz da OpenAI. | `nova` |
| `reserva.voz` | Voz de reserva do `say` do macOS (Siri Natural). | `Voz 2` |
| `frases.terminei`, `frases.permissao`, `frases.espera` | Listas de frases dos avisos; com mais de uma, o `voz` sorteia sem repetir a anterior. | uma frase cada |
| `narrar.ativo` | Liga ou desliga a narração no fim das tarefas. | `true` |
| `narrar.min_segundos` | Só narra tarefas que duraram pelo menos esse tempo. | `0` |
| `narrar.palavras` | Tamanho máximo do resumo falado (40 palavras dão uns 15 segundos). | `40` |

### `fonetica.json`

Dicionário de pronúncia, com expressões regulares e o jeito de falar cada termo:

- `termos` vale para toda voz: nomes que nenhuma acerta sozinha.
- `termos_reserva` vale só para o `say`, que lê termos em inglês (GitHub, API, JSON…) com fonética portuguesa. O Gemini os pronuncia certo sozinho.

Para ver como um texto chega a cada voz:

```zsh
voz fonetica "Abri o PR da API no GitHub"
```

### `tmux.conf`

Configuração do `tmux`, vinculada em `~/.tmux.conf`: tema Dracula, cópia para a área de transferência ao selecionar com o mouse, etiqueta `✦ pasta` no canto de cada painel e atalhos `prefixo + -` (divide na horizontal) e `prefixo + |` (divide na vertical).

---

## Outros comandos

```zsh
voz sintetizar "texto"
voz fonetica "texto"
voz simular conversa.jsonl pasta
voz tocar arquivo.wav
```

- `voz sintetizar`: fala um texto com a voz configurada.
- `voz fonetica`: mostra o texto depois do dicionário de pronúncia.
- `voz simular`: mostra o resumo que a narração falaria a partir de um log de conversa.
- `voz tocar`: toca um arquivo de áudio na fila de falas.

---

## Como conectar o GitHub

### Na sua própria máquina (dono do repositório)

Basta autenticar o GitHub CLI:

```zsh
gh auth login
```

O `instalar.sh` reconhece que a conta logada no `gh` é a dona do repositório e configura o `origin` apontando para ele.

### Outros usuários (fork)

O repositório principal aceita `push` apenas do dono. Para manter suas próprias configurações (`config.json`, `fonetica.json`), autentique o `gh` e crie seu fork:

```zsh
gh auth login
cd ~/.claude/voz && gh repo fork judhagsan/voz --remote=true --remote-name=origin
```

---

## Estrutura de arquivos

| Arquivo / Pasta | Função |
| :--- | :--- |
| `voz.sh` | Entrada dos hooks; repassa para o binário. Sem o binário compilado, responde aos agentes e avisa pelo `say`. |
| `instalar.sh` | Instalador para macOS: dependências, compilação, hooks e `tmux.conf`. |
| `scripts/instalar_hooks.py` | Registra os hooks de Claude, Antigravity e Codex, preservando outras integrações. |
| `config.json` | Voz, modelo do resumo, frases dos avisos e narração. |
| `fonetica.json` | Dicionário de pronúncia. |
| `tmux.conf` | Configuração versionada do `tmux`. |
| `estilo.json` | Perfil de estilo de comunicação guardado como base; o binário atual não o usa. |
| `src/main.rs` | Entrada do binário: painel, hooks e comandos. |
| `src/narrar.rs`, `src/transcript.rs` | Narração do fim de tarefa: lê a conversa e monta a fala. |
| `src/resumo.rs` | Resumo da resposta pelo modelo de texto do Gemini. |
| `src/fala.rs`, `src/audio.rs` | Controle de síntese, cascata de provedores, fila e reprodução no CoreAudio. |
| `src/gemini.rs` | Síntese de voz em streaming pela API do Gemini (PCM 24 kHz). |
| `src/elevenlabs.rs` | Síntese de voz em streaming pela API do ElevenLabs (PCM 24 kHz). |
| `src/openai.rs` | Síntese de voz em streaming pela API da OpenAI (PCM 24 kHz). |
| `src/keychain.rs` | Leitura segura e rotação sequencial de chaves no Keychain do macOS. |
| `src/fonetica.rs` | Aplicação do dicionário de pronúncia. |
| `src/agy.rs` | Decide se um comando do Antigravity espera aprovação manual. |
| `src/codex.rs` | Mapeia eventos do Codex para resumo, permissão e espera. |
| `src/painel.rs`, `src/cotas.rs`, `src/tela.rs` | Painel: repositórios, cotas dos agentes e desenho na tela. |
