use std::io;
use std::sync::mpsc::{self, Sender};

use anyhow::{anyhow, Result};
use axiom_agent::StreamObserver;
use axiom_llm::ChatStreamUpdate;

use crate::chat::{ChatSession, FrontEnd, PlanDecision, TurnCancellation, TurnPrompts};
use crate::ui::out::{emitln_k, LineKind, SinkGuard};

use super::thread::render_loop;

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
