use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

pub(crate) const RESET: &str = "\x1b[0m";
pub(crate) const NEGRITO: &str = "\x1b[1m";
pub(crate) const FRACO: &str = "\x1b[2m";
pub(crate) const VERDE: &str = "\x1b[32m";
pub(crate) const AMARELO: &str = "\x1b[33m";
pub(crate) const AZUL: &str = "\x1b[36m";
pub(crate) const MAGENTA: &str = "\x1b[35m";
pub(crate) const VERMELHO: &str = "\x1b[31m";

const PASTAS_IGNORADAS: &[&str] = &[
    "Library",
    "Pictures",
    "Music",
    "Movies",
    "Public",
    "Downloads",
    "Applications",
    "node_modules",
    "target",
    "dist",
    "build",
    "venv",
    ".cargo",
    ".rustup",
    ".cache",
];

#[derive(Debug, Clone)]
pub struct PrItem {
    pub number: u64,
    pub commits_count: usize,
}

#[derive(Debug, Clone)]
pub struct RepoStatus {
    pub nome: String,
    #[allow(dead_code)]
    pub caminho: PathBuf,
    pub branch: String,
    pub modificados: usize,
    pub ahead: usize,
    pub behind: usize,
    pub prs: Vec<PrItem>,
    pub em_andamento: bool,
}

pub(crate) fn largura_visivel(texto: &str) -> usize {
    let re = Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    let limpo = re.replace_all(texto, "");
    limpo.chars().count()
}

pub(crate) fn alinhar(texto: &str, largura: usize) -> String {
    let vis = largura_visivel(texto);
    let falta = largura.saturating_sub(vis);
    format!("{}{}", texto, " ".repeat(falta))
}

pub fn carregar_cache_prs_disco(base: &Path) -> HashMap<String, serde_json::Value> {
    let arq = base.join("estado").join("cache-prs.json");
    if !arq.is_file() {
        return HashMap::new();
    }
    fs::read_to_string(&arq)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_default()
}

pub fn salvar_cache_prs_disco(base: &Path, cache: &HashMap<String, serde_json::Value>) {
    let estado_dir = base.join("estado");
    fs::create_dir_all(&estado_dir).ok();
    let arq = estado_dir.join("cache-prs.json");
    if let Ok(c) = serde_json::to_string(cache) {
        fs::write(arq, c).ok();
    }
}

pub fn descobrir_repos(base: &Path) -> Vec<PathBuf> {
    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return Vec::new(),
    };

    let mut candidatos = Vec::new();
    let mut vistos = HashSet::new();

    if base.join(".git").exists() {
        if let Ok(canon) = base.canonicalize() {
            vistos.insert(canon);
            candidatos.push(base.to_path_buf());
        }
    }

    if let Ok(entries) = fs::read_dir(&home) {
        let mut itens: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                if let Some(nome) = p.file_name().and_then(|n| n.to_str()) {
                    !nome.starts_with('.') && !PASTAS_IGNORADAS.contains(&nome) && p.is_dir()
                } else {
                    false
                }
            })
            .collect();
        itens.sort();

        for item in itens {
            if item.join(".git").exists() {
                if let Ok(canon) = item.canonicalize() {
                    if vistos.insert(canon) {
                        candidatos.push(item);
                    }
                }
                continue;
            }

            if let Ok(sub_entries) = fs::read_dir(&item) {
                let mut subs: Vec<PathBuf> = sub_entries
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        if let Some(nome) = p.file_name().and_then(|n| n.to_str()) {
                            !nome.starts_with('.')
                                && !PASTAS_IGNORADAS.contains(&nome)
                                && p.is_dir()
                        } else {
                            false
                        }
                    })
                    .collect();
                subs.sort();

                for sub in subs {
                    if sub.join(".git").exists() {
                        if let Ok(canon) = sub.canonicalize() {
                            if vistos.insert(canon) {
                                candidatos.push(sub);
                            }
                        }
                    }
                }
            }
        }
    }

    candidatos
}

pub fn rodar_git(repo: &Path, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

pub fn extrair_slug_github(remoto: &str) -> String {
    let re = Regex::new(r"github\.com[:/]([^/]+/[^/.]+?)(?:\.git)?$").unwrap();
    re.captures(remoto.trim())
        .and_then(|cap| cap.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_default()
}

pub fn info_remoto_repo(repo: &Path) -> (String, String) {
    let remoto = rodar_git(repo, &["config", "--get", "remote.origin.url"]);
    let slug = extrair_slug_github(&remoto);
    let nome = if !slug.is_empty() {
        slug.split('/').next_back().unwrap_or("").to_string()
    } else {
        repo.file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("repo")
            .to_string()
    };
    (slug, nome)
}

pub fn consultar_prs_abertos(slug: &str) -> Option<serde_json::Value> {
    if slug.is_empty() {
        return Some(serde_json::json!([]));
    }
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "-R",
            slug,
            "--state",
            "open",
            "--limit",
            "3",
            "--json",
            "number,headRefName,commits",
        ])
        .output()
        .ok()?;

    if output.status.success() {
        serde_json::from_slice(&output.stdout).ok()
    } else {
        None
    }
}

pub fn parse_prs(val: Option<&serde_json::Value>) -> Vec<PrItem> {
    let arr = match val.and_then(|v| v.as_array()) {
        Some(a) => a,
        None => return Vec::new(),
    };
    arr.iter()
        .filter_map(|item| {
            let num = item.get("number").and_then(|n| n.as_u64())?;
            let commits = item
                .get("commits")
                .and_then(|c| c.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            Some(PrItem {
                number: num,
                commits_count: commits,
            })
        })
        .collect()
}

pub fn inspecionar_repo_local(
    repo: &Path,
    cache_prs: &HashMap<String, serde_json::Value>,
) -> RepoStatus {
    let (slug, nome) = info_remoto_repo(repo);
    let saida = rodar_git(repo, &["status", "--porcelain=v2", "--branch"]);

    let mut branch = "—".to_string();
    let mut oid_curto = String::new();
    let mut ahead = 0;
    let mut behind = 0;
    let mut tem_ab = false;
    let mut modificados = 0;

    for linha in saida.lines() {
        if let Some(rest) = linha.strip_prefix("# branch.head ") {
            let h = rest.trim();
            if h != "(detached)" {
                branch = h.to_string();
            }
        } else if let Some(rest) = linha.strip_prefix("# branch.oid ") {
            oid_curto = rest.trim().chars().take(7).collect();
        } else if let Some(rest) = linha.strip_prefix("# branch.ab ") {
            tem_ab = true;
            for p in rest.split_whitespace() {
                if let Some(num) = p.strip_prefix('+') {
                    ahead = num.parse().unwrap_or(0);
                } else if let Some(num) = p.strip_prefix('-') {
                    behind = num.parse().unwrap_or(0);
                }
            }
        } else if !linha.starts_with('#') && !linha.trim().is_empty() {
            modificados += 1;
        }
    }

    if branch == "—" && !oid_curto.is_empty() {
        branch = oid_curto;
    }

    if !tem_ab && branch != "—" && !branch.is_empty() {
        for base_remota in [
            format!("origin/{}", branch),
            "origin/main".to_string(),
            "origin/master".to_string(),
        ] {
            let contagem = rodar_git(
                repo,
                &[
                    "rev-list",
                    "--left-right",
                    "--count",
                    &format!("{}...HEAD", base_remota),
                ],
            );
            let partes: Vec<&str> = contagem.split_whitespace().collect();
            if partes.len() == 2 {
                behind = partes[0].parse().unwrap_or(0);
                ahead = partes[1].parse().unwrap_or(0);
                break;
            }
        }
    }

    let prs = parse_prs(cache_prs.get(&slug));
    let em_andamento = modificados > 0
        || ahead > 0
        || behind > 0
        || !prs.is_empty()
        || (branch != "main" && branch != "master" && branch != "—");

    RepoStatus {
        nome,
        caminho: repo.to_path_buf(),
        branch,
        modificados,
        ahead,
        behind,
        prs,
        em_andamento,
    }
}

pub fn coletar_status(
    base: &Path,
    repos: &[PathBuf],
    cache_prs: &HashMap<String, serde_json::Value>,
) -> Vec<RepoStatus> {
    let mut resultados = Vec::with_capacity(repos.len());

    let chunks: Vec<Vec<PathBuf>> = repos
        .chunks(4.max(repos.len() / 8 + 1))
        .map(|c| c.to_vec())
        .collect();

    let handles: Vec<_> = chunks
        .into_iter()
        .map(|chunk| {
            let cache_ref = cache_prs.clone();
            thread::spawn(move || {
                chunk
                    .into_iter()
                    .map(|r| inspecionar_repo_local(&r, &cache_ref))
                    .collect::<Vec<_>>()
            })
        })
        .collect();

    for h in handles {
        if let Ok(lista) = h.join() {
            resultados.extend(lista);
        }
    }

    resultados.sort_by(|a, b| {
        b.em_andamento
            .cmp(&a.em_andamento)
            .then_with(|| a.nome.to_lowercase().cmp(&b.nome.to_lowercase()))
    });

    let _ = base;
    resultados
}

fn formatar_andamento(d: &RepoStatus) -> String {
    let b = &d.branch;
    let cor_b = if b != "main" && b != "master" {
        MAGENTA
    } else {
        ""
    };
    let fim_cor = if !cor_b.is_empty() { RESET } else { "" };

    if d.modificados > 0 {
        format!(
            "{cor_b}{b}{fim_cor} {AMARELO}({} modif.){RESET}",
            d.modificados
        )
    } else {
        format!("{cor_b}{b}{fim_cor} {FRACO}(limpo){RESET}")
    }
}

fn formatar_push(d: &RepoStatus) -> String {
    let mut partes = Vec::new();
    if d.ahead > 0 {
        let termo = if d.ahead == 1 { "commit" } else { "commits" };
        partes.push(format!("{AMARELO}↑ {} {termo}{RESET}", d.ahead));
    }
    if d.behind > 0 {
        partes.push(format!("{AZUL}↓ {} pull{RESET}", d.behind));
    }
    if partes.is_empty() {
        format!("{VERDE}✓ em dia{RESET}")
    } else {
        partes.join(" ")
    }
}

fn formatar_pr(d: &RepoStatus) -> String {
    if d.prs.is_empty() {
        return format!("{FRACO}—{RESET}");
    }
    let partes: Vec<String> = d
        .prs
        .iter()
        .map(|p| {
            let qtd = p.commits_count;
            let termo = if qtd == 1 { "commit" } else { "commits" };
            format!("PR #{} · {} {}", p.number, qtd, termo)
        })
        .collect();
    format!("{AZUL}{}{RESET}", partes.join(", "))
}

pub fn renderizar_quadro_repos(dados: &[RepoStatus], apenas_ativos: bool) -> Vec<String> {
    let exibidos: Vec<&RepoStatus> = if apenas_ativos {
        dados.iter().filter(|d| d.em_andamento).collect()
    } else {
        dados.iter().collect()
    };

    let cabecalhos = ["Projeto", "Andamento (local)", "Sem push", "PR aberto"];
    let mut larguras = [
        cabecalhos[0].len(),
        cabecalhos[1].len(),
        cabecalhos[2].len(),
        cabecalhos[3].len(),
    ];

    let mut linhas_formatadas = Vec::new();
    for d in exibidos {
        let icone = if d.em_andamento {
            format!("{VERDE}●{RESET}")
        } else {
            format!("{FRACO}○{RESET}")
        };
        let nome_proj = if d.em_andamento {
            format!("{NEGRITO}{}{RESET}", d.nome)
        } else {
            format!("{FRACO}{}{RESET}", d.nome)
        };

        let col0 = format!("{icone} {nome_proj}");
        let col1 = formatar_andamento(d);
        let col2 = formatar_push(d);
        let col3 = formatar_pr(d);

        larguras[0] = larguras[0].max(largura_visivel(&col0));
        larguras[1] = larguras[1].max(largura_visivel(&col1));
        larguras[2] = larguras[2].max(largura_visivel(&col2));
        larguras[3] = larguras[3].max(largura_visivel(&col3));

        linhas_formatadas.push((col0, col1, col2, col3));
    }

    let largura_interna: usize = larguras.iter().sum::<usize>() + 3 * (larguras.len() - 1);
    let ativos = dados.iter().filter(|d| d.em_andamento).count();
    let titulo_esq = format!(
        "{NEGRITO}VOZ{RESET} · {ativos} em andamento de {} repositórios",
        dados.len()
    );

    let topo_caixa = format!("╭─{}─╮", "─".repeat(largura_interna));
    let linha_titulo = format!("│ {} │", alinhar(&titulo_esq, largura_interna));
    let meio_titulo = format!(
        "├─{}─┤",
        larguras
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<_>>()
            .join("─┬─")
    );

    let linha_cab = format!(
        "│ {} │",
        cabecalhos
            .iter()
            .enumerate()
            .map(|(i, c)| alinhar(&format!("{NEGRITO}{c}{RESET}"), larguras[i]))
            .collect::<Vec<_>>()
            .join(" │ ")
    );

    let sep = format!(
        "├─{}─┤",
        larguras
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<_>>()
            .join("─┼─")
    );

    let base_caixa = format!(
        "╰─{}─╯",
        larguras
            .iter()
            .map(|w| "─".repeat(*w))
            .collect::<Vec<_>>()
            .join("─┴─")
    );

    let mut saida = vec![topo_caixa, linha_titulo, meio_titulo, linha_cab, sep];

    if linhas_formatadas.is_empty() {
        let vazio = alinhar(
            &format!("{FRACO}Nenhum projeto com pendências no momento.{RESET}"),
            largura_interna,
        );
        saida.push(format!("│ {vazio} │"));
        saida.push(format!("╰─{}─╯", "─".repeat(largura_interna)));
        return saida;
    }

    for (c0, c1, c2, c3) in linhas_formatadas {
        saida.push(format!(
            "│ {} │ {} │ {} │ {} │",
            alinhar(&c0, larguras[0]),
            alinhar(&c1, larguras[1]),
            alinhar(&c2, larguras[2]),
            alinhar(&c3, larguras[3])
        ));
    }
    saida.push(base_caixa);

    saida
}

pub fn renderizar_quadro(
    dados: &[RepoStatus],
    apenas_ativos: bool,
    cotas: Option<&crate::cotas::Cotas>,
    largura_tela: Option<usize>,
) -> String {
    let linhas_esq = renderizar_quadro_repos(dados, apenas_ativos);
    let linhas_dir = crate::cotas::renderizar_card(cotas, chrono::Local::now());
    crate::tela::juntar_colunas(&linhas_esq, &linhas_dir, largura_tela)
}

pub fn montar_trechos_resumo(dados: &[RepoStatus]) -> Vec<String> {
    let ativos: Vec<&RepoStatus> = dados.iter().filter(|d| d.em_andamento).collect();
    if ativos.is_empty() {
        return vec![
            "Todos os repositórios estão limpos, em dia com o remoto e sem PR aberto.".to_string(),
        ];
    }

    let mut trechos = Vec::new();
    for d in ativos {
        let mut detalhes = Vec::new();
        if d.branch != "main" && d.branch != "master" {
            detalhes.push(format!("na branch {}", d.branch));
        }
        if d.modificados == 1 {
            detalhes.push("com um arquivo modificado".to_string());
        } else if d.modificados > 1 {
            detalhes.push(format!("com {} arquivos modificados", d.modificados));
        }
        if d.ahead == 1 {
            detalhes.push("um commit sem push".to_string());
        } else if d.ahead > 1 {
            detalhes.push(format!("{} commits sem push", d.ahead));
        }
        if !d.prs.is_empty() {
            for p in &d.prs {
                let qtd = p.commits_count;
                let termo = if qtd == 1 {
                    "commit enviado"
                } else {
                    "commits enviados"
                };
                detalhes.push(format!("PR {} aberto com {} {}", p.number, qtd, termo));
            }
        }
        if !detalhes.is_empty() {
            trechos.push(format!("No {}, {}.", d.nome, detalhes.join(", ")));
        }
    }
    trechos
}

pub fn rodar_painel(base: &Path, apenas_ativos: bool, watch: Option<f64>) {
    let cache_mutex = Arc::new(Mutex::new(carregar_cache_prs_disco(base)));
    let repos = descobrir_repos(base);

    if let Some(intervalo_seg) = watch {
        let intervalo = Duration::from_secs_f64(intervalo_seg.max(0.5));
        let base_thread = base.to_path_buf();
        let repos_thread = repos.clone();
        let cache_clone = Arc::clone(&cache_mutex);

        thread::spawn(move || loop {
            let mut novos_prs = HashMap::new();
            for repo in &repos_thread {
                let (slug, _) = info_remoto_repo(repo);
                if !slug.is_empty() {
                    if let Some(prs) = consultar_prs_abertos(&slug) {
                        novos_prs.insert(slug, prs);
                    }
                }
            }
            if !novos_prs.is_empty() {
                let mut lock = cache_clone.lock().unwrap();
                for (k, v) in novos_prs {
                    lock.insert(k, v);
                }
                salvar_cache_prs_disco(&base_thread, &lock);
            }
            thread::sleep(Duration::from_secs(15));
        });

        // As CLIs levam até alguns segundos para responder; a consulta fica fora
        // do laço de desenho para o quadro não travar enquanto elas respondem.
        let cotas_mutex = Arc::new(Mutex::new(crate::cotas::ler_cache(base)));
        let base_cotas = base.to_path_buf();
        let cotas_clone = Arc::clone(&cotas_mutex);
        thread::spawn(move || loop {
            // Outro painel aberto pode ter atualizado o cache em disco.
            let leitura = match crate::cotas::ler_cache(&base_cotas) {
                Some(c) if crate::cotas::fresca(&c) => c,
                _ => crate::cotas::atualizar(&base_cotas),
            };
            *cotas_clone.lock().unwrap() = Some(leitura);
            thread::sleep(Duration::from_secs(30));
        });

        let dados_iniciais = coletar_status(base, &repos, &cache_mutex.lock().unwrap());
        let trechos_iniciais = montar_trechos_resumo(&dados_iniciais);
        let base_audio = base.to_path_buf();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(500));
            let regras = crate::fonetica::carregar_regras(&base_audio);
            let sock = base_audio.join("estado").join("motor.sock");
            for trecho in trechos_iniciais {
                let fala = crate::fonetica::ajustar_fonetica(&trecho, &regras);
                if let Ok(wav) = crate::ipc::sintetizar_uds(&sock, &fala, 30) {
                    let out = base_audio.join("estado").join("painel_temp.wav");
                    if fs::write(&out, wav).is_ok() {
                        crate::audio::tocar_enfileirado(&base_audio, &out, &fala);
                    }
                }
            }
        });

        crate::tela::esconder_cursor_ate_sair();
        print!("\x1b[2J\x1b[H");
        let mut ultimo_quadro = String::new();
        let mut ultimo_tamanho = None;

        loop {
            let cache_snapshot = cache_mutex.lock().unwrap().clone();
            let dados = coletar_status(base, &repos, &cache_snapshot);
            let cotas = cotas_mutex.lock().unwrap().clone();
            let tamanho = crate::tela::tamanho();
            let quadro =
                renderizar_quadro(&dados, apenas_ativos, cotas.as_ref(), tamanho.map(|t| t.0));
            let tela = crate::tela::ajustar(&quadro, tamanho);

            // Redimensionar reorganiza o que já estava na tela; só redesenhar
            // por cima deixaria pedaços do quadro antigo, então limpa tudo.
            if tamanho != ultimo_tamanho {
                print!("\x1b[2J");
                ultimo_tamanho = tamanho;
                ultimo_quadro.clear();
            }
            if tela != ultimo_quadro {
                print!("\x1b[H{}\x1b[J", tela);
                use std::io::Write;
                std::io::stdout().flush().ok();
                ultimo_quadro = tela;
            }

            thread::sleep(intervalo);
        }
    } else {
        let precisa_rede = {
            let arq = base.join("estado").join("cache-prs.json");
            match fs::metadata(arq) {
                Ok(meta) => {
                    let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    SystemTime::now()
                        .duration_since(mtime)
                        .map(|d| d.as_secs() > 30)
                        .unwrap_or(true)
                }
                Err(_) => true,
            }
        };

        if precisa_rede {
            let mut novos = HashMap::new();
            for repo in &repos {
                let (slug, _) = info_remoto_repo(repo);
                if !slug.is_empty() {
                    if let Some(prs) = consultar_prs_abertos(&slug) {
                        novos.insert(slug, prs);
                    }
                }
            }
            let mut lock = cache_mutex.lock().unwrap();
            for (k, v) in novos {
                lock.insert(k, v);
            }
            salvar_cache_prs_disco(base, &lock);
        }

        let cache_snapshot = cache_mutex.lock().unwrap().clone();
        let t0 = Instant::now();
        let dados = coletar_status(base, &repos, &cache_snapshot);
        let cotas = crate::cotas::obter(base);
        let dur = t0.elapsed();
        let largura_tela = crate::tela::tamanho().map(|t| t.0);
        println!(
            "{}",
            renderizar_quadro(&dados, apenas_ativos, Some(&cotas), largura_tela)
        );
        eprintln!("{FRACO}Varredura concluída em {dur:?}{RESET}");
    }
}
