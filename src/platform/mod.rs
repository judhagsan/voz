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
}
