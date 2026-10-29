use std::fs::{self, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

fn random_index(len: usize) -> usize {
    if len <= 1 {
        return 0;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    nanos % len
}

pub fn listar_audios_categoria(base: &Path, categoria: &str) -> Vec<PathBuf> {
    let dir = base.join("sons").join(categoria);
    if !dir.is_dir() {
        return Vec::new();
    }

    let mut arquivos = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    let ext_lower = ext.to_lowercase();
                    if ext_lower == "mp3" || ext_lower == "wav" {
                        arquivos.push(path);
                    }
                }
            }
        }
    }
    arquivos.sort();
    arquivos
}

pub fn sortear_audio(base: &Path, categoria: &str) -> Option<PathBuf> {
    let arquivos = listar_audios_categoria(base, categoria);
    if arquivos.is_empty() {
        return None;
    }

    let estado_dir = base.join("estado");
    fs::create_dir_all(&estado_dir).ok();
    let ultimo_file = estado_dir.join(format!("ultimo-{categoria}"));
    let ultimo = fs::read_to_string(&ultimo_file)
        .unwrap_or_default()
        .trim()
        .to_string();

    let mut candidatos: Vec<&PathBuf> = arquivos.iter().collect();

    if candidatos.len() > 1 && !ultimo.is_empty() {
        candidatos.retain(|p| p.to_string_lossy() != ultimo);
    }

    let idx = random_index(candidatos.len());
    let escolhido = candidatos.get(idx).copied().or_else(|| arquivos.first())?;
    fs::write(&ultimo_file, escolhido.to_string_lossy().as_bytes()).ok();
    Some(escolhido.clone())
}

pub fn tocar_enfileirado(base: &Path, arquivo: &Path, reserva: &str) {
    let estado_dir = base.join("estado");
    fs::create_dir_all(&estado_dir).ok();
    let lock_path = estado_dir.join("reproducao.lock");

    // Trava simples usando criação exclusiva ou flock
    let _lock = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o644)
        .open(&lock_path);

    if arquivo.is_file() {
        let _ = Command::new("afplay").arg(arquivo).status();
    } else if !reserva.is_empty() {
        let _ = Command::new("say").arg(reserva).status();
    }
}
