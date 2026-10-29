//! Desenho em tela cheia: tamanho do terminal, corte de linhas e cursor.
//!
//! O painel redesenha por cima do quadro anterior. Linha mais larga que o
//! terminal quebraria e embaralharia o redesenho; linha mais curta que a
//! anterior deixaria restos à direita. Aqui tudo é cortado ao tamanho da tela
//! e cada linha apaga o que sobrou dela.

use crate::painel::{largura_visivel, RESET};

/// Colunas e linhas do terminal; None quando a saída não é um terminal.
pub fn tamanho() -> Option<(usize, usize)> {
    let mut ws = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCGWINSZ só preenche a struct winsize apontada.
    let ok = unsafe {
        libc::ioctl(
            libc::STDOUT_FILENO,
            libc::TIOCGWINSZ,
            &mut ws as *mut libc::winsize,
        )
    } == 0;
    if ok && ws.ws_col > 0 && ws.ws_row > 0 {
        Some((ws.ws_col as usize, ws.ws_row as usize))
    } else {
        None
    }
}

/// Corta a linha em `largura` colunas visíveis sem partir as sequências de cor.
pub fn cortar_visivel(linha: &str, largura: usize) -> String {
    let mut saida = String::with_capacity(linha.len());
    let mut visiveis = 0;
    let mut chars = linha.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            saida.push(c);
            // Copia a sequência inteira (ESC [ ... letra) sem contar largura.
            for p in chars.by_ref() {
                saida.push(p);
                if p.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        if visiveis == largura {
            saida.push_str(RESET);
            break;
        }
        saida.push(c);
        visiveis += 1;
    }
    saida
}

/// Põe dois blocos lado a lado; se não couberem na tela, um embaixo do outro.
pub fn juntar_colunas(esq: &[String], dir: &[String], largura_tela: Option<usize>) -> String {
    let largura = |bloco: &[String]| bloco.iter().map(|l| largura_visivel(l)).max().unwrap_or(0);
    let (largura_esq, largura_dir) = (largura(esq), largura(dir));
    if largura_tela.is_some_and(|tela| largura_esq + 2 + largura_dir > tela) {
        return esq
            .iter()
            .chain(dir)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n");
    }

    let completar = |bloco: &[String], i: usize, alvo: usize| {
        let linha = bloco.get(i).map(String::as_str).unwrap_or("");
        let falta = alvo.saturating_sub(largura_visivel(linha));
        format!("{linha}{}", " ".repeat(falta))
    };
    (0..esq.len().max(dir.len()))
        .map(|i| {
            format!(
                "{}  {}",
                completar(esq, i, largura_esq),
                completar(dir, i, largura_dir)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Prepara o quadro para ser desenhado por cima do anterior sem deixar restos.
pub fn ajustar(quadro: &str, tamanho: Option<(usize, usize)>) -> String {
    let (colunas, linhas) = tamanho.unwrap_or((usize::MAX, usize::MAX));
    // Uma coluna de folga: linha encostada na borda deixa o cursor com a quebra
    // pendente, e aí o \x1b[K apagaria o último caractere dela.
    let largura = colunas.saturating_sub(1).max(1);
    quadro
        .lines()
        .take(linhas)
        .map(|l| format!("{}\x1b[K", cortar_visivel(l, largura)))
        .collect::<Vec<_>>()
        .join("\n")
}

extern "C" fn restaurar_e_sair(sinal: libc::c_int) {
    const SEQUENCIA: &[u8] = b"\x1b[0m\x1b[?25h\n";
    // SAFETY: write e _exit são seguras dentro de um tratador de sinal.
    unsafe {
        libc::write(
            libc::STDOUT_FILENO,
            SEQUENCIA.as_ptr().cast(),
            SEQUENCIA.len(),
        );
        libc::_exit(128 + sinal);
    }
}

/// Esconde o cursor e garante que ele volte quando o painel for fechado.
pub fn esconder_cursor_ate_sair() {
    for sinal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: o tratador só chama funções seguras para sinais.
        unsafe {
            libc::signal(
                sinal,
                restaurar_e_sair as extern "C" fn(libc::c_int) as libc::sighandler_t,
            );
        }
    }
    print!("\x1b[?25l");
}

#[cfg(test)]
mod tests {
    use super::*;

    const VERDE: &str = "\x1b[32m";

    #[test]
    fn test_cortar_visivel_preserva_cores() {
        let linha = format!("{VERDE}abcdef{RESET}gh");
        assert_eq!(cortar_visivel(&linha, 3), format!("{VERDE}abc{RESET}"));
        assert_eq!(cortar_visivel(&linha, 20), linha);
        assert_eq!(largura_visivel(&cortar_visivel("│ ✦ ● ↻ │", 4)), 4);
    }

    #[test]
    fn test_ajustar_corta_largura_e_altura() {
        let quadro = "1234567890\nabc\nxyz";
        let tela = ajustar(quadro, Some((6, 2)));
        assert_eq!(tela, format!("12345{RESET}\x1b[K\nabc\x1b[K"));
    }

    #[test]
    fn test_ajustar_fora_do_terminal_nao_corta() {
        assert_eq!(ajustar("1234567890", None), "1234567890\x1b[K");
    }

    #[test]
    fn test_juntar_colunas_lado_a_lado_e_empilhado() {
        let esq = vec!["aaaa".to_string(), "aa".to_string()];
        let dir = vec!["bb".to_string()];
        assert_eq!(juntar_colunas(&esq, &dir, Some(80)), "aaaa  bb\naa      ");
        assert_eq!(juntar_colunas(&esq, &dir, Some(7)), "aaaa\naa\nbb");
        assert_eq!(juntar_colunas(&esq, &dir, None), "aaaa  bb\naa      ");
    }
}
