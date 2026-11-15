use std::fs;
use std::path::Path;
use std::process::Command;

pub const VOZ_RESERVA_PADRAO: &str = "pt-BR";

pub fn falar_reserva(_voz: &str, texto: &str) {
    let script = format!(
        "Add-Type -AssemblyName System.Speech; $s = New-Object System.Speech.Synthesis.SpeechSynthesizer; $s.Speak('{}');",
        texto.replace('\'', "''")
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status();
}

pub fn com_trava<F, R>(_base: &Path, reproduzir: F) -> R
where
    F: FnOnce() -> R,
{
    reproduzir()
}

pub struct TravaExclusiva {
    #[allow(dead_code)]
    arquivo: fs::File,
}

pub fn tentar_trava_exclusiva(caminho: &Path) -> Option<TravaExclusiva> {
    let f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(caminho)
        .ok()?;
    Some(TravaExclusiva { arquivo: f })
}

pub fn tamanho_terminal() -> Option<(usize, usize)> {
    if let (Ok(cols), Ok(lines)) = (std::env::var("COLUMNS"), std::env::var("LINES")) {
        if let (Ok(c), Ok(l)) = (cols.parse::<usize>(), lines.parse::<usize>()) {
            if c > 0 && l > 0 {
                return Some((c, l));
            }
        }
    }
    None
}

pub fn instalar_tratamento_sinais() {}
