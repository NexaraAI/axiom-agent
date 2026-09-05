use std::{
    io::{self, IsTerminal, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use nu_ansi_term::{Color, Style};
use tokio::task::JoinHandle;

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub(crate) struct Spinner {
    handle: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
    #[allow(dead_code)]
    message: Arc<std::sync::RwLock<String>>,
    enabled: bool,
}

impl Spinner {
    pub(crate) fn start(message: impl Into<String>, color: Color) -> Self {
        let enabled = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        let message_str = message.into();
        let message_arc = Arc::new(std::sync::RwLock::new(message_str));
        if !enabled {
            return Self {
                handle: None,
                running: Arc::new(AtomicBool::new(false)),
                message: message_arc,
                enabled: false,
            };
        }

        let running = Arc::new(AtomicBool::new(true));
        let is_running = Arc::clone(&running);
        let msg_clone = Arc::clone(&message_arc);

        let handle = tokio::spawn(async move {
            let mut index = 0;
            let start_time = std::time::Instant::now();
            while is_running.load(Ordering::Relaxed) {
                let frame = SPINNER_FRAMES[index % SPINNER_FRAMES.len()];
                let elapsed = start_time.elapsed().as_secs_f32();
                let styled_frame = Style::new().fg(color).bold().paint(frame);

                let mut current_msg = msg_clone.read().map(|g| g.clone()).unwrap_or_default();
                if current_msg == "Thinking..." && elapsed > 2.5 {
                    current_msg = "Buffering response...".to_string();
                }

                let styled_msg = Style::new().fg(Color::Fixed(245)).paint(&current_msg);
                let timer = Style::new()
                    .fg(Color::Fixed(240))
                    .paint(format!("({elapsed:.1}s)"));

                print!("\r\x1B[2K{styled_frame} {styled_msg} {timer}");
                let _ = io::stdout().flush();

                index = index.wrapping_add(1);
                tokio::time::sleep(Duration::from_millis(80)).await;
            }
        });

        Self {
            handle: Some(handle),
            running,
            message: message_arc,
            enabled: true,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn set_message(&self, message: impl Into<String>) {
        if let Ok(mut guard) = self.message.write() {
            *guard = message.into();
        }
    }

    pub(crate) fn clear_line() {
        if io::stdout().is_terminal() {
            print!("\r\x1B[2K");
            let _ = io::stdout().flush();
        }
    }

    pub(crate) fn stop(&mut self) {
        if !self.enabled {
            return;
        }
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        Self::clear_line();
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
    }
}
