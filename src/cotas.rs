//! Cotas de uso dos agentes, lidas das próprias CLIs.
//!
//! O `claude -p /usage`, o `agy -p /quota` e a interface local do Codex
//! respondem em modo não interativo, sem turno de modelo e sem gastar cota.
//! Eles informam o percentual de cada janela (5 h e semanal) e quando ela
//! renova; nenhum expõe tokens restantes, então o painel não mostra tokens.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeDelta, TimeZone};
use regex::Regex;
use std::fs;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crate::painel::{
    alinhar, largura_visivel, AMARELO, AZUL, FRACO, MAGENTA, NEGRITO, RESET, VERDE, VERMELHO,
};

/// Leitura mais velha que isso é refeita.
pub const VALIDADE_SEG: i64 = 120;
const LIMITE_COMANDO: Duration = Duration::from_secs(40);
const COL_LARGURA: usize = 34;
const LARGURA_CARD: usize = COL_LARGURA * 3 + 6;
const MESES: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Janela {
    pub rotulo: String,
    /// Percentual já usado da janela; passa de 100 quando o limite estoura.
    pub usado: u32,
    /// Instante da renovação, em segundos desde a época Unix.
    pub renova: Option<i64>,
    /// Detalhe textual à direita (ex: saldo de créditos); quando ausente, usa o tempo de renovação.
    pub detalhe: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grupo {
    /// Modelos que dividem as janelas; vazio quando a cota vale para todos.
    pub nome: String,
    pub janelas: Vec<Janela>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Provedor {
    pub grupos: Vec<Grupo>,
    pub erro: Option<String>,
    /// Instante do registro quando a leitura veio do histórico local.
    pub historico_em: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cotas {
    pub claude: Provedor,
    pub antigravity: Provedor,
    /// A assinatura do ChatGPT é a mesma usada pelo Codex.
    pub chatgpt: Provedor,
    pub lido_em: i64,
}

fn provedor_com_erro(erro: String) -> Provedor {
    Provedor {
        grupos: Vec::new(),
        erro: Some(erro),
        historico_em: None,
    }
}

fn iniciar_comando(
    programa: &str,
    args: &[&str],
    cwd: &Path,
    interativo: bool,
) -> Result<Child, String> {
    let home = crate::platform::pasta_usuario()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let candidatos = [
        programa.to_string(),
        format!("{home}/.local/bin/{programa}"),
    ];
    for caminho in &candidatos {
        let tentativa = Command::new(caminho)
            .args(args)
            .current_dir(cwd)
            .stdin(if interativo {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        if let Ok(f) = tentativa {
            return Ok(f);
        }
    }
    Err(format!("{programa} não encontrado"))
}

/// Roda a CLI com prazo; sem ele, uma CLI travada prenderia o painel.
fn executar(programa: &str, args: &[&str], cwd: &Path) -> Result<String, String> {
    let mut filho = iniciar_comando(programa, args, cwd, false)?;

    let inicio = Instant::now();
    loop {
        match filho.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if inicio.elapsed() > LIMITE_COMANDO => {
                let _ = filho.kill();
                let _ = filho.wait();
                return Err(format!("{programa} não respondeu"));
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
    let saida = filho.wait_with_output().map_err(|e| e.to_string())?;
    if !saida.status.success() {
        let stderr = String::from_utf8_lossy(&saida.stderr);
        let detalhe = stderr
            .lines()
            .rev()
            .find(|linha| !linha.trim().is_empty())
            .map(str::trim)
            .unwrap_or("");
        if detalhe.is_empty() {
            return Err(format!("{programa} falhou ({})", saida.status));
        }
        return Err(format!("{programa}: {detalhe}"));
    }
    Ok(String::from_utf8_lossy(&saida.stdout).into_owned())
}

/// Converte "Oct 29 at 6:10am" ou "7pm" no instante da renovação.
///
/// O `/usage` escreve o horário no fuso do sistema e sem o ano.
fn ler_renovacao_claude(texto: &str, agora: DateTime<Local>) -> Option<i64> {
    let re =
        Regex::new(r"^(?:([A-Z][a-z]{2}) (\d{1,2}) at )?(\d{1,2})(?::(\d{2}))?(am|pm)$").ok()?;
    let c = re.captures(texto.trim())?;
    let mut hora: u32 = c[3].parse().ok()?;
    let minuto: u32 = match c.get(4) {
        Some(m) => m.as_str().parse().ok()?,
        None => 0,
    };
    if hora == 12 {
        hora = 0;
    }
    if &c[5] == "pm" {
        hora += 12;
    }

    let montar = |ano: i32, mes: u32, dia: u32| {
        let data = NaiveDate::from_ymd_opt(ano, mes, dia)?;
        Local
            .from_local_datetime(&data.and_hms_opt(hora, minuto, 0)?)
            .earliest()
    };

    let instante = match (c.get(1), c.get(2)) {
        (Some(m), Some(d)) => {
            let mes = MESES.iter().position(|x| *x == m.as_str())? as u32 + 1;
            let dia: u32 = d.as_str().parse().ok()?;
            let neste_ano = montar(agora.year(), mes, dia)?;
            // Sem ano no texto: data que já passou há mais de um dia é do ano seguinte.
            if neste_ano < agora - TimeDelta::days(1) {
                montar(agora.year() + 1, mes, dia)?
            } else {
                neste_ano
            }
        }
        _ => {
            let hoje = montar(agora.year(), agora.month(), agora.day())?;
            // Só o horário: se já passou hoje, é o de amanhã.
            if hoje <= agora {
                hoje + TimeDelta::days(1)
            } else {
                hoje
            }
        }
    };
    Some(instante.timestamp())
}

/// Lê o texto do `claude -p /usage`.
pub fn ler_claude(texto: &str, agora: DateTime<Local>) -> Provedor {
    let re = Regex::new(
        r"(?m)^Current (session|week \(([^)]+)\)): (\d+)% used(?: · resets (.+?))?(?: \([^)]*\))?\s*$",
    )
    .expect("regex do /usage");
    let mut janelas = Vec::new();
    for c in re.captures_iter(texto) {
        let rotulo = match c.get(2).map(|m| m.as_str()) {
            None => "5h".to_string(),
            Some("all models") => "Semana".to_string(),
            Some(modelo) => format!("Semana {modelo}"),
        };
        let usado: u32 = c[3].parse().unwrap_or(100);
        janelas.push(Janela {
            rotulo,
            usado,
            renova: c
                .get(4)
                .and_then(|r| ler_renovacao_claude(r.as_str(), agora)),
            detalhe: None,
        });
    }
    if janelas.is_empty() {
        return provedor_com_erro("sem percentuais no /usage".to_string());
    }
    Provedor {
        grupos: vec![Grupo {
            nome: String::new(),
            janelas,
        }],
        erro: None,
        historico_em: None,
    }
}

/// Lê o JSON do `/quota` do Antigravity.
pub fn ler_antigravity(json: &str) -> Provedor {
    let valor: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return provedor_com_erro("resposta do /quota inválida".to_string()),
    };
    let Some(grupos_json) = valor
        .pointer("/command/data/groups")
        .and_then(|g| g.as_array())
    else {
        return provedor_com_erro("sem grupos no /quota".to_string());
    };

    let mut grupos = Vec::new();
    for g in grupos_json {
        let nome_grupo = g.get("name").and_then(|n| n.as_str()).unwrap_or("");
        let descricao = g.get("description").and_then(|d| d.as_str()).unwrap_or("");
        let nome = descricao
            .strip_prefix("Models within this group: ")
            .unwrap_or(nome_grupo)
            .to_string();
        let mut janelas = Vec::new();
        for b in g
            .get("buckets")
            .and_then(|b| b.as_array())
            .into_iter()
            .flatten()
        {
            let fracao = b
                .get("remaining_fraction")
                .and_then(|f| f.as_f64())
                .unwrap_or(0.0);
            let rotulo = match b.get("window").and_then(|w| w.as_str()) {
                Some("weekly") => "Semana".to_string(),
                Some(outra) => outra.to_string(),
                None => "?".to_string(),
            };
            janelas.push(Janela {
                rotulo,
                // O /quota informa a fração livre; o painel mostra a usada, como o
                // `/usage`. Arredonda como o /quota, para os números baterem.
                usado: ((1.0 - fracao) * 100.0).round().clamp(0.0, 100.0) as u32,
                renova: b
                    .get("reset_time")
                    .and_then(|r| r.as_str())
                    .and_then(|r| DateTime::parse_from_rfc3339(r).ok())
                    .map(|d| d.timestamp()),
                detalhe: None,
            });
        }
        // A API não garante a ordem; 5h antes da semana, igual ao `/usage`.
        janelas.sort_by_key(|j| j.rotulo != "5h");
        grupos.push(Grupo { nome, janelas });
    }
    // O Argon tem cota própria e é o modelo principal; fica no topo.
    grupos.sort_by_key(|g| !g.nome.contains("Argon"));
    Provedor {
        grupos,
        erro: None,
        historico_em: None,
    }
}

fn rotulo_janela_chatgpt(minutos: Option<i64>, indice: usize) -> String {
    match minutos {
        Some(300) => "5h".to_string(),
        Some(10_080) => "Semana".to_string(),
        Some(m) if m >= 1_440 && m % 1_440 == 0 => {
            let dias = m / 1_440;
            if dias == 1 {
                "1 dia".to_string()
            } else {
                format!("{dias} dias")
            }
        }
        Some(m) if m > 0 && m % 60 == 0 => format!("{}h", m / 60),
        Some(m) if m > 0 => format!("{m} min"),
        Some(_) => "?".to_string(),
        // As duas janelas históricas do Codex são 5 horas e semanal. O
        // fallback mantém o painel útil mesmo se uma versão antiga omitir a duração.
        None if indice == 0 => "5h".to_string(),
        None => "Semana".to_string(),
    }
}

fn grupo_chatgpt(quota: &serde_json::Value, nome: String) -> Option<Grupo> {
    let mut janelas = Vec::new();
    for (indice, campo) in ["primary", "secondary"].iter().enumerate() {
        let Some(janela) = quota.get(campo) else {
            continue;
        };
        let Some(usado) = janela
            .get("usedPercent")
            .or_else(|| janela.get("used_percent"))
            .and_then(|u| u.as_f64())
        else {
            continue;
        };
        janelas.push(Janela {
            rotulo: rotulo_janela_chatgpt(
                janela
                    .get("windowDurationMins")
                    .or_else(|| janela.get("window_minutes"))
                    .and_then(|m| m.as_i64()),
                indice,
            ),
            usado: usado.round().clamp(0.0, 100.0) as u32,
            // A interface do Codex entrega timestamp Unix em segundos.
            renova: janela
                .get("resetsAt")
                .or_else(|| janela.get("resets_at"))
                .and_then(|r| r.as_i64()),
            detalhe: None,
        });
    }
    if janelas.is_empty() {
        return None;
    }
    janelas.sort_by_key(|j| j.rotulo != "5h");
    Some(Grupo { nome, janelas })
}

/// Lê a resposta de `account/rateLimits/read` da CLI do Codex.
///
/// São os limites disponíveis ao Codex na conta ChatGPT conectada.
pub fn ler_chatgpt(json: &str) -> Provedor {
    let valor: serde_json::Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(_) => return provedor_com_erro("resposta do Codex inválida".to_string()),
    };
    // A CLI emite JSON-RPC, mas aceitar a resposta interna facilita o teste e
    // tolera uma futura opção que entregue apenas o resultado.
    let resultado = valor.get("result").unwrap_or(&valor);
    if let Some(erro) = valor.pointer("/error/message").and_then(|e| e.as_str()) {
        return provedor_com_erro(format!("Codex: {erro}"));
    }
    let mut grupos = Vec::new();
    if let Some(por_limite) = resultado
        .get("rateLimitsByLimitId")
        .and_then(|limites| limites.as_object())
    {
        let mut limites: Vec<_> = por_limite.iter().collect();
        limites.sort_by_key(|(id, _)| *id);
        for (id, quota) in limites {
            let nome = quota
                .get("limitName")
                .and_then(|n| n.as_str())
                .map(String::from)
                .unwrap_or_else(|| {
                    if id == "codex" {
                        "ChatGPT / Codex".to_string()
                    } else {
                        id.to_string()
                    }
                });
            if let Some(grupo) = grupo_chatgpt(quota, nome) {
                grupos.push(grupo);
            }
        }
    }
    if grupos.is_empty() {
        if let Some(quota) = resultado
            .get("rateLimits")
            .or_else(|| resultado.get("rate_limits"))
        {
            if let Some(grupo) = grupo_chatgpt(quota, "ChatGPT / Codex".to_string()) {
                grupos.push(grupo);
            }
        }
    }
    if grupos.is_empty() {
        return provedor_com_erro("sem cotas na resposta do Codex".to_string());
    }
    if grupos.len() == 1 {
        // O cabeçalho do card já identifica a cota compartilhada.
        grupos[0].nome.clear();
    }
    Provedor {
        grupos,
        erro: None,
        historico_em: None,
    }
}

fn arquivos_jsonl(dir: &Path, encontrados: &mut Vec<PathBuf>) {
    let Ok(itens) = fs::read_dir(dir) else {
        return;
    };
    for item in itens.flatten() {
        let caminho = item.path();
        if caminho.is_dir() {
            arquivos_jsonl(&caminho, encontrados);
        } else if caminho.extension().and_then(|e| e.to_str()) == Some("jsonl") {
            encontrados.push(caminho);
        }
    }
}

/// Última cota que o Codex registrou numa sessão local.
///
/// É um fallback para quando a consulta ao app-server falha. Extrai somente
/// cotas e datas dos eventos `token_count`; não guarda o conteúdo das conversas.
fn ler_cota_do_historico_codex(sessoes: &Path) -> Option<Provedor> {
    let mut arquivos = Vec::new();
    arquivos_jsonl(sessoes, &mut arquivos);
    arquivos.sort_by_key(|arquivo| {
        arquivo
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    let mut mais_recente: Option<Provedor> = None;
    // Limita também os bytes lidos: uma sessão longa pode ter centenas de MB.
    for arquivo in arquivos.iter().rev().take(12) {
        let Ok(mut arquivo) = fs::File::open(arquivo) else {
            continue;
        };
        let tamanho = arquivo.metadata().map(|m| m.len()).unwrap_or(0);
        let inicio = tamanho.saturating_sub(2 * 1024 * 1024);
        if arquivo.seek(SeekFrom::Start(inicio)).is_err() {
            continue;
        }
        let mut bytes = Vec::new();
        if arquivo
            .take(tamanho - inicio)
            .read_to_end(&mut bytes)
            .is_err()
        {
            continue;
        }
        // Descarta a primeira linha quando a leitura começa no meio do arquivo.
        let bytes = if inicio > 0 {
            let Some(nova_linha) = bytes.iter().position(|b| *b == b'\n') else {
                continue;
            };
            &bytes[nova_linha + 1..]
        } else {
            &bytes[..]
        };
        let conteudo = String::from_utf8_lossy(bytes);
        for linha in conteudo.lines().rev() {
            let Some(provedor) = ler_evento_cota_codex(linha) else {
                continue;
            };
            if mais_recente
                .as_ref()
                .is_none_or(|anterior| provedor.historico_em > anterior.historico_em)
            {
                mais_recente = Some(provedor);
            }
            break;
        }
    }
    mais_recente
}

fn ler_evento_cota_codex(linha: &str) -> Option<Provedor> {
    let evento: serde_json::Value = serde_json::from_str(linha).ok()?;
    if evento.get("type")?.as_str()? != "event_msg"
        || evento.pointer("/payload/type")?.as_str()? != "token_count"
    {
        return None;
    }
    let instante = DateTime::parse_from_rfc3339(evento.get("timestamp")?.as_str()?).ok()?;
    let quota = evento.pointer("/payload/rate_limits")?;
    // O registro guarda a cota diretamente, enquanto o parser espera o envelope.
    let mut provedor = ler_chatgpt(&serde_json::json!({"rate_limits": quota}).to_string());
    if provedor.erro.is_some() {
        return None;
    }
    provedor.historico_em = Some(instante.timestamp());
    Some(provedor)
}

fn buscar_claude(cwd: &Path) -> Provedor {
    // Sem as configurações do usuário, a consulta não dispara hooks e não grava sessão.
    let args = [
        "-p",
        "/usage",
        "--setting-sources",
        "project,local",
        "--strict-mcp-config",
        "--no-session-persistence",
        "--output-format",
        "json",
    ];
    match executar("claude", &args, cwd) {
        Ok(saida) => {
            let resultado = serde_json::from_str::<serde_json::Value>(&saida)
                .ok()
                .and_then(|v| v.get("result").and_then(|r| r.as_str()).map(String::from))
                .unwrap_or(saida);
            ler_claude(&resultado, Local::now())
        }
        Err(e) => provedor_com_erro(e),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigCreditosGcp {
    pub ativo: bool,
    pub total: f64,
    pub usado_inicial: f64,
    pub moeda: String,
    pub cotacao_usd: f64,
    pub data_inicio: i64,
}

impl ConfigCreditosGcp {
    pub fn de_json(bloco: Option<&serde_json::Value>) -> Option<ConfigCreditosGcp> {
        let b = bloco?;
        if b.get("ativo").and_then(|a| a.as_bool()) == Some(false) {
            return None;
        }
        let total = b.get("total").and_then(|t| t.as_f64()).unwrap_or(413.28);
        let usado_inicial = b
            .get("usado_inicial")
            .and_then(|u| u.as_f64())
            .unwrap_or(14.69);
        let moeda = b
            .get("moeda")
            .and_then(|m| m.as_str())
            .unwrap_or("R$")
            .to_string();
        let cotacao_usd = b
            .get("cotacao_usd")
            .and_then(|c| c.as_f64())
            .unwrap_or(5.871);
        let data_inicio = b
            .get("data_inicio")
            .and_then(|d| d.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp())
            .unwrap_or(0);

        Some(ConfigCreditosGcp {
            ativo: true,
            total,
            usado_inicial,
            moeda,
            cotacao_usd,
            data_inicio,
        })
    }
}

pub fn calcular_gasto_argon_incremental(
    pasta_brain: &Path,
    desde_timestamp: i64,
    cotacao_usd: f64,
) -> f64 {
    let Ok(entries) = fs::read_dir(pasta_brain) else {
        return 0.0;
    };

    let mut total_in: u64 = 0;
    let mut total_out: u64 = 0;
    let mut total_cache: u64 = 0;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        let transcript = path
            .join(".system_generated")
            .join("logs")
            .join("transcript.jsonl");

        if let Ok(meta) = fs::metadata(&transcript) {
            if let Ok(modificado) = meta.modified() {
                let ts = modificado
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                if ts < desde_timestamp {
                    continue;
                }
            }
        } else {
            continue;
        }

        let Ok(arquivo) = fs::File::open(&transcript) else {
            continue;
        };
        let leitor = BufReader::new(arquivo);
        for linha in leitor.lines().map_while(Result::ok) {
            if !linha.contains("\"PLANNER_RESPONSE\"") {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&linha) {
                if let Some(created) = v.get("created_at").and_then(|c| c.as_str()) {
                    if let Ok(dt) = DateTime::parse_from_rfc3339(created) {
                        if dt.timestamp() < desde_timestamp {
                            continue;
                        }
                    }
                }
                total_in += v.get("input_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                total_out += v.get("output_tokens").and_then(|t| t.as_u64()).unwrap_or(0);
                total_cache += v
                    .get("cache_read_tokens")
                    .and_then(|t| t.as_u64())
                    .unwrap_or(0);
            }
        }
    }

    let custo_usd = (total_in as f64 / 1_000_000.0 * 1.25)
        + (total_out as f64 / 1_000_000.0 * 5.0)
        + (total_cache as f64 / 1_000_000.0 * 0.065);

    custo_usd * cotacao_usd
}

pub fn criar_grupo_creditos_gcp(cfg: &ConfigCreditosGcp, gasto_incremental: f64) -> Grupo {
    let usado_total = cfg.usado_inicial + gasto_incremental;
    let disponivel = (cfg.total - usado_total).max(0.0);
    let pct_usado = if cfg.total > 0.0 {
        ((usado_total / cfg.total) * 100.0)
            .round()
            .clamp(0.0, 100.0) as u32
    } else {
        100
    };

    let texto_livre = format!("{} {:.2} livre", cfg.moeda, disponivel).replace('.', ",");
    let detalhe = if texto_livre.chars().count() <= 15 {
        texto_livre
    } else {
        let texto_curto = format!("{} {:.1} livre", cfg.moeda, disponivel).replace('.', ",");
        if texto_curto.chars().count() <= 15 {
            texto_curto
        } else {
            let texto_int = format!("{} {:.0} livre", cfg.moeda, disponivel);
            texto_int.chars().take(15).collect()
        }
    };

    Grupo {
        nome: "Gemini 4 Argon (API GCP)".to_string(),
        janelas: vec![Janela {
            rotulo: "Créditos".to_string(),
            usado: pct_usado,
            renova: None,
            detalhe: Some(detalhe),
        }],
    }
}

fn buscar_antigravity(base: &Path, cwd: &Path) -> Provedor {
    let mut prov = match executar("agy", &["-p", "/quota", "--output-format", "json"], cwd) {
        Ok(saida) => ler_antigravity(&saida),
        Err(e) => provedor_com_erro(e),
    };

    if let Ok(cfg_texto) = fs::read_to_string(base.join("config.json")) {
        if let Ok(cfg_json) = serde_json::from_str::<serde_json::Value>(&cfg_texto) {
            if let Some(cfg_creditos) = ConfigCreditosGcp::de_json(cfg_json.get("creditos_gcp")) {
                let gasto_incremental = crate::platform::pasta_usuario()
                    .map(|h| h.join(".gemini").join("antigravity-cli").join("brain"))
                    .map(|p| {
                        calcular_gasto_argon_incremental(
                            &p,
                            cfg_creditos.data_inicio,
                            cfg_creditos.cotacao_usd,
                        )
                    })
                    .unwrap_or(0.0);

                let grupo = criar_grupo_creditos_gcp(&cfg_creditos, gasto_incremental);
                if prov.grupos.is_empty() {
                    prov.erro = None;
                }
                if let Some(pos) = prov.grupos.iter().position(|g| g.nome.contains("Argon")) {
                    prov.grupos.insert(pos + 1, grupo);
                } else {
                    prov.grupos.insert(0, grupo);
                }
            }
        }
    }

    prov
}

/// Mantém stdin aberto até a resposta e completa o handshake antes da consulta.
/// Nenhuma thread/conversa ou turno de modelo é criado.
fn consultar_cotas_codex(mut filho: Child, limite: Duration) -> Result<String, String> {
    let (envio, respostas) = mpsc::channel();
    let stdout = filho.stdout.take().expect("stdout redirecionado");
    let leitor = thread::spawn(move || {
        for linha in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(valor) = serde_json::from_str::<serde_json::Value>(&linha) {
                if envio.send(valor).is_err() {
                    break;
                }
            }
        }
    });
    let mut stderr = filho.stderr.take().expect("stderr redirecionado");
    // Drena os logs para que um pipe cheio não trave o protocolo.
    let erros = thread::spawn(move || std::io::copy(&mut stderr, &mut std::io::sink()));
    let resultado = (|| {
        let mut entrada = filho.stdin.take().ok_or("Codex sem entrada JSON-RPC")?;
        let inicio = Instant::now();
        let mut enviar = |mensagem: serde_json::Value| {
            writeln!(entrada, "{mensagem}").map_err(|e| e.to_string())
        };
        let receber = |id: i64| -> Result<serde_json::Value, String> {
            loop {
                let restante = limite.saturating_sub(inicio.elapsed());
                if restante.is_zero() {
                    return Err("Codex não respondeu a tempo".to_string());
                }
                let resposta = respostas.recv_timeout(restante).map_err(|e| match e {
                    mpsc::RecvTimeoutError::Timeout => "Codex não respondeu a tempo".to_string(),
                    mpsc::RecvTimeoutError::Disconnected => {
                        "Codex encerrou sem responder".to_string()
                    }
                })?;
                if resposta.get("id").and_then(|v| v.as_i64()) == Some(id) {
                    if let Some(erro) = resposta.get("error") {
                        return Err(format!(
                            "Codex: {}",
                            erro.get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("falha na consulta")
                        ));
                    }
                    return Ok(resposta);
                }
            }
        };
        enviar(
            serde_json::json!({"id": 1, "method": "initialize", "params": {
                "clientInfo": {"name": "voz", "version": "0.2.0"}
            }}),
        )?;
        receber(1)?;
        enviar(serde_json::json!({"method": "initialized", "params": {}}))?;
        enviar(serde_json::json!({"id": 2, "method": "account/rateLimits/read"}))?;
        Ok(receber(2)?.to_string())
    })();
    // Encerra e recolhe apenas o processo de consulta, inclusive em caso de erro.
    let _ = filho.kill();
    let _ = filho.wait();
    let _ = leitor.join();
    let _ = erros.join();
    resultado
}

fn buscar_chatgpt(cwd: &Path) -> Provedor {
    let resultado_app =
        match iniciar_comando("codex", &["app-server", "--listen", "stdio://"], cwd, true)
            .and_then(|filho| consultar_cotas_codex(filho, LIMITE_COMANDO))
        {
            Ok(saida) => ler_chatgpt(&saida),
            Err(e) => provedor_com_erro(e),
        };
    if resultado_app.erro.is_none() {
        resultado_app
    } else {
        let home_codex = std::env::var_os("CODEX_HOME")
            .filter(|p| !p.is_empty())
            .map(PathBuf::from)
            .or_else(|| crate::platform::pasta_usuario().map(|p| p.join(".codex")));
        home_codex
            .and_then(|p| ler_cota_do_historico_codex(&p.join("sessions")))
            .unwrap_or(resultado_app)
    }
}

fn arquivo_cache(base: &Path) -> std::path::PathBuf {
    base.join("estado").join("cache-cotas.json")
}

fn provedor_para_json(p: &Provedor) -> serde_json::Value {
    serde_json::json!({
        "erro": p.erro,
        "historico_em": p.historico_em,
        "grupos": p.grupos.iter().map(|g| serde_json::json!({
            "nome": g.nome,
            "janelas": g.janelas.iter().map(|j| serde_json::json!({
                "rotulo": j.rotulo,
                "usado": j.usado,
                "renova": j.renova,
                "detalhe": j.detalhe,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    })
}

fn provedor_de_json(v: &serde_json::Value) -> Option<Provedor> {
    let mut grupos = Vec::new();
    for g in v.get("grupos")?.as_array()? {
        let mut janelas = Vec::new();
        for j in g.get("janelas")?.as_array()? {
            janelas.push(Janela {
                rotulo: j.get("rotulo")?.as_str()?.to_string(),
                usado: j.get("usado")?.as_u64()? as u32,
                renova: j.get("renova").and_then(|r| r.as_i64()),
                detalhe: j.get("detalhe").and_then(|d| d.as_str()).map(String::from),
            });
        }
        grupos.push(Grupo {
            nome: g.get("nome")?.as_str()?.to_string(),
            janelas,
        });
    }
    Some(Provedor {
        grupos,
        erro: v.get("erro").and_then(|e| e.as_str()).map(String::from),
        historico_em: v.get("historico_em").and_then(|v| v.as_i64()),
    })
}

fn salvar_cache(base: &Path, cotas: &Cotas) {
    let valor = serde_json::json!({
        "lido_em": cotas.lido_em,
        "claude": provedor_para_json(&cotas.claude),
        "antigravity": provedor_para_json(&cotas.antigravity),
        "chatgpt": provedor_para_json(&cotas.chatgpt),
    });
    if let Ok(texto) = serde_json::to_string_pretty(&valor) {
        let _ = fs::create_dir_all(base.join("estado"));
        let _ = fs::write(arquivo_cache(base), texto);
    }
}

/// Última leitura gravada em disco; outro painel aberto também a atualiza.
pub fn ler_cache(base: &Path) -> Option<Cotas> {
    let texto = fs::read_to_string(arquivo_cache(base)).ok()?;
    let v: serde_json::Value = serde_json::from_str(&texto).ok()?;
    Some(Cotas {
        lido_em: v.get("lido_em")?.as_i64()?,
        claude: provedor_de_json(v.get("claude")?)?,
        antigravity: provedor_de_json(v.get("antigravity")?)?,
        chatgpt: provedor_de_json(v.get("chatgpt")?)?,
    })
}

pub fn fresca(base: &Path, cotas: &Cotas) -> bool {
    if cotas.claude.erro.is_some()
        || cotas.antigravity.erro.is_some()
        || cotas.chatgpt.erro.is_some()
        || Local::now().timestamp() - cotas.lido_em >= VALIDADE_SEG
    {
        return false;
    }
    if let Ok(cfg_texto) = fs::read_to_string(base.join("config.json")) {
        if let Ok(cfg_json) = serde_json::from_str::<serde_json::Value>(&cfg_texto) {
            if ConfigCreditosGcp::de_json(cfg_json.get("creditos_gcp")).is_some() {
                let tem_creditos = cotas
                    .antigravity
                    .grupos
                    .iter()
                    .any(|g| g.nome.contains("Argon (API GCP)"));
                if !tem_creditos {
                    return false;
                }
            }
        }
    }
    true
}

/// Consulta as três CLIs em paralelo e grava o resultado.
pub fn atualizar(base: &Path) -> Cotas {
    let cwd = base.join("estado");
    let _ = fs::create_dir_all(&cwd);
    let (claude, antigravity, chatgpt) = thread::scope(|s| {
        let c = s.spawn(|| buscar_claude(&cwd));
        let a = s.spawn(|| buscar_antigravity(base, &cwd));
        let g = s.spawn(|| buscar_chatgpt(&cwd));
        (
            c.join()
                .unwrap_or_else(|_| provedor_com_erro("falha interna".to_string())),
            a.join()
                .unwrap_or_else(|_| provedor_com_erro("falha interna".to_string())),
            g.join()
                .unwrap_or_else(|_| provedor_com_erro("falha interna".to_string())),
        )
    });
    let cotas = Cotas {
        claude,
        antigravity,
        chatgpt,
        lido_em: Local::now().timestamp(),
    };
    salvar_cache(base, &cotas);
    cotas
}

/// Usa o cache se ainda vale; senão consulta de novo.
pub fn obter(base: &Path) -> Cotas {
    match ler_cache(base) {
        Some(c) if fresca(base, &c) => c,
        _ => atualizar(base),
    }
}

#[allow(dead_code)]
fn formatar_renovacao(renova: i64, agora: DateTime<Local>) -> String {
    let Some(quando) = Local.timestamp_opt(renova, 0).single() else {
        return String::new();
    };
    if quando <= agora {
        "já renovou".to_string()
    } else if quando.date_naive() == agora.date_naive() {
        quando.format("%H:%M").to_string()
    } else {
        quando.format("%d/%m %H:%M").to_string()
    }
}

fn cor_nivel(usado: u32) -> &'static str {
    if usado <= 50 {
        VERDE
    } else if usado <= 80 {
        AMARELO
    } else {
        VERMELHO
    }
}

fn formatar_renovacao_curta(renova: i64, agora: DateTime<Local>) -> String {
    let Some(quando) = Local.timestamp_opt(renova, 0).single() else {
        return String::new();
    };
    if quando <= agora {
        "renov".to_string()
    } else if quando.date_naive() == agora.date_naive() {
        quando.format("%H:%M").to_string()
    } else {
        quando.format("%d/%m").to_string()
    }
}

fn barra(usado: u32) -> String {
    let cheio = (usado.min(100) as usize * 8) / 100;
    let cor = cor_nivel(usado);
    format!(
        "{cor}[{}{RESET}{}{cor}]{RESET}",
        "█".repeat(cheio),
        "░".repeat(8 - cheio)
    )
}

fn linha(conteudo: &str) -> String {
    format!("│ {} │", alinhar(conteudo, LARGURA_CARD))
}

fn cabecalho_coluna(nome: &str, cor: &str, largura: usize) -> String {
    let esquerda = format!("{NEGRITO}{cor}{nome}{RESET}");
    let legenda = "% usado";
    let vis_esq = largura_visivel(&esquerda);
    let vis_leg = legenda.chars().count();
    let espaco = largura.saturating_sub(vis_esq + vis_leg);
    format!("{esquerda}{}{FRACO}{legenda}{RESET}", " ".repeat(espaco))
}

fn format_item_coluna(rotulo: &str, j: &Janela, agora: DateTime<Local>) -> String {
    let mut direita = match &j.detalhe {
        Some(d) => {
            if let Some(pos) = d.find(',') {
                d[..pos].trim().to_string()
            } else if let Some(pos) = d.find('.') {
                d[..pos].trim().to_string()
            } else {
                d.clone()
            }
        }
        None => j
            .renova
            .map(|r| format!("↻ {}", formatar_renovacao_curta(r, agora)))
            .unwrap_or_default(),
    };
    if direita.chars().count() > 7 {
        direita = direita.chars().take(7).collect();
    }
    let espaco_dir = format!("{:<7}", direita);
    format!(
        "{:<10} {} {}{:>3}%{RESET} {FRACO}{espaco_dir}{RESET}",
        rotulo,
        barra(j.usado),
        cor_nivel(j.usado),
        j.usado
    )
}

fn colunas_claude(p: &Provedor, agora: DateTime<Local>) -> Vec<String> {
    if let Some(erro) = &p.erro {
        let curto: String = erro.chars().take(COL_LARGURA).collect();
        return vec![format!(
            "{FRACO}{:<largura$}{RESET}",
            curto,
            largura = COL_LARGURA
        )];
    }
    let mut linhas = Vec::new();
    if let Some(instante) = p
        .historico_em
        .and_then(|t| Local.timestamp_opt(t, 0).single())
    {
        linhas.push(format!(
            "{FRACO}Histórico de {:<largura$}{RESET}",
            instante.format("%d/%m %H:%M"),
            largura = COL_LARGURA.saturating_sub(13)
        ));
    }
    for g in &p.grupos {
        for j in &g.janelas {
            linhas.push(format_item_coluna(&j.rotulo, j, agora));
        }
    }
    linhas
}

fn colunas_antigravity(p: &Provedor, agora: DateTime<Local>) -> Vec<String> {
    if let Some(erro) = &p.erro {
        let curto: String = erro.chars().take(COL_LARGURA).collect();
        return vec![format!(
            "{FRACO}{:<largura$}{RESET}",
            curto,
            largura = COL_LARGURA
        )];
    }
    let mut linhas = Vec::new();
    if let Some(instante) = p
        .historico_em
        .and_then(|t| Local.timestamp_opt(t, 0).single())
    {
        linhas.push(format!(
            "{FRACO}Histórico de {:<largura$}{RESET}",
            instante.format("%d/%m %H:%M"),
            largura = COL_LARGURA.saturating_sub(13)
        ));
    }

    // 1. Argon Assinatura
    if let Some(g_argon) = p
        .grupos
        .iter()
        .find(|g| g.nome.contains("Argon") && !g.nome.contains("API"))
    {
        if let Some(j) = g_argon.janelas.first() {
            linhas.push(format_item_coluna("Argon Sem", j, agora));
        }
    }

    // 2. Argon API
    if let Some(g_api) = p.grupos.iter().find(|g| g.nome.contains("API")) {
        if let Some(j) = g_api.janelas.first() {
            linhas.push(format_item_coluna("Argon API", j, agora));
        }
    }

    // 3. Flash / Pro
    if let Some(g_flash) = p.grupos.iter().find(|g| g.nome.contains("Flash")) {
        let j_5h = g_flash.janelas.iter().find(|j| j.rotulo == "5h");
        let j_sem = g_flash.janelas.iter().find(|j| j.rotulo == "Semana");
        if let Some(j) = j_5h {
            linhas.push(format_item_coluna("Flash 5h", j, agora));
        }
        if let Some(j) = j_sem {
            linhas.push(format_item_coluna("Flash Sem", j, agora));
        }
        if j_5h.is_none() && j_sem.is_none() {
            if let Some(j) = g_flash.janelas.first() {
                linhas.push(format_item_coluna("Flash/Pro", j, agora));
            }
        }
    }

    // 4. Claude / GPT
    if let Some(g_claude) = p
        .grupos
        .iter()
        .find(|g| g.nome.contains("Claude") || g.nome.contains("GPT"))
    {
        let j = g_claude
            .janelas
            .iter()
            .find(|j| j.rotulo == "Semana")
            .or_else(|| g_claude.janelas.first());
        if let Some(j) = j {
            linhas.push(format_item_coluna("Claude/GPT", j, agora));
        }
    }

    // Fallback para outros grupos desconhecidos
    for g in &p.grupos {
        let eh_conhecido = g.nome.contains("Argon")
            || g.nome.contains("Flash")
            || g.nome.contains("Claude")
            || g.nome.contains("GPT");
        if !eh_conhecido && !g.nome.is_empty() {
            if let Some(j) = g.janelas.first() {
                let rotulo: String = g.nome.chars().take(10).collect();
                linhas.push(format_item_coluna(&rotulo, j, agora));
            }
        }
    }

    linhas
}

fn colunas_chatgpt(p: &Provedor, agora: DateTime<Local>) -> Vec<String> {
    if let Some(erro) = &p.erro {
        let curto: String = erro.chars().take(COL_LARGURA).collect();
        return vec![format!(
            "{FRACO}{:<largura$}{RESET}",
            curto,
            largura = COL_LARGURA
        )];
    }
    let mut linhas = Vec::new();
    if let Some(instante) = p
        .historico_em
        .and_then(|t| Local.timestamp_opt(t, 0).single())
    {
        linhas.push(format!(
            "{FRACO}Histórico de {:<largura$}{RESET}",
            instante.format("%d/%m %H:%M"),
            largura = COL_LARGURA.saturating_sub(13)
        ));
    }
    for g in &p.grupos {
        for j in &g.janelas {
            linhas.push(format_item_coluna(&j.rotulo, j, agora));
        }
    }
    linhas
}

#[allow(dead_code)]
fn linhas_provedor(p: &Provedor, agora: DateTime<Local>) -> Vec<String> {
    colunas_claude(p, agora)
        .into_iter()
        .map(|l| linha(&l))
        .collect()
}

/// Card de cotas dos agentes em formato horizontal de 3 colunas.
pub fn renderizar_card(cotas: Option<&Cotas>, agora: DateTime<Local>) -> Vec<String> {
    let topo = format!(
        "╭─{}─┬─{}─┬─{}─╮",
        "─".repeat(COL_LARGURA),
        "─".repeat(COL_LARGURA),
        "─".repeat(COL_LARGURA)
    );
    let sep = format!(
        "├─{}─┼─{}─┼─{}─┤",
        "─".repeat(COL_LARGURA),
        "─".repeat(COL_LARGURA),
        "─".repeat(COL_LARGURA)
    );
    let base = format!(
        "╰─{}─┴─{}─┴─{}─╯",
        "─".repeat(COL_LARGURA),
        "─".repeat(COL_LARGURA),
        "─".repeat(COL_LARGURA)
    );

    let mut linhas = vec![topo];
    match cotas {
        None => {
            linhas.push(linha(&format!("{FRACO}Lendo as cotas dos agentes…{RESET}")));
        }
        Some(c) => {
            let cab1 = cabecalho_coluna("Claude Code", MAGENTA, COL_LARGURA);
            let cab2 = cabecalho_coluna("Antigravity", AZUL, COL_LARGURA);
            let cab3 = cabecalho_coluna("ChatGPT / Codex", VERDE, COL_LARGURA);
            linhas.push(format!("│ {} │ {} │ {} │", cab1, cab2, cab3));
            linhas.push(sep);

            let mut col1 = colunas_claude(&c.claude, agora);
            let col2 = colunas_antigravity(&c.antigravity, agora);
            let mut col3 = colunas_chatgpt(&c.chatgpt, agora);

            let lido = Local
                .timestamp_opt(c.lido_em, 0)
                .single()
                .map(|d| format!("lido às {}", d.format("%H:%M")))
                .unwrap_or_default();

            let total_linhas = col1.len().max(col2.len()).max(col3.len()).max(4);

            while col1.len() < total_linhas {
                col1.push(" ".repeat(COL_LARGURA));
            }
            while col3.len() < total_linhas.saturating_sub(1) {
                col3.push(" ".repeat(COL_LARGURA));
            }
            if col3.len() < total_linhas {
                let espaco = COL_LARGURA.saturating_sub(lido.chars().count());
                col3.push(format!("{}{FRACO}{lido}{RESET}", " ".repeat(espaco)));
            }

            for i in 0..total_linhas {
                let l1 = col1.get(i).map(String::as_str).unwrap_or("");
                let l2 = col2.get(i).map(String::as_str).unwrap_or("");
                let l3 = col3.get(i).map(String::as_str).unwrap_or("");
                linhas.push(format!(
                    "│ {} │ {} │ {} │",
                    alinhar(l1, COL_LARGURA),
                    alinhar(l2, COL_LARGURA),
                    alinhar(l3, COL_LARGURA)
                ));
            }
        }
    }
    linhas.push(base);
    linhas
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn em(ano: i32, mes: u32, dia: u32, hora: u32, minuto: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(ano, mes, dia, hora, minuto, 0)
            .unwrap()
    }

    const USAGE: &str = "You are currently using your subscription to power your Claude Code usage

Current session: 100% used · resets Oct 29 at 6:10am (America/Sao_Paulo)
Current week (all models): 65% used · resets Nov 3 at 7pm (America/Sao_Paulo)

What's contributing to your usage?";

    const QUOTA: &str = r#"{"status":"SUCCESS","command":{"name":"usage","data":{"groups":[
        {"name":"Gemini Models","description":"Models within this group: Gemini Flash, Gemini Pro","buckets":[
            {"window":"weekly","remaining_fraction":0.8187,"reset_time":"2026-11-04T16:30:22Z"},
            {"window":"5h","remaining_fraction":0.4311,"reset_time":"2026-10-29T07:30:22Z"}]},
        {"name":"Claude and GPT models","description":"Models within this group: Claude Opus, Claude Sonnet, GPT-OSS","buckets":[
            {"window":"5h","remaining_fraction":0,"reset_time":"2026-10-29T07:29:08Z"}]},
        {"name":"Gemini 4 Argon","description":"For paid Pro plans, Argon quota is separate.","buckets":[
            {"window":"weekly","remaining_fraction":0,"reset_time":"2026-11-04T16:36:14Z"}]}]}}}"#;

    const CHATGPT: &str = r#"{"jsonrpc":"2.0","id":2,"result":{"rateLimits":{"primary":{"usedPercent":42,"windowDurationMins":300,"resetsAt":1793261400},"secondary":{"usedPercent":71,"windowDurationMins":10080,"resetsAt":1793628000}},"rateLimitsByLimitId":{"codex":{"primary":{"usedPercent":42,"windowDurationMins":300,"resetsAt":1793261400},"secondary":{"usedPercent":71,"windowDurationMins":10080,"resetsAt":1793628000}}}}}"#;

    const CHATGPT_HISTORICO: &str = r#"{"rate_limits":{"primary":{"used_percent":52.0,"window_minutes":43200,"resets_at":1796145972},"secondary":null}}"#;

    #[test]
    fn test_ler_claude_le_percentual_usado() {
        let p = ler_claude(USAGE, em(2026, 10, 29, 4, 20));
        assert_eq!(p.erro, None);
        let j = &p.grupos[0].janelas;
        assert_eq!(j.len(), 2);
        assert_eq!((j[0].rotulo.as_str(), j[0].usado), ("5h", 100));
        assert_eq!(j[0].renova, Some(em(2026, 10, 29, 6, 10).timestamp()));
        assert_eq!((j[1].rotulo.as_str(), j[1].usado), ("Semana", 65));
        assert_eq!(j[1].renova, Some(em(2026, 11, 3, 19, 0).timestamp()));
    }

    #[test]
    fn test_ler_claude_sem_percentuais_vira_erro() {
        let p = ler_claude("Not logged in", em(2026, 10, 29, 4, 20));
        assert!(p.erro.is_some());
        assert!(p.grupos.is_empty());
    }

    #[test]
    fn test_renovacao_claude_vira_o_ano() {
        let agora = em(2026, 12, 30, 10, 0);
        let r = ler_renovacao_claude("Jan 2 at 9am", agora);
        assert_eq!(r, Some(em(2027, 1, 2, 9, 0).timestamp()));
    }

    #[test]
    fn test_renovacao_claude_so_horario() {
        assert_eq!(
            ler_renovacao_claude("6:10am", em(2026, 10, 29, 5, 0)),
            Some(em(2026, 10, 29, 6, 10).timestamp())
        );
        assert_eq!(
            ler_renovacao_claude("6:10am", em(2026, 10, 29, 7, 0)),
            Some(em(2026, 10, 30, 6, 10).timestamp())
        );
        assert_eq!(
            ler_renovacao_claude("12pm", em(2026, 10, 29, 7, 0)),
            Some(em(2026, 10, 29, 12, 0).timestamp())
        );
        assert_eq!(
            ler_renovacao_claude("12am", em(2026, 10, 29, 7, 0)),
            Some(em(2026, 10, 30, 0, 0).timestamp())
        );
    }

    #[test]
    fn test_ler_antigravity_ordena_e_nomeia_grupos() {
        let p = ler_antigravity(QUOTA);
        assert_eq!(p.erro, None);
        let nomes: Vec<&str> = p.grupos.iter().map(|g| g.nome.as_str()).collect();
        assert_eq!(
            nomes,
            [
                "Gemini 4 Argon",
                "Gemini Flash, Gemini Pro",
                "Claude Opus, Claude Sonnet, GPT-OSS"
            ]
        );
        let gemini = &p.grupos[1].janelas;
        assert_eq!((gemini[0].rotulo.as_str(), gemini[0].usado), ("5h", 57));
        assert_eq!((gemini[1].rotulo.as_str(), gemini[1].usado), ("Semana", 18));
        assert_eq!(
            gemini[0].renova,
            Some(
                DateTime::parse_from_rfc3339("2026-10-29T07:30:22Z")
                    .unwrap()
                    .timestamp()
            )
        );
    }

    #[test]
    fn test_ler_antigravity_invalido_vira_erro() {
        assert!(ler_antigravity("não é json").erro.is_some());
        assert!(ler_antigravity("{}").erro.is_some());
    }

    #[test]
    fn test_ler_chatgpt_le_cota_compartilhada_com_codex() {
        let p = ler_chatgpt(CHATGPT);
        assert_eq!(p.erro, None);
        assert_eq!(p.grupos.len(), 1);
        assert!(p.grupos[0].nome.is_empty());
        let j = &p.grupos[0].janelas;
        assert_eq!((j[0].rotulo.as_str(), j[0].usado), ("5h", 42));
        assert_eq!(j[0].renova, Some(1_793_261_400));
        assert_eq!((j[1].rotulo.as_str(), j[1].usado), ("Semana", 71));
        assert_eq!(j[1].renova, Some(1_793_628_000));
    }

    #[test]
    fn test_ler_chatgpt_sem_secundaria() {
        let p =
            ler_chatgpt(r#"{"rateLimits":{"primary":{"usedPercent":4,"windowDurationMins":300}}}"#);
        assert_eq!(p.erro, None);
        assert_eq!(p.grupos[0].janelas.len(), 1);
        assert_eq!(p.grupos[0].janelas[0].rotulo, "5h");
    }

    #[test]
    fn test_ler_chatgpt_aceita_registro_local_do_codex() {
        let p = ler_chatgpt(CHATGPT_HISTORICO);
        assert_eq!(p.erro, None);
        let j = &p.grupos[0].janelas;
        assert_eq!(j.len(), 1);
        assert_eq!((j[0].rotulo.as_str(), j[0].usado), ("30 dias", 52));
        assert_eq!(j[0].renova, Some(1_796_145_972));
    }

    fn evento_cota(instante: &str, usado: u32) -> String {
        serde_json::json!({
            "type": "event_msg", "timestamp": instante,
            "payload": {"type": "token_count", "info": {"total_token_usage": {}},
                "rate_limits": {"primary": {"used_percent": usado, "window_minutes": 300}}}
        })
        .to_string()
    }

    #[test]
    fn test_historico_le_envelope_real_e_preserva_data() {
        let p = ler_evento_cota_codex(&evento_cota("2026-11-01T17:46:21.437Z", 13)).unwrap();
        assert_eq!(p.grupos[0].janelas[0].usado, 13);
        assert_eq!(p.historico_em, Some(1_793_555_181));
        assert_eq!(provedor_de_json(&provedor_para_json(&p)), Some(p.clone()));
        let linhas = linhas_provedor(&p, em(2026, 11, 1, 15, 0));
        assert!(linhas[0].contains("Histórico de"));
        assert!(linhas
            .iter()
            .all(|l| largura_visivel(l) == LARGURA_CARD + 4));
    }

    #[test]
    fn test_historico_ignora_eventos_sem_cota_ou_com_outro_tipo() {
        assert!(ler_evento_cota_codex("{incompleto").is_none());
        let mut evento: serde_json::Value =
            serde_json::from_str(&evento_cota("2026-11-01T17:46:21Z", 13)).unwrap();
        evento["payload"]["type"] = serde_json::json!("agent_message");
        assert!(ler_evento_cota_codex(&evento.to_string()).is_none());
        evento["payload"]["type"] = serde_json::json!("token_count");
        evento["payload"]["rate_limits"] = serde_json::Value::Null;
        assert!(ler_evento_cota_codex(&evento.to_string()).is_none());
    }

    #[test]
    fn test_historico_escolhe_data_do_evento_em_qualquer_projeto() {
        let pasta = std::env::temp_dir().join(format!("voz-historico-{}", std::process::id()));
        fs::create_dir_all(pasta.join("2026/11/01")).unwrap();
        fs::write(
            pasta.join("2026/11/01/atual.jsonl"),
            format!("{}\n{{parcial", evento_cota("2026-11-01T17:46:21Z", 13)),
        )
        .unwrap();
        fs::write(
            pasta.join("antiga.jsonl"),
            evento_cota("2026-11-01T17:40:00Z", 1),
        )
        .unwrap();
        let p = ler_cota_do_historico_codex(&pasta).unwrap();
        assert_eq!(p.grupos[0].janelas[0].usado, 13);
        fs::remove_dir_all(pasta).unwrap();
    }

    fn servidor_falso(script: &str, limite: Duration) -> Result<String, String> {
        let filho = iniciar_comando(
            "/usr/bin/python3",
            &["-u", "-c", script],
            &std::env::temp_dir(),
            true,
        )
        .unwrap();
        consultar_cotas_codex(filho, limite)
    }

    #[test]
    fn test_codex_aguarda_handshake_e_resposta_sem_fechar_stdin() {
        let resposta = servidor_falso(r#"
import json, sys, time
assert json.loads(sys.stdin.readline())['method'] == 'initialize'
time.sleep(0.05)
print(json.dumps({'id': 1, 'result': {}}))
assert json.loads(sys.stdin.readline())['method'] == 'initialized'
assert json.loads(sys.stdin.readline())['method'] == 'account/rateLimits/read'
print(json.dumps({'method': 'account/updated', 'params': {}}))
time.sleep(0.05)
print(json.dumps({'id': 2, 'result': {'rateLimits': {'primary': {'usedPercent': 13, 'windowDurationMins': 300}}}}))
sys.stdin.read()
"#, Duration::from_secs(5)).unwrap();
        assert_eq!(ler_chatgpt(&resposta).grupos[0].janelas[0].usado, 13);
    }

    #[test]
    fn test_codex_preserva_erro_json_rpc() {
        let erro = servidor_falso(
            r#"
import json, sys
json.loads(sys.stdin.readline())
print(json.dumps({'id': 1, 'error': {'code': -1, 'message': 'login necessário'}}))
sys.stdin.read()
"#,
            Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(erro.contains("login necessário"));
    }

    #[test]
    fn test_codex_tem_prazo_e_trata_saida_sem_resposta() {
        let inicio = Instant::now();
        assert!(
            servidor_falso("import time; time.sleep(10)", Duration::from_millis(100))
                .unwrap_err()
                .contains("a tempo")
        );
        assert!(inicio.elapsed() < Duration::from_secs(3));
        assert!(servidor_falso("pass", Duration::from_secs(5))
            .unwrap_err()
            .contains("sem responder"));
    }

    #[test]
    fn test_cache_ida_e_volta() {
        let pasta = std::env::temp_dir().join(format!("voz-cotas-{}", std::process::id()));
        let cotas = Cotas {
            claude: ler_claude(USAGE, em(2026, 10, 29, 4, 20)),
            antigravity: ler_antigravity(QUOTA),
            chatgpt: ler_chatgpt(CHATGPT),
            lido_em: 1_790_000_000,
        };
        salvar_cache(&pasta, &cotas);
        assert_eq!(ler_cache(&pasta), Some(cotas));
        let _ = fs::remove_dir_all(&pasta);
    }

    #[test]
    fn test_cache_no_formato_antigo_e_ignorado() {
        let pasta = std::env::temp_dir().join(format!("voz-cotas-antigo-{}", std::process::id()));
        fs::create_dir_all(pasta.join("estado")).unwrap();
        fs::write(
            arquivo_cache(&pasta),
            r#"{"claude":{"modelos":[{"nome":"Opus 5.5","porcentagem":53}]}}"#,
        )
        .unwrap();
        assert_eq!(ler_cache(&pasta), None);
        // Cache da versão que guardava o percentual livre também é refeito.
        let grupo_livre = r#"{"erro":null,"grupos":[{"nome":"","janelas":[{"rotulo":"5h","livre":4,"renova":null}]}]}"#;
        fs::write(
            arquivo_cache(&pasta),
            format!(r#"{{"lido_em":1,"claude":{grupo_livre},"antigravity":{grupo_livre}}}"#),
        )
        .unwrap();
        assert_eq!(ler_cache(&pasta), None);
        let _ = fs::remove_dir_all(&pasta);
    }

    #[test]
    fn test_cota_com_erro_nao_fica_no_cache() {
        let cotas = Cotas {
            claude: provedor_com_erro("indisponível".to_string()),
            antigravity: ler_antigravity(QUOTA),
            chatgpt: ler_chatgpt(CHATGPT),
            lido_em: Local::now().timestamp(),
        };
        assert!(!fresca(Path::new("."), &cotas));
    }

    #[test]
    fn test_cache_sem_creditos_fica_invalido_quando_configurado() {
        let pasta = std::env::temp_dir().join(format!("voz-fresca-{}", std::process::id()));
        fs::create_dir_all(&pasta).unwrap();
        fs::write(
            pasta.join("config.json"),
            serde_json::json!({
                "creditos_gcp": {
                    "ativo": true,
                    "total": 400.0,
                    "usado_inicial": 10.0
                }
            })
            .to_string(),
        )
        .unwrap();

        let cotas_sem_creditos = Cotas {
            claude: ler_claude(USAGE, em(2026, 10, 29, 4, 20)),
            antigravity: ler_antigravity(QUOTA),
            chatgpt: ler_chatgpt(CHATGPT),
            lido_em: Local::now().timestamp(),
        };

        assert!(!fresca(&pasta, &cotas_sem_creditos));

        let mut agy = ler_antigravity(QUOTA);
        let cfg = ConfigCreditosGcp {
            ativo: true,
            total: 400.0,
            usado_inicial: 10.0,
            moeda: "R$".to_string(),
            cotacao_usd: 5.8,
            data_inicio: 0,
        };
        agy.grupos.push(criar_grupo_creditos_gcp(&cfg, 0.0));
        let cotas_com_creditos = Cotas {
            claude: ler_claude(USAGE, em(2026, 10, 29, 4, 20)),
            antigravity: agy,
            chatgpt: ler_chatgpt(CHATGPT),
            lido_em: Local::now().timestamp(),
        };

        assert!(fresca(&pasta, &cotas_com_creditos));

        let _ = fs::remove_dir_all(&pasta);
    }

    #[test]
    fn test_formatar_renovacao() {
        let agora = em(2026, 10, 29, 4, 20);
        let hoje = em(2026, 10, 29, 6, 10).timestamp();
        let depois = em(2026, 11, 3, 19, 0).timestamp();
        let antes = em(2026, 10, 29, 4, 0).timestamp();
        assert_eq!(formatar_renovacao(hoje, agora), "06:10");
        assert_eq!(formatar_renovacao(depois, agora), "03/11 19:00");
        assert_eq!(formatar_renovacao(antes, agora), "já renovou");
    }

    #[test]
    fn test_card_tem_largura_constante() {
        let agora = em(2026, 10, 29, 4, 20);
        let cotas = Cotas {
            claude: ler_claude(USAGE, agora),
            antigravity: provedor_com_erro("agy não encontrado".to_string()),
            chatgpt: ler_chatgpt(CHATGPT),
            lido_em: agora.timestamp(),
        };
        for card in [
            renderizar_card(Some(&cotas), agora),
            renderizar_card(None, agora),
        ] {
            let larguras: HashSet<usize> = card.iter().map(|l| largura_visivel(l)).collect();
            assert_eq!(
                larguras.len(),
                1,
                "linhas com larguras diferentes: {card:#?}"
            );
        }
    }

    #[test]
    fn test_config_creditos_gcp_de_json() {
        assert_eq!(ConfigCreditosGcp::de_json(None), None);

        let desativado = serde_json::json!({"ativo": false});
        assert_eq!(ConfigCreditosGcp::de_json(Some(&desativado)), None);

        let cfg_json = serde_json::json!({
            "ativo": true,
            "total": 413.28,
            "usado_inicial": 14.69,
            "moeda": "R$",
            "cotacao_usd": 5.871,
            "data_inicio": "2026-11-02T11:45:00Z"
        });
        let cfg = ConfigCreditosGcp::de_json(Some(&cfg_json)).unwrap();
        assert!(cfg.ativo);
        assert_eq!(cfg.total, 413.28);
        assert_eq!(cfg.usado_inicial, 14.69);
        assert_eq!(cfg.moeda, "R$");
        assert_eq!(cfg.cotacao_usd, 5.871);
        assert!(cfg.data_inicio > 0);
    }

    #[test]
    fn test_criar_grupo_creditos_gcp_calcula_saldo_e_detalhe() {
        let cfg = ConfigCreditosGcp {
            ativo: true,
            total: 413.28,
            usado_inicial: 14.69,
            moeda: "R$".to_string(),
            cotacao_usd: 5.871,
            data_inicio: 1_790_000_000,
        };
        let grupo = criar_grupo_creditos_gcp(&cfg, 0.0);
        assert_eq!(grupo.nome, "Gemini 4 Argon (API GCP)");
        assert_eq!(grupo.janelas.len(), 1);
        let j = &grupo.janelas[0];
        assert_eq!(j.rotulo, "Créditos");
        assert_eq!(j.usado, 4);
        assert_eq!(j.renova, None);
        assert_eq!(j.detalhe.as_deref(), Some("R$ 398,59 livre"));
    }

    #[test]
    fn test_calcular_gasto_argon_incremental_com_transcripts() {
        let pasta = std::env::temp_dir().join(format!("voz-brain-test-{}", std::process::id()));
        let logs_dir = pasta
            .join("sessao-1")
            .join(".system_generated")
            .join("logs");
        fs::create_dir_all(&logs_dir).unwrap();

        let t1 = "2026-11-02T12:00:00Z";
        let t0 = "2026-11-01T12:00:00Z";

        let l1 = serde_json::json!({
            "type": "PLANNER_RESPONSE",
            "created_at": t1,
            "input_tokens": 100_000,
            "output_tokens": 10_000,
            "cache_read_tokens": 0
        })
        .to_string();

        let l0 = serde_json::json!({
            "type": "PLANNER_RESPONSE",
            "created_at": t0,
            "input_tokens": 500_000,
            "output_tokens": 50_000,
            "cache_read_tokens": 0
        })
        .to_string();

        fs::write(logs_dir.join("transcript.jsonl"), format!("{l0}\n{l1}\n")).unwrap();

        let dt_inicio = DateTime::parse_from_rfc3339("2026-11-02T11:45:00Z")
            .unwrap()
            .timestamp();

        let gasto = calcular_gasto_argon_incremental(&pasta, dt_inicio, 5.0);
        assert!((gasto - 0.875).abs() < 0.001);

        let _ = fs::remove_dir_all(&pasta);
    }

    #[test]
    fn test_card_com_creditos_gcp_tem_largura_constante() {
        let agora = em(2026, 11, 2, 12, 0);
        let cfg = ConfigCreditosGcp {
            ativo: true,
            total: 413.28,
            usado_inicial: 14.69,
            moeda: "R$".to_string(),
            cotacao_usd: 5.871,
            data_inicio: 1_790_000_000,
        };
        let mut agy = ler_antigravity(QUOTA);
        agy.grupos.insert(0, criar_grupo_creditos_gcp(&cfg, 0.0));

        let cotas = Cotas {
            claude: ler_claude(USAGE, agora),
            antigravity: agy,
            chatgpt: ler_chatgpt(CHATGPT),
            lido_em: agora.timestamp(),
        };

        let card = renderizar_card(Some(&cotas), agora);
        let larguras: HashSet<usize> = card.iter().map(|l| largura_visivel(l)).collect();
        assert_eq!(
            larguras.len(),
            1,
            "largura inconsistente com créditos: {card:#?}"
        );
        assert!(card.iter().any(|l| l.contains("Flash 5h")));
        assert!(card.iter().any(|l| l.contains("Flash Sem")));
        assert!(card.iter().any(|l| l.contains("Argon Sem")));
        assert!(card.iter().any(|l| l.contains("Argon API")));
    }
}
