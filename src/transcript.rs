use regex::Regex;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

fn remover_caminhos(texto: &str) -> String {
    // Reduz caminhos absolutos (2+ segmentos) ou relativos com extensão ao nome do arquivo,
    // preservando comandos curtos (/usage) e expressões como macOS/Linux.
    let re = Regex::new(
        r"(?P<pref>^|[\s(])(?:/[A-Za-z0-9._-]+(?:/[A-Za-z0-9._-]+)+|(?:[A-Za-z0-9._-]+/)+[A-Za-z0-9._-]+\.[A-Za-z0-9]{1,8})(?:#L?\d+(?:-L?\d+)?)?",
    )
    .unwrap();
    re.replace_all(texto, |caps: &regex::Captures| {
        let pref = &caps["pref"];
        let inteiro = caps.get(0).map(|m| m.as_str()).unwrap_or("");
        let caminho = &inteiro[pref.len()..];
        let sem_ancora = caminho.split('#').next().unwrap_or(caminho);
        let nome = sem_ancora
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(sem_ancora);
        format!("{pref}{nome}")
    })
    .to_string()
}

fn eh_hash_hex(token: &str) -> bool {
    let primeiro = token.split("..").next().unwrap_or(token);
    primeiro.bytes().any(|b| b.is_ascii_digit())
        && primeiro.bytes().any(|b| matches!(b, b'a'..=b'f'))
}

fn normalizar_underscores(texto: &str) -> String {
    let chars: Vec<char> = texto.chars().collect();
    let mut limpo = String::with_capacity(texto.len());
    for (i, &c) in chars.iter().enumerate() {
        if c == '_' {
            let ant = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
            let prox = chars.get(i + 1).copied();
            if ant.is_some_and(char::is_alphanumeric) && prox.is_some_and(char::is_alphanumeric) {
                limpo.push(' ');
            }
        } else if !matches!(c, '*' | '`' | '#' | '~') {
            limpo.push(c);
        }
    }
    limpo
}

pub fn limpar_linha(linha: &str) -> String {
    let re_num = Regex::new(r"^[0-9]+[.)]\s*").unwrap();
    let sem_num_ini = re_num.replace(linha.trim(), "");

    let re_bullet = Regex::new(r"^[-*•]\s*").unwrap();
    let sem_bullet_ini = re_bullet.replace(&sem_num_ini, "");

    let re_link = Regex::new(r"!?\[([^\]]+)\]\([^\)]+\)").unwrap();
    let sem_links = re_link.replace_all(&sem_bullet_ini, "$1");

    let re_url = Regex::new(r"(?:https?|file)://[^\s\])>,\x22']+").unwrap();
    let sem_urls = re_url.replace_all(&sem_links, "");

    let re_diff = Regex::new(r"[+-]\d+\s*/\s*[+-]\d+").unwrap();
    let sem_diffs = re_diff.replace_all(&sem_urls, "");

    let re_hash = Regex::new(r"\b[0-9a-f]{7,40}(?:\.\.[0-9a-f]{7,40})?\b").unwrap();
    let sem_hashes = re_hash.replace_all(&sem_diffs, |caps: &regex::Captures| {
        let token = caps.get(0).map(|m| m.as_str()).unwrap_or("");
        if eh_hash_hex(token) {
            String::new()
        } else {
            token.to_string()
        }
    });

    let sem_caminhos = remover_caminhos(&sem_hashes);
    let mut sem_simbolos = sem_caminhos
        .replace(['→', '⇒'], " para ")
        .replace('·', ", ");
    sem_simbolos = Regex::new(r"\[[✓xX ]\]|[│─╭╮╰╯├┤┬┴┼]")
        .unwrap()
        .replace_all(&sem_simbolos, " ")
        .to_string();

    let limpo = normalizar_underscores(&sem_simbolos);
    let sem_num = re_num.replace(&limpo, "");
    let sem_bullet = re_bullet.replace(&sem_num, "");
    let sem_parens = Regex::new(r"\(\s*[,;/\-]*\s*\)")
        .unwrap()
        .replace_all(&sem_bullet, "");
    let colapsado = Regex::new(r"\s+")
        .unwrap()
        .replace_all(&sem_parens, " ")
        .to_string();
    let pontuacao = Regex::new(r"\s+([,.;:!?])")
        .unwrap()
        .replace_all(&colapsado, "$1")
        .to_string();

    pontuacao
        .trim()
        .trim_end_matches(':')
        .trim_matches(|c| matches!(c, '—' | '–' | '-'))
        .trim()
        .to_string()
}

struct LinhaCandidata {
    texto: String,
    eh_secao_resumo: bool,
    eh_intro_lista: bool,
    eh_fatica: bool,
    eh_pergunta: bool,
    palavras: usize,
}

fn eh_secao_resumo(titulo: &str) -> bool {
    matches!(
        titulo.trim().to_lowercase().as_str(),
        "resumo" | "conclusão" | "conclusao" | "síntese" | "sintese"
    )
}

fn eh_abertura_fatica(texto: &str) -> bool {
    let limpo = texto
        .trim()
        .trim_end_matches(['.', '!', ',', ':'])
        .to_lowercase();
    matches!(
        limpo.as_str(),
        "perfeito"
            | "entendido"
            | "ótimo"
            | "otimo"
            | "beleza"
            | "certo"
            | "claro"
            | "com certeza"
            | "boa pergunta"
            | "excelente pergunta"
            | "boa observação"
            | "boa observacao"
            | "faz sentido"
            | "sim"
            | "combinado"
    )
}

fn preparar_candidata(
    mut texto: String,
    secao_resumo: bool,
    terminava_dois_pontos: bool,
) -> Option<LinhaCandidata> {
    let fatica = eh_abertura_fatica(&texto);
    let palavras = texto.split_whitespace().count();
    let sem_separador = texto.trim_end_matches([';', ',']).len();
    texto.truncate(sem_separador);
    let eh_pergunta = texto.ends_with('?');
    let minimo = if eh_pergunta { 2 } else { 4 };
    if palavras < minimo {
        return None;
    }
    if !texto.ends_with(['.', '!', '?', '…']) {
        texto.push('.');
    }
    Some(LinhaCandidata {
        texto,
        eh_secao_resumo: secao_resumo,
        eh_intro_lista: terminava_dois_pontos && palavras <= 10,
        eh_fatica: fatica,
        eh_pergunta,
        palavras,
    })
}

fn coletar_candidatos(resposta: &str) -> Vec<LinhaCandidata> {
    let mut candidatos = Vec::new();
    let mut em_bloco = false;
    let mut secao_resumo = false;

    for linha in resposta.lines() {
        let l = linha.trim();
        if l.starts_with("```") {
            em_bloco = !em_bloco;
            continue;
        }
        if em_bloco || l.is_empty() {
            continue;
        }
        if l.starts_with('#') {
            secao_resumo = eh_secao_resumo(&limpar_linha(l));
            continue;
        }
        if l.starts_with('|')
            || l.starts_with('>')
            || l.starts_with("<!--")
            || l.starts_with("http")
            || l.chars().all(|c| matches!(c, '-' | '=' | '*' | '_'))
        {
            continue;
        }

        let limpo = limpar_linha(l);
        let terminava_dois_pontos = l.trim_end_matches(['*', '`', '_']).ends_with(':');
        if let Some(cand) = preparar_candidata(limpo, secao_resumo, terminava_dois_pontos) {
            candidatos.push(cand);
        }
    }
    candidatos
}

fn selecionar_principais(
    candidatos: &[LinhaCandidata],
    max_palavras: usize,
) -> Vec<&LinhaCandidata> {
    if max_palavras == 0 {
        return candidatos.iter().collect();
    }

    let na_secao_resumo: Vec<&LinhaCandidata> = candidatos
        .iter()
        .filter(|c| c.eh_secao_resumo && !c.eh_fatica)
        .collect();
    let base: Vec<&LinhaCandidata> = if !na_secao_resumo.is_empty() {
        na_secao_resumo
    } else {
        candidatos.iter().collect()
    };

    // Pula aberturas puramente fáticas ou rótulos introdutórios ("O que mudou:") quando há conteúdo útil depois.
    let mut filtrados: Vec<&LinhaCandidata> = Vec::new();
    let total = base.len();
    for (idx, item) in base.iter().enumerate() {
        let ainda_ha_conteudo = idx + 1 < total;
        if filtrados.is_empty() && (item.eh_fatica || (item.eh_intro_lista && ainda_ha_conteudo)) {
            continue;
        }
        if item.eh_intro_lista && ainda_ha_conteudo {
            continue;
        }
        filtrados.push(*item);
    }
    if filtrados.is_empty() {
        base
    } else {
        filtrados
    }
}

fn prefixar_projeto(escolhida: String, projeto: &str) -> String {
    let proj_limpo = normalizar_underscores(projeto);
    let proj = proj_limpo.trim();
    if proj.is_empty() || proj == "geral" {
        return escolhida;
    }

    let minuscula = escolhida.to_lowercase();
    let tem_locativo = minuscula.starts_with("no ")
        || minuscula.starts_with("na ")
        || minuscula.starts_with("em ")
        || minuscula.starts_with("aqui no ");
    if tem_locativo {
        return escolhida;
    }

    let mut chars = escolhida.chars();
    let primeiro = chars.next();
    let segundo = chars.clone().next();
    let manter_maiuscula = segundo.is_some_and(|c| c.is_uppercase());
    let inicio = match primeiro {
        Some(c) if !manter_maiuscula => c.to_lowercase().to_string(),
        Some(c) => c.to_string(),
        None => String::new(),
    };
    let resto: String = chars.collect();
    format!("No {proj}, {inicio}{resto}")
}

/// Extrai a fala curta da narração diretamente do texto corrido da resposta
/// final, pulando títulos, tabelas, citações, links e blocos de código.
/// Com `max_palavras` > 0, prioriza seção de resumo, preserva pergunta final
/// pendente e mantém frases inteiras no limite; com 0, lê tudo.
pub fn extrair_resumo_direto(resposta: &str, projeto: &str, max_palavras: usize) -> String {
    let candidatos = coletar_candidatos(resposta);
    if candidatos.is_empty() {
        let proj = normalizar_underscores(projeto);
        let proj = proj.trim();
        return if !proj.is_empty() && proj != "geral" {
            format!("Tudo pronto por aqui no {proj}.")
        } else {
            "Tudo pronto por aqui.".to_string()
        };
    }

    let selecionados = selecionar_principais(&candidatos, max_palavras);
    let texto_completo = selecionados
        .iter()
        .map(|c| c.texto.as_str())
        .collect::<Vec<_>>()
        .join(" ");

    let mut escolhida = if max_palavras == 0 {
        texto_completo
    } else {
        let pergunta_final = candidatos.iter().rev().take(3).find(|c| c.eh_pergunta);

        let mut resumo = limitar_frases(&texto_completo, max_palavras);
        if let Some(perg) = pergunta_final {
            if !resumo.contains(&perg.texto) && perg.palavras < max_palavras {
                let orcamento = max_palavras.saturating_sub(perg.palavras);
                if orcamento >= 6 {
                    let parte = limitar_frases(&texto_completo, orcamento);
                    resumo = format!("{parte} {}", perg.texto);
                }
            }
        }
        resumo
    };

    if !escolhida.ends_with(['.', '!', '?', '…']) {
        escolhida.push('.');
    }

    prefixar_projeto(escolhida, projeto)
}

fn cortar_em_pausa_ou_reticencias(frase: &str, max_palavras: usize) -> String {
    let tokens: Vec<&str> = frase.split_whitespace().collect();
    let minimo = (max_palavras / 2).max(1);
    if let Some(idx) = (minimo..=max_palavras.min(tokens.len()))
        .rev()
        .find(|&i| tokens[i - 1].ends_with([',', ';']))
    {
        let mut trecho = tokens[..idx].join(" ");
        let sem_pausa = trecho.trim_end_matches([',', ';']).len();
        trecho.truncate(sem_pausa);
        trecho.push('.');
        return trecho;
    }
    let corte: Vec<&str> = tokens.into_iter().take(max_palavras).collect();
    format!("{}...", corte.join(" "))
}

/// Junta frases inteiras até `max_palavras`. Se a primeira sozinha passa do
/// limite, fecha na última vírgula útil ou corta com reticências.
fn limitar_frases(texto: &str, max_palavras: usize) -> String {
    // Pontuação seguida de espaço ou fim: "README.md" e "v0.2.0" não quebram.
    let re_fim = Regex::new(r"[.!?…]+(?:\s+|$)").unwrap();
    let mut frases = Vec::new();
    let mut inicio = 0;
    for m in re_fim.find_iter(texto) {
        frases.push(texto[inicio..m.end()].trim());
        inicio = m.end();
    }
    if inicio < texto.len() {
        frases.push(texto[inicio..].trim());
    }

    let mut escolhida = String::new();
    let mut total = 0;
    for frase in frases.into_iter().filter(|f| !f.is_empty()) {
        let palavras = frase.split_whitespace().count();
        if total == 0 && palavras > max_palavras {
            return cortar_em_pausa_ou_reticencias(frase, max_palavras);
        }
        if total + palavras > max_palavras {
            break;
        }
        if !escolhida.is_empty() {
            escolhida.push(' ');
        }
        escolhida.push_str(frase);
        total += palavras;
    }
    escolhida
}

#[allow(dead_code)]
pub fn extrair_resumo_multiplo(projetos: &[String]) -> String {
    let total = projetos.len();
    if total == 0 {
        return "Concluí as tarefas em paralelo.".to_string();
    }
    if total == 2 {
        return format!(
            "Concluí duas frentes em paralelo: no {} e no {}.",
            projetos[0], projetos[1]
        );
    }
    let nomes = format!(
        "{} e {}",
        projetos[..total - 1].join(", "),
        projetos[total - 1]
    );
    format!("Concluí {total} frentes de trabalho em paralelo: {nomes}.")
}

pub fn ultimo_turno(transcript_path: &Path) -> (Option<f64>, String, Option<String>) {
    turno_codex(transcript_path, None)
}

/// `turn_id` fixa o turno do hook assíncrono, mesmo se outro já tiver começado.
pub fn turno_codex(
    transcript_path: &Path,
    turn_id: Option<&str>,
) -> (Option<f64>, String, Option<String>) {
    let file = match File::open(transcript_path) {
        Ok(f) => f,
        Err(_) => return (None, String::new(), None),
    };

    let reader = BufReader::new(file);
    let mut inicio: Option<f64> = None;
    let mut pedido = String::new();
    let mut textos: Vec<String> = Vec::new();
    let mut turno_selecionado = turn_id.is_none();

    for line in reader.lines().map_while(Result::ok) {
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(val) => val,
            Err(_) => continue,
        };

        let tipo = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

        // Codex: eventos do ciclo de vida e mensagens estruturadas do rollout.
        if tipo == "event_msg" {
            let payload = &v["payload"];
            let evento = payload.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if evento == "task_started" {
                turno_selecionado = turn_id
                    .is_none_or(|id| payload.get("turn_id").and_then(|t| t.as_str()) == Some(id));
                if turno_selecionado {
                    inicio = v
                        .get("timestamp")
                        .and_then(|t| t.as_str())
                        .and_then(|t| chrono_to_ts(t).ok());
                    pedido.clear();
                    textos.clear();
                }
            }
            if !turno_selecionado {
                continue;
            }
            match evento {
                "user_message" => {
                    if let Some(mensagem) = payload.get("message").and_then(|m| m.as_str()) {
                        pedido = mensagem.to_string();
                        textos.clear();
                        if inicio.is_none() {
                            inicio = v
                                .get("timestamp")
                                .and_then(|t| t.as_str())
                                .and_then(|t| chrono_to_ts(t).ok());
                        }
                    }
                }
                "agent_message" if fase_final(payload) => {
                    if let Some(mensagem) = payload
                        .get("message")
                        .and_then(|m| m.as_str())
                        .filter(|m| !m.trim().is_empty())
                    {
                        textos = vec![mensagem.to_string()];
                    }
                }
                "task_complete" => {
                    if let Some(mensagem) = payload
                        .get("last_agent_message")
                        .and_then(|m| m.as_str())
                        .filter(|m| !m.trim().is_empty())
                    {
                        textos = vec![mensagem.to_string()];
                    }
                }
                "turn_aborted" => textos.clear(),
                _ => {}
            }
            continue;
        }
        if tipo == "response_item" {
            if !turno_selecionado {
                continue;
            }
            let payload = &v["payload"];
            match payload.get("type").and_then(|t| t.as_str()) {
                Some("message") => {
                    let texto = payload
                        .get("content")
                        .and_then(|c| c.as_array())
                        .into_iter()
                        .flatten()
                        .filter(|c| {
                            matches!(
                                c.get("type").and_then(|t| t.as_str()),
                                Some("input_text" | "output_text")
                            )
                        })
                        .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    match payload.get("role").and_then(|r| r.as_str()) {
                        Some("user") if !texto.trim().is_empty() => {
                            pedido = texto;
                            textos.clear();
                            if inicio.is_none() {
                                inicio = v
                                    .get("timestamp")
                                    .and_then(|t| t.as_str())
                                    .and_then(|t| chrono_to_ts(t).ok());
                            }
                        }
                        Some("assistant") if fase_final(payload) && !texto.trim().is_empty() => {
                            textos = vec![texto]
                        }
                        _ => {}
                    }
                }
                Some("function_call" | "custom_tool_call" | "web_search_call") => textos.clear(),
                _ => {}
            }
            continue;
        }

        // Claude CLI: mensagem de usuário
        if tipo == "user" {
            if let Some(conteudo) = v.get("message").and_then(|m| m.get("content")) {
                if let Some(arr) = conteudo.as_array() {
                    let has_tool_res = arr
                        .iter()
                        .any(|i| i.get("type").and_then(|t| t.as_str()) == Some("tool_result"));
                    if has_tool_res {
                        textos.clear();
                        continue;
                    }
                    let mut txt = String::new();
                    for item in arr {
                        if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(t) = item.get("text").and_then(|s| s.as_str()) {
                                txt.push_str(t);
                            }
                        }
                    }
                    if !txt.is_empty() {
                        pedido = txt;
                        textos.clear();
                        continue;
                    }
                }
            }
        }

        // Claude CLI: assistente
        if tipo == "assistant" {
            if let Some(conteudo) = v.get("message").and_then(|m| m.get("content")) {
                if let Some(arr) = conteudo.as_array() {
                    for item in arr {
                        if item.get("type").and_then(|t| t.as_str()) == Some("text") {
                            if let Some(t) = item.get("text").and_then(|s| s.as_str()) {
                                if !t.trim().is_empty() {
                                    textos.push(t.to_string());
                                }
                            }
                        }
                    }
                }
            }
            continue;
        }

        // Antigravity / Gemini CLI: USER_INPUT
        if tipo == "USER_INPUT" {
            if let Some(content) = v.get("content").and_then(|c| c.as_str()) {
                pedido = content.to_string();
                textos.clear();
            }
            if let Some(ts) = v.get("created_at").and_then(|c| c.as_str()) {
                // ISO timestamp básico ou contagem
                if let Ok(dt) = chrono_to_ts(ts) {
                    inicio = Some(dt);
                }
            }
            continue;
        }

        // Antigravity: PLANNER_RESPONSE
        if tipo == "PLANNER_RESPONSE" {
            let has_tools = v
                .get("tool_calls")
                .and_then(|tc| tc.as_array())
                .map(|a| !a.is_empty())
                .unwrap_or(false);

            if has_tools {
                textos.clear();
            } else if let Some(content) = v.get("content").and_then(|c| c.as_str()) {
                if !content.trim().is_empty() {
                    textos.push(content.to_string());
                }
            }
        }
    }

    let final_resp = if textos.is_empty() {
        None
    } else {
        Some(textos.join("\n").trim().to_string())
    };

    (inicio, pedido, final_resp)
}

fn chrono_to_ts(iso: &str) -> Result<f64, ()> {
    chrono::DateTime::parse_from_rfc3339(iso)
        .map(|d| d.timestamp_millis() as f64 / 1000.0)
        .map_err(|_| ())
}

fn fase_final(payload: &serde_json::Value) -> bool {
    matches!(
        payload.get("phase").and_then(|p| p.as_str()),
        None | Some("final_answer")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ler_fixture(
        nome: &str,
        texto: &str,
        turno: Option<&str>,
    ) -> (Option<f64>, String, Option<String>) {
        let arquivo = std::env::temp_dir().join(format!(
            "voz-transcript-{nome}-{}.jsonl",
            std::process::id()
        ));
        std::fs::write(&arquivo, texto).unwrap();
        let resultado = turno_codex(&arquivo, turno);
        std::fs::remove_file(arquivo).unwrap();
        resultado
    }

    const CODEX: &str = r#"
{"type":"event_msg","timestamp":"2026-11-01T15:00:00Z","payload":{"type":"task_started","turn_id":"primeiro"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Integre a voz ao Codex."}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","phase":"commentary","content":[{"type":"output_text","text":"Vou verificar os hooks."}]}}
{"type":"response_item","payload":{"type":"reasoning","summary":[{"text":"Raciocínio privado"}]}}
{"type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"segredo"}}
{"type":"response_item","payload":{"type":"function_call_output","output":"saída que não deve ser narrada"}}
{"type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"Integrei o Voz e validei os hooks."}]}}
{"type":"event_msg","payload":{"type":"task_complete","turn_id":"primeiro","last_agent_message":"Integrei o Voz e validei os hooks."}}
{"type":"event_msg","timestamp":"2026-11-01T15:05:00Z","payload":{"type":"task_started","turn_id":"segundo"}}
{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Agora ajuste a tela."}]}}
{"type":"response_item","payload":{"type":"message","role":"assistant","phase":"final_answer","content":[{"type":"output_text","text":"Ajustei a tela do painel."}]}}
"#;

    #[test]
    fn codex_le_ultimo_turno_e_ignora_comentarios_ferramentas_e_raciocinio() {
        let (inicio, pedido, resposta) = ler_fixture("atual", CODEX, None);
        assert_eq!(inicio, Some(1793545500.0));
        assert_eq!(pedido, "Agora ajuste a tela.");
        assert_eq!(resposta.as_deref(), Some("Ajustei a tela do painel."));
    }

    #[test]
    fn codex_hook_assincrono_fica_no_turno_original_sem_duplicar_resposta() {
        let (inicio, pedido, resposta) = ler_fixture("fixo", CODEX, Some("primeiro"));
        assert_eq!(inicio, Some(1793545200.0));
        assert_eq!(pedido, "Integre a voz ao Codex.");
        assert_eq!(
            resposta.as_deref(),
            Some("Integrei o Voz e validei os hooks.")
        );
    }

    #[test]
    fn codex_interrompido_nao_reaproveita_resposta_antiga() {
        let texto = format!(
            "{CODEX}\n{}",
            r#"{"type":"event_msg","payload":{"type":"turn_aborted"}}"#
        );
        let (_, _, resposta) = ler_fixture("interrompido", &texto, None);
        assert_eq!(resposta, None);
    }

    #[test]
    fn codex_aceita_eventos_antigos_sem_narrar_duas_vezes() {
        let texto = r#"
{"type":"event_msg","timestamp":"2026-11-01T15:00:00Z","payload":{"type":"user_message","message":"Revise o projeto."}}
{"type":"event_msg","payload":{"type":"agent_message","phase":"commentary","message":"Vou revisar."}}
{"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Revisei o projeto e corrigi os erros."}]}}
{"type":"event_msg","payload":{"type":"agent_message","message":"Revisei o projeto e corrigi os erros."}}
"#;
        let (inicio, pedido, resposta) = ler_fixture("antigo", texto, None);
        assert_eq!(inicio, Some(1793545200.0));
        assert_eq!(pedido, "Revise o projeto.");
        assert_eq!(
            resposta.as_deref(),
            Some("Revisei o projeto e corrigi os erros.")
        );
    }

    #[test]
    fn preserva_leitura_do_claude_e_antigravity() {
        let claude = r#"
{"type":"user","message":{"content":[{"type":"text","text":"Corrija o teste."}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Vou corrigir."}]}}
{"type":"user","message":{"content":[{"type":"tool_result","content":"resultado"}]}}
{"type":"assistant","message":{"content":[{"type":"text","text":"Corrigi e o teste passou."}]}}
"#;
        let (_, pedido, resposta) = ler_fixture("claude", claude, None);
        assert_eq!(pedido, "Corrija o teste.");
        assert_eq!(resposta.as_deref(), Some("Corrigi e o teste passou."));
        let agy = r#"
{"type":"USER_INPUT","created_at":"2026-11-01T12:00:00-03:00","content":"Corrija o teste."}
{"type":"PLANNER_RESPONSE","content":"Vou corrigir.","tool_calls":[{}]}
{"type":"PLANNER_RESPONSE","content":"Corrigi e o teste passou.","tool_calls":[]}
"#;
        let (inicio, pedido, resposta) = ler_fixture("agy", agy, None);
        assert_eq!(inicio, Some(1793545200.0));
        assert_eq!(pedido, "Corrija o teste.");
        assert_eq!(resposta.as_deref(), Some("Corrigi e o teste passou."));
    }

    #[test]
    fn test_limpar_linha() {
        let bruta = "1. **[Documentação](https://exemplo.com)**: Tudo *pronto* por aqui:";
        let limpa = limpar_linha(bruta);
        assert_eq!(limpa, "Documentação: Tudo pronto por aqui");
    }

    #[test]
    fn test_extrair_resumo_direto() {
        let resposta =
            "Identifiquei a causa raiz dos erros.\n\n1. O comando ps falhava no sandbox.";
        let resumo = extrair_resumo_direto(resposta, "voz", 0);
        assert_eq!(
            resumo,
            "No voz, identifiquei a causa raiz dos erros. O comando ps falhava no sandbox."
        );
    }

    #[test]
    fn test_sem_limite_le_tudo_menos_codigo_tabela_e_titulo() {
        let resposta = "## Resumo\n\nCorrigi o painel de cotas hoje.\n\n```rust\nlet x = 1; // isto não deve ser lido\n```\n\n| a | b |\n\n- Ajustei o arquivo README.md da versão v0.2.0;\n- Rodei todos os testes do projeto.";
        assert_eq!(
            extrair_resumo_direto(resposta, "", 0),
            "Corrigi o painel de cotas hoje. Ajustei o arquivo README.md da versão v0.2.0. Rodei todos os testes do projeto."
        );
    }

    #[test]
    fn test_limite_fica_em_frases_inteiras() {
        let resposta = "Primeira frase com cinco palavras. Segunda frase também tem cinco. Terceira frase passaria do limite.";
        assert_eq!(
            extrair_resumo_direto(resposta, "", 11),
            "Primeira frase com cinco palavras. Segunda frase também tem cinco."
        );
    }

    #[test]
    fn test_limite_corta_primeira_frase_longa() {
        let resposta = "Uma frase comprida demais para caber no limite pedido.";
        assert_eq!(
            extrair_resumo_direto(resposta, "", 4),
            "Uma frase comprida demais..."
        );
    }

    #[test]
    fn test_limpar_linha_para_fala_tts() {
        let bruta = "- **Card ([cotas.rs](file:///Users/judhagsan/voz/src/cotas.rs#L10-L20)):** ajustei `renderizar_card` (b36309b, +300 / -168) em /Users/judhagsan/voz/src/painel.rs e https://exemplo.com (main → main) com `macOS/Linux`.";
        assert_eq!(
            limpar_linha(bruta),
            "Card (cotas.rs): ajustei renderizar card em painel.rs e (main para main) com macOS/Linux."
        );
    }

    #[test]
    fn test_resumo_prioriza_secao_resumo_e_preserva_pergunta_final() {
        let resposta = "Perfeito!\n\nAnalisei vários detalhes técnicos do sistema antes de mudar qualquer coisa.\n\n## Resumo\n\nImplementei o filtro automático de serviços ativos.\n\nO que mudou:\n- O painel agora oculta colunas inativas.\n\nPosso commitar agora?";
        let fala = extrair_resumo_direto(resposta, "voz", 20);
        assert!(fala.starts_with("No voz, implementei o filtro automático de serviços ativos."));
        assert!(fala.ends_with("Posso commitar agora?"));
        assert!(!fala.contains("Perfeito"));
        assert!(!fala.contains("O que mudou"));
    }

    #[test]
    fn test_resumo_preserva_siglas_e_fecha_em_virgula() {
        assert_eq!(
            extrair_resumo_direto("API de cotas retornou erro de autenticação.", "meu_app", 20),
            "No meu app, API de cotas retornou erro de autenticação."
        );

        let longa = "Atualizei todos os provedores de voz e o painel de projetos ativos, removendo código antigo que não precisava mais ficar ali.";
        assert_eq!(
            extrair_resumo_direto(longa, "", 13),
            "Atualizei todos os provedores de voz e o painel de projetos ativos."
        );
    }
}
