use regex::RegexBuilder;
use std::fs;
use std::path::Path;

pub struct RegraFonetica {
    pub regex: regex::Regex,
    pub substituicao: String,
}

/// Nomes próprios e siglas técnicas que nenhum motor acerta sozinho.
const TERMOS_GERAIS: &[(&str, &str)] = &[(r"\bjudhagsan\b", "Judágê-San"), (r"\bjson\b", "Jêison")];

/// Termos em inglês que a voz de reserva (`say`) lê com fonética portuguesa.
/// O Gemini os pronuncia certo sozinho, e o respelling só atrapalharia.
const TERMOS_RESERVA: &[(&str, &str)] = &[
    (r"\bgithub\b", "Guít-râb"),
    (r"\bgitlab\b", "Guít-lébi"),
    (r"\bgit\b", "Guite"),
    (r"\bapis\b", "A P Is"),
    (r"\bapi\b", "A P I"),
    (r"\bprs\b", "P Rs"),
    (r"\bpr\b", "P R"),
    (r"\bcli\b", "C L I"),
    (r"\burls\b", "U R Ls"),
    (r"\burl\b", "U R L"),
    (r"\bui\b", "U I"),
    (r"\byaml\b", "iâmol"),
    (r"\bsql\b", "ése-quê-éle"),
    (r"\bwav\b", "uáve"),
    (r"\bdeploy\b", "deplói"),
    (r"\bbugs\b", "báguis"),
    (r"\bbug\b", "bágui"),
];

fn compilar<'a>(termos: impl Iterator<Item = (&'a str, &'a str)>) -> Vec<RegraFonetica> {
    termos
        .filter_map(|(p, s)| {
            RegexBuilder::new(p)
                .case_insensitive(true)
                .build()
                .ok()
                .map(|re| RegraFonetica {
                    regex: re,
                    substituicao: s.to_string(),
                })
        })
        .collect()
}

/// Regras do `fonetica.json`: `termos` vale para toda voz, `termos_reserva` só
/// para a voz de reserva.
pub fn carregar_regras(base: &Path, reserva: bool) -> Vec<RegraFonetica> {
    let json = fs::read_to_string(base.join("fonetica.json"))
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok());
    let Some(json) = json else {
        return regras_padrao(reserva);
    };

    let blocos: &[&str] = if reserva {
        &["termos", "termos_reserva"]
    } else {
        &["termos"]
    };
    let pares: Vec<(&str, &str)> = blocos
        .iter()
        .filter_map(|b| json.get(*b).and_then(|t| t.as_object()))
        .flatten()
        .filter_map(|(p, v)| v.as_str().map(|s| (p.as_str(), s)))
        .collect();

    let regras = compilar(pares.into_iter());
    if regras.is_empty() {
        regras_padrao(reserva)
    } else {
        regras
    }
}

pub fn regras_padrao(reserva: bool) -> Vec<RegraFonetica> {
    let extras: &[(&str, &str)] = if reserva { TERMOS_RESERVA } else { &[] };
    compilar(TERMOS_GERAIS.iter().chain(extras).copied())
}

pub fn ajustar_fonetica(texto: &str, regras: &[RegraFonetica]) -> String {
    let mut resultado = texto.to_string();
    for regra in regras {
        resultado = regra
            .regex
            .replace_all(&resultado, regra.substituicao.as_str())
            .to_string();
    }
    resultado
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ajustar_fonetica_termos_basicos() {
        let regras = regras_padrao(true);
        let entrada = "Subi o commit no git e abri o PR para a API no GitHub.";
        let saida = ajustar_fonetica(entrada, &regras);
        assert_eq!(
            saida,
            "Subi o commit no Guite e abri o P R para a A P I no Guít-râb."
        );
    }

    #[test]
    fn test_ajustar_fonetica_cli_json_wav() {
        let regras = regras_padrao(true);
        let entrada = "Rodei a CLI para gerar JSON e WAV.";
        let saida = ajustar_fonetica(entrada, &regras);
        assert_eq!(saida, "Rodei a C L I para gerar Jêison e uáve.");
    }

    #[test]
    fn test_ajustar_fonetica_judhagsan() {
        for reserva in [true, false] {
            let regras = regras_padrao(reserva);
            let entrada = "Trabalhando no repositório judhagsan agora.";
            let saida = ajustar_fonetica(entrada, &regras);
            assert_eq!(saida, "Trabalhando no repositório Judágê-San agora.");
        }
    }

    #[test]
    fn test_gemini_mantem_termos_em_ingles() {
        let regras = regras_padrao(false);
        let entrada = "Abri o PR da API no GitHub do judhagsan.";
        let saida = ajustar_fonetica(entrada, &regras);
        assert_eq!(saida, "Abri o PR da API no GitHub do Judágê-San.");
    }

    #[test]
    fn test_gemini_converte_json() {
        let regras = regras_padrao(false);
        let entrada = "Atualizei o arquivo JSON e rodei o Git.";
        let saida = ajustar_fonetica(entrada, &regras);
        assert_eq!(saida, "Atualizei o arquivo Jêison e rodei o Git.");
    }

    #[test]
    fn test_carregar_regras_separa_blocos_do_json() {
        let pasta = std::env::temp_dir().join(format!("voz-fonetica-{}", std::process::id()));
        fs::create_dir_all(&pasta).unwrap();
        fs::write(
            pasta.join("fonetica.json"),
            r#"{"termos": {"\\bfulano\\b": "Fulâno"}, "termos_reserva": {"\\bgit\\b": "Guite"}}"#,
        )
        .unwrap();
        let texto = "O fulano rodou o git.";
        let reserva = ajustar_fonetica(texto, &carregar_regras(&pasta, true));
        let gemini = ajustar_fonetica(texto, &carregar_regras(&pasta, false));
        assert_eq!(reserva, "O Fulâno rodou o Guite.");
        assert_eq!(gemini, "O Fulâno rodou o git.");
        let _ = fs::remove_dir_all(&pasta);
    }
}
