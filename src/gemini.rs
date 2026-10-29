//! Síntese de voz pela API do Gemini, em streaming.
//!
//! A chave fica no Keychain do macOS (serviço `gemini-api-key`), nunca em
//! arquivo do repositório. O áudio chega em pedaços (eventos `step.delta` de
//! um stream SSE) como PCM de 16 bits, mono, a 24 kHz, e cada pedaço é
//! entregue assim que chega, para a fala começar antes de o texto todo estar
//! pronto. O idioma é detectado pelo próprio texto; o sotaque e o tom vêm do
//! campo `estilo`, escrito em linguagem natural.

use base64::Engine;
use std::io::{BufRead, BufReader};
use std::time::Duration;

const ENDPOINT: &str = "https://generativelanguage.googleapis.com/v1beta/interactions";
const SERVICO_CHAVE: &str = "gemini-api-key";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub modelos: Vec<String>,
    pub voz: String,
    pub estilo: String,
    /// Modelo de texto que escreve o resumo da narração.
    pub modelo_resumo: String,
}

impl Config {
    /// Lê o bloco `gemini` do config.json; o que faltar usa o padrão.
    pub fn de_json(bloco: Option<&serde_json::Value>) -> Config {
        let campo = |nome: &str, padrao: &str| {
            bloco
                .and_then(|b| b.get(nome))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(padrao)
                .to_string()
        };
        let modelos = bloco
            .and_then(|b| b.get("modelos"))
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(String::from)
                    .collect::<Vec<_>>()
            })
            .filter(|m| !m.is_empty())
            .or_else(|| {
                bloco
                    .and_then(|b| b.get("modelo"))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.trim().is_empty())
                    .map(|s| vec![s.to_string()])
            })
            .unwrap_or_else(|| {
                vec![
                    "gemini-3.8-flash-tts".to_string(),
                    "gemini-3.8-flash-lite-tts".to_string(),
                    "gemini-3.1-flash-tts-preview".to_string(),
                    "gemini-2.5-flash-preview-tts".to_string(),
                    "gemini-2.5-pro-preview-tts".to_string(),
                ]
            });

        Config {
            modelos,
            voz: campo("voz", "Vindemiatrix"),
            estilo: campo(
                "estilo",
                "Português do Brasil, sotaque brasileiro. Voz feminina calma, natural e profissional.",
            ),
            modelo_resumo: campo("modelo_resumo", "gemini-3.5-flash-lite"),
        }
    }

    #[cfg(test)]
    pub fn modelo_principal(&self) -> &str {
        self.modelos
            .first()
            .map(|s| s.as_str())
            .unwrap_or("gemini-3.8-flash-tts")
    }
}

pub(crate) fn chaves() -> Vec<String> {
    crate::keychain::chaves(SERVICO_CHAVE)
}

pub(crate) fn chave() -> Result<String, String> {
    crate::keychain::ler_chave(SERVICO_CHAVE)
}

pub fn corpo_pedido(modelo: &str, cfg: &Config, texto: &str) -> serde_json::Value {
    let mut conteudo = serde_json::json!({
        "type": "text",
        "text": texto,
    });
    // O campo annotations com speech_metadata só é suportado na família 3.8+
    if modelo.contains("3.8") {
        conteudo["annotations"] = serde_json::json!([
            {"type": "speech_metadata", "style": cfg.estilo}
        ]);
    }
    serde_json::json!({
        "model": modelo,
        "stream": true,
        "input": [{
            "type": "user_input",
            "content": [conteudo],
        }],
        "response_format": {"type": "audio"},
        "generation_config": {"speech_config": [{"voice": cfg.voz}]},
    })
}

/// Lê uma linha do stream e devolve as amostras se ela trouxer um pedaço de
/// áudio; um evento de erro vira `Err`.
pub fn audio_do_evento(linha: &str) -> Result<Option<Vec<i16>>, String> {
    let Some(dados) = linha.strip_prefix("data:") else {
        return Ok(None);
    };
    let Ok(evento) = serde_json::from_str::<serde_json::Value>(dados.trim()) else {
        return Ok(None);
    };
    if let Some(erro) = evento.get("error") {
        let motivo: String = erro.to_string().chars().take(300).collect();
        return Err(format!("erro no stream: {motivo}"));
    }
    let delta = &evento["delta"];
    if evento["event_type"] != "step.delta" || delta["type"] != "audio" {
        return Ok(None);
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(delta["data"].as_str().unwrap_or(""))
        .map_err(|e| format!("áudio inválido: {e}"))?;
    let (pares, _) = bytes.as_chunks::<2>();
    Ok(Some(
        pares.iter().map(|&par| i16::from_le_bytes(par)).collect(),
    ))
}

/// Pede a fala e entrega cada pedaço de áudio a `ao_receber` conforme chega.
/// Se um modelo/chave der erro de cota (429) ou temporário antes de enviar áudio,
/// tenta automaticamente o próximo da cascata.
pub fn sintetizar_em_stream(
    cfg: &Config,
    texto: &str,
    timeout_seg: u64,
    mut ao_receber: impl FnMut(Vec<i16>),
) -> Result<(), String> {
    let chaves = chaves();
    if chaves.is_empty() {
        return Err(format!("sem chave no Keychain ({SERVICO_CHAVE})"));
    }

    let palavras = texto.split_whitespace().count() as u64;
    let timeout = Duration::from_secs(timeout_seg + palavras);
    let mut ultimo_erro = String::new();

    for modelo in &cfg.modelos {
        for chave in &chaves {
            let agente: ureq::Agent = ureq::Agent::config_builder()
                .timeout_global(Some(timeout))
                .http_status_as_error(false)
                .build()
                .into();

            let req_corpo = corpo_pedido(modelo, cfg, texto).to_string();
            let mut resposta = match agente
                .post(ENDPOINT)
                .header("x-goog-api-key", chave)
                .header("Content-Type", "application/json")
                .send(req_corpo)
            {
                Ok(r) => r,
                Err(e) => {
                    ultimo_erro = format!("conexão com {modelo} falhou: {e}");
                    continue;
                }
            };

            let status = resposta.status();
            if !status.is_success() {
                let corpo = resposta.body_mut().read_to_string().unwrap_or_default();
                let motivo: String = corpo.chars().take(300).collect();
                ultimo_erro = format!("HTTP {} em {modelo}: {motivo}", status.as_u16());
                eprintln!(
                    "[voz] {modelo} indisponível (HTTP {}); tentando próximo da cascata.",
                    status.as_u16()
                );
                continue;
            }

            let mut recebeu = false;
            let mut erro_stream = None;
            for linha in BufReader::new(resposta.body_mut().as_reader()).lines() {
                match linha {
                    Ok(l) => match audio_do_evento(&l) {
                        Ok(Some(amostras)) => {
                            recebeu = true;
                            ao_receber(amostras);
                        }
                        Ok(None) => {}
                        Err(e) => {
                            erro_stream = Some(e);
                            break;
                        }
                    },
                    Err(e) => {
                        erro_stream = Some(e.to_string());
                        break;
                    }
                }
            }

            if let Some(e) = erro_stream {
                if recebeu {
                    return Err(format!("stream de {modelo} interrompido: {e}"));
                }
                ultimo_erro = format!("erro no stream de {modelo}: {e}");
                continue;
            }

            if recebeu {
                return Ok(());
            } else {
                ultimo_erro = format!("stream de {modelo} sem áudio");
            }
        }
    }

    Err(if ultimo_erro.is_empty() {
        "nenhum modelo disponível".to_string()
    } else {
        ultimo_erro
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_usa_padrao_no_que_falta() {
        let bloco = serde_json::json!({"voz": "Kore", "estilo": ""});
        let cfg = Config::de_json(Some(&bloco));
        assert_eq!(cfg.voz, "Kore");
        assert_eq!(
            cfg.modelos,
            vec![
                "gemini-3.8-flash-tts",
                "gemini-3.8-flash-lite-tts",
                "gemini-3.1-flash-tts-preview",
                "gemini-2.5-flash-preview-tts",
                "gemini-2.5-pro-preview-tts"
            ]
        );
        assert_eq!(cfg.modelo_resumo, "gemini-3.5-flash-lite");
        assert!(cfg.estilo.contains("Português do Brasil"));
        assert_eq!(Config::de_json(None).voz, "Vindemiatrix");
    }

    #[test]
    fn test_config_aceita_modelo_ou_modelos() {
        let bloco_unico = serde_json::json!({"modelo": "gemini-custom-tts"});
        let cfg_unico = Config::de_json(Some(&bloco_unico));
        assert_eq!(cfg_unico.modelos, vec!["gemini-custom-tts"]);

        let bloco_multi = serde_json::json!({"modelos": ["mod1", "mod2"]});
        let cfg_multi = Config::de_json(Some(&bloco_multi));
        assert_eq!(cfg_multi.modelos, vec!["mod1", "mod2"]);
    }

    #[test]
    fn test_corpo_pedido_pede_stream_com_voz_estilo_e_texto() {
        let cfg = Config::de_json(None);
        let corpo = corpo_pedido(cfg.modelo_principal(), &cfg, "Fiz o commit.");
        assert_eq!(corpo["model"], "gemini-3.8-flash-tts");
        assert_eq!(corpo["stream"], true);
        assert_eq!(corpo["input"][0]["content"][0]["text"], "Fiz o commit.");
        assert_eq!(
            corpo["input"][0]["content"][0]["annotations"][0]["style"],
            cfg.estilo.as_str()
        );
        assert_eq!(
            corpo["generation_config"]["speech_config"][0]["voice"],
            "Vindemiatrix"
        );
    }

    #[test]
    fn test_corpo_pedido_modelos_sem_annotations() {
        let cfg = Config::de_json(None);
        let corpo = corpo_pedido("gemini-2.5-flash-preview-tts", &cfg, "Fiz o commit.");
        assert_eq!(corpo["model"], "gemini-2.5-flash-preview-tts");
        assert!(corpo["input"][0]["content"][0].get("annotations").is_none());
    }

    #[test]
    fn test_audio_do_evento_decodifica_pcm() {
        // 0x0001 e 0x7fff em little-endian.
        let linha = r#"data: {"index":0,"delta":{"mime_type":"audio/l16","data":"AQD/fw==","type":"audio"},"event_type":"step.delta"}"#;
        assert_eq!(audio_do_evento(linha), Ok(Some(vec![1, 32767])));
    }

    #[test]
    fn test_audio_do_evento_ignora_o_que_nao_e_audio() {
        for linha in [
            "event: step.delta",
            "",
            r#"data: {"index":0,"step":{"type":"model_output"},"event_type":"step.start"}"#,
            r#"data: {"delta":{"type":"text","text":"x"},"event_type":"step.delta"}"#,
            "data: [DONE]",
        ] {
            assert_eq!(audio_do_evento(linha), Ok(None), "{linha}");
        }
    }

    #[test]
    fn test_audio_do_evento_erro_vira_err() {
        let linha = r#"data: {"error":{"code":429,"message":"quota"}}"#;
        assert!(audio_do_evento(linha).is_err());
    }

    /// Fala com a API de verdade (precisa da chave e de rede), sem tocar nada.
    #[test]
    #[ignore]
    fn test_stream_de_verdade() {
        let mut amostras = 0;
        let mut pedacos = 0;
        sintetizar_em_stream(&Config::de_json(None), "Teste do stream.", 30, |p| {
            amostras += p.len();
            pedacos += 1;
        })
        .unwrap();
        println!(
            "{pedacos} pedaços, {:.2} s de áudio",
            amostras as f64 / 24_000.0
        );
        assert!(amostras > 24_000 / 2);
    }
}
