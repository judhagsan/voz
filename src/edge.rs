//! Síntese de voz com Edge-TTS (redes neurais gratuitas de alta fidelidade).

use std::fs;
use std::path::Path;
use std::process::Command;

pub const VOZ_PADRAO: &str = "pt-BR-ThalitaMultilingualNeural";

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub voz: String,
    pub velocidade: String,
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
            voz: campo("voz", VOZ_PADRAO),
            velocidade: campo("velocidade", "+0%"),
        }
    }
}

pub fn sintetizar_e_tocar(base: &Path, cfg: &Config, texto: &str) -> Result<(), String> {
    let id_processo = std::process::id();
    let agora = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let temp_arquivo = std::env::temp_dir().join(format!("voz_edge_{id_processo}_{agora}.mp3"));

    let executar = |arquivo: &Path| -> Result<std::process::ExitStatus, std::io::Error> {
        let mut cmd = Command::new("edge-tts");
        cmd.args([
            "--no-progress",
            "--voice",
            &cfg.voz,
            "--rate",
            &cfg.velocidade,
            "--text",
            texto,
            "--write-media",
        ])
        .arg(arquivo);

        cmd.status().or_else(|_| {
            let mut cmd_py = Command::new("python3");
            cmd_py
                .args([
                    "-m",
                    "edge_tts",
                    "--no-progress",
                    "--voice",
                    &cfg.voz,
                    "--rate",
                    &cfg.velocidade,
                    "--text",
                    texto,
                    "--write-media",
                ])
                .arg(arquivo)
                .status()
        })
    };

    let mut status =
        executar(&temp_arquivo).map_err(|e| format!("falha ao executar edge-tts: {e}"))?;

    // Tenta uma segunda vez com pequeno atraso se houver instabilidade momentânea na conexão
    if !status.success() {
        std::thread::sleep(std::time::Duration::from_millis(250));
        status = executar(&temp_arquivo)
            .map_err(|e| format!("falha na segunda tentativa do edge-tts: {e}"))?;
    }

    if !status.success() {
        let _ = fs::remove_file(&temp_arquivo);
        return Err(format!("edge-tts finalizou com código {status}"));
    }

    if !temp_arquivo.exists() || fs::metadata(&temp_arquivo).map(|m| m.len()).unwrap_or(0) == 0 {
        let _ = fs::remove_file(&temp_arquivo);
        return Err("edge-tts não gerou nenhum áudio".to_string());
    }

    crate::audio::tocar(base, &temp_arquivo);
    let _ = fs::remove_file(&temp_arquivo);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_edge_padrao() {
        let cfg = Config::de_json(None);
        assert_eq!(cfg.voz, VOZ_PADRAO);
        assert_eq!(cfg.velocidade, "+0%");
    }

    #[test]
    fn test_config_edge_personalizado() {
        let json = serde_json::json!({
            "voz": "pt-BR-FranciscaNeural",
            "velocidade": "+10%"
        });
        let cfg = Config::de_json(Some(&json));
        assert_eq!(cfg.voz, "pt-BR-FranciscaNeural");
        assert_eq!(cfg.velocidade, "+10%");
    }
}
