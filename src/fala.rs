//! Fala em voz alta. O Gemini manda o áudio em streaming e a fala começa no
//! primeiro pedaço, sem esperar o resto; sem chave, sem rede ou com a cota
//! esgotada, quem fala é o `say` do macOS. Nenhum áudio é gravado em disco.

use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use crate::audio;
use crate::fonetica::{ajustar_fonetica, carregar_regras};
use crate::gemini;

/// Frases usadas quando o config.json não traz as do aviso.
const FRASES_PADRAO: &[(&str, &str)] = &[
    ("terminei", "Terminei."),
    ("permissao", "Preciso da sua permissão."),
    ("espera", "Tô te esperando."),
];

fn ler_config(base: &Path) -> serde_json::Value {
    fs::read_to_string(base.join("config.json"))
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or(serde_json::Value::Null)
}

fn semente() -> usize {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0)
}

/// Escolhe uma frase diferente da última, quando há outra opção.
fn escolher(frases: &[String], ultima: &str, semente: usize) -> String {
    let outras: Vec<&String> = frases.iter().filter(|f| f.as_str() != ultima).collect();
    let candidatas: Vec<&String> = if outras.is_empty() {
        frases.iter().collect()
    } else {
        outras
    };
    candidatas[semente % candidatas.len()].clone()
}

/// Sorteia a frase do aviso entre as do `frases` do config.json.
pub fn sortear_frase(base: &Path, categoria: &str) -> Option<String> {
    let cfg = ler_config(base);
    let mut frases: Vec<String> = cfg
        .pointer(&format!("/frases/{categoria}"))
        .and_then(|f| f.as_array())
        .into_iter()
        .flatten()
        .filter_map(|f| f.as_str())
        .filter(|f| !f.trim().is_empty())
        .map(String::from)
        .collect();
    if frases.is_empty() {
        let (_, padrao) = FRASES_PADRAO.iter().find(|(c, _)| *c == categoria)?;
        frases.push(padrao.to_string());
    }

    let estado = base.join("estado");
    fs::create_dir_all(&estado).ok();
    let arquivo_ultima = estado.join(format!("ultimo-{categoria}"));
    let ultima = fs::read_to_string(&arquivo_ultima).unwrap_or_default();
    let escolhida = escolher(&frases, ultima.trim(), semente());
    fs::write(&arquivo_ultima, &escolhida).ok();
    Some(escolhida)
}

pub fn falar(base: &Path, texto: &str, timeout_seg: u64) {
    let cfg = gemini::Config::de_json(ler_config(base).get("gemini"));
    let para_gemini = ajustar_fonetica(texto, &carregar_regras(base, false));

    // O tocador roda em paralelo e começa assim que o primeiro pedaço chega.
    let (envia, recebe) = mpsc::channel();
    let base_tocador = base.to_path_buf();
    let tocador = thread::spawn(move || audio::tocar_stream(&base_tocador, recebe));
    let resultado = gemini::sintetizar_em_stream(&cfg, &para_gemini, timeout_seg, |pedaco| {
        let _ = envia.send(pedaco);
    });
    drop(envia);
    let tocou = tocador.join().unwrap_or(false);

    match resultado {
        Ok(()) if tocou => return,
        // Parou no meio: repetir a frase inteira no `say` soaria pior que o corte.
        Err(e) if tocou => {
            eprintln!("[voz] o stream do Gemini parou no meio ({e}).");
            return;
        }
        Err(e) => eprintln!("[voz] Gemini indisponível ({e}); usando a voz de reserva."),
        Ok(()) => eprintln!("[voz] saída de áudio indisponível; usando a voz de reserva."),
    }
    audio::dizer(base, &ajustar_fonetica(texto, &carregar_regras(base, true)));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frases(lista: &[&str]) -> Vec<String> {
        lista.iter().map(|f| f.to_string()).collect()
    }

    #[test]
    fn test_escolher_evita_repetir_a_ultima() {
        let lista = frases(&["a", "b"]);
        for semente in 0..10 {
            assert_eq!(escolher(&lista, "a", semente), "b");
        }
    }

    #[test]
    fn test_escolher_com_uma_frase_repete() {
        assert_eq!(escolher(&frases(&["a"]), "a", 7), "a");
    }

    #[test]
    fn test_sortear_frase_usa_config_e_padrao() {
        let pasta = std::env::temp_dir().join(format!("voz-fala-{}", std::process::id()));
        fs::create_dir_all(&pasta).unwrap();
        fs::write(
            pasta.join("config.json"),
            r#"{"frases": {"espera": ["Tô aqui."]}}"#,
        )
        .unwrap();
        assert_eq!(sortear_frase(&pasta, "espera").as_deref(), Some("Tô aqui."));
        assert_eq!(
            sortear_frase(&pasta, "permissao").as_deref(),
            Some("Preciso da sua permissão.")
        );
        assert_eq!(sortear_frase(&pasta, "outra"), None);
        let _ = fs::remove_dir_all(&pasta);
    }
}
