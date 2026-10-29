//! Resumo falado da resposta final do agente, escrito por um modelo de texto
//! do Gemini. O pedido e a resposta vão para a API; volta só a fala curta, que
//! segue para a voz.

use std::time::Duration;

const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/models";
/// Corte do que vai para a API: respostas maiores raramente mudam o resumo.
const MAX_CARACTERES_RESPOSTA: usize = 12_000;
const MAX_CARACTERES_PEDIDO: usize = 2_000;

fn instrucao(palavras: usize) -> String {
    format!(
        "Você resume, para ser falado em voz alta, o que um agente de programação acabou de \
         responder ao usuário.\n\
         Regras:\n\
         - português do Brasil, ortografia e acentuação corretas;\n\
         - tom sério e humano, sem exageros, sem onomatopeias, sem emojis;\n\
         - no máximo {palavras} palavras, em uma a três frases curtas;\n\
         - comece dizendo o projeto, exatamente no formato \"No <projeto>, ...\", a não ser que \
         o projeto seja \"geral\";\n\
         - diga o resultado principal; se o agente fez uma pergunta ou espera uma decisão do \
         usuário, termine dizendo isso;\n\
         - não invente nada que não esteja na resposta;\n\
         - escreva como se fala: sem caminhos de arquivo, código, comandos, símbolos, links ou \
         listas.\n\
         Responda só com o texto da fala."
    )
}

fn cortar(texto: &str, max: usize) -> String {
    texto.chars().take(max).collect()
}

pub fn corpo_pedido(
    palavras: usize,
    projeto: &str,
    pedido: &str,
    resposta: &str,
    nivel_raciocinio: &str,
) -> serde_json::Value {
    let conteudo = format!(
        "Projeto: {projeto}\n\nPedido do usuário:\n{}\n\nResposta do agente:\n{}",
        cortar(pedido, MAX_CARACTERES_PEDIDO),
        cortar(resposta, MAX_CARACTERES_RESPOSTA)
    );
    serde_json::json!({
        "systemInstruction": {"parts": [{"text": instrucao(palavras)}]},
        "contents": [{"role": "user", "parts": [{"text": conteudo}]}],
        "generationConfig": {
            "temperature": 0.3,
            "maxOutputTokens": 400,
            "thinkingConfig": {"thinkingLevel": nivel_raciocinio},
        },
    })
}

/// Junta o texto da resposta, sem as partes de raciocínio.
pub fn extrair_texto(resposta: &serde_json::Value) -> Result<String, String> {
    let texto: String = resposta
        .pointer("/candidates/0/content/parts")
        .and_then(|p| p.as_array())
        .into_iter()
        .flatten()
        .filter(|p| p["thought"] != true)
        .filter_map(|p| p["text"].as_str())
        .collect();
    let texto = texto.trim().to_string();
    if texto.is_empty() {
        return Err("resumo vazio".to_string());
    }
    Ok(texto)
}

fn chamar(
    modelo: &str,
    chave: &str,
    corpo: &serde_json::Value,
    timeout_seg: u64,
) -> Result<String, String> {
    let agente: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_seg)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut resposta = agente
        .post(&format!("{ENDPOINT}/{modelo}:generateContent"))
        .header("x-goog-api-key", chave)
        .header("Content-Type", "application/json")
        .send(corpo.to_string())
        .map_err(|e| e.to_string())?;
    let status = resposta.status();
    let corpo = resposta
        .body_mut()
        .read_to_string()
        .map_err(|e| e.to_string())?;
    if !status.is_success() {
        let motivo: String = corpo.chars().take(300).collect();
        return Err(format!("HTTP {}: {motivo}", status.as_u16()));
    }
    let json: serde_json::Value =
        serde_json::from_str(&corpo).map_err(|e| format!("resposta inválida: {e}"))?;
    extrair_texto(&json)
}

pub fn resumir(
    cfg: &crate::gemini::Config,
    palavras: usize,
    projeto: &str,
    pedido: &str,
    resposta: &str,
    timeout_seg: u64,
) -> Result<String, String> {
    let chave = crate::gemini::chave()?;
    let corpo = corpo_pedido(palavras, projeto, pedido, resposta, "minimal");
    match chamar(&cfg.modelo_resumo, &chave, &corpo, timeout_seg) {
        // Nem todo modelo aceita o raciocínio mínimo; o mais curto seguinte é "low".
        Err(e) if e.contains("Thinking level") => {
            let corpo = corpo_pedido(palavras, projeto, pedido, resposta, "low");
            chamar(&cfg.modelo_resumo, &chave, &corpo, timeout_seg)
        }
        outro => outro,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_corpo_pedido_leva_limite_projeto_e_textos() {
        let corpo = corpo_pedido(30, "voz", "faz o teste", "Testei e passou.", "minimal");
        let instrucao = corpo["systemInstruction"]["parts"][0]["text"]
            .as_str()
            .unwrap();
        assert!(instrucao.contains("no máximo 30 palavras"));
        let conteudo = corpo["contents"][0]["parts"][0]["text"].as_str().unwrap();
        assert!(conteudo.starts_with("Projeto: voz"));
        assert!(conteudo.contains("faz o teste") && conteudo.contains("Testei e passou."));
        assert_eq!(
            corpo["generationConfig"]["thinkingConfig"]["thinkingLevel"],
            "minimal"
        );
    }

    #[test]
    fn test_corpo_pedido_corta_resposta_enorme() {
        let enorme = "á".repeat(MAX_CARACTERES_RESPOSTA * 2);
        let corpo = corpo_pedido(40, "voz", "", &enorme, "minimal");
        let conteudo = corpo["contents"][0]["parts"][0]["text"].as_str().unwrap();
        assert!(conteudo.chars().count() < MAX_CARACTERES_RESPOSTA + 200);
    }

    #[test]
    fn test_extrair_texto_ignora_raciocinio() {
        let resposta = serde_json::json!({"candidates": [{"content": {"parts": [
            {"text": "pensando…", "thought": true},
            {"text": " No voz, tudo pronto. "},
        ]}}]});
        assert_eq!(extrair_texto(&resposta).unwrap(), "No voz, tudo pronto.");
    }

    #[test]
    fn test_extrair_texto_vazio_vira_erro() {
        assert!(extrair_texto(&serde_json::json!({"candidates": []})).is_err());
    }
}
