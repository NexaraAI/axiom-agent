//! `axiom mcp` — inspect the external MCP servers Axiom is configured to use,
//! and expose Axiom's own tools to other MCP clients.
//!
//! This is the operator-facing half of MCP support. The wiring that hands
//! remote tools to the model happens in [`crate::chat`]; everything here is
//! about making the integration observable and testable from a terminal.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use axiom_core::{AxiomConfig, McpServerConfig};
use axiom_engine::{
    load_installed_skills, SideEffectClass, SideEffectPolicy, SkillExecutionContext,
};
use axiom_mcp::{McpServer, McpServerOptions, McpToolSource, StdinStdoutTransport};

use crate::McpCommands;

/// Entry point shared by every `axiom mcp` subcommand.
pub(crate) async fn run(command: McpCommands) -> Result<()> {
    match command {
        McpCommands::List => list(),
        McpCommands::Tools {
            server,
            include_disabled,
        } => tools(server.as_deref(), include_disabled).await,
        McpCommands::Serve {
            approve,
            read_only,
            allow,
            deny,
        } => serve(approve, read_only, allow, deny).await,
    }
}

/// Shows configured servers without starting anything. Declaring a server never
/// implies access; this view exists so an operator can see what *would* run.
fn list() -> Result<()> {
    let config = load_config()?;
    let mcp = &config.mcp;

    println!(
        "MCP client: {}",
        if mcp.enabled { "enabled" } else { "disabled" }
    );
    println!(
        "Limits: connect {}s, request {}s, max response {} bytes",
        mcp.connect_timeout_secs, mcp.request_timeout_secs, mcp.max_response_bytes
    );

    if mcp.servers.is_empty() {
        println!();
        println!("No MCP servers configured. Add one under `[[mcp.servers]]` in the Axiom config.");
        println!("Use `axiom config path` to find it.");
        return Ok(());
    }

    println!();
    println!("Configured servers:");
    for server in &mcp.servers {
        let state = if !mcp.enabled {
            "off (mcp.enabled = false)"
        } else if server.enabled {
            "enabled"
        } else {
            "disabled"
        };
        println!("  {} [{}]", server.name, state);
        println!("    command: {}", command_line(server));
        if !server.env.is_empty() || !server.env_from_secret.is_empty() {
            let literal = server.env.keys().cloned().collect::<Vec<_>>().join(", ");
            let secrets = server.env_from_secret.join(", ");
            let mut parts = Vec::new();
            if !literal.is_empty() {
                parts.push(format!("plaintext env: {literal}"));
            }
            if !secrets.is_empty() {
                parts.push(format!("secret env: {secrets}"));
            }
            println!("    env: {}", parts.join("; "));
        }
        if server.auto_approve {
            println!("    auto-approve: yes (ask decisions become allow)");
        }
        if let Some(classes) = &server.side_effects {
            println!("    side effects (override): {}", classes.join(", "));
        }
        if !server.allow_tools.is_empty() {
            println!("    allow: {}", server.allow_tools.join(", "));
        }
        if !server.deny_tools.is_empty() {
            println!("    deny: {}", server.deny_tools.join(", "));
        }
    }
    println!();
    println!("Run `axiom mcp tools` to connect them and list the tools they contribute.");
    Ok(())
}

/// Connects the configured servers and prints the tools they publish, with the
/// side-effect classes Axiom will enforce for each one.
async fn tools(server_name: Option<&str>, include_disabled: bool) -> Result<()> {
    let config = load_config()?;
    let mcp = &config.mcp;

    if mcp.servers.is_empty() {
        println!("No MCP servers configured.");
        return Ok(());
    }

    let source = if let Some(name) = server_name {
        let named = mcp
            .server(name)
            .ok_or_else(|| anyhow::anyhow!("unknown MCP server `{name}`"))?;
        let resolved = resolve_env(std::iter::once(named))?;
        McpToolSource::connect_named(mcp, &resolved, name)
            .await
            .with_context(|| format!("connecting MCP server `{name}`"))?
    } else {
        let mut servers: Vec<&McpServerConfig> = mcp
            .servers
            .iter()
            .filter(|server| include_disabled || (mcp.enabled && server.enabled))
            .collect();
        servers.sort_by(|left, right| left.name.cmp(&right.name));
        let resolved = resolve_env(servers.iter().copied())?;
        McpToolSource::connect(mcp, &resolved).await
    };

    for warning in source.warnings() {
        println!("warning: {warning}");
    }
    if source.is_empty() {
        println!("No tools available from the selected MCP servers.");
        return Ok(());
    }

    println!(
        "Connected {} MCP tool{} from {} server{}:",
        source.tools().len(),
        plural(source.tools().len()),
        source.connected_servers().len(),
        plural(source.connected_servers().len())
    );
    for server in source.connected_servers() {
        let label = source.server_label(server).unwrap_or(server);
        let count = source
            .tools()
            .iter()
            .filter(|tool| tool.server == server)
            .count();
        println!("  {server}: {label} ({count} tool{})", plural(count));
        if let Some(instructions) = source.instructions(server) {
            println!("    {instructions}");
        }
    }

    println!();
    println!("Tools (invoked as `mcp.<server>.<tool>`):");
    for tool in source.tools() {
        let classes = tool
            .side_effects
            .iter()
            .map(side_effect_label)
            .collect::<Vec<_>>()
            .join(", ");
        let auto = if tool.auto_approve {
            ", auto-approve"
        } else {
            ""
        };
        println!("  {} [{}{}]", tool.id, classes, auto);
        println!("    {}", tool.description);
    }
    Ok(())
}

/// Runs Axiom as an MCP server over stdio. Startup prints only to stderr,
/// because stdout *is* the protocol channel the client reads.
async fn serve(
    approve: bool,
    read_only: bool,
    allow: Vec<String>,
    deny: Vec<String>,
) -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    let config = AxiomConfig::load_from_path(&config_path)?;

    let skills_dir = config_path
        .parent()
        .map(|config_dir| config_dir.join(&config.skills.local_dir))
        .unwrap_or_else(|| PathBuf::from(&config.skills.local_dir));
    let installed_skills =
        load_installed_skills(&skills_dir).context("loading installed skills for MCP serve")?;

    let options = McpServerOptions {
        name: "axiom".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        auto_approve: approve,
        read_only,
        allow_tools: allow,
        deny_tools: deny,
    };
    let mut server = McpServer::new(
        options,
        serve_context(&config, &skills_dir),
        installed_skills,
        serve_policy(&config)?,
    );

    eprintln!(
        "axiom mcp serve: exposing {} tool{} over stdio (approval: {}).",
        server.tools().len(),
        plural(server.tools().len()),
        if approve {
            "auto-approve ask"
        } else {
            "deny ask"
        }
    );

    let mut transport = StdinStdoutTransport::new(config.mcp.max_response_bytes.max(1024));
    server
        .serve(&mut transport)
        .await
        .context("serving MCP over stdio")
}

fn load_config() -> Result<AxiomConfig> {
    let config_path = AxiomConfig::default_config_path()?;
    Ok(AxiomConfig::load_from_path(&config_path)?)
}

/// Resolves the credential store variables each server asks for, so an MCP
/// server can receive a token without it ever being written to the config.
pub(crate) fn resolve_env<'a, I>(servers: I) -> Result<BTreeMap<String, BTreeMap<String, String>>>
where
    I: IntoIterator<Item = &'a McpServerConfig>,
{
    let mut resolved = BTreeMap::new();
    for server in servers {
        let mut values = BTreeMap::new();
        for variable in &server.env_from_secret {
            if let Some(secret) = crate::credentials::resolve_credential(variable)? {
                values.insert(variable.clone(), secret);
            }
        }
        if !values.is_empty() {
            resolved.insert(server.name.clone(), values);
        }
    }
    Ok(resolved)
}

fn serve_context(config: &AxiomConfig, skills_dir: &Path) -> SkillExecutionContext {
    SkillExecutionContext {
        workspace_root: config.default_workspace_path(),
        max_file_read_bytes: config.coder.max_file_read_bytes,
        web_timeout_secs: 20,
        max_web_response_bytes: 1_000_000,
        web_fetch_https_only: config.network.web_fetch_https_only,
        web_fetch_allowed_hosts: config.network.web_fetch_allowed_hosts.clone(),
        web_fetch_denied_hosts: config.network.web_fetch_denied_hosts.clone(),
        web_fetch_use_system_proxy: config.network.web_fetch_use_system_proxy,
        auto_approve_medium_risk: config.coder.approval_mode == "trusted",
        credential_env_names: crate::credentials::credential_environment_names(config)
            .unwrap_or_default(),
        skills_dir: Some(skills_dir.to_path_buf()),
    }
}

fn serve_policy(config: &AxiomConfig) -> Result<SideEffectPolicy> {
    crate::chat::side_effect_policy_for_config(config)
}

fn command_line(server: &McpServerConfig) -> String {
    let mut parts = vec![server.command.clone()];
    parts.extend(server.args.iter().cloned());
    parts.join(" ")
}

fn side_effect_label(class: &SideEffectClass) -> &'static str {
    match class {
        SideEffectClass::FilesystemRead => "filesystem_read",
        SideEffectClass::FilesystemWrite => "filesystem_write",
        SideEffectClass::Network => "network",
        SideEffectClass::Process => "process",
        SideEffectClass::Git => "git",
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}
