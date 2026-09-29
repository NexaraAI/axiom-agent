//! The full-screen TUI front end.
//!
//! The session loop stays exactly as it is: it reports through [`crate::ui::out`]'s sink and
//! asks its questions through [`crate::chat::TurnPrompts`]. This module supplies both halves for the TUI —
//! a render thread that owns the terminal and a set of adapters that translate between the
//! session and that thread — so there is one turn loop rather than one per surface.
//!
//! Two OS threads are involved. The session runs on the async runtime as usual and blocks only
//! while a modal question is on screen; the render thread owns the ratatui `Terminal`, drains
//! the session's events, and reads keys. Neither ever touches the other's state directly.

mod app;
mod commands;
mod session;
mod text;
mod thread;

pub(crate) use session::{
    run_tui_session, tui_supported, HeaderInfo, ToRenderer, TuiBridge, TuiObserver, TuiPrompts,
};

/// How much of a transcript line is kept before it is truncated.
const MAX_LINE_CHARS: usize = 4_000;
/// How many transcript lines are kept. Older lines are dropped so a long session cannot grow
/// without bound.
const MAX_TRANSCRIPT_LINES: usize = 4_000;
