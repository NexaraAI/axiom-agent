use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{export, report, ProofTrace, ProofTraceLoadError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProofFormat {
    Json,
    Markdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofIndexEntry {
    pub date: String,
    pub session_id: String,
    pub task_id: String,
    pub mode: String,
    pub status: String,
    pub summary: String,
    pub json_path: PathBuf,
    pub markdown_path: Option<PathBuf>,
}

#[derive(Debug, Error)]
pub enum ProofStorageError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Trace(#[from] ProofTraceLoadError),
    #[error("proof not found: {0}")]
    NotFound(String),
    #[error("proof id is ambiguous: {0}")]
    Ambiguous(String),
}

pub type Result<T> = std::result::Result<T, ProofStorageError>;

pub fn list_proofs(proofs_dir: impl AsRef<Path>) -> Result<Vec<ProofIndexEntry>> {
    let proofs_dir = proofs_dir.as_ref();
    if !proofs_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for date_entry in fs::read_dir(proofs_dir)? {
        let date_entry = date_entry?;
        if !date_entry.file_type()?.is_dir() {
            continue;
        }
        let date = date_entry.file_name().to_string_lossy().to_string();
        for session_entry in fs::read_dir(date_entry.path())? {
            let session_entry = session_entry?;
            if !session_entry.file_type()?.is_dir() {
                continue;
            }
            let session_id = session_entry.file_name().to_string_lossy().to_string();
            for proof_file in fs::read_dir(session_entry.path())? {
                let proof_file = proof_file?;
                let path = proof_file.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                let trace = load_proof_trace(&path)?;
                entries.push(ProofIndexEntry {
                    date: date.clone(),
                    session_id: session_id.clone(),
                    task_id: trace.task_id.clone(),
                    mode: format!("{:?}", trace.mode).to_ascii_lowercase(),
                    status: format!("{:?}", trace.status).to_ascii_lowercase(),
                    summary: trace
                        .summary
                        .clone()
                        .unwrap_or_else(|| trace.user_prompt.clone()),
                    markdown_path: {
                        let md = path.with_extension("md");
                        md.exists().then_some(md)
                    },
                    json_path: path,
                });
            }
        }
    }

    entries.sort_by(|left, right| {
        right
            .date
            .cmp(&left.date)
            .then_with(|| right.task_id.cmp(&left.task_id))
    });
    Ok(entries)
}

pub fn load_proof_trace(path: impl AsRef<Path>) -> Result<ProofTrace> {
    let input = fs::read_to_string(path)?;
    Ok(ProofTrace::from_json_str(&input)?)
}

pub fn latest_proof(proofs_dir: impl AsRef<Path>) -> Result<Option<ProofIndexEntry>> {
    Ok(list_proofs(proofs_dir)?.into_iter().next())
}

pub fn find_proof(proofs_dir: impl AsRef<Path>, id: &str) -> Result<ProofIndexEntry> {
    if id == "latest" {
        return latest_proof(proofs_dir)?
            .ok_or_else(|| ProofStorageError::NotFound(id.to_string()));
    }

    let matches = list_proofs(proofs_dir)?
        .into_iter()
        .filter(|entry| entry.task_id == id || entry.task_id.starts_with(id))
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [] => Err(ProofStorageError::NotFound(id.to_string())),
        [entry] => Ok(entry.clone()),
        _ => Err(ProofStorageError::Ambiguous(id.to_string())),
    }
}

pub fn export_trace_to_format(
    trace: &ProofTrace,
    format: ProofFormat,
) -> serde_json::Result<String> {
    match format {
        ProofFormat::Json => export::to_json(trace),
        ProofFormat::Markdown => report::markdown_summary(trace),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use crate::{recorder::ProofSettings, ProofMode, ProofRecorder};

    use super::*;

    #[test]
    fn latest_lookup_and_list_sorting_work() {
        let dir = temp_dir();
        let settings = ProofSettings {
            enabled: true,
            proofs_dir: dir.clone(),
            trace_json: true,
            auto_export_markdown: true,
            redact_secrets: true,
            max_capture_chars: 4_000,
            retention_days: 30,
        };
        let mut first = ProofRecorder::start_trace(
            settings.clone(),
            ProofMode::Chat,
            "first",
            None,
            None,
            None,
        );
        first.finish_trace("first summary");
        first.export().expect("export first");

        let mut second =
            ProofRecorder::start_trace(settings, ProofMode::Coder, "second", None, None, None);
        second.finish_trace("second summary");
        second.export().expect("export second");

        let list = list_proofs(&dir).expect("list");
        assert_eq!(list.len(), 2);
        let latest = latest_proof(&dir).expect("latest").expect("some");
        assert_eq!(latest.task_id, list[0].task_id);
        let unique_prefix = unique_prefix_for(&latest.task_id, &list);
        let found = find_proof(&dir, unique_prefix).expect("partial");
        assert_eq!(found.task_id, latest.task_id);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn proof_indexing_rejects_unsupported_trace_versions() {
        let dir = temp_dir();
        let session_dir = dir.join("2026-01-01").join("session-test");
        fs::create_dir_all(&session_dir).expect("create proof directory");
        let trace = ProofTrace::new(ProofMode::Chat, "session-test", "task-test", "hello");
        let mut value = serde_json::to_value(trace).expect("serialize trace");
        value["trace_version"] = serde_json::Value::String("9.0".to_string());
        fs::write(
            session_dir.join("task-test.json"),
            serde_json::to_vec(&value).expect("serialize unsupported trace"),
        )
        .expect("write unsupported trace");

        let error = list_proofs(&dir).expect_err("unsupported trace must stop indexing");
        assert!(matches!(
            error,
            ProofStorageError::Trace(ProofTraceLoadError::UnsupportedVersion(_))
        ));
        let _ = fs::remove_dir_all(dir);
    }

    fn unique_prefix_for<'a>(task_id: &'a str, entries: &[ProofIndexEntry]) -> &'a str {
        for index in 1..=task_id.len() {
            let prefix = &task_id[..index];
            let matches = entries
                .iter()
                .filter(|entry| entry.task_id.starts_with(prefix))
                .count();
            if matches == 1 {
                return prefix;
            }
        }
        task_id
    }

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-proof-storage-test-{nanos}"))
    }
}
