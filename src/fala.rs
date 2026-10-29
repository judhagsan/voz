//! Fala em voz alta. O Gemini manda o áudio em streaming e a fala começa no
//! primeiro pedaço, sem esperar o resto; sem chave, sem rede ou com a cota
//! esgotada, quem fala é o `say` do macOS. Nenhum áudio é gravado em disco.

use std::fs;
use std::path::Path;
use std::sync::mpsc;
use std::thread;

use crate::audio;
use crate::fonetica::{ajustar_fonetica, carregar_regras};

/// Frases usadas quando o config.json não traz as do aviso.
const FRASES_PADRAO: &[(&str, &str)] = &[
    ("terminei", "Terminei."),
    ("permissao", "Preciso da sua permissão."),
    ("espera", "Tô te esperando."),
];

pub(crate) fn ler_config(base: &Path) -> serde_json::Value {
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
    let cfg_json = ler_config(base);
    let provedores: Vec<String> = cfg_json
        .get("provedores")
        .and_then(|p| p.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_else(|| {
            vec![
                "gemini".to_string(),
                "elevenlabs".to_string(),
                "openai".to_string(),
            ]
        });

    let para_voz = ajustar_fonetica(texto, &carregar_regras(base, false));

    for prov in &provedores {
        let (envia, recebe) = mpsc::channel();
        let base_tocador = base.to_path_buf();
        let tocador = thread::spawn(move || audio::tocar_stream(&base_tocador, recebe));

        let res = match prov.as_str() {
            "gemini" => {
                let cfg = crate::gemini::Config::de_json(cfg_json.get("gemini"));
                crate::gemini::sintetizar_em_stream(&cfg, &para_voz, timeout_seg, |pedaco| {
                    let _ = envia.send(pedaco);
                })
            }
            "elevenlabs" => {
                let cfg = crate::elevenlabs::Config::de_json(cfg_json.get("elevenlabs"));
                crate::elevenlabs::sintetizar_em_stream(&cfg, &para_voz, timeout_seg, |pedaco| {
                    let _ = envia.send(pedaco);
                })
            }
            "openai" => {
                let cfg = crate::openai::Config::de_json(cfg_json.get("openai"));
                crate::openai::sintetizar_em_stream(&cfg, &para_voz, timeout_seg, |pedaco| {
                    let _ = envia.send(pedaco);
                })
            }
            outro => {
                drop(envia);
                let _ = tocador.join();
                eprintln!("[voz] provedor desconhecido: {outro}");
                continue;
            }
        };

        drop(envia);
        let tocou = tocador.join().unwrap_or(false);

        match res {
            Ok(()) if tocou => return,
            // Parou no meio: repetir a frase inteira no say soaria pior que o corte.
            Err(e) if tocou => {
                eprintln!("[voz] o stream de {prov} parou no meio ({e}).");
                return;
            }
            Err(e) => {
                eprintln!("[voz] {prov} indisponível ({e}); tentando próximo provedor.");
            }
            Ok(()) => {
                eprintln!("[voz] saída de áudio indisponível para {prov}.");
            }
        }
    }

    // Se todos os provedores em nuvem falharem, aciona a reserva local do macOS (Siri Natural Voz 2)
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
