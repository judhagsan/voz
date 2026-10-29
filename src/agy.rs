use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

pub fn comando_autorizado_agy(cmd: &str) -> bool {
    let home = match std::env::var("HOME") {
        Ok(h) => PathBuf::from(h),
        Err(_) => return false,
    };
    let cfg_path = home
        .join(".gemini")
        .join("antigravity-cli")
        .join("settings.json");

    if !cfg_path.is_file() || cmd.trim().is_empty() {
        return false;
    }

    let conteudo = match fs::read_to_string(&cfg_path) {
        Ok(c) => c,
        Err(_) => return false,
    };

    let val: serde_json::Value = match serde_json::from_str(&conteudo) {
        Ok(v) => v,
        Err(_) => return false,
    };

    let allow_arr = match val
        .get("permissions")
        .and_then(|p| p.get("allow"))
        .and_then(|a| a.as_array())
    {
        Some(arr) => arr,
        None => return false,
    };

    let mut allow_set = HashSet::new();
    for item in allow_arr {
        if let Some(s) = item.as_str() {
            allow_set.insert(s.to_string());
        }
    }

    if allow_set.contains(&format!("command({cmd})")) {
        return true;
    }

    let builtins_livres = [
        "cd", "true", "false", ":", "echo", "printf", "sleep", "for", "do", "done", "test", "[",
        "setopt",
    ];

    let re_split = regex::Regex::new(r"\s*(?:&&|\|\||;|\||\n)\s*").unwrap();
    let subcomandos: Vec<&str> = re_split
        .split(cmd)
        .filter(|s| !s.trim().is_empty())
        .collect();
    if subcomandos.is_empty() {
        return false;
    }

    for sub in subcomandos {
        if allow_set.contains(&format!("command({sub})")) {
            continue;
        }
        let tokens: Vec<&str> = sub.split_whitespace().collect();
        if tokens.is_empty() {
            continue;
        }
        let mut prim_idx = 0;
        while prim_idx < tokens.len() && tokens[prim_idx].contains('=') {
            prim_idx += 1;
        }
        if prim_idx >= tokens.len() {
            continue;
        }
        let prim = tokens[prim_idx];
        if builtins_livres.contains(&prim) {
            continue;
        }
        if allow_set.contains(&format!("command({prim})")) {
            continue;
        }
        if tokens.len() > prim_idx + 1 {
            let seg = tokens[prim_idx + 1];
            if allow_set.contains(&format!("command({prim} {seg})")) {
                continue;
            }
        }
        return false;
    }

    true
}

pub fn agy_precisa_permissao(entrada_json: &str) -> bool {
    let val: serde_json::Value = match serde_json::from_str(entrada_json) {
        Ok(v) => v,
        Err(_) => return false,
    };

    let tc = match val.get("toolCall") {
        Some(t) => t,
        None => return false,
    };

    let nome = tc.get("name").and_then(|n| n.as_str()).unwrap_or("");
    let args = tc.get("args").and_then(|a| a.as_object());

    if nome == "ask_question" {
        thread::sleep(Duration::from_millis(1000));
        return true;
    }

    if nome == "run_command" {
        let bypass = args
            .and_then(|a| a.get("BypassSandbox"))
            .and_then(|b| b.as_bool())
            .unwrap_or(false);

        // Se roda em sandbox normal, é 100% automático e nunca requer permissão
        if !bypass {
            return false;
        }

        let cmd = args
            .and_then(|a| a.get("CommandLine"))
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .trim();

        if comando_autorizado_agy(cmd) {
            return false;
        }

        return true;
    }

    false
}
