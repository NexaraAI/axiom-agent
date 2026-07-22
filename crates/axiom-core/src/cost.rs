use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions, TryLockError},
    io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::atomic_write;

pub const CURRENT_COST_LEDGER_VERSION: u32 = 1;

const DEFAULT_COST_LEDGER_LOCK_TIMEOUT: Duration = Duration::from_secs(5);
const COST_LEDGER_LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostLedgerEvent {
    pub event_id: String,
    pub session_id: String,
    pub month_utc: String,
    pub recorded_at_unix_seconds: u64,
    pub cost_microusd: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostLedger {
    pub ledger_version: u32,
    #[serde(default)]
    events: BTreeMap<String, CostLedgerEvent>,
}

impl Default for CostLedger {
    fn default() -> Self {
        Self {
            ledger_version: CURRENT_COST_LEDGER_VERSION,
            events: BTreeMap::new(),
        }
    }
}

impl CostLedger {
    pub fn events(&self) -> impl Iterator<Item = &CostLedgerEvent> {
        self.events.values()
    }

    pub fn record(&mut self, event: CostLedgerEvent) -> Result<bool, CostLedgerError> {
        validate_event(&event)?;
        match self.events.get(&event.event_id) {
            Some(existing) if existing == &event => Ok(false),
            Some(_) => Err(CostLedgerError::EventIdCollision(event.event_id)),
            None => {
                self.events.insert(event.event_id.clone(), event);
                Ok(true)
            }
        }
    }

    pub fn session_total_microusd(&self, session_id: &str) -> u64 {
        self.events
            .values()
            .filter(|event| event.session_id == session_id)
            .fold(0, |total, event| total.saturating_add(event.cost_microusd))
    }

    pub fn month_total_microusd(&self, month_utc: &str) -> u64 {
        self.events
            .values()
            .filter(|event| event.month_utc == month_utc)
            .fold(0, |total, event| total.saturating_add(event.cost_microusd))
    }

    pub fn session_totals_for_month(&self, month_utc: &str) -> BTreeMap<String, u64> {
        let mut totals = BTreeMap::<String, u64>::new();
        for event in self
            .events
            .values()
            .filter(|event| event.month_utc == month_utc)
        {
            let total = totals.entry(event.session_id.clone()).or_default();
            *total = total.saturating_add(event.cost_microusd);
        }
        totals
    }

    pub fn budget_status(
        &self,
        session_id: &str,
        month_utc: &str,
        session_budget_microusd: Option<u64>,
        monthly_budget_microusd: Option<u64>,
    ) -> CostBudgetStatus {
        let session_spent_microusd = self.session_total_microusd(session_id);
        let monthly_spent_microusd = self.month_total_microusd(month_utc);
        let session_remaining =
            session_budget_microusd.map(|budget| budget.saturating_sub(session_spent_microusd));
        let monthly_remaining =
            monthly_budget_microusd.map(|budget| budget.saturating_sub(monthly_spent_microusd));
        let remaining_microusd = match (session_remaining, monthly_remaining) {
            (Some(session), Some(monthly)) => Some(session.min(monthly)),
            (Some(session), None) => Some(session),
            (None, Some(monthly)) => Some(monthly),
            (None, None) => None,
        };
        CostBudgetStatus {
            session_spent_microusd,
            monthly_spent_microusd,
            session_budget_microusd,
            monthly_budget_microusd,
            remaining_microusd,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostBudgetStatus {
    pub session_spent_microusd: u64,
    pub monthly_spent_microusd: u64,
    pub session_budget_microusd: Option<u64>,
    pub monthly_budget_microusd: Option<u64>,
    pub remaining_microusd: Option<u64>,
}

impl CostBudgetStatus {
    pub fn is_exhausted(self) -> bool {
        self.remaining_microusd == Some(0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostLedgerStore {
    path: PathBuf,
    lock_timeout: Duration,
}

impl CostLedgerStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            lock_timeout: DEFAULT_COST_LEDGER_LOCK_TIMEOUT,
        }
    }

    /// Overrides how long a write waits for another Axiom process to finish
    /// updating this ledger.
    pub fn with_lock_timeout(mut self, timeout: Duration) -> Self {
        self.lock_timeout = timeout;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<CostLedger, CostLedgerError> {
        if !self.path.exists() {
            return Ok(CostLedger::default());
        }
        let ledger: CostLedger = serde_json::from_str(&fs::read_to_string(&self.path)?)?;
        if ledger.ledger_version != CURRENT_COST_LEDGER_VERSION {
            return Err(CostLedgerError::UnsupportedVersion {
                found: ledger.ledger_version,
                supported: CURRENT_COST_LEDGER_VERSION,
            });
        }
        for (event_id, event) in &ledger.events {
            if event_id != &event.event_id {
                return Err(CostLedgerError::InvalidEvent(format!(
                    "event map key `{event_id}` does not match event_id `{}`",
                    event.event_id
                )));
            }
            validate_event(event)?;
        }
        Ok(ledger)
    }

    pub fn save(&self, ledger: &CostLedger) -> Result<(), CostLedgerError> {
        let _lock = self.acquire_write_lock()?;
        self.save_unlocked(ledger)
    }

    fn save_unlocked(&self, ledger: &CostLedger) -> Result<(), CostLedgerError> {
        if ledger.ledger_version != CURRENT_COST_LEDGER_VERSION {
            return Err(CostLedgerError::UnsupportedVersion {
                found: ledger.ledger_version,
                supported: CURRENT_COST_LEDGER_VERSION,
            });
        }
        atomic_write(&self.path, &serde_json::to_vec_pretty(ledger)?)?;
        Ok(())
    }

    pub fn record(&self, event: CostLedgerEvent) -> Result<bool, CostLedgerError> {
        // Keep the lock across the complete read-modify-write transaction. The
        // final persistence still uses atomic replacement, so readers either
        // observe the old complete ledger or the new complete ledger.
        let _lock = self.acquire_write_lock()?;
        let mut ledger = self.load()?;
        let inserted = ledger.record(event)?;
        if inserted {
            self.save_unlocked(&ledger)?;
        }
        Ok(inserted)
    }

    fn acquire_write_lock(&self) -> Result<CostLedgerWriteLock, CostLedgerError> {
        let lock_path = ledger_lock_path(&self.path);
        let parent = lock_path.parent().unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent).map_err(|source| CostLedgerError::LockIo {
            path: lock_path.clone(),
            source,
        })?;

        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&lock_path)
            .map_err(|source| CostLedgerError::LockIo {
                path: lock_path.clone(),
                source,
            })?;
        restrict_lock_file_permissions(&file, &lock_path)?;

        let started = Instant::now();
        let mut attempted = false;
        loop {
            if attempted {
                let waited = started.elapsed();
                if waited >= self.lock_timeout {
                    return Err(CostLedgerError::LockTimeout {
                        path: lock_path,
                        waited,
                    });
                }
            }
            attempted = true;
            match file.try_lock() {
                Ok(()) => return Ok(CostLedgerWriteLock { file }),
                Err(TryLockError::WouldBlock) => {
                    let waited = started.elapsed();
                    if waited >= self.lock_timeout {
                        return Err(CostLedgerError::LockTimeout {
                            path: lock_path,
                            waited,
                        });
                    }
                    thread::sleep(COST_LEDGER_LOCK_RETRY_INTERVAL.min(self.lock_timeout - waited));
                }
                Err(TryLockError::Error(source)) if source.kind() == io::ErrorKind::Interrupted => {
                    continue;
                }
                Err(TryLockError::Error(source)) => {
                    return Err(CostLedgerError::LockIo {
                        path: lock_path,
                        source,
                    });
                }
            }
        }
    }
}

/// Owns the kernel lock for a ledger transaction.
///
/// The sibling lock file intentionally remains on disk. Removing a locked file
/// can let another process create and lock a different inode while the original
/// owner is still live. The kernel releases this advisory lock when either this
/// guard or its process exits, so a crashed owner is recovered without guessing
/// from timestamps or deleting a potentially live owner's lock.
struct CostLedgerWriteLock {
    file: File,
}

impl Drop for CostLedgerWriteLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn ledger_lock_path(path: &Path) -> PathBuf {
    let mut lock_path = path.as_os_str().to_os_string();
    lock_path.push(".lock");
    PathBuf::from(lock_path)
}

#[cfg(unix)]
fn restrict_lock_file_permissions(file: &File, path: &Path) -> Result<(), CostLedgerError> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = file
        .metadata()
        .map_err(|source| CostLedgerError::LockIo {
            path: path.to_path_buf(),
            source,
        })?
        .permissions();
    permissions.set_mode(0o600);
    file.set_permissions(permissions)
        .map_err(|source| CostLedgerError::LockIo {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(not(unix))]
fn restrict_lock_file_permissions(_file: &File, _path: &Path) -> Result<(), CostLedgerError> {
    Ok(())
}

#[derive(Debug, Error)]
pub enum CostLedgerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid cost ledger JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(
        "cost ledger schema version {found} is unsupported; this Axiom build supports {supported}"
    )]
    UnsupportedVersion { found: u32, supported: u32 },
    #[error("invalid cost ledger event: {0}")]
    InvalidEvent(String),
    #[error("cost ledger event id was reused with different data: {0}")]
    EventIdCollision(String),
    #[error("failed to use cost ledger lock `{path}`: {source}")]
    LockIo {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("timed out after {waited:?} waiting for cost ledger lock `{path}`")]
    LockTimeout { path: PathBuf, waited: Duration },
}

pub fn current_utc_month() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    utc_month_from_unix_seconds(seconds)
}

pub fn utc_month_from_unix_seconds(seconds: u64) -> String {
    let days = (seconds / 86_400) as i64;
    let (year, month, _) = civil_from_days(days);
    format!("{year:04}-{month:02}")
}

pub fn now_unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

pub fn usd_to_microusd(usd: f64) -> Option<u64> {
    if !usd.is_finite() || usd < 0.0 {
        return None;
    }
    Some((usd * 1_000_000.0).round().clamp(0.0, u64::MAX as f64) as u64)
}

fn validate_event(event: &CostLedgerEvent) -> Result<(), CostLedgerError> {
    if event.event_id.trim().is_empty() {
        return Err(CostLedgerError::InvalidEvent(
            "event_id cannot be empty".to_string(),
        ));
    }
    if event.session_id.trim().is_empty() {
        return Err(CostLedgerError::InvalidEvent(
            "session_id cannot be empty".to_string(),
        ));
    }
    let month = event.month_utc.as_bytes();
    if month.len() != 7
        || month[4] != b'-'
        || !month[..4].iter().all(u8::is_ascii_digit)
        || !month[5..].iter().all(u8::is_ascii_digit)
        || !(1..=12).contains(&event.month_utc[5..].parse::<u8>().unwrap_or_default())
    {
        return Err(CostLedgerError::InvalidEvent(format!(
            "month_utc must use YYYY-MM: {}",
            event.month_utc
        )));
    }
    Ok(())
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Barrier},
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn totals_are_keyed_by_session_and_utc_month() {
        let mut ledger = CostLedger::default();
        ledger
            .record(event("one", "session-a", "2026-07", 125))
            .unwrap();
        ledger
            .record(event("two", "session-a", "2026-08", 75))
            .unwrap();
        ledger
            .record(event("three", "session-b", "2026-07", 250))
            .unwrap();

        assert_eq!(ledger.session_total_microusd("session-a"), 200);
        assert_eq!(ledger.month_total_microusd("2026-07"), 375);
        assert_eq!(
            ledger.session_totals_for_month("2026-07"),
            BTreeMap::from([
                ("session-a".to_string(), 125),
                ("session-b".to_string(), 250)
            ])
        );
    }

    #[test]
    fn recording_is_idempotent_and_rejects_event_id_collisions() {
        let mut ledger = CostLedger::default();
        let original = event("same", "session-a", "2026-07", 125);

        assert!(ledger.record(original.clone()).expect("first record"));
        assert!(!ledger.record(original).expect("idempotent replay"));
        assert!(matches!(
            ledger.record(event("same", "session-a", "2026-07", 126)),
            Err(CostLedgerError::EventIdCollision(_))
        ));
        assert_eq!(ledger.session_total_microusd("session-a"), 125);
    }

    #[test]
    fn store_persists_idempotent_events_atomically() {
        let dir = temp_dir();
        let path = dir.join("cost-ledger.json");
        let store = CostLedgerStore::new(&path);
        let event = event("same", "session-a", "2026-07", 125);

        assert!(store.record(event.clone()).expect("first record"));
        assert!(!store.record(event).expect("idempotent replay"));
        assert_eq!(
            store
                .load()
                .expect("load ledger")
                .session_total_microusd("session-a"),
            125
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
            let lock_mode = fs::metadata(ledger_lock_path(&path))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(lock_mode, 0o600);
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_stores_serialize_read_modify_write_transactions() {
        const WRITERS: usize = 12;

        let dir = temp_dir();
        let path = dir.join("cost-ledger.json");
        let barrier = Arc::new(Barrier::new(WRITERS));
        let handles = (0..WRITERS)
            .map(|index| {
                let path = path.clone();
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let store =
                        CostLedgerStore::new(path).with_lock_timeout(Duration::from_secs(5));
                    let id = format!("writer-{index}");
                    barrier.wait();
                    store
                        .record(event(&id, "shared-session", "2026-07", 1))
                        .expect("record concurrent event");
                })
            })
            .collect::<Vec<_>>();

        for handle in handles {
            handle.join().expect("writer thread");
        }

        let ledger = CostLedgerStore::new(&path).load().expect("load ledger");
        assert_eq!(ledger.events().count(), WRITERS);
        assert_eq!(
            ledger.session_total_microusd("shared-session"),
            WRITERS as u64
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn lock_contention_times_out_with_the_lock_path() {
        let dir = temp_dir();
        let path = dir.join("cost-ledger.json");
        let store = CostLedgerStore::new(&path);
        let held_lock = store.acquire_write_lock().expect("hold ledger lock");
        let contender = CostLedgerStore::new(&path).with_lock_timeout(Duration::from_millis(60));
        let started = Instant::now();

        let error = contender
            .record(event("blocked", "session-a", "2026-07", 1))
            .expect_err("contended writer must time out");

        assert!(matches!(
            error,
            CostLedgerError::LockTimeout {
                path: ref timed_out_path,
                ..
            } if timed_out_path == &ledger_lock_path(&path)
        ));
        assert!(started.elapsed() >= Duration::from_millis(50));
        drop(held_lock);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn stale_lock_file_does_not_block_and_raii_releases_the_kernel_lock() {
        let dir = temp_dir();
        let path = dir.join("cost-ledger.json");
        let lock_path = ledger_lock_path(&path);
        fs::create_dir_all(&dir).expect("create temp dir");
        fs::write(&lock_path, b"left behind by a terminated process")
            .expect("write stale lock file");
        let store = CostLedgerStore::new(&path).with_lock_timeout(Duration::from_millis(250));

        {
            let guard = store.acquire_write_lock().expect("acquire stale lock file");
            drop(guard);
        }
        assert!(store
            .record(event("after-stale", "session-a", "2026-07", 1))
            .expect("record after stale lock"));
        assert!(
            lock_path.exists(),
            "lock inode should be retained for reuse"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn store_rejects_future_schema_versions() {
        let dir = temp_dir();
        let path = dir.join("cost-ledger.json");
        fs::create_dir_all(&dir).expect("create temp dir");
        fs::write(&path, r#"{"ledger_version":2,"events":{}}"#).expect("write ledger");

        let error = CostLedgerStore::new(&path)
            .load()
            .expect_err("future schema must fail");

        assert!(matches!(
            error,
            CostLedgerError::UnsupportedVersion {
                found: 2,
                supported: CURRENT_COST_LEDGER_VERSION
            }
        ));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn exhausted_budget_is_a_hard_stop() {
        let mut ledger = CostLedger::default();
        ledger
            .record(event("one", "session-a", "2026-07", 500))
            .unwrap();

        let status = ledger.budget_status("session-a", "2026-07", Some(500), Some(1_000));

        assert!(status.is_exhausted());
        assert_eq!(status.remaining_microusd, Some(0));
        assert_eq!(status.session_spent_microusd, 500);
        assert_eq!(status.monthly_spent_microusd, 500);
    }

    #[test]
    fn unix_month_conversion_uses_utc_calendar_months() {
        assert_eq!(utc_month_from_unix_seconds(0), "1970-01");
        assert_eq!(utc_month_from_unix_seconds(1_720_396_800), "2024-07");
    }

    fn event(id: &str, session: &str, month: &str, cost_microusd: u64) -> CostLedgerEvent {
        CostLedgerEvent {
            event_id: id.to_string(),
            session_id: session.to_string(),
            month_utc: month.to_string(),
            recorded_at_unix_seconds: 1,
            cost_microusd,
            prompt_tokens: 10,
            completion_tokens: 5,
            provider: "test".to_string(),
            model: "test-model".to_string(),
        }
    }

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-cost-ledger-test-{nanos}"))
    }
}
