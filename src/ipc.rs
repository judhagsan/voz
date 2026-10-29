use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

pub fn sintetizar_uds(
    socket_path: &Path,
    texto: &str,
    timeout_secs: u64,
) -> Result<Vec<u8>, String> {
    if !socket_path.exists() {
        return Err(format!(
            "Socket não encontrado em {}",
            socket_path.display()
        ));
    }

    let mut stream = UnixStream::connect(socket_path).map_err(|e| {
        format!(
            "Falha ao conectar no socket {}: {}",
            socket_path.display(),
            e
        )
    })?;

    let timeout = Some(Duration::from_secs(timeout_secs));
    stream.set_read_timeout(timeout).ok();
    stream.set_write_timeout(timeout).ok();

    let payload = serde_json::json!({
        "texto": texto,
    });
    let mut req_str = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
    req_str.push('\n');

    stream
        .write_all(req_str.as_bytes())
        .map_err(|e| format!("Falha ao enviar requisição via socket: {e}"))?;

    let mut tam_buf = [0u8; 4];
    stream
        .read_exact(&mut tam_buf)
        .map_err(|e| format!("Falha ao ler cabeçalho de resposta: {e}"))?;

    let tam = u32::from_be_bytes(tam_buf) as usize;
    if tam == 0 {
        return Err("O motor de voz retornou tamanho de áudio zero (erro na síntese)".to_string());
    }

    let mut wav_bytes = vec![0u8; tam];
    stream
        .read_exact(&mut wav_bytes)
        .map_err(|e| format!("Falha ao receber bytes do WAV (esperados {tam}): {e}"))?;

    Ok(wav_bytes)
}
