use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use anyhow::{bail, Result};
use axiom_core::AxiomConfig;
use axiom_proof::{
    export_trace_to_format, find_proof, latest_proof, list_proofs, load_proof_trace, ProofFormat,
    ProofSettings,
};

use crate::ProofCommands;

const PROOF_EXPORT_PRIVACY_WARNING: &str = "Privacy warning: Proof exports can contain project paths, prompts, code, and metadata even after automatic redaction. Review the output before sharing it.";

pub(crate) fn run(command: ProofCommands) -> Result<()> {
    match command {
        ProofCommands::List => list(),
        ProofCommands::Latest => latest(),
        ProofCommands::Show { proof_id } => show(&proof_id),
        ProofCommands::Export { proof_id, format } => export(&proof_id, &format),
        ProofCommands::Open { proof_id } => open(&proof_id),
        ProofCommands::Clean { older_than } => clean(older_than),
    }
}

pub(crate) fn settings_from_config(config_path: &Path, config: &AxiomConfig) -> ProofSettings {
    ProofSettings {
        enabled: config.proof.enabled,
        proofs_dir: proofs_dir(config_path),
        trace_json: config.proof.trace_json,
        auto_export_markdown: config.proof.auto_export_markdown,
        redact_secrets: config.proof.redact_secrets,
        max_capture_chars: config.proof.max_capture_chars,
        retention_days: config.proof.retention_days,
    }
}

pub(crate) fn proofs_dir(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(|dir| dir.join("proofs"))
        .unwrap_or_else(|| PathBuf::from("proofs"))
}

fn list() -> Result<()> {
    let (config_path, _config) = load_config()?;
    let entries = list_proofs(proofs_dir(&config_path))?;
    if entries.is_empty() {
        println!("No proof traces found.");
        return Ok(());
    }

    println!("Proof traces:");
    for entry in entries {
        println!(
            "{} {} {} {} - {}",
            entry.date, entry.task_id, entry.mode, entry.status, entry.summary
        );
    }
    Ok(())
}

fn latest() -> Result<()> {
    let (config_path, _config) = load_config()?;
    let Some(entry) = latest_proof(proofs_dir(&config_path))? else {
        println!("No proof traces found.");
        return Ok(());
    };
    println!(
        "Latest proof: {}",
        entry
            .markdown_path
            .as_ref()
            .unwrap_or(&entry.json_path)
            .display()
    );
    println!("task: {}", entry.task_id);
    println!("mode: {}", entry.mode);
    println!("status: {}", entry.status);
    println!("summary: {}", entry.summary);
    Ok(())
}

fn show(proof_id: &str) -> Result<()> {
    let (config_path, _config) = load_config()?;
    let entry = find_proof(proofs_dir(&config_path), proof_id)?;
    let path = entry.markdown_path.unwrap_or(entry.json_path);
    println!("{}", fs::read_to_string(path)?);
    Ok(())
}

fn export(proof_id: &str, format: &str) -> Result<()> {
    let (config_path, _config) = load_config()?;
    let entry = find_proof(proofs_dir(&config_path), proof_id)?;
    let trace = load_proof_trace(&entry.json_path)?;
    let format = parse_format(format)?;
    let output = export_trace_to_format(&trace, format)?;
    eprintln!("{PROOF_EXPORT_PRIVACY_WARNING}");
    println!("{output}");
    Ok(())
}

fn open(proof_id: &str) -> Result<()> {
    let (config_path, _config) = load_config()?;
    let entry = find_proof(proofs_dir(&config_path), proof_id)?;
    let path = entry.markdown_path.unwrap_or(entry.json_path);
    println!("Proof report path: {}", path.display());
    Ok(())
}

fn clean(older_than: u64) -> Result<()> {
    let (config_path, _config) = load_config()?;
    let root = proofs_dir(&config_path);
    if !root.exists() {
        println!("No proof traces found.");
        return Ok(());
    }

    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(older_than * 86_400))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut removed = 0usize;
    for entry in walk_files(&root)? {
        let metadata = fs::metadata(&entry)?;
        if metadata.modified().unwrap_or(SystemTime::now()) < cutoff {
            fs::remove_file(&entry)?;
            removed += 1;
        }
    }
    println!("Removed {removed} proof files.");
    Ok(())
}

fn parse_format(format: &str) -> Result<ProofFormat> {
    match format {
        "markdown" | "md" => Ok(ProofFormat::Markdown),
        "json" => Ok(ProofFormat::Json),
        _ => bail!("unsupported proof export format: {format}"),
    }
}

fn walk_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    if !root.exists() {
        return Ok(files);
    }
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            files.extend(walk_files(&path)?);
        } else {
            files.push(path);
        }
    }
    Ok(files)
}

fn load_config() -> Result<(PathBuf, AxiomConfig)> {
    let config_path = AxiomConfig::default_config_path()?;
    let config = AxiomConfig::load_or_create(&config_path)?;
    Ok((config_path, config))
}
