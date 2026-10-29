mod agy;
mod audio;
mod fonetica;
mod ipc;
mod narrar;
mod painel;
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
                println!("{{\"decision\": \"allow\"}}");

                // Avalia se o comando requer permissão real
                if agy::agy_precisa_permissao(&entrada) {
                    if let Some(arq) = audio::sortear_audio(&base, "permissao") {
                        audio::tocar_enfileirado(&base, &arq, "Preciso da sua permissão");
                    }
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

                // Dispara processo desacoplado para a categoria
                if let Ok(exe) = std::env::current_exe() {
                    let mut child = Command::new(exe)
                        .arg(categoria)
                        .stdin(std::process::Stdio::piped())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn();

                    if let Ok(ref mut c) = child {
                        if let Some(ref mut sin) = c.stdin {
                            use std::io::Write;
                            sin.write_all(entrada.as_bytes()).ok();
                        }
                    }
                }
            }
        }
        return;
    }

    if args.len() > 1 && args[1] == "simular" {
        let transcript = args.get(2).map(Path::new);
        let cwd = args.get(3).map(Path::new);
        if let Some(t) = transcript {
            let (inicio, _pedido, resp) = transcript::ultimo_turno(t);
            let resp_texto = resp.unwrap_or_else(|| "Nenhuma resposta encontrada.".to_string());
            let proj = narrar::nome_do_projeto(cwd);
            let resumo = transcript::extrair_resumo_direto(&resp_texto, &proj);
            let regras = fonetica::carregar_regras(&base);
            let fonetico = fonetica::ajustar_fonetica(&resumo, &regras);

            println!("--- Simulação de Narração ---");
            println!("Início: {:?}", inicio);
            println!("Projeto: {proj}");
            println!("Original: {resumo}");
            println!("Fonético: {fonetico}");
        } else {
            eprintln!("Uso: voz simular <transcript.jsonl> [cwd]");
        }
        return;
    }

    if args.len() > 1 && args[1] == "fonetica" {
        let texto = args[2..].join(" ");
        let regras = fonetica::carregar_regras(&base);
        println!("{}", fonetica::ajustar_fonetica(&texto, &regras));
        return;
    }

    if args.len() > 1 && args[1] == "sintetizar" {
        let texto = args[2..].join(" ");
        let regras = fonetica::carregar_regras(&base);
        let texto_fonetico = fonetica::ajustar_fonetica(&texto, &regras);
        let sock = base.join("estado").join("motor.sock");
        match ipc::sintetizar_uds(&sock, &texto_fonetico, 40) {
            Ok(bytes) => {
                let out = base.join("estado").join("teste_rust.wav");
                fs::write(&out, bytes).ok();
                println!("Sintetizado com sucesso! Tocando...");
                audio::tocar_enfileirado(&base, &out, &texto);
            }
            Err(e) => eprintln!("Erro: {e}"),
        }
        return;
    }

    if args.len() > 1 && args[1] == "tocar" {
        let arq = args.get(2).map(Path::new);
        let res = args.get(3).map(|s| s.as_str()).unwrap_or("");
        if let Some(a) = arq {
            audio::tocar_enfileirado(&base, a, res);
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

    let reserva = match categoria {
        "terminei" => "Terminei",
        "permissao" => "Preciso da sua permissão",
        "espera" => "Tô te esperando",
        _ => return,
    };

    if let Some(arq) = audio::sortear_audio(&base, categoria) {
        audio::tocar_enfileirado(&base, &arq, reserva);
    } else {
        let _ = Command::new("say").arg(reserva).status();
    }
}
