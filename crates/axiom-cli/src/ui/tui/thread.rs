use std::io;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use ratatui::crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event};
use ratatui::crossterm::execute;

use crate::chat::TurnCancellation;

use super::app::App;
use super::session::ToRenderer;

/// How often the render thread looks for a key while nothing else is happening.
///
/// Short enough that streamed deltas look live, long enough that an idle session is not a
/// busy loop.
const TICK: Duration = Duration::from_millis(30);

/// Restores the terminal even if the render loop unwinds.
struct RestoreOnDrop;

impl Drop for RestoreOnDrop {
    fn drop(&mut self) {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        ratatui::restore();
    }
}

/// The render thread: own the terminal, drain the session's events, read keys.
pub(super) fn render_loop(
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
