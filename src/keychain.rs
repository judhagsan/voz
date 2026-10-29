//! Acesso ao Keychain do macOS para leitura de chaves de API.
//! Suporta chaves sequenciais para rotação automática (ex: chave, chave-2, chave-3).

use std::process::Command;

pub fn ler_chave(servico: &str) -> Result<String, String> {
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

/// Devolve a lista de todas as chaves cadastradas para o serviço base.
/// Exemplo: para "gemini-api-key", busca "gemini-api-key", "gemini-api-key-2", etc.
pub fn chaves(servico_base: &str) -> Vec<String> {
    let mut lista = Vec::new();
    if let Ok(c) = ler_chave(servico_base) {
        lista.push(c);
    }
    let mut idx = 2;
    loop {
        let servico = format!("{servico_base}-{idx}");
        match ler_chave(&servico) {
            Ok(c) => {
                lista.push(c);
                idx += 1;
            }
            Err(_) => break,
        }
    }
    lista
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chave_inexistente_retorna_erro() {
        assert!(ler_chave("chave-que-certamente-nao-existe-12345").is_err());
    }

    #[test]
    fn test_chaves_inexistente_retorna_vazio() {
        let lista = chaves("chave-que-certamente-nao-existe-12345");
        assert!(lista.is_empty());
    }
}
