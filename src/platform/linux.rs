use std::fs::{self, File};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process::Command;

pub const VOZ_RESERVA_PADRAO: &str = "pt-br";

pub fn ler_segredo(servico: &str) -> Result<String, String> {
    // 1. Variável de ambiente (ex: GEMINI_API_KEY)
    let env_var = servico.to_uppercase().replace('-', "_");
    if let Ok(val) = std::env::var(&env_var) {
        let trimmed = val.trim().to_string();
        if !trimmed.is_empty() {
            return Ok(trimmed);
        }
    }

    // 2. Secret Service API via secret-tool (GNOME Keyring / KWallet)
    if let Ok(saida) = Command::new("secret-tool")
        .args(["lookup", "service", servico])
        .output()
    {
        if saida.status.success() {
            let chave = String::from_utf8_lossy(&saida.stdout).trim().to_string();
            if !chave.is_empty() {
                return Ok(chave);
            }
        }
    }

    // 3. Arquivo de credenciais seguro em ~/.config/voz/credentials ou ~/.config/voz/<servico>
    if let Some(home) = super::pasta_usuario() {
        let conf_dir = home.join(".config").join("voz");
        let arquivo_servico = conf_dir.join(servico);
        if let Ok(conteudo) = fs::read_to_string(&arquivo_servico) {
            let chave = conteudo.trim().to_string();
            if !chave.is_empty() {
                return Ok(chave);
            }
        }
        let arquivo_geral = conf_dir.join("credentials");
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

    Err(format!("sem chave configurada para {servico}"))
}

pub fn falar_reserva(voz: &str, texto: &str) {
    let status_spd = Command::new("spd-say").args(["-l", voz, texto]).status();
    if status_spd.map(|s| s.success()).unwrap_or(false) {
        return;
    }

    let status_espeak = Command::new("espeak-ng").args(["-v", voz, texto]).status();
    if status_espeak.map(|s| s.success()).unwrap_or(false) {
        return;
    }

    let _ = Command::new("espeak").args(["-v", voz, texto]).status();
}

pub fn com_trava<F, R>(base: &Path, reproduzir: F) -> R
where
    F: FnOnce() -> R,
{
    let caminho_trava = base.join("trava-fala.lock");
    match File::create(&caminho_trava) {
        Ok(trava) => {
            unsafe { libc::flock(trava.as_raw_fd(), libc::LOCK_EX) };
            reproduzir()
        }
        Err(_) => reproduzir(),
    }
}

pub fn tamanho_terminal() -> Option<(usize, usize)> {
    unsafe {
        let mut ws = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if libc::ioctl(
            libc::STDOUT_FILENO,
            libc::TIOCGWINSZ,
            &mut ws as *mut libc::winsize,
        ) == 0
            && ws.ws_col > 0
            && ws.ws_row > 0
        {
            Some((ws.ws_col as usize, ws.ws_row as usize))
        } else {
            None
        }
    }
}

extern "C" fn restaurar_e_sair(sinal: libc::c_int) {
    let _ = sinal;
    let seq = b"\x1b[?25h\x1b[0m";
    unsafe {
        libc::write(
            libc::STDOUT_FILENO,
            seq.as_ptr() as *const libc::c_void,
            seq.len(),
        );
        libc::_exit(128 + sinal);
    }
}

pub fn instalar_tratamento_sinais() {
    for sinal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        unsafe {
            libc::signal(
                sinal,
                restaurar_e_sair as extern "C" fn(libc::c_int) as libc::sighandler_t,
            );
        }
    }
}
