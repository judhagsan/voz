//! Reprodução em fila: avisos de agentes diferentes que chegam juntos tocam um
//! depois do outro, sem se atropelar.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::fs::{self, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Taxa do PCM que o Gemini devolve.
const TAXA_ORIGEM: f64 = 24_000.0;

/// Roda `reproduzir` segurando a trava global de reprodução.
fn com_trava(base: &Path, reproduzir: impl FnOnce()) {
    let estado = base.join("estado");
    fs::create_dir_all(&estado).ok();
    match OpenOptions::new()
        .create(true)
        .append(true)
        .open(estado.join("reproducao.lock"))
    {
        Ok(trava) => {
            // SAFETY: flock só usa o descritor, válido enquanto `trava` existir;
            // a trava cai quando o arquivo é fechado no fim do bloco.
            unsafe { libc::flock(trava.as_raw_fd(), libc::LOCK_EX) };
            reproduzir();
        }
        Err(_) => reproduzir(),
    }
}

pub fn tocar(base: &Path, arquivo: &Path) {
    com_trava(base, || {
        let _ = Command::new("afplay").arg(arquivo).status();
    });
}

/// Voz de reserva do macOS, para quando nenhuma API de IA responde.
/// Usa a voz neural nativa da Siri (Voz 2) do macOS 27.
pub fn dizer(base: &Path, texto: &str) {
    let voz_reserva = crate::fala::ler_config(base)
        .pointer("/reserva/voz")
        .and_then(|v| v.as_str())
        .unwrap_or("Voz 2")
        .to_string();
    com_trava(base, || {
        let status = Command::new("say")
            .args(["-v", &voz_reserva, texto])
            .status();
        if status.map(|s| !s.success()).unwrap_or(true) {
            let _ = Command::new("say").arg(texto).status();
        }
    });
}

/// Toca o PCM conforme os pedaços chegam pelo canal; devolve se tocou algo.
///
/// Só pega a trava e abre a saída depois do primeiro pedaço: se o Gemini
/// falhar antes, nada disso acontece e a voz de reserva assume.
pub fn tocar_stream(base: &Path, pedacos: Receiver<Vec<i16>>) -> bool {
    let Ok(primeiro) = pedacos.recv() else {
        return false;
    };
    let mut tocou = false;
    com_trava(base, || match tocar_pcm(primeiro, pedacos) {
        Ok(()) => tocou = true,
        Err(e) => eprintln!("[voz] erro na saída de áudio: {e}"),
    });
    tocou
}

#[derive(Default)]
struct Fila {
    amostras: VecDeque<f32>,
    terminou: bool,
}

fn para_f32(pcm: &[i16]) -> impl Iterator<Item = f32> + '_ {
    pcm.iter().map(|&s| f32::from(s) / 32768.0)
}

/// Preenche o buffer da saída com a fila, convertendo 24 kHz para a taxa do
/// dispositivo por interpolação linear (`passo` = origem / destino). Sem dados
/// ainda, solta silêncio e espera. Devolve true quando a fila acabou de vez.
fn preencher(fila: &mut Fila, pos: &mut f64, passo: f64, canais: usize, saida: &mut [f32]) -> bool {
    for quadro in saida.chunks_mut(canais) {
        let i = *pos as usize;
        let valor = match (fila.amostras.get(i), fila.amostras.get(i + 1)) {
            (Some(&a), Some(&b)) => {
                let fracao = (*pos - i as f64) as f32;
                *pos += passo;
                a + (b - a) * fracao
            }
            _ => 0.0,
        };
        quadro.fill(valor);
    }
    let tocadas = (*pos as usize).min(fila.amostras.len());
    fila.amostras.drain(..tocadas);
    *pos -= tocadas as f64;
    fila.terminou && fila.amostras.len() <= 1
}

fn tocar_pcm(primeiro: Vec<i16>, resto: Receiver<Vec<i16>>) -> Result<(), String> {
    let dispositivo = cpal::default_host()
        .default_output_device()
        .ok_or("sem saída de áudio")?;
    let suportada = dispositivo
        .default_output_config()
        .map_err(|e| e.to_string())?;
    let config = suportada.config();
    let canais = usize::from(config.channels);
    let passo = TAXA_ORIGEM / f64::from(config.sample_rate.0);

    let fila = Arc::new(Mutex::new(Fila {
        amostras: para_f32(&primeiro).collect(),
        terminou: false,
    }));
    let (avisa_fim, fim) = mpsc::channel::<()>();
    let fila_saida = Arc::clone(&fila);
    let mut pos = 0.0;
    let mut avisou = false;
    let mut alimentar = move |saida: &mut [f32]| {
        let mut f = fila_saida.lock().unwrap_or_else(|e| e.into_inner());
        if preencher(&mut f, &mut pos, passo, canais, saida) && !avisou {
            avisou = true;
            let _ = avisa_fim.send(());
        }
    };
    let erro = |e: cpal::StreamError| eprintln!("[voz] erro na saída de áudio: {e}");
    let saida = match suportada.sample_format() {
        cpal::SampleFormat::F32 => dispositivo.build_output_stream(
            &config,
            move |s: &mut [f32], _: &cpal::OutputCallbackInfo| alimentar(s),
            erro,
            None,
        ),
        cpal::SampleFormat::I16 => {
            let mut temporario = Vec::new();
            dispositivo.build_output_stream(
                &config,
                move |s: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    temporario.resize(s.len(), 0.0);
                    alimentar(&mut temporario);
                    for (destino, v) in s.iter_mut().zip(&temporario) {
                        *destino = (v * 32767.0) as i16;
                    }
                },
                erro,
                None,
            )
        }
        outro => return Err(format!("formato de saída {outro:?} não suportado")),
    }
    .map_err(|e| e.to_string())?;
    saida.play().map_err(|e| e.to_string())?;

    for pedaco in resto {
        fila.lock()
            .unwrap_or_else(|e| e.into_inner())
            .amostras
            .extend(para_f32(&pedaco));
    }
    let restante = {
        let mut f = fila.lock().unwrap_or_else(|e| e.into_inner());
        f.terminou = true;
        f.amostras.len() as f64 / TAXA_ORIGEM
    };
    // Espera a fila esvaziar; o prazo é o que falta tocar mais uma folga, só
    // para não travar se a saída de som sumir no meio.
    let _ = fim.recv_timeout(Duration::from_secs_f64(restante + 5.0));
    // Deixa o último buffer sair do dispositivo antes de fechar a saída.
    thread::sleep(Duration::from_millis(150));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fila(amostras: &[f32], terminou: bool) -> Fila {
        Fila {
            amostras: amostras.iter().copied().collect(),
            terminou,
        }
    }

    #[test]
    fn test_preencher_dobra_a_taxa_interpolando() {
        // 24 kHz para 48 kHz: cada amostra de origem vira duas, com o meio-termo.
        let mut f = fila(&[0.0, 1.0, 0.0], false);
        let mut pos = 0.0;
        let mut saida = [9.0; 4];
        let acabou = preencher(&mut f, &mut pos, 0.5, 1, &mut saida);
        assert_eq!(saida, [0.0, 0.5, 1.0, 0.5]);
        assert!(!acabou);
    }

    #[test]
    fn test_preencher_copia_para_todos_os_canais() {
        let mut f = fila(&[0.25, 0.25], false);
        let mut pos = 0.0;
        let mut saida = [0.0; 2];
        preencher(&mut f, &mut pos, 0.5, 2, &mut saida);
        assert_eq!(saida, [0.25, 0.25]);
    }

    #[test]
    fn test_preencher_sem_dados_solta_silencio_e_espera() {
        let mut f = fila(&[], false);
        let mut pos = 0.0;
        let mut saida = [9.0; 3];
        assert!(!preencher(&mut f, &mut pos, 0.5, 1, &mut saida));
        assert_eq!(saida, [0.0; 3]);
        assert_eq!(pos, 0.0);
    }

    #[test]
    fn test_preencher_avisa_quando_acaba() {
        let mut f = fila(&[0.1, 0.2, 0.3], true);
        let mut pos = 0.0;
        let mut saida = [0.0; 8];
        assert!(preencher(&mut f, &mut pos, 1.0, 1, &mut saida));
    }

    #[test]
    fn test_tocar_stream_sem_pedacos_nao_toca() {
        let (envia, recebe) = mpsc::channel::<Vec<i16>>();
        drop(envia);
        assert!(!tocar_stream(Path::new("/nao/existe"), recebe));
    }
}
