use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process::Command;

pub const VOZ_RESERVA_PADRAO: &str = "Voz 2";

pub fn ler_segredo(servico: &str) -> Result<String, String> {
    let saida = Command::new("security")
        .args(["find-generic-password", "-s", servico, "-w"])
        .output()
        .map_err(|e| e.to_string())?;
    let chave = String::from_utf8_lossy(&saida.stdout).trim().to_string();
    if chave.is_empty() {
        return Err(format!("sem chave no Keychain ({servico})"));
    }
    Ok(chave)
}

pub fn falar_reserva(voz: &str, texto: &str) {
    let status = Command::new("say").args(["-v", voz, texto]).status();
    if status.map(|s| !s.success()).unwrap_or(true) {
        let _ = Command::new("say").arg(texto).status();
    }
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
