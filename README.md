# Voz

Voz para o terminal do macOS. Quando um agente de programação termina uma tarefa, pede permissão ou fica esperando você, o Voz avisa em voz alta, e no fim de cada tarefa fala um resumo curto do que foi feito. No terminal, o comando `voz` mostra um painel com o estado dos seus repositórios e das cotas de uso dos agentes.

A voz é a **Vindemiatrix** do Gemini, gerada sob demanda e tocada enquanto chega. Nenhum áudio fica guardado, nem no disco nem no Git. Sem chave ou sem internet, os avisos saem pela voz do próprio macOS (`say`).

---

## Como funciona

Os agentes chamam o `voz.sh` nos eventos de fim de tarefa, pedido de permissão e espera. Ele repassa tudo para o binário `voz`, escrito em Rust:

```
evento do agente ─▶ voz.sh ─▶ voz
                               ├─ terminei ──────────▶ resumo da resposta ─┐
                               ├─ permissao, espera ─▶ frase do config ────┤
                               │                                           ▼
                               │                     voz do Gemini, em streaming ─▶ alto-falante
                               │
                               └─ sem chave ou sem internet ─▶ say do macOS
```

1. **Fim de tarefa (`terminei`):** o `voz` lê na conversa o que você pediu e a resposta final do agente. Um modelo de texto do Gemini (`gemini-3.5-flash-lite`) escreve um resumo falado de até 40 palavras, com o projeto, o resultado e, se for o caso, a decisão que o agente espera de você. Leva cerca de 1 segundo.
2. **Permissão e espera:** fala uma frase curta, sorteada entre as do `config.json` sem repetir a anterior. No Antigravity, o aviso de permissão só toca quando o comando realmente espera aprovação manual.
3. **Pronúncia:** antes da voz, o texto passa pelo dicionário `fonetica.json`, para nomes que nenhuma voz acerta sozinha (como `judhagsan`).
4. **Voz:** o modelo `gemini-3.8-flash-lite-tts` devolve o áudio em pedaços, e a fala começa no primeiro deles, cerca de 1,5 segundo depois do pedido. O áudio vai direto para a saída de som do macOS (CoreAudio), sem passar por arquivo.
5. **Fila:** quando vários agentes terminam ao mesmo tempo, as falas tocam uma depois da outra, sem se atropelar.
6. **Reserva:** se a chave faltar, a internet cair ou a cota acabar, a mesma frase sai pelo `say` do macOS, com o dicionário completo de pronúncia. Sem o resumo, a narração usa o começo da resposta, cortado no mesmo tamanho.

---

## Painel no terminal

```zsh
voz          # todos os repositórios, os que estão em andamento no topo
voz -a       # só os que estão em andamento
voz -w       # modo contínuo: atualiza a cada 1 segundo
voz -w 10    # modo contínuo com outro intervalo, em segundos
```

- **Tabela de repositórios:** para cada repositório na sua pasta pessoal, mostra a branch ativa e os arquivos modificados, quantos commits faltam enviar (`↑`) ou puxar (`↓`) e os Pull Requests abertos, com quantos commits cada um tem.
- **Cotas dos agentes:** ao lado da tabela, quanto já foi usado da janela de 5 horas e da semana de cada agente, e quando cada uma renova. Os números vêm das próprias ferramentas, sem gastar cota, e são relidos a cada 2 minutos.
- **Modo contínuo (`-w`):** fala um resumo das pendências ao abrir e se ajusta quando o painel do `tmux` muda de tamanho. Quando a tabela e as cotas não cabem lado a lado, as cotas descem para baixo da tabela.

---

## Pré-requisitos

O instalador verifica e instala, via Homebrew:

- **macOS** (usa a saída de som do sistema, `afplay` e `say`)
- **tmux** (configuração versionada em `tmux.conf`, descrita abaixo)
- **GitHub CLI (`gh`)** (para consultar Pull Requests e configurar seu fork)
- **Rust (`cargo`)** (para compilar o binário `voz`)
- **Command Line Tools da Apple** (o `cargo` precisa delas; se faltarem, o instalador avisa para rodar `xcode-select --install`)

A **chave da API do Gemini** é opcional: sem ela, só a voz de reserva funciona.

---

## Instalação

```zsh
curl -fsSL https://raw.githubusercontent.com/judhagsan/voz/main/instalar.sh | bash
```

Ou, com o repositório já clonado:

```zsh
chmod +x ~/.claude/voz/instalar.sh && ~/.claude/voz/instalar.sh
```

O instalador compila o `voz`, cria o atalho em `~/.local/bin/voz`, registra os hooks nos agentes (rodar de novo só atualiza, sem duplicar), vincula o `tmux.conf` e avisa se a chave do Gemini ainda não estiver salva. Se encontrar restos de uma versão antiga do Voz (o motor de voz em Python, o ambiente `.venv` e o gancho de pre-commit que ela criava), ele os remove. Depois, recarregue o shell:

```zsh
source ~/.zshrc
```

---

## Como conectar a voz (Gemini)

1. Crie uma chave em [Google AI Studio](https://aistudio.google.com/apikey).
2. Salve a chave no Keychain do macOS. Ela é pedida duas vezes e não aparece na tela; rode numa janela de terminal comum, para não ficar registrada em histórico de ferramenta nenhuma:

   ```zsh
   security add-generic-password -a "$USER" -s gemini-api-key -w
   ```

3. Teste:

   ```zsh
   voz sintetizar "Pronto, a voz está funcionando."
   ```

A chave nunca fica em arquivo do repositório. Para trocá-la, rode o mesmo comando com `-U` logo depois de `add-generic-password`.

**Privacidade:** na narração, o pedido e a resposta final do agente vão para o modelo que escreve o resumo, e o resumo vai para a voz. No plano gratuito da API, o Google pode usar esses textos e áudios para melhorar os produtos, inclusive com revisão humana. Ativar a cobrança no projeto do Google AI Studio tira esse uso.

**Custo:** no plano pago, uma narração (resumo e cerca de 15 segundos de voz) sai por cerca de US$ 0,003, e um aviso curto, por menos de US$ 0,001. É uma estimativa feita com os preços de outubro de 2026, que dobram em janeiro de 2027. Confira a [página de preços](https://ai.google.dev/gemini-api/docs/pricing).

---

## Configuração

### `config.json`

| Chave | O que faz | Padrão |
| :--- | :--- | :--- |
| `gemini.modelo` | Modelo da voz. | `gemini-3.8-flash-lite-tts` |
| `gemini.voz` | Voz do Gemini. | `Vindemiatrix` |
| `gemini.estilo` | Sotaque e tom, em linguagem natural. | português do Brasil, calma, um pouco grave |
| `gemini.modelo_resumo` | Modelo de texto que escreve o resumo da narração. | `gemini-3.5-flash-lite` |
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
voz sintetizar "texto"                    # fala um texto com a voz configurada
voz fonetica "texto"                      # mostra o texto depois do dicionário de pronúncia
voz simular <conversa.jsonl> [pasta]      # mostra o resumo que a narração falaria
voz tocar <arquivo>                       # toca um arquivo de áudio na fila de falas
```

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
| `config.json` | Voz, modelo do resumo, frases dos avisos e narração. |
| `fonetica.json` | Dicionário de pronúncia. |
| `tmux.conf` | Configuração versionada do `tmux`. |
| `estilo.json` | Perfil de estilo de comunicação guardado como base; o binário atual não o usa. |
| `src/main.rs` | Entrada do binário: painel, hooks e comandos. |
| `src/narrar.rs`, `src/transcript.rs` | Narração do fim de tarefa: lê a conversa e monta a fala. |
| `src/resumo.rs` | Resumo da resposta pelo modelo de texto do Gemini. |
| `src/fala.rs`, `src/gemini.rs`, `src/audio.rs` | Voz: escolha da frase, síntese em streaming, fila e reprodução. |
| `src/fonetica.rs` | Aplicação do dicionário de pronúncia. |
| `src/agy.rs` | Decide se um comando do Antigravity espera aprovação manual. |
| `src/painel.rs`, `src/cotas.rs`, `src/tela.rs` | Painel: repositórios, cotas dos agentes e desenho na tela. |
