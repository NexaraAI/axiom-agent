//! The full-screen TUI front end.
//!
//! The session loop stays exactly as it is: it reports through [`crate::ui::out`]'s sink and
//! asks its questions through [`TurnPrompts`]. This module supplies both halves for the TUI —
//! a render thread that owns the terminal and a set of adapters that translate between the
//! session and that thread — so there is one turn loop rather than one per surface.
//!
//! Two OS threads are involved. The session runs on the async runtime as usual and blocks only
//! while a modal question is on screen; the render thread owns the ratatui `Terminal`, drains
//! the session's events, and reads keys. Neither ever touches the other's state directly.

use std::collections::VecDeque;
use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use anyhow::{anyhow, Result};
use axiom_agent::StreamObserver;
use axiom_llm::ChatStreamUpdate;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr as _;

use crate::chat::{ChatSession, FrontEnd, PlanDecision, TurnCancellation, TurnPrompts};
use crate::ui::out::{emitln_k, LineKind, SinkGuard};

/// How often the render thread looks for a key while nothing else is happening.
///
/// Short enough that streamed deltas look live, long enough that an idle session is not a
/// busy loop.
const TICK: Duration = Duration::from_millis(30);
/// How much of a transcript line is kept before it is truncated.
const MAX_LINE_CHARS: usize = 4_000;
/// How many transcript lines are kept. Older lines are dropped so a long session cannot grow
/// without bound.
const MAX_TRANSCRIPT_LINES: usize = 4_000;

/// The session's status, drawn once at the top of the screen.
pub(crate) struct HeaderInfo {
    pub(crate) provider: String,
    pub(crate) model: String,
    pub(crate) variant: String,
    pub(crate) permission: String,
    pub(crate) work_mode: String,
    pub(crate) workspace: String,
    pub(crate) version: String,
}

/// One request the session makes of the user, drawn as a modal over the transcript.
pub(crate) enum PromptRequest {
    /// Approve, adjust, or cancel the plan the agent just proposed.
    Plan,
    /// Keep this task as a reusable skill.
    SkillCapture(String),
    /// A multiple-choice question the agent raised in its response.
    Choose {
        question: String,
        options: Vec<String>,
    },
}

/// The user's answer to a [`PromptRequest`].
pub(crate) enum PromptReply {
    Plan(PlanDecision),
    YesNo(bool),
    Choice(Option<String>),
}

/// Everything the session sends the render thread.
pub(crate) enum ToRenderer {
    /// Providers, model, workspace, and mode for the header line.
    Header(Box<HeaderInfo>),
    /// One line of agent output, already the session's wording, without escape sequences.
    Output { kind: LineKind, text: String },
    /// A chunk of streamed assistant or reasoning text.
    StreamDelta { text: String, reasoning: bool },
    /// The streamed block finished, so the next delta starts a new one.
    StreamClosed,
    /// The current plan, as the session would render it.
    Plan(Vec<String>),
    /// Ask the user something and block until they answer.
    Prompt(PromptRequest, Sender<PromptReply>),
    /// The turn ended and the session is waiting for input again.
    TurnFinished,
    /// Leave the TUI.
    Shutdown,
}

/// The render thread's handles, held by the session loop.
pub(crate) struct TuiBridge {
    /// Outbound to the render thread.
    pub(crate) events: Sender<ToRenderer>,
    /// Messages the user submitted, in order.
    pub(crate) input: tokio::sync::mpsc::UnboundedReceiver<String>,
}

/// Whether the terminal can host a full-screen session at all.
pub(crate) fn tui_supported() -> bool {
    use std::ffi::OsStr;
    use std::io::IsTerminal;

    if std::env::var_os("AXIOM_TUI").is_some_and(|value| value == OsStr::new("0")) {
        return false;
    }
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Run the session on the full-screen TUI, falling back to the inline prompt if the terminal
/// cannot be prepared.
pub(crate) async fn run_tui_session(mut session: ChatSession) -> Result<()> {
    let (events, events_rx) = mpsc::channel::<ToRenderer>();
    let (input, input_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    let (ready, ready_rx) = mpsc::channel::<io::Result<()>>();
    let cancel = TurnCancellation::default();

    let render_cancel = cancel.clone();
    let handle = std::thread::Builder::new()
        .name("axiom-tui".to_string())
        .spawn(move || render_loop(&events_rx, &input, &ready, render_cancel))
        .map_err(|error| anyhow!("could not start the TUI renderer: {error}"))?;

    // The render thread reports once the alternate screen is up. Until then the session must not
    // install the sink, or a failure would swallow the fallback banner.
    let started = matches!(ready_rx.recv(), Ok(Ok(())));
    if !started {
        drop(events);
        handle.join().ok();
        emitln_k!(
            LineKind::Warning,
            "Full-screen TUI unavailable; using the inline prompt."
        );
        return crate::chat::run_terminal_session(session, FrontEnd::Inline).await;
    }

    let sink_events = events.clone();
    let _sink = SinkGuard::install(Box::new(move |kind, text| {
        sink_events.send(ToRenderer::Output { kind, text }).ok();
    }));

    // The render thread owns the keyboard, so it is the one that has to reach the running
    // turn; the session only publishes the token.
    session.turn_cancellation = Some(cancel);
    let result = crate::chat::run_terminal_session(
        session,
        FrontEnd::Tui(TuiBridge {
            events: events.clone(),
            input: input_rx,
        }),
    )
    .await;

    events.send(ToRenderer::Shutdown).ok();
    handle.join().ok();
    result
}

/// The session's questions, answered by the render thread.
pub(crate) struct TuiPrompts {
    events: Sender<ToRenderer>,
}

impl TuiPrompts {
    pub(crate) fn new(events: Sender<ToRenderer>) -> Self {
        Self { events }
    }

    /// Put the question on screen and wait for the answer.
    ///
    /// The render thread is a separate OS thread, so waiting here cannot deadlock it. It does
    /// hold this runtime worker, which is harmless: the agent loop is paused on this answer and
    /// has nothing else to do.
    fn ask(&self, request: PromptRequest) -> Option<PromptReply> {
        let (reply, answer) = mpsc::channel();
        self.events.send(ToRenderer::Prompt(request, reply)).ok()?;
        answer.recv().ok()
    }
}

impl TurnPrompts for TuiPrompts {
    fn approve_plan(&mut self) -> PlanDecision {
        match self.ask(PromptRequest::Plan) {
            Some(PromptReply::Plan(decision)) => decision,
            // The render thread is gone, so nothing can be agreed. Changing nothing is the only
            // safe reading of "no answer".
            _ => PlanDecision::Cancel,
        }
    }

    fn confirm_skill_capture(&mut self, skill_id: &str) -> bool {
        matches!(
            self.ask(PromptRequest::SkillCapture(skill_id.to_string())),
            Some(PromptReply::YesNo(true))
        )
    }

    fn choose(&mut self, question: &str, options: &[String]) -> Option<String> {
        match self.ask(PromptRequest::Choose {
            question: question.to_string(),
            options: options.to_vec(),
        }) {
            Some(PromptReply::Choice(reply)) => reply,
            _ => None,
        }
    }
}

/// The turn's live view, streamed into the transcript.
pub(crate) struct TuiObserver {
    events: Sender<ToRenderer>,
    /// Whether any assistant text reached the transcript, so the final message is not repeated.
    saw_visible: bool,
    /// Whether a streamed block is currently open.
    open: bool,
}

impl TuiObserver {
    pub(crate) fn new(events: Sender<ToRenderer>) -> Self {
        Self {
            events,
            saw_visible: false,
            open: false,
        }
    }

    /// True once the transcript already carries the assistant's text for this turn.
    pub(crate) fn echoed_content(&self) -> bool {
        self.saw_visible
    }

    /// Close the streamed block so the next line starts on its own row.
    pub(crate) fn finish_line(&mut self) {
        if self.open {
            self.events.send(ToRenderer::StreamClosed).ok();
            self.open = false;
        }
    }
}

impl StreamObserver for TuiObserver {
    fn on_step_started(&mut self) {
        self.finish_line();
    }

    fn on_step_finished(&mut self) {
        self.finish_line();
    }

    fn on_stream_update(&mut self, update: &ChatStreamUpdate) {
        if !update.reasoning_delta.is_empty() {
            self.events
                .send(ToRenderer::StreamDelta {
                    text: update.reasoning_delta.clone(),
                    reasoning: true,
                })
                .ok();
        }
        if !update.visible_delta.is_empty() {
            self.saw_visible = true;
            self.open = true;
            self.events
                .send(ToRenderer::StreamDelta {
                    text: update.visible_delta.clone(),
                    reasoning: false,
                })
                .ok();
        }
        if update.done {
            self.finish_line();
        }
    }
}

/// Restores the terminal even if the render loop unwinds.
struct RestoreOnDrop;

impl Drop for RestoreOnDrop {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}

/// A modal question waiting for the user.
enum Modal {
    Plan {
        selected: usize,
        answer: Sender<PromptReply>,
    },
    SkillCapture {
        skill_id: String,
        answer: Sender<PromptReply>,
    },
    Choose {
        question: String,
        options: Vec<String>,
        selected: usize,
        answer: Sender<PromptReply>,
    },
}

impl Modal {
    fn answer(self) -> Sender<PromptReply> {
        match self {
            Modal::Plan { answer, .. }
            | Modal::SkillCapture { answer, .. }
            | Modal::Choose { answer, .. } => answer,
        }
    }
}

/// One rendered row of the transcript.
struct TranscriptLine {
    kind: LineKind,
    text: String,
}

pub(crate) struct SlashCommandDef {
    pub(crate) name: &'static str,
    pub(crate) args: &'static str,
    pub(crate) desc: &'static str,
}

pub(crate) const SLASH_COMMANDS: &[SlashCommandDef] = &[
    SlashCommandDef {
        name: "/help",
        args: "",
        desc: "Show help and command reference",
    },
    SlashCommandDef {
        name: "/model",
        args: "[name]",
        desc: "Switch or view active LLM model",
    },
    SlashCommandDef {
        name: "/models",
        args: "[filter]",
        desc: "List catalog view of available models",
    },
    SlashCommandDef {
        name: "/provider",
        args: "[name]",
        desc: "Show or switch active LLM provider",
    },
    SlashCommandDef {
        name: "/plan",
        args: "",
        desc: "Switch to plan mode (read-only until applied)",
    },
    SlashCommandDef {
        name: "/build",
        args: "",
        desc: "Switch to build mode (tool execution)",
    },
    SlashCommandDef {
        name: "/todo",
        args: "",
        desc: "Show the plan Axiom is tracking",
    },
    SlashCommandDef {
        name: "/skills",
        args: "",
        desc: "List active and installed skills",
    },
    SlashCommandDef {
        name: "/workspace",
        args: "[path]",
        desc: "Show or change active workspace directory",
    },
    SlashCommandDef {
        name: "/status",
        args: "",
        desc: "Show version, install mode, and binary health",
    },
    SlashCommandDef {
        name: "/update",
        args: "",
        desc: "Check for and automatically install updates",
    },
    SlashCommandDef {
        name: "/variant",
        args: "[xhigh|high|medium|low]",
        desc: "Configure model effort/variant",
    },
    SlashCommandDef {
        name: "/thinking",
        args: "[on|off|auto]",
        desc: "Toggle reasoning/thinking mode",
    },
    SlashCommandDef {
        name: "/test",
        args: "[command]",
        desc: "Auto-detect and run workspace tests",
    },
    SlashCommandDef {
        name: "/permission",
        args: "[velocity|full|strict]",
        desc: "Switch permission mode",
    },
    SlashCommandDef {
        name: "/theme",
        args: "[axiom|blood|ash|high]",
        desc: "Switch visual color theme",
    },
    SlashCommandDef {
        name: "/clear",
        args: "",
        desc: "Clear conversation history",
    },
    SlashCommandDef {
        name: "/undo",
        args: "",
        desc: "Restore latest workspace checkpoint",
    },
    SlashCommandDef {
        name: "/checkpoints",
        args: "",
        desc: "List recovery snapshots",
    },
    SlashCommandDef {
        name: "/restore",
        args: "<id>",
        desc: "Restore an agent recovery snapshot",
    },
    SlashCommandDef {
        name: "/proof",
        args: "[on|off|status|latest]",
        desc: "Audit and execution provenance",
    },
    SlashCommandDef {
        name: "/history",
        args: "[id]",
        desc: "List past sessions or switch to one",
    },
    SlashCommandDef {
        name: "/resume",
        args: "<id>",
        desc: "Continue a previous conversation",
    },
    SlashCommandDef {
        name: "/show",
        args: "<output_id>",
        desc: "Display durable tool output",
    },
    SlashCommandDef {
        name: "/commands",
        args: "",
        desc: "Display interactive command palette",
    },
    SlashCommandDef {
        name: "/exit",
        args: "",
        desc: "Exit Axiom session",
    },
];

/// Everything the render thread knows.
struct App {
    transcript: VecDeque<TranscriptLine>,
    plan: Vec<String>,
    header: Option<HeaderInfo>,
    input: String,
    cursor: usize,
    /// How many wrapped lines above the newest one the view is scrolled.
    scroll_back: usize,
    busy: bool,
    stream_open: bool,
    modal: Option<Modal>,
    /// Set when the user asked to leave.
    quit: bool,
    /// Reaches the turn that is running, so Ctrl+C can cancel it.
    cancel: TurnCancellation,
    /// Highlighted index in slash command autocomplete popup.
    autocomplete_index: usize,
    /// Whether slash command autocomplete was dismissed by Esc for the current prefix.
    autocomplete_dismissed: bool,
}

impl App {
    fn new(cancel: TurnCancellation) -> Self {
        Self {
            cancel,
            ..Self::default()
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self {
            transcript: VecDeque::from([TranscriptLine {
                kind: LineKind::Notice,
                text: "Axiom ready. Type a task, or /help for commands. Ctrl+C leaves.".to_string(),
            }]),
            plan: Vec::new(),
            header: None,
            input: String::new(),
            cursor: 0,
            scroll_back: 0,
            busy: false,
            stream_open: false,
            modal: None,
            quit: false,
            cancel: TurnCancellation::default(),
            autocomplete_index: 0,
            autocomplete_dismissed: false,
        }
    }
}

impl App {
    /// Take one event from the session.
    fn apply(&mut self, event: ToRenderer) {
        match event {
            ToRenderer::Header(info) => self.header = Some(*info),
            ToRenderer::Output { kind, text } => {
                self.stream_open = false;
                for line in text.split('\n') {
                    self.push_line(kind, line.to_string());
                }
            }
            ToRenderer::StreamDelta { text, reasoning } => {
                // Reasoning is one muted block; the visible answer is the assistant's own row.
                let kind = if reasoning {
                    LineKind::Notice
                } else {
                    LineKind::Plain
                };
                self.append_streamed(kind, &text);
            }
            ToRenderer::StreamClosed => self.stream_open = false,
            ToRenderer::Plan(lines) => self.plan = lines,
            ToRenderer::Prompt(request, answer) => self.modal = Some(open_modal(request, answer)),
            ToRenderer::TurnFinished => self.busy = false,
            ToRenderer::Shutdown => {}
        }
        self.scroll_back = 0;
    }

    fn push_line(&mut self, kind: LineKind, text: String) {
        let text = truncate_chars(&text, MAX_LINE_CHARS);
        self.transcript.push_back(TranscriptLine { kind, text });
        if self.transcript.len() > MAX_TRANSCRIPT_LINES {
            self.transcript.pop_front();
        }
    }

    /// Continue the last row of the same kind, so a streamed answer grows as one paragraph.
    fn append_streamed(&mut self, kind: LineKind, delta: &str) {
        if self.stream_open {
            if let Some(last) = self.transcript.back_mut() {
                if last.kind == kind {
                    let combined = format!("{}{}", last.text, delta);
                    last.text = truncate_chars(&combined, MAX_LINE_CHARS);
                    return;
                }
            }
        }
        self.stream_open = true;
        self.push_line(kind, delta.to_string());
    }

    fn submit(&mut self, input: &tokio::sync::mpsc::UnboundedSender<String>) {
        let text = self.input.trim().to_string();
        if text.is_empty() || self.busy {
            return;
        }
        self.push_line(LineKind::Success, format!("❯ {text}"));
        self.input.clear();
        self.cursor = 0;
        self.busy = true;
        self.autocomplete_dismissed = false;
        self.autocomplete_index = 0;
        input.send(text).ok();
    }

    /// Return the list of slash commands matching the user's current typed prefix.
    fn matching_slash_commands(&self) -> Vec<&'static SlashCommandDef> {
        if self.busy || self.modal.is_some() || self.autocomplete_dismissed {
            return Vec::new();
        }
        let trimmed = self.input.trim_start();
        if !trimmed.starts_with('/') {
            return Vec::new();
        }
        if trimmed.contains(' ') {
            return Vec::new();
        }
        let prefix = trimmed.to_lowercase();
        SLASH_COMMANDS
            .iter()
            .filter(|cmd| cmd.name.starts_with(&prefix))
            .collect()
    }

    fn on_key(&mut self, key: KeyEvent, input: &tokio::sync::mpsc::UnboundedSender<String>) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('c') => {
                    self.should_quit_by_signal();
                    return;
                }
                KeyCode::Char('l') => {
                    self.transcript.clear();
                    return;
                }
                _ => {}
            }
        }
        if self.modal.is_some() {
            self.on_modal_key(key);
            return;
        }
        let slash_matches = self.matching_slash_commands();
        if !slash_matches.is_empty() {
            match key.code {
                KeyCode::Tab => {
                    let idx = self
                        .autocomplete_index
                        .min(slash_matches.len().saturating_sub(1));
                    let cmd = slash_matches[idx];
                    self.input = format!("{} ", cmd.name);
                    self.cursor = self.input.chars().count();
                    self.autocomplete_index = 0;
                    return;
                }
                KeyCode::Up => {
                    self.autocomplete_index = self.autocomplete_index.saturating_sub(1);
                    return;
                }
                KeyCode::Down => {
                    self.autocomplete_index =
                        (self.autocomplete_index + 1).min(slash_matches.len().saturating_sub(1));
                    return;
                }
                KeyCode::Esc => {
                    self.autocomplete_dismissed = true;
                    return;
                }
                _ => {}
            }
        }
        match key.code {
            KeyCode::Enter => {
                if !slash_matches.is_empty() && self.input.trim() == "/" {
                    let idx = self
                        .autocomplete_index
                        .min(slash_matches.len().saturating_sub(1));
                    let cmd = slash_matches[idx];
                    self.input = format!("{} ", cmd.name);
                    self.cursor = self.input.chars().count();
                    self.autocomplete_index = 0;
                    return;
                }
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    || key.modifiers.contains(KeyModifiers::ALT)
                {
                    self.insert_char('\n');
                } else {
                    self.submit(input);
                }
            }
            KeyCode::Char(c) => {
                self.autocomplete_dismissed = false;
                self.autocomplete_index = 0;
                self.insert_char(c);
            }
            KeyCode::Backspace => {
                self.autocomplete_dismissed = false;
                self.autocomplete_index = 0;
                self.backspace();
            }
            KeyCode::Delete => {
                self.autocomplete_dismissed = false;
                self.autocomplete_index = 0;
                self.delete_forward();
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.input.chars().count()),
            KeyCode::Home => self.cursor = 0,
            KeyCode::End => self.cursor = self.input.chars().count(),
            KeyCode::PageUp => self.scroll_back = self.scroll_back.saturating_add(10),
            KeyCode::PageDown => self.scroll_back = self.scroll_back.saturating_sub(10),
            KeyCode::Up => self.scroll_back = self.scroll_back.saturating_add(1),
            KeyCode::Down => self.scroll_back = self.scroll_back.saturating_sub(1),
            _ => {}
        }
    }

    /// Paste text into the input field or active modal.
    fn on_paste(&mut self, text: &str) {
        let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
        if normalized.is_empty() {
            return;
        }
        if let Some(modal) = self.modal.take() {
            match modal {
                Modal::Plan { answer, .. } => {
                    self.modal = Some(Modal::Choose {
                        question: "Adjust the plan, then press Enter.".to_string(),
                        options: Vec::new(),
                        selected: 0,
                        answer,
                    });
                    self.input.clear();
                    self.cursor = 0;
                    self.insert_str(&normalized);
                }
                Modal::Choose {
                    question,
                    options,
                    selected,
                    answer,
                } => {
                    self.insert_str(&normalized);
                    self.modal = Some(Modal::Choose {
                        question,
                        options,
                        selected,
                        answer,
                    });
                }
                other => {
                    self.modal = Some(other);
                }
            }
            return;
        }
        if self.busy {
            return;
        }
        self.autocomplete_dismissed = false;
        self.autocomplete_index = 0;
        self.insert_str(&normalized);
    }

    fn insert_str(&mut self, value: &str) {
        let mut chars: Vec<char> = self.input.chars().collect();
        let at = self.cursor.min(chars.len());
        let insert_chars: Vec<char> = value.chars().collect();
        let count = insert_chars.len();
        chars.splice(at..at, insert_chars);
        self.input = chars.into_iter().collect();
        self.cursor = at + count;
    }

    /// Ctrl+C at an idle prompt leaves, which is the same gesture that leaves the inline
    /// session. A modal is dismissed instead, and a running turn is left alone because
    /// cancelling belongs to the session rather than to the view.
    fn should_quit_by_signal(&mut self) {
        if let Some(modal) = self.modal.take() {
            modal.answer().send(PromptReply::YesNo(false)).ok();
            return;
        }
        if self.busy {
            // Raw mode turns Ctrl+C into an ordinary key event, so the process signal the inline
            // session relies on is never delivered. Reach the running turn's token directly.
            self.cancel.cancel();
            self.push_line(
                LineKind::Warning,
                "Cancelling the current turn…".to_string(),
            );
            return;
        }
        self.quit = true;
    }

    fn insert_char(&mut self, value: char) {
        let mut chars: Vec<char> = self.input.chars().collect();
        let at = self.cursor.min(chars.len());
        chars.insert(at, value);
        self.input = chars.into_iter().collect();
        self.cursor = at + 1;
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut chars: Vec<char> = self.input.chars().collect();
        let at = self.cursor.min(chars.len());
        if at > 0 {
            chars.remove(at - 1);
            self.input = chars.into_iter().collect();
            self.cursor = at - 1;
        }
    }

    fn delete_forward(&mut self) {
        let mut chars: Vec<char> = self.input.chars().collect();
        let at = self.cursor.min(chars.len());
        if at < chars.len() {
            chars.remove(at);
            self.input = chars.into_iter().collect();
        }
    }

    fn on_modal_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.modal.take() else {
            return;
        };
        match modal {
            Modal::Plan { selected, answer } => {
                let typed: String = key_char(key).into_iter().collect();
                match key.code {
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.modal = Some(Modal::Plan {
                            selected: selected.saturating_sub(1),
                            answer,
                        });
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.modal = Some(Modal::Plan {
                            selected: (selected + 1).min(1),
                            answer,
                        });
                    }
                    KeyCode::Esc => {
                        answer.send(PromptReply::Plan(PlanDecision::Cancel)).ok();
                    }
                    KeyCode::Enter => {
                        let decision = if selected == 0 {
                            PlanDecision::Proceed
                        } else {
                            PlanDecision::Cancel
                        };
                        answer.send(PromptReply::Plan(decision)).ok();
                    }
                    KeyCode::Char(c) if !typed.is_empty() => {
                        // Anything else typed is the user rewriting the plan instead of
                        // approving it, so the modal becomes a one-line adjustment box.
                        self.modal = Some(Modal::Choose {
                            question: "Adjust the plan, then press Enter.".to_string(),
                            options: Vec::new(),
                            selected: 0,
                            answer,
                        });
                        self.input.clear();
                        self.cursor = 0;
                        self.insert_char(c);
                    }
                    _ => {
                        self.modal = Some(Modal::Plan { selected, answer });
                    }
                }
            }
            Modal::SkillCapture { skill_id, answer } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    answer.send(PromptReply::YesNo(true)).ok();
                    self.push_line(LineKind::Success, format!("Learned `{skill_id}`."));
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Enter => {
                    answer.send(PromptReply::YesNo(false)).ok();
                }
                _ => self.modal = Some(Modal::SkillCapture { skill_id, answer }),
            },
            Modal::Choose {
                question,
                options,
                selected,
                answer,
            } => match key.code {
                KeyCode::Esc => {
                    answer.send(PromptReply::Choice(None)).ok();
                }
                KeyCode::Up => {
                    self.modal = Some(Modal::Choose {
                        question,
                        options,
                        selected: selected.saturating_sub(1),
                        answer,
                    });
                }
                KeyCode::Down => {
                    let last = options.len().saturating_sub(1);
                    self.modal = Some(Modal::Choose {
                        question,
                        options,
                        selected: (selected + 1).min(last),
                        answer,
                    });
                }
                KeyCode::Enter => {
                    if self.input.trim().is_empty() {
                        let reply = options.get(selected).cloned();
                        answer.send(PromptReply::Choice(reply)).ok();
                    } else {
                        let reply = self.input.trim().to_string();
                        self.input.clear();
                        self.cursor = 0;
                        answer.send(PromptReply::Choice(Some(reply))).ok();
                    }
                }
                KeyCode::Char(c) => {
                    self.insert_char(c);
                    self.modal = Some(Modal::Choose {
                        question,
                        options,
                        selected,
                        answer,
                    });
                }
                KeyCode::Backspace => {
                    self.backspace();
                    self.modal = Some(Modal::Choose {
                        question,
                        options,
                        selected,
                        answer,
                    });
                }
                _ => {
                    self.modal = Some(Modal::Choose {
                        question,
                        options,
                        selected,
                        answer,
                    });
                }
            },
        }
    }

    fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        let show_plan = area.width >= 100 && !self.plan.is_empty();
        let inner_width = area.width.saturating_sub(2).max(1) as usize;
        let input_lines = wrap_text(&self.input, inner_width).len();
        let max_input_height = (area.height / 3).clamp(3, 8);
        let input_box_height = (input_lines as u16 + 2).clamp(3, max_input_height);

        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Min(3),
                Constraint::Length(input_box_height),
                Constraint::Length(1),
            ])
            .split(area);

        self.draw_header(frame, rows[0]);

        let columns = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(if show_plan {
                vec![Constraint::Min(40), Constraint::Length(38)]
            } else {
                vec![Constraint::Min(40)]
            })
            .split(rows[1]);
        self.draw_transcript(frame, columns[0]);
        if show_plan {
            self.draw_plan(frame, columns[1]);
        }
        self.draw_input(frame, rows[2]);
        self.draw_status(frame, rows[3]);
        if let Some(modal) = self.modal.as_ref() {
            draw_modal(frame, area, modal, &self.input, &self.plan);
        } else {
            self.draw_autocomplete(frame, rows[2]);
        }
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let mut fields = vec![HeaderField {
            text: " AXIOM ".to_string(),
            style: Style::default()
                .fg(Color::Black)
                .bg(Color::Indexed(75))
                .add_modifier(Modifier::BOLD),
            essential: true,
        }];
        match self.header.as_ref() {
            Some(header) => {
                fields.push(HeaderField {
                    text: format!(" {} · {}", header.provider, header.model),
                    style: Style::default().fg(Color::Indexed(255)),
                    essential: true,
                });
                fields.push(HeaderField {
                    text: format!("  [{} MODE]", header.work_mode.to_uppercase()),
                    style: Style::default().fg(Color::Indexed(208)),
                    essential: true,
                });
                fields.push(HeaderField {
                    text: format!("  variant: {}", header.variant),
                    style: Style::default().fg(Color::Indexed(243)),
                    essential: false,
                });
                fields.push(HeaderField {
                    text: format!("  perm: {}", header.permission),
                    style: Style::default().fg(Color::Indexed(243)),
                    essential: false,
                });
                fields.push(HeaderField {
                    text: format!("  {}", header.workspace),
                    style: Style::default().fg(Color::Indexed(243)),
                    essential: false,
                });
            }
            None => fields.push(HeaderField {
                text: " starting…".to_string(),
                style: Style::default().fg(Color::Indexed(243)),
                essential: true,
            }),
        }
        frame.render_widget(
            Paragraph::new(Line::from(fit_header(fields, area.width))),
            area,
        );
    }

    fn draw_transcript(&self, frame: &mut Frame, area: Rect) {
        let inner_width = area.width.saturating_sub(2) as usize;
        let mut rendered: Vec<Line> = Vec::new();
        for entry in &self.transcript {
            let style = style_for(entry.kind);
            for wrapped in wrap_text(&entry.text, inner_width.max(1)) {
                rendered.push(Line::from(Span::styled(wrapped, style)));
            }
        }
        let view_height = area.height.saturating_sub(2) as usize;
        let max_offset = rendered.len().saturating_sub(view_height);
        let offset = max_offset.saturating_sub(self.scroll_back);
        frame.render_widget(
            Paragraph::new(rendered)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Indexed(240)))
                        .title(" Session "),
                )
                // The offset is a count of wrapped rows, which a very long session could in
                // theory push past what the cursor protocol can address.
                .scroll((offset.min(u16::MAX as usize) as u16, 0)),
            area,
        );
    }

    fn draw_plan(&self, frame: &mut Frame, area: Rect) {
        let items: Vec<ListItem> = self
            .plan
            .iter()
            .map(|line| ListItem::new(Line::from(Span::raw(line.clone()))))
            .collect();
        frame.render_widget(
            List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Indexed(240)))
                        .title(" Plan "),
                )
                .style(Style::default().fg(Color::Indexed(255))),
            area,
        );
    }

    fn draw_input(&self, frame: &mut Frame, area: Rect) {
        let title = if self.busy {
            " Working… (Ctrl+C leaves when idle) "
        } else {
            " Message "
        };
        let inner_width = area.width.saturating_sub(2).max(1) as usize;
        let inner_height = area.height.saturating_sub(2).max(1) as usize;
        let before = &self.input[..byte_offset(&self.input, self.cursor)];
        let wrapped_before = wrap_text(before, inner_width);
        let cursor_row = wrapped_before.len().saturating_sub(1);
        let cursor_col = wrapped_before.last().map_or(0, |line| line.width() as u16);

        let scroll_offset = if cursor_row >= inner_height {
            cursor_row - inner_height + 1
        } else {
            0
        };

        frame.render_widget(
            Paragraph::new(self.input.as_str())
                .style(Style::default().fg(Color::Indexed(255)))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(if self.busy {
                            Color::Indexed(208)
                        } else {
                            Color::Indexed(75)
                        }))
                        .title(title),
                )
                .wrap(Wrap { trim: false })
                .scroll((scroll_offset as u16, 0)),
            area,
        );
        if self.modal.is_none() {
            let visible_row = cursor_row.saturating_sub(scroll_offset);
            frame.set_cursor_position((
                area.x + 1 + cursor_col.min(area.width.saturating_sub(2)),
                area.y + 1 + visible_row as u16,
            ));
        }
    }

    /// Draw a Minecraft/IDE-style floating command palette above the input box when typing `/`.
    fn draw_autocomplete(&self, frame: &mut Frame, input_area: Rect) {
        let matches = self.matching_slash_commands();
        if matches.is_empty() {
            return;
        }

        let max_visible = 6usize;
        let total = matches.len();
        let selected = self.autocomplete_index.min(total.saturating_sub(1));

        let start = if selected >= max_visible {
            selected - max_visible + 1
        } else {
            0
        };
        let end = (start + max_visible).min(total);
        let visible_items = &matches[start..end];

        let content_height = visible_items.len() as u16;
        let box_height = content_height + 2;
        let box_width = (input_area.width.saturating_sub(2)).clamp(45, 78);

        let box_y = input_area.y.saturating_sub(box_height);
        let box_x = input_area.x + 1;

        let popup_area = Rect {
            x: box_x,
            y: box_y,
            width: box_width,
            height: box_height,
        };

        let mut lines = Vec::new();
        for (i, cmd) in visible_items.iter().enumerate() {
            let actual_idx = start + i;
            let is_selected = actual_idx == selected;

            let marker = if is_selected { "▶ " } else { "  " };
            let cmd_with_args = if cmd.args.is_empty() {
                cmd.name.to_string()
            } else {
                format!("{} {}", cmd.name, cmd.args)
            };

            let left_col_width = 30usize;
            let cmd_formatted = format!("{:<left_col_width$}", cmd_with_args);

            let available_for_desc =
                (box_width.saturating_sub(4) as usize).saturating_sub(left_col_width + 2);
            let desc_truncated = if cmd.desc.chars().count() > available_for_desc {
                let kept: String = cmd
                    .desc
                    .chars()
                    .take(available_for_desc.saturating_sub(1))
                    .collect();
                format!("{kept}…")
            } else {
                cmd.desc.to_string()
            };

            let line_style = if is_selected {
                Style::default().bg(Color::Indexed(238))
            } else {
                Style::default()
            };

            let cmd_style = if is_selected {
                Style::default()
                    .fg(Color::Indexed(75))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Indexed(114))
            };

            let desc_style = if is_selected {
                Style::default()
                    .fg(Color::Indexed(255))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Indexed(243))
            };

            lines.push(
                Line::from(vec![
                    Span::styled(marker, cmd_style),
                    Span::styled(cmd_formatted, cmd_style),
                    Span::styled(desc_truncated, desc_style),
                ])
                .style(line_style),
            );
        }

        let title = format!(" Commands ({}/{}) · Tab to complete ", selected + 1, total);

        frame.render_widget(Clear, popup_area);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Indexed(75)))
                    .title(title),
            ),
            popup_area,
        );
    }

    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        let width = area.width as usize;
        let version = self
            .header
            .as_ref()
            .map(|header| header.version.as_str())
            .unwrap_or_default();
        // The version is pinned to the right edge and never dropped, so the hint gives way
        // first and the run stays readable however narrow the terminal gets.
        let mut right = format!("v{version} ");
        let hint = self
            .status_hints()
            .into_iter()
            .find(|hint| hint.width() + 1 + right.width() <= width);

        if right.width() > width {
            right = right.chars().take(width).collect();
        }
        let left = match hint {
            Some(hint) => format!(" {hint}"),
            None => String::new(),
        };
        let padding = width.saturating_sub(left.width() + right.width());
        let line = Line::from(vec![
            Span::styled(left, Style::default().fg(Color::Indexed(243))),
            Span::raw(" ".repeat(padding)),
            Span::styled(right, Style::default().fg(Color::Indexed(243))),
        ]);
        frame.render_widget(
            Paragraph::new(line).style(Style::default().bg(Color::Indexed(236))),
            area,
        );
    }

    /// Status-bar hints from most to least informative, so a narrow terminal loses detail
    /// rather than losing the version.
    fn status_hints(&self) -> Vec<&'static str> {
        if self.busy {
            vec![
                "working…  Ctrl+C cancel · PgUp/PgDn scroll",
                "working… Ctrl+C cancels",
                "working…",
            ]
        } else if self.modal.is_some() {
            vec![
                "Enter select · Esc dismiss · type to answer",
                "Enter select · Esc dismiss",
                "Esc dismiss",
            ]
        } else if !self.matching_slash_commands().is_empty() {
            vec![
                "Tab complete · ↑/↓ choose · Esc dismiss · Enter send",
                "Tab complete · ↑/↓ choose",
                "Tab complete",
            ]
        } else {
            vec![
                "Enter send (Shift+Enter newline) · PgUp/PgDn scroll · Ctrl+L clear · Ctrl+C exit",
                "Enter send · PgUp/PgDn scroll · Ctrl+C exit",
                "Enter send · Ctrl+C exit",
                "Ctrl+C exit",
            ]
        }
    }
}

/// Build the modal for a prompt request.
fn open_modal(request: PromptRequest, answer: Sender<PromptReply>) -> Modal {
    match request {
        PromptRequest::Plan => Modal::Plan {
            selected: 0,
            answer,
        },
        PromptRequest::SkillCapture(skill_id) => Modal::SkillCapture { skill_id, answer },
        PromptRequest::Choose { question, options } => Modal::Choose {
            question,
            options,
            selected: 0,
            answer,
        },
    }
}

fn key_char(key: KeyEvent) -> Option<char> {
    match key.code {
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => Some(c),
        _ => None,
    }
}

/// One field of the header line, in priority order.
struct HeaderField {
    text: String,
    style: Style,
    /// Survives a narrow terminal: identity and mode beat configuration detail.
    essential: bool,
}

/// Lay out as many header fields as fit, dropping the least important first and truncating what
/// is left, so a narrow terminal never silently loses which model is answering.
fn fit_header(fields: Vec<HeaderField>, width: u16) -> Vec<Span<'static>> {
    let width = width as usize;
    let mut fields = fields;
    let total =
        |fields: &[HeaderField]| fields.iter().map(|field| field.text.width()).sum::<usize>();
    while total(&fields) > width {
        match fields.iter().rposition(|field| !field.essential) {
            Some(index) => {
                fields.remove(index);
            }
            // Everything left is essential, so the remainder gets truncated below.
            None => break,
        }
    }
    truncate_spans(
        fields
            .into_iter()
            .map(|field| Span::styled(field.text, field.style))
            .collect(),
        width,
    )
}

/// Cut a styled run down to `width` columns, marking the cut with an ellipsis.
fn truncate_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let total: usize = spans.iter().map(Span::width).sum();
    if total <= width {
        return spans;
    }
    // One column is spent on the ellipsis that says the line continues.
    let budget = width.saturating_sub(1);
    let mut used = 0usize;
    let mut kept: Vec<Span<'static>> = Vec::new();
    for span in spans {
        if used >= budget {
            break;
        }
        let mut text = String::new();
        let mut taken = 0usize;
        for character in span.content.chars() {
            let character_width = character.to_string().width();
            if used + taken + character_width > budget {
                break;
            }
            text.push(character);
            taken += character_width;
        }
        used += taken;
        let truncated = taken < span.content.width();
        if !text.is_empty() {
            kept.push(Span::styled(text, span.style));
        }
        if truncated {
            break;
        }
    }
    kept.push(Span::styled("…", Style::default()));
    kept
}

fn style_for(kind: LineKind) -> Style {
    match kind {
        LineKind::Notice => Style::default().fg(Color::Indexed(243)),
        LineKind::Success => Style::default().fg(Color::Indexed(114)),
        LineKind::Warning => Style::default().fg(Color::Indexed(208)),
        LineKind::Error => Style::default()
            .fg(Color::Indexed(196))
            .add_modifier(Modifier::BOLD),
        LineKind::Plain => Style::default().fg(Color::Indexed(255)),
    }
}

/// Byte index of the `n`th character, clamped to the end of the string.
fn byte_offset(text: &str, nth: usize) -> usize {
    text.char_indices()
        .nth(nth)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{kept}…")
}

/// Soft-wrap a single line without embedded newlines to `width` display columns.
fn wrap_single_line(line: &str, width: usize) -> Vec<String> {
    if line.is_empty() {
        return vec![String::new()];
    }
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0usize;
    for word in line.split_whitespace() {
        let word_width = word.width();
        if current_width > 0 {
            if current_width + 1 + word_width <= width {
                current.push(' ');
                current.push_str(word);
                current_width += 1 + word_width;
                continue;
            } else {
                lines.push(std::mem::take(&mut current));
            }
        }
        if word_width <= width {
            current.push_str(word);
            current_width = word_width;
        } else {
            // A single token wider than the pane has to be broken mid-word.
            let mut taken = 0usize;
            for character in word.chars() {
                let char_width = unicode_width::UnicodeWidthChar::width(character).unwrap_or(1);
                if taken + char_width > width && !current.is_empty() {
                    lines.push(std::mem::take(&mut current));
                    taken = 0;
                }
                current.push(character);
                taken += char_width;
            }
            current_width = taken;
        }
    }
    lines.push(current);
    lines
}

/// Soft-wrap plain text to `width` display columns, respecting embedded newlines.
///
/// The transcript is pre-wrapped so the scroll offset can be a plain line count, which keeps
/// "scroll to the bottom" exact instead of a guess about ratatui's own wrapping.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    for raw_line in text.split('\n') {
        let trimmed = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        lines.extend(wrap_single_line(trimmed, width));
    }
    lines
}

fn draw_modal(frame: &mut Frame, area: Rect, modal: &Modal, input: &str, plan: &[String]) {
    let (title, body, footer) = match modal {
        Modal::Plan { selected, .. } => {
            let mut body: Vec<String> = Vec::new();
            if plan.is_empty() {
                body.push("The agent proposed a plan; its steps are in the plan pane.".to_string());
            } else {
                body.extend(plan.iter().cloned());
            }
            body.push(String::new());
            let options = ["Approve — implement this plan", "Cancel — change nothing"];
            for (index, option) in options.iter().enumerate() {
                let marker = if index == *selected { "▶" } else { " " };
                body.push(format!("{marker} {option}"));
            }
            (
                " Approve this plan? ",
                body,
                "↑/↓ choose · Enter confirm · Esc cancel · or type a reply to adjust",
            )
        }
        Modal::SkillCapture { skill_id, .. } => (
            " Save this as a skill? ",
            vec![
                format!("Axiom can reuse the procedure it just verified as `{skill_id}`."),
                "Future sessions will pick it up automatically.".to_string(),
            ],
            "y keep · n discard",
        ),
        Modal::Choose {
            question,
            options,
            selected,
            ..
        } => {
            let mut body = vec![question.clone(), String::new()];
            for (index, option) in options.iter().enumerate() {
                let marker = if index == *selected { "▶" } else { " " };
                body.push(format!("{marker} [{}] {option}", index + 1));
            }
            if !input.trim().is_empty() {
                body.push(String::new());
                body.push(format!("custom: {input}"));
            }
            (
                " Axiom needs one answer ",
                body,
                "↑/↓ or 1-9 choose · Enter send · Esc dismiss · type to answer instead",
            )
        }
    };

    let width = area.width.saturating_sub(8).clamp(30, 84);
    let height = (body.len() as u16 + 4).min(area.height.saturating_sub(2));
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let mut lines: Vec<Line> = body
        .into_iter()
        .map(|line| Line::from(Span::styled(line, Style::default().fg(Color::Indexed(255)))))
        .collect();
    lines.push(Line::from(Span::styled(
        footer,
        Style::default().fg(Color::Indexed(243)),
    )));
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Indexed(215)))
                .title(title),
        ),
        popup,
    );
}

/// The render thread: own the terminal, drain the session's events, read keys.
fn render_loop(
    events: &Receiver<ToRenderer>,
    input: &tokio::sync::mpsc::UnboundedSender<String>,
    ready: &Sender<io::Result<()>>,
    cancel: TurnCancellation,
) {
    let mut terminal = match ratatui::try_init() {
        Ok(terminal) => terminal,
        Err(error) => {
            ready.send(Err(error)).ok();
            return;
        }
    };
    let _ = execute!(io::stdout(), EnableBracketedPaste);
    ready.send(Ok(())).ok();
    let _restore = RestoreOnDrop;
    let mut app = App::new(cancel);
    loop {
        let mut leaving = app.quit;
        loop {
            match events.try_recv() {
                Ok(ToRenderer::Shutdown) => {
                    leaving = true;
                    break;
                }
                Ok(event) => app.apply(event),
                Err(mpsc::TryRecvError::Empty) => break,
                // The session dropped its sender, so there is nothing left to draw.
                Err(mpsc::TryRecvError::Disconnected) => {
                    leaving = true;
                    break;
                }
            }
        }
        leaving |= app.quit;
        if terminal.draw(|frame| app.draw(frame)).is_err() {
            break;
        }
        if leaving {
            break;
        }
        if event::poll(TICK).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) => app.on_key(key, input),
                Ok(Event::Paste(text)) => app.on_paste(&text),
                // Any other event just means the next loop iteration redraws.
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Draw one frame off-screen and read back the cells that were produced.
    fn render(app: &App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        // `draw` swaps buffers, so the frame that was actually rendered is the one it returns;
        // the terminal's "current" buffer is the cleared next one.
        let completed = terminal.draw(|frame| app.draw(frame)).expect("draw");
        let buffer = completed.buffer;
        let area = buffer.area;
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                if let Some(cell) = buffer.cell((x, y)) {
                    out.push_str(cell.symbol());
                }
            }
            out.push('\n');
        }
        out
    }

    /// Flatten a styled run back to plain text, for asserting on what was laid out.
    fn render_spans(spans: &[Span<'static>]) -> String {
        spans.iter().map(|span| span.content.as_ref()).collect()
    }

    fn populated() -> App {
        let mut app = App::default();
        app.apply(ToRenderer::Header(Box::new(HeaderInfo {
            provider: "openrouter".to_string(),
            model: "space-bunny-alpha".to_string(),
            variant: "xhigh".to_string(),
            permission: "velocity".to_string(),
            work_mode: "build".to_string(),
            workspace: "~/Axiom".to_string(),
            version: "1.0.23".to_string(),
        })));
        app.apply(ToRenderer::Output {
            kind: LineKind::Success,
            text: "✔ wrote aurora-snake/main.js".to_string(),
        });
        app.apply(ToRenderer::Plan(vec![
            "Plan (2 steps)".to_string(),
            "✔ scaffold".to_string(),
            "▶ wire the loop".to_string(),
        ]));
        app.input = "fix the TODO in auth.rs".to_string();
        app.cursor = app.input.chars().count();
        app
    }

    #[test]
    fn the_frame_draws_the_header_transcript_plan_and_input_together() {
        let frame = render(&populated(), 120, 30);
        for expected in [
            "AXIOM",
            "openrouter · space-bunny-alpha",
            "[BUILD MODE]",
            "~/Axiom",
            "wrote aurora-snake/main.js",
            "wire the loop",
            "fix the TODO in auth.rs",
            "Ctrl+C exit",
        ] {
            assert!(
                frame.contains(expected),
                "missing {expected:?} in:\n{frame}"
            );
        }
    }

    #[test]
    fn an_eighty_column_header_keeps_the_agent_and_drops_configuration_detail() {
        let frame = render(&populated(), 80, 24);
        // Identity, model, and mode are what a user needs to trust the screen.
        for expected in ["AXIOM", "space-bunny-alpha", "[BUILD MODE]"] {
            assert!(
                frame.contains(expected),
                "missing {expected:?} in:\n{frame}"
            );
        }
        // The workspace is the first thing to go, because it is the least likely to have
        // changed since the user last looked.
        assert!(
            !frame.contains("~/Axiom"),
            "workspace should have been dropped at 80 columns:\n{frame}"
        );
    }

    #[test]
    fn a_sixty_column_header_still_names_the_model() {
        let frame = render(&populated(), 60, 20);
        for expected in ["AXIOM", "space-bunny-alpha", "[BUILD MODE]"] {
            assert!(
                frame.contains(expected),
                "missing {expected:?} in:\n{frame}"
            );
        }
    }

    #[test]
    fn the_version_survives_every_narrow_terminal_we_render() {
        let app = populated();
        for width in [24u16, 30, 40, 60, 80, 120] {
            let frame = render(&app, width, 14);
            assert!(
                frame.contains("v1.0.23"),
                "version lost at {width} columns:\n{frame}"
            );
        }
    }

    #[test]
    fn the_status_hint_steps_down_as_the_terminal_narrows() {
        let wide = render(&populated(), 80, 14);
        assert!(wide.contains("PgUp/PgDn scroll"), "in:\n{wide}");

        let medium = render(&populated(), 40, 14);
        assert!(medium.contains("Enter send"), "in:\n{medium}");
        assert!(
            !medium.contains("PgUp/PgDn scroll"),
            "the long hint should have given way first:\n{medium}"
        );

        // Narrow enough that only the one key that cannot be guessed is worth the room.
        let narrow = render(&populated(), 24, 14);
        assert!(narrow.contains("Ctrl+C exit"), "in:\n{narrow}");
        assert!(!narrow.contains("Enter send"), "in:\n{narrow}");
        assert!(narrow.contains("v1.0.23"), "in:\n{narrow}");
    }

    #[test]
    fn a_very_narrow_header_truncates_without_losing_the_agent_name() {
        let frame = render(&populated(), 24, 14);
        assert!(frame.contains("AXIOM"), "in:\n{frame}");
        assert!(
            frame.contains('…'),
            "a truncated header should say so:\n{frame}"
        );
    }

    #[test]
    fn header_fields_are_dropped_from_the_end_and_the_tail_is_marked() {
        let field = |text: &str, essential| HeaderField {
            text: text.to_string(),
            style: Style::default(),
            essential,
        };

        // Room for everything: nothing is cut.
        let all = fit_header(vec![field("AAAA", true), field("BBBB", false)], 40);
        assert_eq!(render_spans(&all), "AAAABBBB");

        // No room for the optional field, so it goes rather than being half-shown.
        let dropped = fit_header(vec![field("AAAA", true), field("BBBB", false)], 6);
        assert_eq!(render_spans(&dropped), "AAAA");

        // Essential fields that still do not fit are truncated and marked.
        let truncated = fit_header(vec![field("AAAA", true), field("BB", true)], 5);
        assert_eq!(render_spans(&truncated), "AAAA…");
    }

    #[test]
    fn a_narrow_terminal_still_renders_and_drops_the_plan_pane() {
        let app = populated();
        for (width, height) in [(40, 12), (20, 6), (10, 4)] {
            let frame = render(&app, width, height);
            assert!(!frame.is_empty(), "{width}x{height} produced nothing");
        }
        // The plan pane would leave the transcript unreadably thin, so it is dropped.
        assert!(!render(&app, 40, 12).contains("wire the loop"));
    }

    #[test]
    fn a_modal_is_drawn_over_the_session() {
        let (answer, _replies) = mpsc::channel();
        let mut app = populated();
        app.apply(ToRenderer::Prompt(PromptRequest::Plan, answer));
        let frame = render(&app, 120, 30);
        assert!(frame.contains("Approve this plan?"), "in:\n{frame}");
        assert!(
            frame.contains("Approve — implement this plan"),
            "in:\n{frame}"
        );
        assert!(frame.contains("Cancel — change nothing"), "in:\n{frame}");
    }

    #[test]
    fn wrapping_never_exceeds_the_pane_width() {
        let text = "Axiom confines file, shell, git, and test tools to one directory.";
        for width in 4..40 {
            for line in wrap_text(text, width) {
                assert!(
                    line.width() <= width,
                    "width {width} produced {line:?} at {} columns",
                    line.width()
                );
            }
        }
    }

    #[test]
    fn wrapping_hard_breaks_a_word_wider_than_the_pane() {
        let long = "x".repeat(37);
        let lines = wrap_text(&long, 10);
        assert!(lines.len() >= 4, "expected a hard break, got {lines:?}");
        assert!(lines.iter().all(|line| line.width() <= 10));
    }

    #[test]
    fn wrapping_keeps_an_empty_line_so_blank_rows_survive() {
        assert_eq!(wrap_text("", 10), vec![String::new()]);
    }

    #[test]
    fn truncation_and_byte_offsets_clamp_instead_of_panicking() {
        assert_eq!(truncate_chars("short", 10), "short");
        assert_eq!(truncate_chars("abcdef", 3), "abc…");
        assert_eq!(byte_offset("abc", 0), 0);
        // Past the end must clamp, since the cursor can briefly outrun the buffer.
        assert_eq!(byte_offset("abc", 99), 3);
        // Multi-byte characters must land on a boundary, not inside one.
        assert_eq!(byte_offset("é!", 1), 2);
    }

    #[test]
    fn multi_line_output_becomes_one_row_per_line() {
        let mut app = App::default();
        app.apply(ToRenderer::Output {
            kind: LineKind::Error,
            text: "first\nsecond\n".to_string(),
        });
        let tail: Vec<&str> = app
            .transcript
            .iter()
            .rev()
            .take(3)
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(tail, vec!["", "second", "first"]);
    }

    #[test]
    fn streamed_deltas_accumulate_into_one_row_per_kind() {
        let mut app = App::default();
        app.apply(ToRenderer::StreamDelta {
            text: "Hello".to_string(),
            reasoning: false,
        });
        app.apply(ToRenderer::StreamDelta {
            text: ", world".to_string(),
            reasoning: false,
        });
        app.apply(ToRenderer::StreamDelta {
            text: "thinking".to_string(),
            reasoning: true,
        });
        let tail: Vec<&str> = app
            .transcript
            .iter()
            .rev()
            .take(2)
            .map(|line| line.text.as_str())
            .collect();
        assert_eq!(tail, vec!["thinking", "Hello, world"]);
    }

    #[test]
    fn a_choose_modal_takes_the_highlighted_option() {
        let (answer, replies) = mpsc::channel();
        let mut app = App::default();
        app.apply(ToRenderer::Prompt(
            PromptRequest::Choose {
                question: "Which one?".to_string(),
                options: vec!["alpha".to_string(), "beta".to_string()],
            },
            answer,
        ));

        // Enter with an empty box takes whichever option is highlighted.
        app.on_modal_key(key(KeyCode::Down));
        app.on_modal_key(key(KeyCode::Enter));
        assert!(matches!(
            replies.recv().expect("an answer"),
            PromptReply::Choice(Some(choice)) if choice == "beta"
        ));
    }

    #[test]
    fn a_choose_modal_accepts_a_typed_answer_over_the_options() {
        let (answer, replies) = mpsc::channel();
        let mut app = App::default();
        app.apply(ToRenderer::Prompt(
            PromptRequest::Choose {
                question: "Which one?".to_string(),
                options: vec!["alpha".to_string()],
            },
            answer,
        ));
        app.on_modal_key(key(KeyCode::Char('z')));
        app.on_modal_key(key(KeyCode::Enter));
        assert!(matches!(
            replies.recv().expect("an answer"),
            PromptReply::Choice(Some(choice)) if choice == "z"
        ));
    }

    #[test]
    fn dismissing_a_plan_modal_changes_nothing() {
        let (answer, replies) = mpsc::channel();
        let mut app = App::default();
        app.apply(ToRenderer::Prompt(PromptRequest::Plan, answer));
        app.on_modal_key(key(KeyCode::Esc));
        assert!(matches!(
            replies.recv().expect("an answer"),
            PromptReply::Plan(PlanDecision::Cancel)
        ));
    }

    #[test]
    fn submitting_a_message_marks_the_session_busy_and_clears_the_box() {
        let (input, mut lines) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::default();
        for character in "plan a snake game".chars() {
            app.on_key(key(KeyCode::Char(character)), &input);
        }
        app.on_key(key(KeyCode::Enter), &input);

        assert_eq!(
            lines.try_recv().expect("submitted line"),
            "plan a snake game"
        );
        assert!(app.busy);
        assert_eq!(app.input, "");
        // A second Enter while busy must not queue anything.
        app.on_key(key(KeyCode::Enter), &input);
        assert!(lines.try_recv().is_err());
        app.apply(ToRenderer::TurnFinished);
        assert!(!app.busy);
    }

    #[test]
    fn ctrl_c_cancels_a_running_turn_instead_of_leaving() {
        let mut running = App {
            busy: true,
            ..App::default()
        };
        running.should_quit_by_signal();
        assert!(
            !running.quit,
            "a running turn must not drop the whole session"
        );

        let mut idle = App::default();
        idle.should_quit_by_signal();
        assert!(idle.quit, "Ctrl+C at an idle prompt should leave");
    }

    #[test]
    fn the_transcript_is_bounded() {
        let mut app = App::default();
        for index in 0..(MAX_TRANSCRIPT_LINES + 25) {
            app.push_line(LineKind::Plain, format!("line {index}"));
        }
        assert_eq!(app.transcript.len(), MAX_TRANSCRIPT_LINES);
        assert_eq!(
            app.transcript.back().expect("newest line").text,
            format!("line {}", MAX_TRANSCRIPT_LINES + 24)
        );
    }

    #[test]
    fn pasting_inserts_multiline_text_and_positions_cursor_at_end() {
        let mut app = App::default();
        let prompt =
            "You are the dataset agent.\nCraftyAI is Minecraft-focused.\nYour job is to validate.";
        app.on_paste(prompt);
        assert_eq!(app.input, prompt);
        assert_eq!(app.cursor, prompt.chars().count());
    }

    #[test]
    fn pasting_normalizes_crlf_to_lf() {
        let mut app = App::default();
        app.on_paste("line1\r\nline2\rline3");
        assert_eq!(app.input, "line1\nline2\nline3");
    }

    #[test]
    fn pasting_is_ignored_when_busy() {
        let mut app = App {
            busy: true,
            ..App::default()
        };
        app.on_paste("should not appear");
        assert_eq!(app.input, "");
    }

    #[test]
    fn shift_enter_inserts_newline_into_input() {
        let (input, _lines) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::default();
        app.insert_char('a');
        let shift_enter = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        app.on_key(shift_enter, &input);
        app.insert_char('b');
        assert_eq!(app.input, "a\nb");
        assert_eq!(app.cursor, 3);
    }

    #[test]
    fn wrapping_respects_embedded_newlines() {
        let text = "Paragraph 1\n\nParagraph 2 is longer than the width";
        let lines = wrap_text(text, 15);
        assert_eq!(lines[0], "Paragraph 1");
        assert_eq!(lines[1], "");
        assert_eq!(lines[2], "Paragraph 2 is");
        assert_eq!(lines[3], "longer than the");
        assert_eq!(lines[4], "width");
    }

    #[test]
    fn typing_slash_triggers_autocomplete_and_tab_completes() {
        let (input, _lines) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::default();
        app.on_key(key(KeyCode::Char('/')), &input);
        assert!(!app.matching_slash_commands().is_empty());

        // Tab autocompletes the first option (/help)
        app.on_key(key(KeyCode::Tab), &input);
        assert_eq!(app.input, "/help ");
        assert!(app.matching_slash_commands().is_empty());
    }

    #[test]
    fn typing_slash_prefix_filters_matching_commands() {
        let (input, _lines) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::default();
        app.on_key(key(KeyCode::Char('/')), &input);
        app.on_key(key(KeyCode::Char('m')), &input);
        let matches = app.matching_slash_commands();
        assert!(matches.iter().all(|c| c.name.starts_with("/m")));
        assert!(matches.iter().any(|c| c.name == "/model"));
        assert!(matches.iter().any(|c| c.name == "/models"));

        // Tab completes the first match (/model)
        app.on_key(key(KeyCode::Tab), &input);
        assert_eq!(app.input, "/model ");
    }

    #[test]
    fn up_and_down_arrows_navigate_autocomplete_options() {
        let (input, _lines) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::default();
        app.on_key(key(KeyCode::Char('/')), &input);
        app.on_key(key(KeyCode::Char('m')), &input);
        assert_eq!(app.autocomplete_index, 0);

        // Down arrow selects the next suggestion (/models)
        app.on_key(key(KeyCode::Down), &input);
        assert_eq!(app.autocomplete_index, 1);

        app.on_key(key(KeyCode::Tab), &input);
        assert_eq!(app.input, "/models ");
    }

    #[test]
    fn esc_dismisses_autocomplete_until_next_character() {
        let (input, _lines) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::default();
        app.on_key(key(KeyCode::Char('/')), &input);
        assert!(!app.matching_slash_commands().is_empty());

        // Esc dismisses the autocomplete popup
        app.on_key(key(KeyCode::Esc), &input);
        assert!(app.matching_slash_commands().is_empty());

        // Typing another char reactivates it
        app.on_key(key(KeyCode::Char('p')), &input);
        assert!(!app.matching_slash_commands().is_empty());
    }
}
