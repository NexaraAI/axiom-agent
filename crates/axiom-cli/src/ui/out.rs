//! Shared vocabulary for how a line of agent output is presented, and the sink that lets a
//! front end take those lines over.
//!
//! The inline session writes straight to stdout. A full-screen TUI cannot: anything printed
//! while the alternate screen is up lands on top of the frame. Rather than thread a writer
//! through every reporting site, the interactive code prints with [`emitln!`], which forwards
//! to a sink while one is installed and otherwise behaves exactly like `println!`. The inline
//! session never installs a sink, so its behaviour is unchanged.
//!
//! The sink is process-wide rather than per-thread because an async turn can resume on a
//! different runtime worker thread, and a per-thread sink would silently stop capturing at the
//! first await that migrated.

use std::sync::{Mutex, OnceLock};

/// How a line should be presented.
///
/// Carried as a semantic kind rather than pre-styled text so each surface can style it
/// its own way: the inline session turns these into ANSI escapes, while the full-screen
/// TUI maps them onto ratatui `Style`s, which cannot hold escape sequences at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LineKind {
    /// Orchestrator and phase notices.
    Notice,
    /// Something completed successfully.
    Success,
    /// Something degraded but did not fail.
    Warning,
    /// Something failed.
    Error,
    /// Ordinary text.
    Plain,
}

/// Where captured lines go.
///
/// `Fn` rather than `FnMut` so the sink can live in a process-wide static and be called from
/// whichever runtime worker thread a turn happens to resume on.
pub(crate) type Sink = Box<dyn Fn(LineKind, String) + Send + Sync>;

fn sink_slot() -> &'static Mutex<Option<Sink>> {
    static SINK: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();
    SINK.get_or_init(|| Mutex::new(None))
}

/// Install (`Some`) or remove (`None`) the sink that captures every `emitln!` in this process.
///
/// Prefer [`SinkGuard::install`], which removes the sink even if the work in between unwinds.
pub(crate) fn set_sink(sink: Option<Sink>) {
    if let Ok(mut slot) = sink_slot().lock() {
        *slot = sink;
    }
}

/// True while a sink is installed.
#[cfg(test)]
pub(crate) fn has_sink() -> bool {
    sink_slot()
        .lock()
        .map(|slot| slot.is_some())
        .unwrap_or(false)
}

/// Send one already-formatted line of output to the sink, or to stdout when there is none.
///
/// This is the single funnel behind [`emitln!`]; call the macro rather than this directly.
pub(crate) fn emit_line(kind: LineKind, text: String) {
    // `text` is handed back whenever the sink does not take it, so ownership (rather than a
    // flag the borrow checker cannot follow) decides which path prints.
    let unclaimed = match sink_slot().lock() {
        Ok(slot) => match slot.as_ref() {
            Some(sink) => {
                sink(kind, text);
                None
            }
            None => Some(text),
        },
        // A poisoned slot means another thread panicked mid-emit. Losing the output would be
        // worse than the poison, so fall through and let the line reach stdout.
        Err(_) => Some(text),
    };
    if let Some(text) = unclaimed {
        println!("{text}");
    }
}

/// Removes the installed sink when dropped, so a panicking front end cannot leave every later
/// line of output routed into a dead channel.
pub(crate) struct SinkGuard;

impl SinkGuard {
    /// Install `sink` and return a guard that clears it on drop.
    pub(crate) fn install(sink: Sink) -> Self {
        set_sink(Some(sink));
        Self
    }
}

impl Drop for SinkGuard {
    fn drop(&mut self) {
        set_sink(None);
    }
}

/// Print a line, preferring the installed sink over stdout.
///
/// Mirrors `println!` argument forms, including the bare `emitln!()`.
macro_rules! emitln {
    () => {
        $crate::ui::out::emit_line($crate::ui::out::LineKind::Plain, String::new())
    };
    ($($arg:tt)*) => {
        $crate::ui::out::emit_line($crate::ui::out::LineKind::Plain, format!($($arg)*))
    };
}

/// Like [`emitln!`], but tags the line so the front end can style it.
macro_rules! emitln_k {
    ($kind:expr) => {
        $crate::ui::out::emit_line($kind, String::new())
    };
    ($kind:expr, $($arg:tt)*) => {
        $crate::ui::out::emit_line($kind, format!($($arg)*))
    };
}

pub(crate) use emitln;
pub(crate) use emitln_k;

#[cfg(test)]
mod tests {
    use super::{emit_line, has_sink, LineKind, SinkGuard};

    #[test]
    fn line_kinds_are_pairwise_distinct() {
        // The TUI styles by kind, so two kinds collapsing into one variant would
        // silently erase a visual distinction.
        let kinds = [
            LineKind::Notice,
            LineKind::Success,
            LineKind::Warning,
            LineKind::Error,
            LineKind::Plain,
        ];
        for (index, kind) in kinds.iter().enumerate() {
            for other in kinds.iter().skip(index + 1) {
                assert_ne!(kind, other);
            }
        }
    }

    #[test]
    fn an_installed_sink_claims_lines_and_dropping_the_guard_releases_them() {
        use std::sync::{Arc, Mutex};

        let captured: Arc<Mutex<Vec<(LineKind, String)>>> = Arc::new(Mutex::new(Vec::new()));
        let target = Arc::clone(&captured);
        {
            let _guard = SinkGuard::install(Box::new(move |kind, text| {
                target.lock().expect("capture lock").push((kind, text));
            }));
            assert!(has_sink());
            emit_line(LineKind::Warning, "captured".to_string());
        }
        assert!(!has_sink());
        // With no sink installed the line falls through to stdout, which libtest captures.
        emit_line(LineKind::Plain, "on stdout".to_string());

        let captured = captured.lock().expect("capture lock");
        assert_eq!(
            captured.as_slice(),
            [(LineKind::Warning, "captured".to_string())]
        );
    }
}
