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
    enabled: bool,
}

impl Spinner {
    pub(crate) fn start(message: impl Into<String>, color: Color) -> Self {
        let enabled = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
        if !enabled {
            return Self {
                handle: None,
                running: Arc::new(AtomicBool::new(false)),
                enabled: false,
            };
        }

        let message = message.into();
        let running = Arc::new(AtomicBool::new(true));
        let is_running = Arc::clone(&running);

        let handle = tokio::spawn(async move {
            let mut index = 0;
            let start_time = std::time::Instant::now();
            while is_running.load(Ordering::Relaxed) {
                let frame = SPINNER_FRAMES[index % SPINNER_FRAMES.len()];
                let elapsed = start_time.elapsed().as_secs_f32();
                let styled_frame = Style::new().fg(color).bold().paint(frame);
                let styled_msg = Style::new().fg(Color::Fixed(245)).paint(&message);
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
            enabled: true,
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
        print!("\r\x1B[2K");
        let _ = io::stdout().flush();
    }
}

impl Drop for Spinner {
    fn drop(&mut self) {
        self.stop();
    }
}
