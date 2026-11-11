mod agy;
mod audio;
mod codex;
mod cotas;
mod edge;
mod elevenlabs;
mod fala;
mod fonetica;
mod gemini;
mod keychain;
mod narrar;
mod openai;
mod painel;
mod platform;
mod resumo;
mod tela;
mod transcript;

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

fn caminho_base() -> PathBuf {
    if let Ok(dir) = std::env::var("VOZ_DIR") {
        let p = PathBuf::from(dir);
        if p.exists() {
            return p;
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Ok(real_exe) = fs::canonicalize(&exe) {
            if let Some(repo) = real_exe
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
            {
                if repo.join("config.json").exists() {
                    return repo.to_path_buf();
                }
            }
            if let Some(repo) = real_exe.parent() {
                if repo.join("config.json").exists() {
                    return repo.to_path_buf();
                }
            }
        }
    }

    if let Some(home_path) = platform::pasta_usuario() {
        for candidata in [home_path.join("voz"), home_path.join(".config").join("voz")] {
            if candidata.join("config.json").exists() {
                return candidata;
            }
        }
        return home_path.join("voz");
    }

    PathBuf::from(".")
}

fn ler_stdin() -> String {
    let mut buffer = String::new();
    let _ = io::stdin().read_to_string(&mut buffer);
    buffer
}

/// Repete o hook num processo separado, para quem chamou não esperar a fala.
fn disparar_desacoplado(categoria: &str, entrada: &str) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let filho = Command::new(exe)
        .arg(categoria)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut f) = filho {
        if let Some(mut entrada_filho) = f.stdin.take() {
            use std::io::Write;
            let _ = entrada_filho.write_all(entrada.as_bytes());
            let _ = entrada_filho.flush();
            drop(entrada_filho);
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let base = caminho_base();

    if args.get(1).map(String::as_str) == Some("--codex") {
        let entrada = ler_stdin();
        if std::env::var("VOZ_GERANDO").is_err() {
            if let Ok(valor) = serde_json::from_str(&entrada) {
                if let Some(categoria) = codex::categoria(&valor) {
                    processar_aviso(&base, categoria, &entrada);
                }
            }
        }
        // O hook é assíncrono no Codex. Stdout contém somente JSON neutro:
        // a fala não aprova ferramentas, não bloqueia nem prolonga o turno.
        println!("{{}}");
        return;
    }

    if args.get(1).map(String::as_str) == Some("--agy-monitorar-confirmacao") {
        let estado_dir = base.join("estado");
        fs::create_dir_all(&estado_dir).ok();
        let lock_file = estado_dir.join("agy-monitor.lock");

        let _trava = match platform::tentar_trava_exclusiva(&lock_file) {
            Some(f) => f,
            None => return,
        };

        let aviso_file = estado_dir.join("agy-ultimo-aviso-step");
        let encerrar_file = estado_dir.join("agy-encerrar-monitor");
        fs::remove_file(&encerrar_file).ok();

        let mut ultimo_avisado: Option<String> = fs::read_to_string(&aviso_file)
            .ok()
            .map(|s| s.trim().to_string());

        for _ in 0..240 {
            if encerrar_file.exists() {
                break;
            }

            if let Some((arquivo, step)) = agy::detectar_confirmacao_pendente() {
                let chave = format!("{arquivo}:{step}");
                if ultimo_avisado.as_deref() != Some(&chave) {
                    ultimo_avisado = Some(chave.clone());
                    fs::write(&aviso_file, &chave).ok();
                    disparar_desacoplado("permissao", "");
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(250));
        }

        fs::remove_file(&lock_file).ok();
        return;
    }

    if args.get(1).map(String::as_str) == Some("--agy-verificar-espera") {
        let token = ler_stdin();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let pendente_file = base.join("estado").join("agy-pendente");
        if let Ok(conteudo) = fs::read_to_string(&pendente_file) {
            if conteudo.trim() == token.trim() {
                processar_aviso(&base, "espera", "");
            }
        }
        return;
    }

    if args.len() > 1 && args[1] == "--agy" {
        let categoria = args.get(2).map(|s| s.as_str()).unwrap_or("");
        let entrada = ler_stdin();
        let estado_dir = base.join("estado");
        fs::create_dir_all(&estado_dir).ok();
        let pendente_file = estado_dir.join("agy-pendente");

        match categoria {
            "pre-invocation" => {
                let encerrar_file = estado_dir.join("agy-encerrar-monitor");
                fs::remove_file(&encerrar_file).ok();
                println!("{{}}");
                disparar_desacoplado("--agy-monitorar-confirmacao", "");
            }
            "pos-tool" => {
                fs::remove_file(&pendente_file).ok();
                println!("{{}}");
            }
            "permissao" => {
                disparar_desacoplado("--agy-monitorar-confirmacao", "");

                if agy::analisar_pre_tool(&entrada) == agy::AgyDecisao::Espera {
                    let nanos = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.subsec_nanos() % 10000)
                        .unwrap_or(0);
                    let token = format!("{}-{}", std::process::id(), nanos);
                    fs::write(&pendente_file, &token).ok();
                    println!("{{\"decision\": \"allow\"}}");
                    disparar_desacoplado("--agy-verificar-espera", &token);
                } else {
                    println!("{{\"decision\": \"allow\"}}");
                }
            }
            _ => {
                let encerrar_file = estado_dir.join("agy-encerrar-monitor");
                fs::write(&encerrar_file, "1").ok();
                fs::remove_file(&pendente_file).ok();
                println!("{{}}");

                if entrada.contains("\"terminationReason\":\"USER_CANCELED\"")
                    || entrada.contains("\"fullyIdle\":false")
                {
                    return;
                }

                disparar_desacoplado(categoria, &entrada);
            }
        }
        return;
    }

    if args.len() > 1 && args[1] == "simular" {
        let transcript = args.get(2).map(Path::new);
        let cwd = args.get(3).map(Path::new);
        if let Some(t) = transcript {
            let (inicio, pedido, resp) = transcript::ultimo_turno(t);
            let resp_texto = resp.unwrap_or_else(|| "Nenhuma resposta encontrada.".to_string());
            let proj = narrar::nome_do_projeto(cwd);
            let fala = narrar::montar_fala(&base, &proj, &pedido, &resp_texto);
            let para_voz =
                fonetica::ajustar_fonetica(&fala, &fonetica::carregar_regras(&base, false));

            println!("--- Simulação de Narração ---");
            println!("Início: {:?}", inicio);
            println!("Projeto: {proj}");
            println!("Resumo: {fala}");
            println!("Para a voz: {para_voz}");
        } else {
            eprintln!("Uso: voz simular <transcript.jsonl> [cwd]");
        }
        return;
    }

    if args.len() > 1 && args[1] == "fonetica" {
        let texto = args[2..].join(" ");
        for (voz, reserva) in [("gemini", false), ("reserva", true)] {
            let regras = fonetica::carregar_regras(&base, reserva);
            println!("{voz}: {}", fonetica::ajustar_fonetica(&texto, &regras));
        }
        return;
    }

    if args.len() > 1 && args[1] == "sintetizar" {
        fala::falar(&base, &args[2..].join(" "), 40);
        return;
    }

    if args.len() > 1 && args[1] == "tocar" {
        if let Some(arquivo) = args.get(2) {
            audio::tocar(&base, Path::new(arquivo));
        }
        return;
    }

    // Modo Painel: se chamado sem argumentos ou com flags do painel (-a, -w, --ativos, --watch)
    if args.len() == 1
        || args
            .iter()
            .any(|a| a == "-a" || a == "--ativos" || a == "-w" || a == "--watch")
    {
        let apenas_ativos = args.iter().any(|a| a == "-a" || a == "--ativos");
        let mut watch = None;
        for (i, a) in args.iter().enumerate() {
            if a == "-w" || a == "--watch" {
                let seg = args
                    .get(i + 1)
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(1.0);
                watch = Some(seg);
                break;
            }
        }
        painel::rodar_painel(&base, apenas_ativos, watch);
        return;
    }

    // Modo hook comum (terminei, permissao, espera)
    let categoria = args.get(1).map(|s| s.as_str()).unwrap_or("terminei");
    if std::env::var("VOZ_GERANDO").is_ok() {
        return;
    }

    let entrada = ler_stdin();
    processar_aviso(&base, categoria, &entrada);
}

fn processar_aviso(base: &Path, categoria: &str, entrada: &str) {
    if categoria == "terminei" && narrar::tentar_narrar(base, entrada, None, None) {
        return;
    }

    if let Some(frase) = fala::sortear_frase(base, categoria) {
        fala::falar(base, &frase, 30);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_caminho_base_respeita_env() {
        let temp = std::env::temp_dir();
        std::env::set_var("VOZ_DIR", &temp);
        assert_eq!(caminho_base(), temp);
        std::env::remove_var("VOZ_DIR");
    }
}
