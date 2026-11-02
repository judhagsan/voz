//! Abstração de chamadas e primitivas do sistema operacional (macOS, Linux e Windows).

use std::path::PathBuf;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

/// Retorna o diretório base do usuário ($HOME no Unix ou %USERPROFILE% no Windows).
pub fn pasta_usuario() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pasta_usuario_nao_vazia() {
        assert!(pasta_usuario().is_some());
    }

    #[test]
    fn test_tentar_trava_exclusiva() {
        let temp_dir = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let lock_path = temp_dir.join(format!("voz-teste-lock-{}-{nanos}", std::process::id()));

        let trava1 = tentar_trava_exclusiva(&lock_path);
        assert!(trava1.is_some());

        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let trava2 = tentar_trava_exclusiva(&lock_path);
            assert!(trava2.is_none());
        }

        drop(trava1);
        let trava3 = tentar_trava_exclusiva(&lock_path);
        assert!(trava3.is_some());
        drop(trava3);
        let _ = std::fs::remove_file(&lock_path);
    }
}
