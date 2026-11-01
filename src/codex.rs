//! Adapta os hooks do Codex aos avisos do Voz, sem devolver decisões de permissão.

pub fn categoria(entrada: &serde_json::Value) -> Option<&'static str> {
    match entrada.get("hook_event_name")?.as_str()? {
        "Stop" => Some("terminei"),
        "PermissionRequest" => Some("permissao"),
        "PreToolUse" => {
            let ferramenta = entrada.get("tool_name")?.as_str()?.rsplit('.').next()?;
            match ferramenta {
                "request_user_input" | "request_user_input_async" => Some("espera"),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn avisa_no_fim_e_na_aprovacao_real() {
        assert_eq!(
            categoria(&json!({"hook_event_name": "Stop"})),
            Some("terminei")
        );
        assert_eq!(
            categoria(&json!({"hook_event_name": "PermissionRequest", "tool_name": "Bash"})),
            Some("permissao")
        );
        assert_eq!(
            categoria(&json!({"hook_event_name": "PreToolUse", "tool_name": "Bash"})),
            None
        );
    }

    #[test]
    fn avisa_quando_ha_pergunta_ao_usuario() {
        for nome in [
            "request_user_input",
            "request_user_input_async",
            "functions.request_user_input",
        ] {
            assert_eq!(
                categoria(&json!({"hook_event_name": "PreToolUse", "tool_name": nome})),
                Some("espera")
            );
        }
    }

    #[test]
    fn ignora_interrupcao_subagentes_e_entradas_invalidas() {
        for evento in ["Interrupt", "SessionEnd", "SubagentStop", "PostToolUse"] {
            assert_eq!(categoria(&json!({"hook_event_name": evento})), None);
        }
        assert_eq!(categoria(&json!({})), None);
        assert_eq!(categoria(&serde_json::Value::Null), None);
    }
}
