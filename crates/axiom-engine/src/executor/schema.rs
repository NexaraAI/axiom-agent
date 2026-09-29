use serde_json::Value;

use super::ToolRequest;

/// Validates a JSON value against the subset of JSON Schema Axiom enforces for
/// tool inputs and outputs. External tool sources use this to gate calls with
/// the same rules as built-in executors.
pub fn validate_schema_value(value: &Value, schema: &Value) -> std::result::Result<(), String> {
    if let Some(expected) = schema.get("type").and_then(Value::as_str) {
        let matches = match expected {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            other => return Err(format!("unsupported schema type `{other}`")),
        };
        if !matches {
            return Err(format!("expected {expected}"));
        }
    }
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                return Err(format!("missing required property `{key}`"));
            }
        }
    }
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
        if let Some(key) = object.keys().find(|key| !properties.contains_key(*key)) {
            return Err(format!("unknown property `{key}`"));
        }
    }
    for (key, property_schema) in properties {
        let Some(property) = object.get(&key) else {
            continue;
        };
        validate_schema_value(property, &property_schema)
            .map_err(|message| format!("property `{key}`: {message}"))?;
        if let (Some(value), Some(minimum)) = (
            property.as_str(),
            property_schema.get("minLength").and_then(Value::as_u64),
        ) {
            if value.chars().count() < usize::try_from(minimum).unwrap_or(usize::MAX) {
                return Err(format!("property `{key}` is shorter than {minimum}"));
            }
        }
        if let Some(number) = property.as_u64() {
            if property_schema
                .get("minimum")
                .and_then(Value::as_u64)
                .is_some_and(|minimum| number < minimum)
            {
                return Err(format!("property `{key}` is below minimum"));
            }
            if property_schema
                .get("maximum")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| number > maximum)
            {
                return Err(format!("property `{key}` is above maximum"));
            }
        }
    }
    Ok(())
}

pub fn normalize_tool_arguments(request: &mut ToolRequest) {
    let Some(map) = request.arguments.as_object_mut() else {
        return;
    };

    match request.skill_id.as_str() {
        "question.ask" => {
            if !map.contains_key("options") {
                if let Some(opts) = map
                    .remove("choices")
                    .or_else(|| map.remove("items"))
                    .or_else(|| map.remove("answers"))
                {
                    map.insert("options".to_string(), opts);
                }
            }
            if let Some(options_val) = map.get("options").cloned() {
                match options_val {
                    Value::Array(_) => {}
                    Value::String(s) => {
                        let trimmed = s.trim();
                        if trimmed.starts_with('[') && trimmed.ends_with(']') {
                            if let Ok(Value::Array(arr)) = serde_json::from_str(&s) {
                                map.insert("options".to_string(), Value::Array(arr));
                            }
                        } else if trimmed.contains('\n') {
                            let items = trimmed
                                .lines()
                                .map(|line| {
                                    line.trim_start_matches(|c: char| {
                                        c.is_numeric()
                                            || c == '.'
                                            || c == '-'
                                            || c == ')'
                                            || c == ' '
                                    })
                                    .trim()
                                })
                                .filter(|line| !line.is_empty())
                                .map(|line| Value::String(line.to_string()))
                                .collect::<Vec<_>>();
                            if !items.is_empty() {
                                map.insert("options".to_string(), Value::Array(items));
                            }
                        } else if trimmed.contains(',') {
                            let items = trimmed
                                .split(',')
                                .map(str::trim)
                                .filter(|item| !item.is_empty())
                                .map(|item| Value::String(item.to_string()))
                                .collect::<Vec<_>>();
                            if !items.is_empty() {
                                map.insert("options".to_string(), Value::Array(items));
                            }
                        } else if !trimmed.is_empty() {
                            map.insert(
                                "options".to_string(),
                                Value::Array(vec![Value::String(trimmed.to_string())]),
                            );
                        }
                    }
                    Value::Object(obj) => {
                        let mut entries = obj.into_iter().collect::<Vec<_>>();
                        entries.sort_by(|a, b| a.0.cmp(&b.0));
                        let items = entries
                            .into_iter()
                            .map(|(_, v)| match v {
                                Value::String(s) => Value::String(s),
                                other => Value::String(other.to_string()),
                            })
                            .collect::<Vec<_>>();
                        map.insert("options".to_string(), Value::Array(items));
                    }
                    _ => {}
                }
            }
            if let Some(Value::String(custom_str)) = map.get("allow_custom") {
                if let Ok(b) = custom_str.parse::<bool>() {
                    map.insert("allow_custom".to_string(), Value::Bool(b));
                }
            }
        }
        "github.search" => {
            if !map.contains_key("org") {
                if let Some(org) = map
                    .remove("organization")
                    .or_else(|| map.remove("user"))
                    .or_else(|| map.remove("owner"))
                {
                    map.insert("org".to_string(), org);
                }
            }
            if !map.contains_key("repo") {
                if let Some(repo) = map.remove("repository").or_else(|| map.remove("project")) {
                    map.insert("repo".to_string(), repo);
                }
            }
            if !map.contains_key("query") {
                if let Some(query) = map
                    .remove("q")
                    .or_else(|| map.remove("search"))
                    .or_else(|| map.remove("keyword"))
                {
                    map.insert("query".to_string(), query);
                }
            }
        }
        "file.read" => {
            if !map.contains_key("path") {
                if let Some(path) = map
                    .remove("file")
                    .or_else(|| map.remove("filepath"))
                    .or_else(|| map.remove("filename"))
                {
                    map.insert("path".to_string(), path);
                }
            }
            if let Some(Value::String(offset_str)) = map.get("offset") {
                if let Ok(n) = offset_str.parse::<u64>() {
                    map.insert("offset".to_string(), Value::Number(n.into()));
                }
            }
            if let Some(Value::String(limit_str)) = map.get("limit") {
                if let Ok(n) = limit_str.parse::<u64>() {
                    map.insert("limit".to_string(), Value::Number(n.into()));
                }
            }
        }
        "file.write" => {
            if !map.contains_key("path") {
                if let Some(path) = map
                    .remove("file")
                    .or_else(|| map.remove("filepath"))
                    .or_else(|| map.remove("filename"))
                {
                    map.insert("path".to_string(), path);
                }
            }
            if !map.contains_key("content") {
                if let Some(content) = map
                    .remove("text")
                    .or_else(|| map.remove("body"))
                    .or_else(|| map.remove("code"))
                {
                    map.insert("content".to_string(), content);
                }
            }
        }
        "file.replace" => {
            if !map.contains_key("path") {
                if let Some(path) = map
                    .remove("file")
                    .or_else(|| map.remove("filepath"))
                    .or_else(|| map.remove("filename"))
                {
                    map.insert("path".to_string(), path);
                }
            }
            if !map.contains_key("target_content") {
                if let Some(target) = map
                    .remove("target")
                    .or_else(|| map.remove("find"))
                    .or_else(|| map.remove("old_content"))
                    .or_else(|| map.remove("old_string"))
                {
                    map.insert("target_content".to_string(), target);
                }
            }
            if !map.contains_key("replacement_content") {
                if let Some(rep) = map
                    .remove("replacement")
                    .or_else(|| map.remove("replace"))
                    .or_else(|| map.remove("new_content"))
                    .or_else(|| map.remove("new_string"))
                {
                    map.insert("replacement_content".to_string(), rep);
                }
            }
            if let Some(Value::String(mult_str)) = map.get("allow_multiple") {
                if let Ok(b) = mult_str.parse::<bool>() {
                    map.insert("allow_multiple".to_string(), Value::Bool(b));
                }
            }
        }
        "web.fetch" => {
            if !map.contains_key("url") {
                if let Some(url) = map
                    .remove("link")
                    .or_else(|| map.remove("uri"))
                    .or_else(|| map.remove("target_url"))
                {
                    map.insert("url".to_string(), url);
                }
            }
            if !map.contains_key("query") {
                if let Some(query) = map.remove("search").or_else(|| map.remove("q")) {
                    map.insert("query".to_string(), query);
                }
            }
        }
        "project.scan" if !map.contains_key("path") => {
            if let Some(p) = map
                .remove("directory")
                .or_else(|| map.remove("dir"))
                .or_else(|| map.remove("folder"))
            {
                map.insert("path".to_string(), p);
            }
        }
        _ => {}
    }
}
