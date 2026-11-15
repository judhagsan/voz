//! Reprodução em fila: avisos de agentes diferentes que chegam juntos tocam um
//! depois do outro, sem se atropelar.

use std::path::Path;
use std::process::Command;

/// Roda `reproduzir` segurando a trava global de reprodução.
fn com_trava<F, R>(base: &Path, reproduzir: F) -> R
where
    F: FnOnce() -> R,
{
    crate::platform::com_trava(base, reproduzir)
}

pub fn tocar(base: &Path, arquivo: &Path) {
    com_trava(base, || {
        #[cfg(target_os = "macos")]
        let _ = Command::new("afplay").arg(arquivo).status();
        #[cfg(target_os = "linux")]
        {
            let _ = Command::new("pw-play")
                .arg(arquivo)
                .status()
                .or_else(|_| Command::new("paplay").arg(arquivo).status())
                .or_else(|_| Command::new("aplay").arg(arquivo).status());
        }
        #[cfg(target_os = "windows")]
        {
            let script = format!(
                "(New-Object Media.SoundPlayer '{}').PlaySync()",
                arquivo.display()
            );
            let _ = Command::new("powershell")
                .args(["-NoProfile", "-Command", &script])
                .status();
        }
    });
}

/// Voz de reserva do sistema, para quando a síntese principal não responde.
pub fn dizer(base: &Path, texto: &str) {
    let voz_reserva = crate::fala::ler_config(base)
        .pointer("/reserva/voz")
        .and_then(|v| v.as_str())
        .unwrap_or(crate::platform::VOZ_RESERVA_PADRAO)
        .to_string();
    com_trava(base, || {
        crate::platform::falar_reserva(&voz_reserva, texto);
    });
}
