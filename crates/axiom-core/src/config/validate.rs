use std::path::{Path, PathBuf};

use super::schema::*;
use super::AgentWorkMode;
use crate::{AxiomError, Result};

pub(super) fn validate_mcp_name_segment(
    field: &'static str,
    value: &str,
    max_len: usize,
) -> Result<()> {
    let valid = !value.is_empty()
        && value.len() <= max_len
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        });
    if valid {
        Ok(())
    } else {
        Err(AxiomError::InvalidConfig {
            field,
            message: format!(
                "`{value}` must be 1-{max_len} characters of lowercase letters, digits, `-`, or `_`"
            ),
        })
    }
}

pub(super) fn validate_mcp_side_effects(field: &'static str, classes: &[String]) -> Result<()> {
    for class in classes {
        if !MCP_SIDE_EFFECT_NAMES.contains(&class.as_str()) {
            return Err(AxiomError::InvalidConfig {
                field,
                message: format!(
                    "unknown side-effect class `{class}`; expected one of {}",
                    MCP_SIDE_EFFECT_NAMES.join(", ")
                ),
            });
        }
    }
    Ok(())
}

pub fn validate_variant(variant: &str) -> Result<&'static str> {
    let trimmed = variant.trim();
    if trimmed.eq_ignore_ascii_case("default") {
        Ok("Default")
    } else if trimmed.eq_ignore_ascii_case("low") {
        Ok("low")
    } else if trimmed.eq_ignore_ascii_case("medium") {
        Ok("medium")
    } else if trimmed.eq_ignore_ascii_case("high") {
        Ok("high")
    } else if trimmed.eq_ignore_ascii_case("xhigh") {
        Ok("xhigh")
    } else {
        Err(AxiomError::InvalidConfig {
            field: "variant",
            message: format!(
                "invalid variant `{variant}`; expected Default, low, medium, high, or xhigh"
            ),
        })
    }
}

pub fn validate_mode(mode: &str) -> Result<AgentWorkMode> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "plan" => Ok(AgentWorkMode::Plan),
        "build" => Ok(AgentWorkMode::Build),
        _ => Err(AxiomError::InvalidConfig {
            field: "work_mode",
            message: format!("invalid work mode `{mode}`; expected plan or build"),
        }),
    }
}

pub fn validate_permission(mode: &str) -> Result<PermissionMode> {
    match mode.trim().to_ascii_lowercase().as_str() {
        "velocity" => Ok(PermissionMode::Velocity),
        "full_machine" => Ok(PermissionMode::FullMachine),
        "strict" => Ok(PermissionMode::Strict),
        _ => Err(AxiomError::InvalidConfig {
            field: "permission_mode",
            message: format!(
                "invalid permission mode `{mode}`; expected velocity, full_machine, or strict"
            ),
        }),
    }
}

/// Validates a variable name that will hold a gateway or MCP server secret.
///
/// Delegates to the shared `credentials::validate_credential_env_name` so the
/// config-time check and the runtime credential check cannot disagree. This
/// previously used its own, weaker list: 11 names compared case-sensitively,
/// against the credential path's 27 case-insensitively, so `TEMP` and
/// `COMSPEC` passed config validation and failed at runtime.
pub(super) fn validate_gateway_token_env_name(variable: &str) -> Result<()> {
    crate::credentials::validate_credential_env_name(variable).map_err(|error| {
        AxiomError::InvalidConfig {
            field: "gateway token env",
            message: error.to_string(),
        }
    })
}

pub(super) fn validate_host_pattern(field: &'static str, pattern: &str) -> Result<()> {
    let host = pattern.strip_prefix("*.").unwrap_or(pattern);
    let valid = !host.is_empty()
        && host == host.trim()
        && !host
            .chars()
            .any(|character| matches!(character, '/' | ':' | '@'))
        && !host.contains('*')
        && !host.chars().any(char::is_whitespace)
        && !host.starts_with('.')
        && !host.ends_with('.');
    if valid {
        Ok(())
    } else {
        Err(AxiomError::InvalidConfig {
            field,
            message: format!(
                "invalid host pattern `{pattern}`; use an exact hostname or `*.example.com`"
            ),
        })
    }
}

pub(super) fn ensure_non_negative_finite(field: &'static str, value: f64) -> Result<()> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(AxiomError::InvalidConfig {
            field,
            message: "expected a finite number greater than or equal to zero".to_string(),
        })
    }
}

pub(super) fn backup_path_for_migration(path: &Path, from_version: u32) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    let backup_name = format!("{file_name}.v{from_version}.bak");
    path.with_file_name(backup_name)
}

pub(super) fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    }

    if let Some(rest) = path.strip_prefix("~/") {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest);
    }

    if let Some(rest) = path.strip_prefix("~\\") {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest);
    }

    PathBuf::from(path)
}
