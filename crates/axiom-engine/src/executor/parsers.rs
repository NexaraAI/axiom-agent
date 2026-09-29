use super::{SkillExecutionError, ToolRequest};

pub fn extract_tool_request(text: &str) -> Result<ToolRequest, SkillExecutionError> {
    let markers = &[
        "```axiom-tool",
        "```axiom_tool",
        "```tool-call",
        "```tool_call",
        "```tool",
    ];
    let mut found = None;
    for marker in markers {
        if let Some(pos) = text.find(marker) {
            found = Some((pos, marker.len()));
            break;
        }
    }
    if found.is_none() {
        if let Some(pos) = text.find("```json") {
            let json_start = pos + "```json".len();
            if let Some(end) = text[json_start..].find("```") {
                let candidate = &text[json_start..json_start + end];
                if candidate.contains("\"skill_id\"")
                    || (candidate.contains("\"name\"")
                        && (candidate.contains("file.")
                            || candidate.contains("project.")
                            || candidate.contains("axiom_")))
                {
                    found = Some((pos, "```json".len()));
                }
            }
        }
    }
    let (start, marker_len) = found.ok_or(SkillExecutionError::MissingToolBlock)?;
    let json_start = start + marker_len;
    let after_start = text[json_start..].trim_start();
    let json_text = if let Some(end) = after_start.find("```") {
        after_start[..end].trim()
    } else {
        after_start.trim()
    };

    let raw: serde_json::Value = repair_and_parse_json(json_text)?;
    normalize_tool_request_value(raw)
}

fn repair_and_parse_json(text: &str) -> Result<serde_json::Value, serde_json::Error> {
    if let Ok(val) = serde_json::from_str(text) {
        return Ok(val);
    }
    let mut cleaned = text.trim().to_string();
    if cleaned.ends_with('>') {
        cleaned.pop();
        cleaned.push('}');
    }
    if let Ok(val) = serde_json::from_str(&cleaned) {
        return Ok(val);
    }
    let open_braces = cleaned.chars().filter(|c| *c == '{').count();
    let close_braces = cleaned.chars().filter(|c| *c == '}').count();
    if open_braces > close_braces {
        let quote_count = cleaned.chars().filter(|c| *c == '"').count();
        if quote_count % 2 == 1 {
            cleaned.push('"');
        }
        for _ in 0..(open_braces - close_braces) {
            cleaned.push('}');
        }
    }
    serde_json::from_str(&cleaned)
}

fn normalize_tool_request_value(
    raw: serde_json::Value,
) -> Result<ToolRequest, SkillExecutionError> {
    if let serde_json::Value::Object(map) = raw {
        let mut skill_id = if let Some(id) = map.get("skill_id").and_then(serde_json::Value::as_str)
        {
            id.to_string()
        } else if let Some(name) = map.get("name").and_then(serde_json::Value::as_str) {
            name.strip_prefix("axiom_").unwrap_or(name).to_string()
        } else if let Some(tool) = map.get("tool").and_then(serde_json::Value::as_str) {
            tool.to_string()
        } else {
            return Err(SkillExecutionError::SchemaValidation {
                skill_id: "unknown".to_string(),
                direction: "input",
                message: "missing skill_id or name in tool request".to_string(),
            });
        };

        if skill_id == "shell_run"
            || skill_id == "shell.run"
            || skill_id == "run_shell"
            || skill_id == "exec"
            || skill_id == "shell"
        {
            #[cfg(windows)]
            {
                skill_id = "shell.powershell.safe".to_string();
            }
            #[cfg(target_os = "macos")]
            {
                skill_id = "shell.zsh.safe".to_string();
            }
            #[cfg(not(any(windows, target_os = "macos")))]
            {
                skill_id = "shell.bash.safe".to_string();
            }
        } else if !skill_id.contains('.') && skill_id.contains('_') {
            skill_id = skill_id.replace('_', ".");
        }

        let arguments = if let Some(args) = map.get("arguments") {
            args.clone()
        } else if let Some(params) = map.get("parameters") {
            params.clone()
        } else if let Some(args) = map.get("args") {
            args.clone()
        } else {
            serde_json::json!({})
        };

        Ok(ToolRequest {
            skill_id,
            arguments,
        })
    } else {
        Err(SkillExecutionError::SchemaValidation {
            skill_id: "unknown".to_string(),
            direction: "input",
            message: "expected json object for tool request".to_string(),
        })
    }
}
