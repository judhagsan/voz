use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::transcript::{extrair_resumo_direto, turno_codex};

pub fn nome_do_projeto(cwd: Option<&Path>) -> String {
    let dir = match cwd {
        Some(d) if d.is_dir() => d,
        _ => return "geral".to_string(),
    };

    if let Ok(output) = Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "remote", "get-url", "origin"])
        .output()
    {
        if output.status.success() {
            let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !url.is_empty() {
                let stem = Path::new(url.trim_end_matches('/')).file_stem();
                if let Some(s) = stem.and_then(|st| st.to_str()) {
                    return s.to_string();
                }
            }
        }
    }

    if let Ok(output) = Command::new("git")
        .args(["-C", &dir.to_string_lossy(), "rev-parse", "--show-toplevel"])
        .output()
    {
        if output.status.success() {
            let raiz = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if let Some(nome) = Path::new(&raiz).file_name().and_then(|f| f.to_str()) {
                return nome.to_string();
            }
        }
    }

    dir.file_name()
        .and_then(|f| f.to_str())
        .unwrap_or("geral")
        .to_string()
}

/// Tamanho máximo da fala da narração (`narrar.palavras`, padrão 40).
fn palavras_da_fala(cfg: &serde_json::Value) -> usize {
    cfg.pointer("/narrar/palavras")
        .and_then(|m| m.as_u64())
        .filter(|&n| n > 0)
        .unwrap_or(40) as usize
}

/// Escreve a fala da narração: um resumo do Gemini; sem chave ou sem rede, o
/// começo da resposta, cortado no mesmo tamanho.
pub fn montar_fala(base: &Path, projeto: &str, pedido: &str, resposta: &str) -> String {
    let cfg = crate::fala::ler_config(base);
    let palavras = palavras_da_fala(&cfg);
    let gemini = crate::gemini::Config::de_json(cfg.get("gemini"));
    match crate::resumo::resumir(&gemini, palavras, projeto, pedido, resposta, 20) {
        Ok(fala) => fala,
        Err(e) => {
            if std::env::var("VOZ_DEBUG").is_ok() {
                eprintln!("[voz] resumo indisponível ({e}); lendo o começo da resposta.");
            }
            extrair_resumo_direto(resposta, projeto, palavras)
        }
    }
}

fn dados_do_turno(
    val: &serde_json::Value,
    transcript_override: Option<&Path>,
) -> (Option<f64>, String, Option<String>) {
    let transcript = transcript_override.map(PathBuf::from).or_else(|| {
        val.get("transcript_path")
            .or_else(|| val.get("transcriptPath"))
            .and_then(|t| t.as_str())
            .map(PathBuf::from)
    });
    let (inicio, pedido, resposta) = transcript
        .as_deref()
        .map(|p| turno_codex(p, val.get("turn_id").and_then(|t| t.as_str())))
        .unwrap_or_default();
    // O Stop do Codex traz a resposta estável no próprio evento; a narração
    // continua funcionando quando o transcript ainda não foi gravado.
    let resposta = val
        .get("last_assistant_message")
        .and_then(|r| r.as_str())
        .filter(|r| !r.trim().is_empty())
        .map(String::from)
        .or(resposta);
    (inicio, pedido, resposta)
}

pub fn tentar_narrar(
    base: &Path,
    entrada_json: &str,
    transcript_override: Option<&Path>,
    cwd_override: Option<&Path>,
) -> bool {
    let cfg_path = base.join("config.json");
    let mut narrar_ativo = true;
    let mut min_segundos = 0.0;

    if cfg_path.is_file() {
        if let Ok(conteudo) = fs::read_to_string(&cfg_path) {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&conteudo) {
                if let Some(narrar_obj) = v.get("narrar") {
                    narrar_ativo = narrar_obj
                        .get("ativo")
                        .and_then(|a| a.as_bool())
                        .unwrap_or(true);
                    min_segundos = narrar_obj
                        .get("min_segundos")
                        .and_then(|m| m.as_f64())
                        .unwrap_or(0.0);
                }
            }
        }
    }

    if !narrar_ativo {
        return false;
    }

    let val: serde_json::Value =
        serde_json::from_str(entrada_json).unwrap_or(serde_json::Value::Null);

    let cwd_path = cwd_override.map(PathBuf::from).or_else(|| {
        val.get("cwd")
            .and_then(|c| c.as_str())
            .map(PathBuf::from)
            .or_else(|| {
                val.get("workspacePaths")
                    .and_then(|w| w.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|v| v.as_str())
                    .map(PathBuf::from)
            })
    });

    let (inicio, pedido, resposta) = dados_do_turno(&val, transcript_override);

    if min_segundos > 0.0 {
        if let Some(ini) = inicio {
            let agora = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            if (agora - ini) < min_segundos {
                return false;
            }
        }
    }

    let resposta_texto = match resposta {
        Some(r) if !r.trim().is_empty() => r,
        _ => return false,
    };

    let projeto = nome_do_projeto(cwd_path.as_deref());
    let fala = montar_fala(base, &projeto, &pedido, &resposta_texto);
    if fala.trim().is_empty() {
        return false;
    }

    crate::fala::falar(base, &fala, 40);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_narra_resposta_do_hook_sem_transcript() {
        for caminho in [
            serde_json::Value::Null,
            serde_json::json!("/nao-existe-voz.jsonl"),
        ] {
            let (inicio, pedido, resposta) = dados_do_turno(
                &serde_json::json!({
                    "transcript_path": caminho,
                    "last_assistant_message": "Concluí a integração com o Codex."
                }),
                None,
            );
            assert_eq!(inicio, None);
            assert!(pedido.is_empty());
            assert_eq!(
                resposta.as_deref(),
                Some("Concluí a integração com o Codex.")
            );
        }
    }
}
