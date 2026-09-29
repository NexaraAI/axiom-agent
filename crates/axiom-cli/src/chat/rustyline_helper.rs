use axiom_core::AxiomConfig;
use rustyline::{
    completion::{Completer, Pair},
    highlight::Highlighter,
    hint::{Hint, Hinter},
    validate::{ValidationContext, ValidationResult, Validator},
    Cmd, ConditionalEventHandler, Context, Event, Helper,
};

use super::session_store_for_config;

pub(super) enum PromptRead {
    Line(String),
    CommandPalette,
    Interrupted,
    EndOfInput,
}

#[derive(Default, Clone)]
pub(super) struct AxiomCommandHelper {
    pub(super) colored_prompt: Option<String>,
}

pub(super) struct AxiomHint(String);

impl Hint for AxiomHint {
    fn display(&self) -> &str {
        &self.0
    }

    fn completion(&self) -> Option<&str> {
        Some(&self.0)
    }
}

const COMMAND_HINTS: &[(&str, &str)] = &[
    ("plan", ""),
    ("build", ""),
    ("variant", " [Default|low|medium|high|xhigh]"),
    ("variants", " [Default|low|medium|high|xhigh]"),
    ("model", " [name]"),
    ("models", " [filter]"),
    ("permission", " [velocity|full_machine|strict]"),
    ("mode", " [plan|build|velocity|full_machine|strict]"),
    ("todo", ""),
    ("todos", ""),
    ("workspace", " [path]"),
    ("theme", " [axiom|blood_red|ash|high_contrast]"),
    ("update", ""),
    ("provider", " [name]"),
    ("queue", " [add <task>|list|clear]"),
    ("skills", ""),
    ("undo", ""),
    ("clear", ""),
    ("checkpoints", ""),
    ("restore", " <checkpoint_id>"),
    ("history", " [number|session_id]"),
    ("sessions", " [number|session_id]"),
    ("resume", " <session_id>"),
    ("proof", " [on|off|status|latest]"),
    ("multi", ""),
    ("commands", ""),
    ("palette", ""),
    ("menu", ""),
    ("help", ""),
    ("exit", ""),
];

impl Completer for AxiomCommandHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let current = &line[..pos];
        if !current.starts_with('/') {
            return Ok((0, Vec::new()));
        }
        let prefix = "/";
        let rest = &current[1..];

        if let Some(sub) = rest.strip_prefix("permission ") {
            let start = pos - sub.len();
            let mut candidates = Vec::new();
            for opt in &["velocity", "full_machine", "strict"] {
                if opt.starts_with(sub) {
                    candidates.push(Pair {
                        display: opt.to_string(),
                        replacement: opt.to_string(),
                    });
                }
            }
            return Ok((start, candidates));
        }

        if let Some(sub) = rest.strip_prefix("mode ") {
            let start = pos - sub.len();
            let mut candidates = Vec::new();
            for opt in &["plan", "build", "velocity", "full_machine", "strict"] {
                if opt.starts_with(sub) {
                    candidates.push(Pair {
                        display: opt.to_string(),
                        replacement: opt.to_string(),
                    });
                }
            }
            return Ok((start, candidates));
        }

        if let Some(sub) = rest.strip_prefix("theme ") {
            let start = pos - sub.len();
            let mut candidates = Vec::new();
            for opt in &["axiom", "blood_red", "ash", "high_contrast"] {
                if opt.starts_with(sub) {
                    candidates.push(Pair {
                        display: opt.to_string(),
                        replacement: opt.to_string(),
                    });
                }
            }
            return Ok((start, candidates));
        }

        if let Some(sub) = rest
            .strip_prefix("variant ")
            .or_else(|| rest.strip_prefix("variants "))
        {
            let start = pos - sub.len();
            let mut candidates = Vec::new();
            for opt in &["Default", "low", "medium", "high", "xhigh"] {
                if opt
                    .to_ascii_lowercase()
                    .starts_with(&sub.to_ascii_lowercase())
                {
                    candidates.push(Pair {
                        display: opt.to_string(),
                        replacement: opt.to_string(),
                    });
                }
            }
            return Ok((start, candidates));
        }

        if let Some(sub) = rest.strip_prefix("queue ") {
            let start = pos - sub.len();
            let mut candidates = Vec::new();
            for opt in &["add ", "list", "clear"] {
                if opt.starts_with(sub) {
                    candidates.push(Pair {
                        display: opt.to_string(),
                        replacement: opt.to_string(),
                    });
                }
            }
            return Ok((start, candidates));
        }

        if let Some(sub) = rest.strip_prefix("model ") {
            let start = pos - sub.len();
            let mut candidates = Vec::new();
            for opt in &["current", "list", "use "] {
                if opt.starts_with(sub) {
                    candidates.push(Pair {
                        display: opt.to_string(),
                        replacement: opt.to_string(),
                    });
                }
            }
            return Ok((start, candidates));
        }

        if let Some(sub) = rest.strip_prefix("proof ") {
            let start = pos - sub.len();
            let mut candidates = Vec::new();
            for opt in &["on", "off", "status", "latest"] {
                if opt.starts_with(sub) {
                    candidates.push(Pair {
                        display: opt.to_string(),
                        replacement: opt.to_string(),
                    });
                }
            }
            return Ok((start, candidates));
        }

        if let Some(sub) = rest
            .strip_prefix("history ")
            .or_else(|| rest.strip_prefix("sessions "))
            .or_else(|| rest.strip_prefix("resume "))
        {
            let start = pos - sub.len();
            let mut candidates = Vec::new();
            if let Ok(config_path) = AxiomConfig::default_config_path() {
                if let Ok(sessions) = session_store_for_config(&config_path).list() {
                    for (i, s) in sessions.iter().enumerate() {
                        let num = (i + 1).to_string();
                        let id = s.id.as_str();
                        if num.starts_with(sub) {
                            candidates.push(Pair {
                                display: format!("{num} ({})", &id[..id.len().min(8)]),
                                replacement: num,
                            });
                        }
                        if id.starts_with(sub) {
                            candidates.push(Pair {
                                display: id.to_string(),
                                replacement: id.to_string(),
                            });
                        }
                    }
                }
            }
            return Ok((start, candidates));
        }

        let mut candidates = Vec::new();
        for (cmd, desc) in COMMAND_HINTS {
            if cmd.starts_with(rest) {
                candidates.push(Pair {
                    display: format!("{prefix}{cmd}{desc}"),
                    replacement: format!("{prefix}{cmd} "),
                });
            }
        }

        Ok((0, candidates))
    }
}

impl Hinter for AxiomCommandHelper {
    type Hint = AxiomHint;

    fn hint(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> Option<Self::Hint> {
        if pos < line.len() {
            return None;
        }
        if !line.starts_with('/') {
            return None;
        }
        let rest = &line[1..];
        if rest.is_empty() {
            return Some(AxiomHint(
                " [type command or press Enter for menu]".to_string(),
            ));
        }
        if let Some(sub) = rest.strip_prefix("permission ") {
            for opt in &["velocity", "full_machine", "strict"] {
                if let Some(suffix) = opt.strip_prefix(sub) {
                    if !suffix.is_empty() {
                        return Some(AxiomHint(suffix.to_string()));
                    }
                }
            }
            return None;
        }
        if let Some(sub) = rest.strip_prefix("mode ") {
            for opt in &["plan", "build", "velocity", "full_machine", "strict"] {
                if let Some(suffix) = opt.strip_prefix(sub) {
                    if !suffix.is_empty() {
                        return Some(AxiomHint(suffix.to_string()));
                    }
                }
            }
            return None;
        }
        if let Some(sub) = rest
            .strip_prefix("variant ")
            .or_else(|| rest.strip_prefix("variants "))
        {
            for opt in &["Default", "low", "medium", "high", "xhigh"] {
                if let Some(suffix) = opt.strip_prefix(sub) {
                    if !suffix.is_empty() {
                        return Some(AxiomHint(suffix.to_string()));
                    }
                }
            }
            return None;
        }
        if let Some(sub) = rest.strip_prefix("queue ") {
            for opt in &["add <task>", "list", "clear"] {
                if let Some(suffix) = opt.strip_prefix(sub) {
                    if !suffix.is_empty() {
                        return Some(AxiomHint(suffix.to_string()));
                    }
                }
            }
            return None;
        }
        for (cmd, desc) in COMMAND_HINTS {
            if let Some(suffix) = cmd.strip_prefix(rest) {
                return Some(AxiomHint(format!("{suffix}{desc}")));
            }
        }
        None
    }
}

impl Highlighter for AxiomCommandHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> std::borrow::Cow<'b, str> {
        // Always return raw prompt without ANSI escape codes to ensure Rustyline
        // accurately calculates visual column width and prevents cursor drifting or line-wrap collisions.
        std::borrow::Cow::Borrowed(prompt)
    }

    fn highlight_hint<'h>(&self, hint: &'h str) -> std::borrow::Cow<'h, str> {
        std::borrow::Cow::Owned(format!("\x1b[90m{hint}\x1b[0m"))
    }
}

impl Validator for AxiomCommandHelper {
    fn validate(&self, _ctx: &mut ValidationContext<'_>) -> rustyline::Result<ValidationResult> {
        Ok(ValidationResult::Valid(None))
    }
}

impl Helper for AxiomCommandHelper {}

#[derive(Clone)]
pub(super) struct PaletteTriggerHandler {
    pub(super) triggered: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl ConditionalEventHandler for PaletteTriggerHandler {
    fn handle(
        &self,
        _evt: &Event,
        _n: rustyline::RepeatCount,
        _positive: bool,
        _ctx: &rustyline::EventContext,
    ) -> Option<Cmd> {
        self.triggered
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Some(Cmd::Interrupt)
    }
}
