use std::fs;
use std::path::Path;
use std::process::Command;

pub const VOZ_RESERVA_PADRAO: &str = "pt-BR";

pub fn ler_segredo(servico: &str) -> Result<String, String> {
    // 1. Variável de ambiente (ex: GEMINI_API_KEY)
    let env_var = servico.to_uppercase().replace('-', "_");
    if let Ok(val) = std::env::var(&env_var) {
        let trimmed = val.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }

    // 2. Arquivo de credenciais em %APPDATA%\voz\credentials ou %USERPROFILE%\.config\voz\credentials
    if let Some(home) = super::pasta_usuario() {
        for pasta in [
            std::env::var("APPDATA").ok().map(std::path::PathBuf::from),
            Some(home.join(".config").join("voz")),
            Some(home.join("voz")),
        ]
        .into_iter()
        .flatten()
        {
            let arquivo_servico = pasta.join(servico);
            if let Ok(conteudo) = fs::read_to_string(&arquivo_servico) {
                let chave = conteudo.trim().to_string();
                if !chave.is_empty() {
                    return Ok(chave);
                }
            }
            let arquivo_geral = pasta.join("credentials");
            if let Ok(conteudo) = fs::read_to_string(&arquivo_geral) {
                for linha in conteudo.lines() {
                    if let Some((chave, valor)) = linha.split_once('=') {
                        if chave.trim() == servico {
                            let v = valor.trim().to_string();
                            if !v.is_empty() {
                                return Ok(v);
                            }
                        }
                    }
                }
            }
        }
    }

    Err(format!("sem chave configurada para {servico}"))
}

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
