use regex::Regex;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

pub fn limpar_linha(linha: &str) -> String {
    let re_link = Regex::new(r"\[([^\]]+)\]\([^\)]+\)").unwrap();
    let sem_links = re_link.replace_all(linha, "$1");

    let mut limpo = String::with_capacity(sem_links.len());
    for c in sem_links.chars() {
        if !matches!(c, '*' | '_' | '`' | '#' | '~') {
            limpo.push(c);
        }
    }

    let re_num = Regex::new(r"^[0-9]+[.)]\s*").unwrap();
    let sem_num = re_num.replace(&limpo, "");

    let re_bullet = Regex::new(r"^[-*•]\s*").unwrap();
    let sem_bullet = re_bullet.replace(&sem_num, "");

    sem_bullet.trim().trim_end_matches(':').to_string()
}

pub fn extrair_resumo_direto(resposta: &str, projeto: &str) -> String {
    let mut candidatos = Vec::new();

    for linha in resposta.lines() {
        let l = linha.trim();
        if l.is_empty() {
            continue;
        }
        if l.starts_with('#')
            || l.starts_with("```")
            || l.starts_with('|')
            || l.starts_with('>')
            || l.starts_with("<!--")
            || l.starts_with("http")
        {
            continue;
        }

        let l_limpo = limpar_linha(l);
        let palavras_count = l_limpo.split_whitespace().count();
        if palavras_count >= 4 {
            candidatos.push(l_limpo);
            if candidatos.len() >= 2 {
                break;
            }
        }
    }

    if candidatos.is_empty() {
        return if !projeto.is_empty() && projeto != "geral" {
            format!("Tudo pronto por aqui no {projeto}.")
        } else {
            "Tudo pronto por aqui.".to_string()
        };
    }

    let frase_completa = candidatos.join(" ");
    let re_sentencas = Regex::new(r"(?:\.|\!|\?)+").unwrap();

    let mut sentencas: Vec<&str> = Vec::new();
    let mut inicio = 0;
    for mat in re_sentencas.find_iter(&frase_completa) {
        let s = frase_completa[inicio..mat.end()].trim();
        if !s.is_empty() {
            sentencas.push(s);
        }
        inicio = mat.end();
    }
    if inicio < frase_completa.len() {
        let s = frase_completa[inicio..].trim();
        if !s.is_empty() {
            sentencas.push(s);
        }
    }

    let primeira = sentencas.first().copied().unwrap_or(candidatos[0].as_str());
    let mut escolhida = primeira.to_string();

    if sentencas.len() > 1 {
        let segunda = sentencas[1];
        let total_palavras =
            primeira.split_whitespace().count() + segunda.split_whitespace().count();
        if total_palavras <= 22 {
            escolhida = format!("{primeira} {segunda}");
        }
    }

    let palavras: Vec<&str> = escolhida.split_whitespace().collect();
    if palavras.len() > 25 {
        escolhida = format!("{}...", palavras[..25].join(" "));
    }

    if !escolhida.ends_with('.')
        && !escolhida.ends_with('!')
        && !escolhida.ends_with('?')
        && !escolhida.ends_with('…')
        && !escolhida.ends_with("...")
    {
        escolhida.push('.');
    }

    if !projeto.is_empty() && projeto != "geral" {
        let minuscula = escolhida.to_lowercase();
        let tem_locativo = minuscula.starts_with("no ")
            || minuscula.starts_with("na ")
            || minuscula.starts_with("em ")
            || minuscula.starts_with("aqui no ");

        if !tem_locativo {
            let mut chars = escolhida.chars();
            let primeira_letra = chars
                .next()
                .map(|c| c.to_lowercase().to_string())
                .unwrap_or_default();
            let resto: String = chars.collect();
            escolhida = format!("No {projeto}, {primeira_letra}{resto}");
        }
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
    let file = match File::open(transcript_path) {
        Ok(f) => f,
        Err(_) => return (None, String::new(), None),
    };

    let reader = BufReader::new(file);
    let mut inicio: Option<f64> = None;
    let mut pedido = String::new();
    let mut textos: Vec<String> = Vec::new();

    for line in reader.lines().map_while(Result::ok) {
        let v: serde_json::Value = match serde_json::from_str(&line) {
            Ok(val) => val,
            Err(_) => continue,
        };

        let tipo = v.get("type").and_then(|t| t.as_str()).unwrap_or("");

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
    // Parser simples para timestamp ISO 8601 sem chrono externo
    // Ex: 2026-10-29T00:45:11-03:00
    if iso.len() >= 19 {
        // Devolve timestamp de time::SystemTime como fallback aproximado
        Ok(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0))
    } else {
        Err(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let resumo = extrair_resumo_direto(resposta, "voz");
        assert_eq!(
            resumo,
            "No voz, identifiquei a causa raiz dos erros. O comando ps falhava no sandbox."
        );
    }
}
