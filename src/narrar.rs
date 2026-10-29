use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::audio::tocar_enfileirado;
use crate::fonetica::{ajustar_fonetica, carregar_regras};
use crate::ipc::sintetizar_uds;
use crate::transcript::{extrair_resumo_direto, ultimo_turno};

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

    let transcript_str = transcript_override
        .map(|p| p.to_string_lossy().to_string())
        .or_else(|| {
            val.get("transcript_path")
                .or_else(|| val.get("transcriptPath"))
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        });

    let transcript_path = match transcript_str {
        Some(t) if Path::new(&t).is_file() => PathBuf::from(t),
        _ => return false,
    };

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

    let (inicio, _pedido, final_resp) = ultimo_turno(&transcript_path);

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

    let resposta = val
        .get("last_assistant_message")
        .and_then(|r| r.as_str())
        .map(|s| s.to_string())
        .or(final_resp);

    let resposta_texto = match resposta {
        Some(r) if !r.trim().is_empty() => r,
        _ => return false,
    };

    let projeto = nome_do_projeto(cwd_path.as_deref());
    let resumo = extrair_resumo_direto(&resposta_texto, &projeto);
    if resumo.trim().is_empty() {
        return false;
    }

    let regras = carregar_regras(base);
    let fala_fonetica = ajustar_fonetica(&resumo, &regras);

    let socket_path = base.join("estado").join("motor.sock");
    let wav_bytes = match sintetizar_uds(&socket_path, &fala_fonetica, 40) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[voz] Erro na síntese via socket: {e}");
            return false;
        }
    };

    let estado_dir = base.join("estado");
    fs::create_dir_all(&estado_dir).ok();
    let temp_wav = estado_dir.join("narracao_temp.wav");
    if fs::write(&temp_wav, wav_bytes).is_err() {
        return false;
    }

    tocar_enfileirado(base, &temp_wav, "Terminei");
    true
}
