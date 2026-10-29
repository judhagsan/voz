mod agy;
mod audio;
mod cotas;
mod elevenlabs;
mod fala;
mod fonetica;
mod gemini;
mod keychain;
mod narrar;
mod openai;
mod painel;
mod resumo;
mod tela;
mod transcript;

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;

fn caminho_base() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".claude").join("voz")
    } else {
        PathBuf::from(".")
    }
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
            entrada_filho.write_all(entrada.as_bytes()).ok();
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let base = caminho_base();

    if args.len() > 1 && args[1] == "--agy" {
        let categoria = args.get(2).map(|s| s.as_str()).unwrap_or("");
        let entrada = ler_stdin();
        let estado_dir = base.join("estado");
        fs::create_dir_all(&estado_dir).ok();
        let pendente_file = estado_dir.join("agy-pendente");

        match categoria {
            "pos-tool" => {
                fs::remove_file(&pendente_file).ok();
                println!("{{}}");
            }
            "permissao" => {
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.subsec_nanos() % 10000)
                    .unwrap_or(0);
                let token = format!("{}-{}", std::process::id(), nanos);
                fs::write(&pendente_file, &token).ok();

                // Só avisa quando o comando espera aprovação de verdade; a fala
                // sai num processo à parte para não segurar o agente.
                if agy::agy_precisa_permissao(&entrada) {
                    println!("{{\"decision\": \"ask\"}}");
                    disparar_desacoplado("permissao", "");
                } else {
                    println!("{{\"decision\": \"allow\"}}");
                }
            }
            _ => {
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

    if categoria == "terminei" && narrar::tentar_narrar(&base, &entrada, None, None) {
        return;
    }

    if let Some(frase) = fala::sortear_frase(&base, categoria) {
        fala::falar(&base, &frase, 30);
    }
}
