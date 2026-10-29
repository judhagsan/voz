//! Síntese de voz pela API do ElevenLabs em streaming PCM 24 kHz.

use std::io::Read;
use std::time::Duration;

use crate::keychain;

pub const SERVICO_CHAVE: &str = "elevenlabs-api-key";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub modelo: String,
    pub voz_id: String,
}

impl Config {
    pub fn de_json(bloco: Option<&serde_json::Value>) -> Config {
        let campo = |nome: &str, padrao: &str| {
            bloco
                .and_then(|b| b.get(nome))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(padrao)
                .to_string()
        };
        Config {
            modelo: campo("modelo", "eleven_turbo_v2_5"),
            voz_id: campo("voz_id", "21m00Tcm4TlvDq8ikWAM"),
        }
    }
}

pub fn corpo_pedido(cfg: &Config, texto: &str) -> serde_json::Value {
    serde_json::json!({
        "text": texto,
        "model_id": cfg.modelo,
    })
}

pub fn sintetizar_em_stream(
    cfg: &Config,
    texto: &str,
    timeout_seg: u64,
    mut ao_receber: impl FnMut(Vec<i16>),
) -> Result<(), String> {
    let chaves = keychain::chaves(SERVICO_CHAVE);
    if chaves.is_empty() {
        return Err(format!("sem chave no Keychain ({SERVICO_CHAVE})"));
    }

    let endpoint = format!(
        "https://api.elevenlabs.io/v1/text-to-speech/{}/stream?output_format=pcm_24000",
        cfg.voz_id
    );

    let palavras = texto.split_whitespace().count() as u64;
    let timeout = Duration::from_secs(timeout_seg + palavras);
    let mut ultimo_erro = String::new();

    for chave in &chaves {
        let agente: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(timeout))
            .http_status_as_error(false)
            .build()
            .into();

        let req_corpo = corpo_pedido(cfg, texto).to_string();
        let mut resposta = match agente
            .post(&endpoint)
            .header("xi-api-key", chave)
            .header("Content-Type", "application/json")
            .send(req_corpo)
        {
            Ok(r) => r,
            Err(e) => {
                ultimo_erro = format!("conexão com ElevenLabs falhou: {e}");
                continue;
            }
        };

        let status = resposta.status();
        if !status.is_success() {
            let corpo = resposta.body_mut().read_to_string().unwrap_or_default();
            let motivo: String = corpo.chars().take(300).collect();
            ultimo_erro = format!("HTTP {} no ElevenLabs: {motivo}", status.as_u16());
            eprintln!(
                "[voz] ElevenLabs indisponível (HTTP {}); tentando próximo da cascata.",
                status.as_u16()
            );
            continue;
        }

        // Lê pedaços do PCM cru de 24 kHz (16 bits little-endian, mono)
        let mut leitor = resposta.body_mut().as_reader();
        let mut buffer = [0u8; 2400]; // 1200 amostras (50 ms a 24 kHz)
        let mut recebeu = false;
        let mut erro_leitura = None;

        loop {
            match leitor.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let valido = n - (n % 2);
                    if valido > 0 {
                        recebeu = true;
                        let (pares, _) = buffer[..valido].as_chunks::<2>();
                        let amostras: Vec<i16> = pares
                            .iter()
                            .map(|par| i16::from_le_bytes([par[0], par[1]]))
                            .collect();
                        ao_receber(amostras);
                    }
                }
                Err(e) => {
                    erro_leitura = Some(e.to_string());
                    break;
                }
            }
        }

        if let Some(e) = erro_leitura {
            if recebeu {
                return Err(format!("stream do ElevenLabs interrompido: {e}"));
            }
            ultimo_erro = format!("erro no stream do ElevenLabs: {e}");
            continue;
        }

        if recebeu {
            return Ok(());
        } else {
            ultimo_erro = "ElevenLabs não devolveu áudio".to_string();
        }
    }

    Err(if ultimo_erro.is_empty() {
        "nenhuma chave do ElevenLabs disponível".to_string()
    } else {
        ultimo_erro
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_elevenlabs_padrao() {
        let cfg = Config::de_json(None);
        assert_eq!(cfg.modelo, "eleven_turbo_v2_5");
        assert_eq!(cfg.voz_id, "21m00Tcm4TlvDq8ikWAM");
    }

    #[test]
    fn test_corpo_pedido_elevenlabs() {
        let cfg = Config::de_json(None);
        let corpo = corpo_pedido(&cfg, "Olá mundo.");
        assert_eq!(corpo["text"], "Olá mundo.");
        assert_eq!(corpo["model_id"], "eleven_turbo_v2_5");
    }
}
