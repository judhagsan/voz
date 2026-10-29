use regex::RegexBuilder;
use std::fs;
use std::path::Path;

pub struct RegraFonetica {
    pub regex: regex::Regex,
    pub substituicao: String,
}

pub fn carregar_regras(base: &Path) -> Vec<RegraFonetica> {
    let caminho = base.join("fonetica.json");
    if !caminho.is_file() {
        return regras_padrao();
    }

    let conteudo = match fs::read_to_string(&caminho) {
        Ok(c) => c,
        Err(_) => return regras_padrao(),
    };

    let json: serde_json::Value = match serde_json::from_str(&conteudo) {
        Ok(v) => v,
        Err(_) => return regras_padrao(),
    };

    let termos = match json.get("termos").and_then(|t| t.as_object()) {
        Some(t) => t,
        None => return regras_padrao(),
    };

    let mut regras = Vec::new();
    for (padrao, valor) in termos {
        if let Some(subst) = valor.as_str() {
            if let Ok(re) = RegexBuilder::new(padrao).case_insensitive(true).build() {
                regras.push(RegraFonetica {
                    regex: re,
                    substituicao: subst.to_string(),
                });
            }
        }
    }

    if regras.is_empty() {
        regras_padrao()
    } else {
        regras
    }
}

pub fn regras_padrao() -> Vec<RegraFonetica> {
    let termos = [
        (r"\bjudhagsan\b", "Judá-gê-sâm"),
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
        (r"\bjson\b", "Jêison"),
        (r"\byaml\b", "iâmol"),
        (r"\bsql\b", "ése-quê-éle"),
        (r"\bwav\b", "uáve"),
        (r"\bdeploy\b", "deplói"),
        (r"\bbugs\b", "báguis"),
        (r"\bbug\b", "bágui"),
    ];

    termos
        .iter()
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
        let regras = regras_padrao();
        let entrada = "Subi o commit no git e abri o PR para a API no GitHub.";
        let saida = ajustar_fonetica(entrada, &regras);
        assert_eq!(
            saida,
            "Subi o commit no Guite e abri o P R para a A P I no Guít-râb."
        );
    }

    #[test]
    fn test_ajustar_fonetica_cli_json_wav() {
        let regras = regras_padrao();
        let entrada = "Rodei a CLI para gerar JSON e WAV.";
        let saida = ajustar_fonetica(entrada, &regras);
        assert_eq!(saida, "Rodei a C L I para gerar Jêison e uáve.");
    }

    #[test]
    fn test_ajustar_fonetica_judhagsan() {
        let regras = regras_padrao();
        let entrada = "Trabalhando no repositório judhagsan agora.";
        let saida = ajustar_fonetica(entrada, &regras);
        assert_eq!(saida, "Trabalhando no repositório Judá-gê-sâm agora.");
    }
}
