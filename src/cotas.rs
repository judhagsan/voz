//! Cotas de uso dos agentes, lidas das próprias CLIs.
//!
//! O `claude -p /usage` e o `agy -p /quota` respondem em modo
//! não interativo, sem turno de modelo e sem gastar cota. Os dois informam só
//! o percentual de cada janela (5 h e semanal) e quando ela renova; nenhum
//! expõe tokens restantes, então o painel não mostra tokens.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeDelta, TimeZone};
use regex::Regex;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use crate::painel::{
    alinhar, largura_visivel, AMARELO, AZUL, FRACO, MAGENTA, NEGRITO, RESET, VERDE, VERMELHO,
};

/// Leitura mais velha que isso é refeita.
pub const VALIDADE_SEG: i64 = 120;
const LIMITE_COMANDO: Duration = Duration::from_secs(40);
const LARGURA_CARD: usize = 48;
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cotas {
    pub claude: Provedor,
    pub antigravity: Provedor,
    pub lido_em: i64,
}

fn provedor_com_erro(erro: String) -> Provedor {
    Provedor {
        grupos: Vec::new(),
        erro: Some(erro),
    }
}

/// Roda a CLI com prazo; sem ele, uma CLI travada prenderia o painel.
fn executar(programa: &str, args: &[&str], cwd: &Path) -> Result<String, String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let candidatos = [
        programa.to_string(),
        format!("{home}/.local/bin/{programa}"),
    ];
    let mut filho = None;
    for caminho in &candidatos {
        let tentativa = Command::new(caminho)
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(f) = tentativa {
            filho = Some(f);
            break;
        }
    }
    let mut filho = filho.ok_or_else(|| format!("{programa} não encontrado"))?;

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
        return Err(format!("{programa} falhou ({})", saida.status));
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
            });
        }
        // A API não garante a ordem; 5h antes da semana, igual ao `/usage`.
        janelas.sort_by_key(|j| j.rotulo != "5h");
        grupos.push(Grupo { nome, janelas });
    }
    // O Argon tem cota própria e é o modelo principal; fica no topo.
    grupos.sort_by_key(|g| !g.nome.contains("Argon"));
    Provedor { grupos, erro: None }
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

fn buscar_antigravity(cwd: &Path) -> Provedor {
    match executar("agy", &["-p", "/quota", "--output-format", "json"], cwd) {
        Ok(saida) => ler_antigravity(&saida),
        Err(e) => provedor_com_erro(e),
    }
}

fn arquivo_cache(base: &Path) -> std::path::PathBuf {
    base.join("estado").join("cache-cotas.json")
}

fn provedor_para_json(p: &Provedor) -> serde_json::Value {
    serde_json::json!({
        "erro": p.erro,
        "grupos": p.grupos.iter().map(|g| serde_json::json!({
            "nome": g.nome,
            "janelas": g.janelas.iter().map(|j| serde_json::json!({
                "rotulo": j.rotulo,
                "usado": j.usado,
                "renova": j.renova,
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
    })
}

fn salvar_cache(base: &Path, cotas: &Cotas) {
    let valor = serde_json::json!({
        "lido_em": cotas.lido_em,
        "claude": provedor_para_json(&cotas.claude),
        "antigravity": provedor_para_json(&cotas.antigravity),
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
    })
}

pub fn fresca(cotas: &Cotas) -> bool {
    Local::now().timestamp() - cotas.lido_em < VALIDADE_SEG
}

/// Consulta as duas CLIs em paralelo e grava o resultado.
pub fn atualizar(base: &Path) -> Cotas {
    let cwd = base.join("estado");
    let _ = fs::create_dir_all(&cwd);
    let (claude, antigravity) = thread::scope(|s| {
        let c = s.spawn(|| buscar_claude(&cwd));
        let a = s.spawn(|| buscar_antigravity(&cwd));
        (
            c.join()
                .unwrap_or_else(|_| provedor_com_erro("falha interna".to_string())),
            a.join()
                .unwrap_or_else(|_| provedor_com_erro("falha interna".to_string())),
        )
    });
    let cotas = Cotas {
        claude,
        antigravity,
        lido_em: Local::now().timestamp(),
    };
    salvar_cache(base, &cotas);
    cotas
}

/// Usa o cache se ainda vale; senão consulta de novo.
pub fn obter(base: &Path) -> Cotas {
    match ler_cache(base) {
        Some(c) if fresca(&c) => c,
        _ => atualizar(base),
    }
}

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

fn barra(usado: u32) -> String {
    let cheio = (usado.min(100) as usize * 10) / 100;
    let cor = cor_nivel(usado);
    format!(
        "{cor}[{}{RESET}{}{cor}]{RESET}",
        "█".repeat(cheio),
        "░".repeat(10 - cheio)
    )
}

fn linha(conteudo: &str) -> String {
    format!("│ {} │", alinhar(conteudo, LARGURA_CARD))
}

fn cabecalho(nome: &str, cor: &str, legenda: &str) -> String {
    let esquerda = format!("{NEGRITO}{cor}{nome}{RESET}");
    let espaco = LARGURA_CARD.saturating_sub(largura_visivel(&esquerda) + legenda.chars().count());
    linha(&format!(
        "{esquerda}{}{FRACO}{legenda}{RESET}",
        " ".repeat(espaco)
    ))
}

fn linhas_provedor(p: &Provedor, agora: DateTime<Local>) -> Vec<String> {
    if let Some(erro) = &p.erro {
        let curto: String = erro.chars().take(LARGURA_CARD - 2).collect();
        return vec![linha(&format!("  {FRACO}{curto}{RESET}"))];
    }
    let mut linhas = Vec::new();
    for g in &p.grupos {
        if !g.nome.is_empty() {
            linhas.push(linha(&g.nome));
        }
        for j in &g.janelas {
            let renova = j
                .renova
                .map(|r| format!("↻ {}", formatar_renovacao(r, agora)))
                .unwrap_or_default();
            linhas.push(linha(&format!(
                "  {:<11} {} {}{:>3}%{RESET}  {FRACO}{renova}{RESET}",
                j.rotulo,
                barra(j.usado),
                cor_nivel(j.usado),
                j.usado
            )));
        }
    }
    linhas
}

/// Card da direita do painel; sem leitura ainda, avisa que está lendo.
pub fn renderizar_card(cotas: Option<&Cotas>, agora: DateTime<Local>) -> Vec<String> {
    let mut linhas = vec![format!("╭─{}─╮", "─".repeat(LARGURA_CARD))];
    match cotas {
        None => linhas.push(linha(&format!("{FRACO}Lendo as cotas dos agentes…{RESET}"))),
        Some(c) => {
            linhas.push(cabecalho("Claude Code", MAGENTA, "% usado"));
            linhas.extend(linhas_provedor(&c.claude, agora));
            linhas.push(format!("├─{}─┤", "─".repeat(LARGURA_CARD)));
            linhas.push(cabecalho("Antigravity", AZUL, "% usado"));
            linhas.extend(linhas_provedor(&c.antigravity, agora));
            let lido = Local
                .timestamp_opt(c.lido_em, 0)
                .single()
                .map(|d| format!("lido às {}", d.format("%H:%M")))
                .unwrap_or_default();
            let espaco = LARGURA_CARD.saturating_sub(lido.chars().count());
            linhas.push(linha(&format!(
                "{}{FRACO}{lido}{RESET}",
                " ".repeat(espaco)
            )));
        }
    }
    linhas.push(format!("╰─{}─╯", "─".repeat(LARGURA_CARD)));
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
    fn test_cache_ida_e_volta() {
        let pasta = std::env::temp_dir().join(format!("voz-cotas-{}", std::process::id()));
        let cotas = Cotas {
            claude: ler_claude(USAGE, em(2026, 10, 29, 4, 20)),
            antigravity: ler_antigravity(QUOTA),
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
}
