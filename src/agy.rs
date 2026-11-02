use std::collections::HashSet;
use std::fs;

const BUILTINS_LIVRES: &[&str] = &[
    "cd", "true", "false", ":", "echo", "printf", "sleep", "for", "do", "done", "test", "[", "]",
    "setopt", "set", "export", "source", ".",
];

const UTILITARIOS_LEITURA: &[&str] = &[
    "ls", "cat", "head", "tail", "more", "less", "pwd", "dirname", "basename", "readlink",
    "realpath", "grep", "egrep", "fgrep", "rg", "ag", "find", "fd", "which", "where", "whereis",
    "type", "file", "stat", "diff", "cmp", "wc", "sort", "uniq", "cut", "tr", "column", "fmt",
    "fold", "tee", "sips", "afinfo", "strings", "otool", "nm", "uname", "whoami", "id", "date",
    "cal", "env", "printenv",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgyDecisao {
    Allow,
    Ask,
    Espera,
}

fn extrair_tokens(sub: &str) -> Vec<&str> {
    let mut tokens: Vec<&str> = sub.split_whitespace().collect();
    // Remove atribuições de variáveis de ambiente no início: FOO=bar cmd
    while !tokens.is_empty() && tokens[0].contains('=') && !tokens[0].starts_with('-') {
        tokens.remove(0);
    }
    // Remove comando "env" inicial e flags associadas se presente
    if !tokens.is_empty() && tokens[0] == "env" {
        tokens.remove(0);
        while !tokens.is_empty()
            && (tokens[0].contains('=') || tokens[0] == "-i" || tokens[0] == "-u")
        {
            tokens.remove(0);
        }
    }
    tokens
}

fn git_subcomando_autorizado(tokens: &[&str]) -> bool {
    let mut i = 1;
    // Pula opções globais do git como -C <dir>, --git-dir=<dir>, -c <key=val>, etc.
    while i < tokens.len() {
        let t = tokens[i];
        if t == "-C" || t == "-c" || t == "--git-dir" || t == "--work-tree" {
            i += 2;
        } else if t.starts_with('-') {
            i += 1;
        } else {
            break;
        }
    }
    if i >= tokens.len() {
        return true;
    }
    let subcmd = tokens[i];
    match subcmd {
        "status" | "log" | "diff" | "show" | "grep" | "rev-parse" | "describe" | "version"
        | "ls-files" | "shortlog" | "check-ref-format" => true,
        "branch" => !tokens
            .iter()
            .any(|&t| t == "-d" || t == "-D" || t == "-m" || t == "-M"),
        "remote" => !tokens
            .iter()
            .any(|&t| t == "add" || t == "set-url" || t == "remove" || t == "rm"),
        "tag" => !tokens.iter().any(|&t| t == "-d" || t == "-a"),
        "stash" => tokens
            .get(i + 1)
            .map(|&s| s == "list" || s == "show")
            .unwrap_or(false),
        _ => false,
    }
}

fn cargo_subcomando_autorizado(tokens: &[&str]) -> bool {
    let mut i = 1;
    while i < tokens.len() && tokens[i].starts_with('-') {
        if tokens[i] == "--manifest-path" || tokens[i] == "--target-dir" {
            i += 2;
        } else {
            i += 1;
        }
    }
    if i >= tokens.len() {
        return true;
    }
    let subcmd = tokens[i];
    matches!(
        subcmd,
        "check" | "clippy" | "test" | "fmt" | "doc" | "tree" | "metadata" | "version"
    )
}

fn npm_subcomando_autorizado(tokens: &[&str]) -> bool {
    if tokens.len() < 2 {
        return true;
    }
    let subcmd = tokens[1];
    if matches!(subcmd, "test" | "ls" | "-v" | "--version") {
        return true;
    }
    if subcmd == "run" && tokens.len() > 2 {
        let script = tokens[2];
        return script.starts_with("lint")
            || script.starts_with("check")
            || script.starts_with("test")
            || script.starts_with("prettier")
            || script.starts_with("eslint")
            || script.starts_with("format")
            || script.starts_with("build");
    }
    false
}

fn npx_subcomando_autorizado(tokens: &[&str]) -> bool {
    if tokens.len() < 2 {
        return true;
    }
    let tool = tokens[1];
    matches!(
        tool,
        "eslint" | "prettier" | "tsc" | "next" | "stylelint" | "--version" | "-v"
    )
}

fn subcomando_autorizado(sub: &str, allow_set: &HashSet<String>) -> bool {
    if allow_set.contains(&format!("command({sub})")) {
        return true;
    }

    let tokens = extrair_tokens(sub);
    if tokens.is_empty() {
        return true;
    }
    let prim = tokens[0];

    // Checa se o comando ou prefixo consta explicitamente no settings.json
    if allow_set.contains(&format!("command({prim})")) {
        return true;
    }
    if tokens.len() > 1 && allow_set.contains(&format!("command({prim} {})", tokens[1])) {
        return true;
    }
    if tokens.len() > 2
        && allow_set.contains(&format!("command({prim} {} {})", tokens[1], tokens[2]))
    {
        return true;
    }

    // Comandos built-in do shell
    if BUILTINS_LIVRES.contains(&prim) {
        return true;
    }

    // Utilitários de leitura do sistema
    if UTILITARIOS_LEITURA.contains(&prim) {
        if prim == "find" && tokens.iter().any(|&t| t == "-delete" || t == "rm") {
            return false;
        }
        return true;
    }

    // awk e sed para formatação de texto (sem -i que sobrescreve arquivos)
    if prim == "awk" {
        return true;
    }
    if prim == "sed" && !tokens.iter().any(|t| t == &"-i" || t.starts_with("-i")) {
        return true;
    }

    // Subcomandos de inspeção do Git
    if prim == "git" {
        return git_subcomando_autorizado(&tokens);
    }

    // Subcomandos de checagem do Cargo
    if prim == "cargo" {
        return cargo_subcomando_autorizado(&tokens);
    }

    // Scripts de verificação Node/NPM/NPX
    if prim == "npm" {
        return npm_subcomando_autorizado(&tokens);
    }
    if prim == "npx" {
        return npx_subcomando_autorizado(&tokens);
    }
    if prim == "yarn" || prim == "pnpm" || prim == "bun" {
        if tokens.len() > 1 && matches!(tokens[1], "test" | "lint" | "--version" | "-v") {
            return true;
        }
        if tokens.len() > 2
            && tokens[1] == "run"
            && (tokens[2].starts_with("lint")
                || tokens[2].starts_with("check")
                || tokens[2].starts_with("test"))
        {
            return true;
        }
    }

    // Scripts de checagem Python / Node
    if (prim == "python" || prim == "python3") && tokens.len() > 1 {
        let seg = tokens[1];
        if seg == "-c" || seg == "-V" || seg == "--version" {
            return true;
        }
        if seg == "-m" && tokens.len() > 2 {
            let mod_name = tokens[2];
            if matches!(
                mod_name,
                "py_compile" | "unittest" | "doctest" | "json.tool"
            ) {
                return true;
            }
        }
    }
    if prim == "node"
        && tokens.len() > 1
        && (tokens[1] == "-e" || tokens[1] == "-v" || tokens[1] == "--version")
    {
        return true;
    }

    false
}

fn carregar_allow_set_agy() -> HashSet<String> {
    let mut allow_set = HashSet::new();
    let home = match crate::platform::pasta_usuario() {
        Some(h) => h,
        None => return allow_set,
    };
    let cfg_path = home
        .join(".gemini")
        .join("antigravity-cli")
        .join("settings.json");

    if !cfg_path.is_file() {
        return allow_set;
    }

    let conteudo = match fs::read_to_string(&cfg_path) {
        Ok(c) => c,
        Err(_) => return allow_set,
    };

    let val: serde_json::Value = match serde_json::from_str(&conteudo) {
        Ok(v) => v,
        Err(_) => return allow_set,
    };

    if let Some(arr) = val
        .get("permissions")
        .and_then(|p| p.get("allow"))
        .and_then(|a| a.as_array())
    {
        for item in arr {
            if let Some(s) = item.as_str() {
                allow_set.insert(s.to_string());
            }
        }
    }

    allow_set
}

fn dividir_subcomandos(cmd: &str) -> Vec<&str> {
    let mut pedacos = Vec::new();
    let mut inicio = 0;
    let mut em_aspas_duplas = false;
    let mut em_aspas_simples = false;
    let bytes = cmd.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && em_aspas_duplas && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if b == b'\'' && !em_aspas_duplas {
            em_aspas_simples = !em_aspas_simples;
        } else if b == b'"' && !em_aspas_simples {
            em_aspas_duplas = !em_aspas_duplas;
        } else if !em_aspas_duplas && !em_aspas_simples {
            if b == b';' || b == b'\n' {
                let trecho = cmd[inicio..i].trim();
                if !trecho.is_empty() {
                    pedacos.push(trecho);
                }
                inicio = i + 1;
            } else if b == b'&' && i + 1 < bytes.len() && bytes[i + 1] == b'&' {
                let trecho = cmd[inicio..i].trim();
                if !trecho.is_empty() {
                    pedacos.push(trecho);
                }
                i += 1;
                inicio = i + 1;
            } else if b == b'|' {
                if i + 1 < bytes.len() && bytes[i + 1] == b'|' {
                    let trecho = cmd[inicio..i].trim();
                    if !trecho.is_empty() {
                        pedacos.push(trecho);
                    }
                    i += 1;
                    inicio = i + 1;
                } else {
                    let trecho = cmd[inicio..i].trim();
                    if !trecho.is_empty() {
                        pedacos.push(trecho);
                    }
                    inicio = i + 1;
                }
            }
        }
        i += 1;
    }
    let trecho = cmd[inicio..].trim();
    if !trecho.is_empty() {
        pedacos.push(trecho);
    }
    pedacos
}

pub fn comando_autorizado_com_allow(cmd: &str, allow_set: &HashSet<String>) -> bool {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return false;
    }

    let subcomandos = dividir_subcomandos(trimmed);

    if subcomandos.is_empty() {
        return false;
    }

    for sub in subcomandos {
        if !subcomando_autorizado(sub, allow_set) {
            return false;
        }
    }

    true
}

pub fn comando_autorizado_agy(cmd: &str) -> bool {
    comando_autorizado_com_allow(cmd, &carregar_allow_set_agy())
}

pub fn analisar_pre_tool(entrada_json: &str) -> AgyDecisao {
    let val: serde_json::Value = match serde_json::from_str(entrada_json) {
        Ok(v) => v,
        Err(_) => return AgyDecisao::Allow,
    };

    let tc = match val.get("toolCall") {
        Some(t) => t,
        None => return AgyDecisao::Allow,
    };

    let nome = tc.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = tc.get("args").and_then(|a| a.as_object());

    if nome == "ask_question" {
        return AgyDecisao::Espera;
    }

    if nome == "write_file" || nome == "edit_file" {
        return AgyDecisao::Ask;
    }

    if nome == "run_command" {
        let cmd = args
            .and_then(|a| a.get("CommandLine"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim();

        if comando_autorizado_agy(cmd) {
            return AgyDecisao::Allow;
        }

        return AgyDecisao::Ask;
    }

    AgyDecisao::Allow
}

pub fn analisar_confirmacao_pendente(texto_log: &str) -> Option<u64> {
    let mut ultimo_surfacing = None;
    let mut ultimo_responding = None;

    for linha in texto_log.lines() {
        if let Some(pos) = linha.find("Surfacing tool confirmation:") {
            if let Some(step_pos) = linha[pos..].find("at step ") {
                let resto = &linha[pos + step_pos + 8..];
                let num_str: String = resto.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(num) = num_str.parse::<u64>() {
                    ultimo_surfacing = Some(num);
                }
            }
        } else if let Some(pos) = linha.find("Responding to tool confirmation:") {
            if let Some(step_pos) = linha[pos..].find("stepIdx=") {
                let resto = &linha[pos + step_pos + 8..];
                let num_str: String = resto.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(num) = num_str.parse::<u64>() {
                    ultimo_responding = Some(num);
                }
            }
        }
    }

    match (ultimo_surfacing, ultimo_responding) {
        (Some(s), Some(r)) if s > r => Some(s),
        (Some(s), None) => Some(s),
        _ => None,
    }
}

pub fn detectar_confirmacao_pendente() -> Option<(String, u64)> {
    let home = crate::platform::pasta_usuario()?;
    let log_dir = home.join(".gemini").join("antigravity-cli").join("log");
    if !log_dir.is_dir() {
        return None;
    }

    let mut entradas: Vec<_> = fs::read_dir(log_dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter(|e| {
            let nome = e.file_name();
            let s = nome.to_string_lossy();
            s.starts_with("cli-") && s.ends_with(".log")
        })
        .collect();

    entradas.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });

    let agora = std::time::SystemTime::now();

    for entry in entradas.into_iter().rev() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let mtime = match meta.modified() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if let Ok(diff) = agora.duration_since(mtime) {
            if diff.as_secs() > 120 {
                break;
            }
        }

        let mut f = match fs::File::open(entry.path()) {
            Ok(file) => file,
            Err(_) => continue,
        };
        use std::io::{Read, Seek, SeekFrom};
        let len = meta.len();
        let ler_desde = len.saturating_sub(32_768);
        if f.seek(SeekFrom::Start(ler_desde)).is_err() {
            continue;
        }
        let mut buffer = String::new();
        if f.read_to_string(&mut buffer).is_err() {
            let mut bytes = Vec::new();
            if f.seek(SeekFrom::Start(ler_desde)).is_ok() && f.read_to_end(&mut bytes).is_ok() {
                buffer = String::from_utf8_lossy(&bytes).to_string();
            }
        }

        if let Some(step) = analisar_confirmacao_pendente(&buffer) {
            let nome = entry.file_name().to_string_lossy().to_string();
            return Some((nome, step));
        }
    }

    None
}

#[allow(dead_code)]
pub fn agy_precisa_permissao(entrada_json: &str) -> bool {
    matches!(
        analisar_pre_tool(entrada_json),
        AgyDecisao::Ask | AgyDecisao::Espera
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ask_question_precisa_permissao() {
        let json = r#"{"toolCall":{"name":"ask_question","args":{}}}"#;
        assert_eq!(analisar_pre_tool(json), AgyDecisao::Espera);
        assert!(agy_precisa_permissao(json));
    }

    #[test]
    fn test_comando_desconhecido_precisa_permissao() {
        let json = r#"{"toolCall":{"name":"run_command","args":{"CommandLine":"comando_desconhecido_123"}}}"#;
        assert_eq!(analisar_pre_tool(json), AgyDecisao::Ask);
        assert!(agy_precisa_permissao(json));
    }

    #[test]
    fn test_comando_vazio_precisa_permissao() {
        let json = r#"{"toolCall":{"name":"run_command","args":{"CommandLine":""}}}"#;
        assert_eq!(analisar_pre_tool(json), AgyDecisao::Ask);
        assert!(agy_precisa_permissao(json));
    }

    #[test]
    fn test_ferramenta_sem_permissao() {
        let json = r#"{"toolCall":{"name":"read_file","args":{}}}"#;
        assert_eq!(analisar_pre_tool(json), AgyDecisao::Allow);
        assert!(!agy_precisa_permissao(json));
    }

    #[test]
    fn test_criacao_e_edicao_arquivo_pedem_permissao() {
        let write_json = r#"{"toolCall":{"name":"write_file","args":{"Path":"novo.txt"}}}"#;
        assert_eq!(analisar_pre_tool(write_json), AgyDecisao::Ask);
        assert!(agy_precisa_permissao(write_json));

        let edit_json = r#"{"toolCall":{"name":"edit_file","args":{"Path":"existente.txt"}}}"#;
        assert_eq!(analisar_pre_tool(edit_json), AgyDecisao::Ask);
        assert!(agy_precisa_permissao(edit_json));
    }

    #[test]
    fn test_comandos_leitura_autorizados() {
        assert!(comando_autorizado_agy("find . -name '*.rs'"));
        assert!(comando_autorizado_agy("sips -g all imagem.png"));
        assert!(comando_autorizado_agy("grep -rn 'teste' src/"));
        assert!(comando_autorizado_agy("which cargo"));
        assert!(comando_autorizado_agy("stat Cargo.toml"));
    }

    #[test]
    fn test_git_inspecao_autorizado() {
        assert!(comando_autorizado_agy("git status"));
        assert!(comando_autorizado_agy("git -C /tmp log -n 5 --oneline"));
        assert!(comando_autorizado_agy("git diff HEAD~1"));
        assert!(comando_autorizado_agy(
            "git show ba81d3b:globals.css | grep noise"
        ));
    }

    #[test]
    fn test_git_mutacao_precisa_permissao() {
        let allow_vazio = HashSet::new();
        assert!(!comando_autorizado_com_allow(
            "git commit -m 'feat: teste'",
            &allow_vazio
        ));
        assert!(!comando_autorizado_com_allow(
            "git push origin main",
            &allow_vazio
        ));
        assert!(!comando_autorizado_com_allow(
            "git branch -D feat/antiga",
            &allow_vazio
        ));

        let mut allow_custom = HashSet::new();
        allow_custom.insert("command(git commit)".to_string());
        assert!(comando_autorizado_com_allow(
            "git commit -m 'feat: teste'",
            &allow_custom
        ));
    }

    #[test]
    fn test_ferramentas_desenvolvimento_autorizadas() {
        assert!(comando_autorizado_agy("cargo check"));
        assert!(comando_autorizado_agy("cargo clippy"));
        assert!(comando_autorizado_agy("cargo test"));
        assert!(comando_autorizado_agy("npm run lint:prettier:check"));
        assert!(comando_autorizado_agy("npx next build --webpack"));
        assert!(comando_autorizado_agy(
            "python3 -c \"import json; print('ok')\""
        ));
    }

    #[test]
    fn test_analisar_confirmacao_pendente_detecta_espera() {
        let log_pendente = r#"
I1101 21:18:51.377147 169 tool_confirmation_manager.go:236] Surfacing tool confirmation: "RunCommand" at step 403
I1101 21:20:01.379076 239 input_loop.go:778] Responding to tool confirmation: convID=c6426cfa, stepIdx=403, approved=true
I1101 21:22:13.145764 169 tool_confirmation_manager.go:236] Surfacing tool confirmation: "EditFile" at step 437
"#;
        assert_eq!(analisar_confirmacao_pendente(log_pendente), Some(437));

        let log_respondido = r#"
I1101 21:18:51.377147 169 tool_confirmation_manager.go:236] Surfacing tool confirmation: "RunCommand" at step 403
I1101 21:20:01.379076 239 input_loop.go:778] Responding to tool confirmation: convID=c6426cfa, stepIdx=403, approved=true
I1101 21:22:13.145764 169 tool_confirmation_manager.go:236] Surfacing tool confirmation: "EditFile" at step 437
I1101 21:22:22.696316 239 input_loop.go:778] Responding to tool confirmation: convID=c6426cfa, stepIdx=437, approved=true
"#;
        assert_eq!(analisar_confirmacao_pendente(log_respondido), None);
    }
}
