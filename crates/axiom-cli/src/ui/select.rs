use std::io::{self, IsTerminal, Write};

use crate::ui::render::{pad_card_line, visible_width, Renderer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SelectionResult {
    Selected { index: usize, text: String },
    Custom(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Up,
    Down,
    Enter,
    Escape,
    Char(char),
    Other,
}

#[cfg(windows)]
mod platform {
    use super::Key;

    pub(super) fn read_key() -> Key {
        extern "C" {
            fn _getch() -> std::ffi::c_int;
        }
        let ch = unsafe { _getch() };
        if ch == 0 || ch == 224 {
            let code = unsafe { _getch() };
            match code {
                72 => Key::Up,
                80 => Key::Down,
                _ => Key::Other,
            }
        } else {
            match ch {
                13 => Key::Enter,
                27 => Key::Escape,
                c if (32..=126).contains(&c) => {
                    if let Some(ch) = char::from_u32(c as u32) {
                        Key::Char(ch)
                    } else {
                        Key::Other
                    }
                }
                _ => Key::Other,
            }
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use std::io::{self, Read};

    use super::Key;

    struct RawModeGuard {
        saved: Option<String>,
    }

    impl RawModeGuard {
        fn new() -> Self {
            let saved = std::process::Command::new("stty")
                .arg("-g")
                .output()
                .ok()
                .and_then(|out| {
                    if out.status.success() {
                        String::from_utf8(out.stdout).ok()
                    } else {
                        None
                    }
                });
            let _ = std::process::Command::new("stty")
                .arg("-echo")
                .arg("raw")
                .arg("min")
                .arg("1")
                .status();
            Self { saved }
        }
    }

    impl Drop for RawModeGuard {
        fn drop(&mut self) {
            if let Some(ref saved) = self.saved {
                let trimmed = saved.trim();
                if !trimmed.is_empty() {
                    let _ = std::process::Command::new("stty").arg(trimmed).status();
                    return;
                }
            }
            let _ = std::process::Command::new("stty")
                .arg("echo")
                .arg("-raw")
                .status();
        }
    }

    pub(super) fn read_key() -> Key {
        let _guard = RawModeGuard::new();
        let mut buf = [0u8; 1];
        if io::stdin().read_exact(&mut buf).is_err() {
            return Key::Other;
        }
        match buf[0] {
            13 | 10 => Key::Enter,
            3 => Key::Escape,
            27 => {
                let _ = std::process::Command::new("stty")
                    .arg("min")
                    .arg("0")
                    .arg("time")
                    .arg("1")
                    .status();
                let mut seq = [0u8; 2];
                if io::stdin().read_exact(&mut seq[0..1]).is_ok() && seq[0] == b'[' {
                    if io::stdin().read_exact(&mut seq[1..2]).is_ok() {
                        match seq[1] {
                            b'A' => Key::Up,
                            b'B' => Key::Down,
                            _ => Key::Other,
                        }
                    } else {
                        Key::Escape
                    }
                } else {
                    Key::Escape
                }
            }
            c if (32..=126).contains(&c) => Key::Char(c as char),
            _ => Key::Other,
        }
    }
}

pub(crate) fn interactive_select(
    title: &str,
    options: &[String],
    initial_index: usize,
    allow_custom: bool,
    renderer: &Renderer,
) -> SelectionResult {
    let is_term = io::stdin().is_terminal() && io::stdout().is_terminal();
    if !is_term || options.is_empty() {
        return fallback_select(title, options, allow_custom, renderer);
    }

    let total_options = if allow_custom {
        options.len() + 1
    } else {
        options.len()
    };
    let mut selected_idx = initial_index.min(total_options.saturating_sub(1));

    let initial_lines = render_card(title, options, selected_idx, allow_custom, renderer);
    let line_count = initial_lines.len();
    for line in &initial_lines {
        println!("{line}");
    }
    let _ = io::stdout().flush();

    loop {
        let key = platform::read_key();
        match key {
            Key::Up => {
                if selected_idx > 0 {
                    selected_idx -= 1;
                } else {
                    selected_idx = total_options - 1;
                }
            }
            Key::Down => {
                if selected_idx + 1 < total_options {
                    selected_idx += 1;
                } else {
                    selected_idx = 0;
                }
            }
            Key::Char(c) => {
                if let Some(digit) = c.to_digit(10) {
                    let num = digit as usize;
                    if num >= 1 && num <= total_options {
                        selected_idx = num - 1;
                    }
                } else if c == 'k' {
                    if selected_idx > 0 {
                        selected_idx -= 1;
                    } else {
                        selected_idx = total_options - 1;
                    }
                } else if c == 'j' {
                    if selected_idx + 1 < total_options {
                        selected_idx += 1;
                    } else {
                        selected_idx = 0;
                    }
                } else if c == 'q' {
                    return SelectionResult::Cancelled;
                }
            }
            Key::Escape => {
                return SelectionResult::Cancelled;
            }
            Key::Enter => {
                break;
            }
            Key::Other => {}
        }

        print!("\x1b[{}A\r", line_count);
        let updated_lines = render_card(title, options, selected_idx, allow_custom, renderer);
        for line in &updated_lines {
            println!("{line}");
        }
        let _ = io::stdout().flush();
    }

    if allow_custom && selected_idx == options.len() {
        print!(
            "  {} {}",
            renderer.primary("│"),
            renderer.bone("Type custom reply: ")
        );
        let _ = io::stdout().flush();
        let mut custom_input = String::new();
        let _ = io::stdin().read_line(&mut custom_input);
        let trimmed = custom_input.trim();
        if trimmed.is_empty() {
            SelectionResult::Cancelled
        } else {
            SelectionResult::Custom(trimmed.to_string())
        }
    } else {
        SelectionResult::Selected {
            index: selected_idx,
            text: options[selected_idx].clone(),
        }
    }
}

fn render_card(
    title: &str,
    options: &[String],
    selected_idx: usize,
    allow_custom: bool,
    renderer: &Renderer,
) -> Vec<String> {
    let border_top = "  ┌─────────────────────────────────────────────────────────────┐";
    let border_bottom = "  └─────────────────────────────────────────────────────────────┘";
    let border_mid = "  ├─────────────────────────────────────────────────────────────┤";
    let card_empty = pad_card_line("│", "", 58);

    let mut lines = Vec::new();
    lines.push(renderer.border(border_top));

    let title_vis = visible_width(title).min(45);
    let title_display = if visible_width(title) > 45 {
        format!(
            "{}...",
            &title[..title.chars().take(42).map(|c| c.len_utf8()).sum()]
        )
    } else {
        title.to_string()
    };
    let title_styled = renderer.bone(&title_display);
    let esc_styled = renderer.smoke("esc");
    let esc_vis = 3;
    let spaces = " ".repeat(58_usize.saturating_sub(title_vis + esc_vis));
    let header_line = format!("{title_styled}{spaces}{esc_styled}");
    lines.push(pad_card_line(&renderer.border("│"), &header_line, 58));
    lines.push(renderer.border(border_mid));
    lines.push(renderer.border(&card_empty));

    for (i, opt) in options.iter().enumerate() {
        let is_selected = i == selected_idx;
        let line_content = format_option_line(i + 1, opt, is_selected, renderer);
        lines.push(line_content);
    }

    if allow_custom {
        let is_selected = selected_idx == options.len();
        let line_content =
            format_option_line(options.len() + 1, "Custom reply...", is_selected, renderer);
        lines.push(line_content);
    }

    lines.push(renderer.border(&card_empty));
    let controls = renderer.smoke("↑ / ↓ navigate · enter select · 1-9 jump · esc cancel");
    lines.push(pad_card_line(&renderer.border("│"), &controls, 58));
    lines.push(renderer.border(border_bottom));

    lines
}

fn format_option_line(num: usize, text: &str, is_selected: bool, renderer: &Renderer) -> String {
    let raw_content = format!("[{num}] {text}");
    let raw_vis = visible_width(&raw_content);
    let max_text_width: usize = 52;
    let (truncated, trunc_vis) = if raw_vis > max_text_width {
        let mut budget = max_text_width.saturating_sub(3);
        let mut out = String::new();
        for c in raw_content.chars() {
            let w = visible_width(&c.to_string());
            if budget >= w {
                out.push(c);
                budget -= w;
            } else {
                break;
            }
        }
        out.push_str("...");
        let v = visible_width(&out);
        (out, v)
    } else {
        (raw_content, raw_vis)
    };

    if is_selected {
        let padding_spaces = " ".repeat(55_usize.saturating_sub(trunc_vis));
        let highlighted = format!("\x1b[48;5;215;38;5;16;1m▌ {truncated}{padding_spaces}\x1b[0m");
        pad_card_line(&renderer.border("│"), &highlighted, 58)
    } else {
        let content = format!("  {truncated}");
        pad_card_line(&renderer.border("│"), &renderer.bone(&content), 58)
    }
}

fn fallback_select(
    title: &str,
    options: &[String],
    allow_custom: bool,
    renderer: &Renderer,
) -> SelectionResult {
    println!();
    println!("{}", renderer.mcq_card(title, options, allow_custom));
    let num_choices = if allow_custom {
        options.len() + 1
    } else {
        options.len()
    };
    print!(
        "{}{} ",
        renderer.prompt(),
        renderer.plain(&format!("Select [1-{num_choices}] or type custom reply:"))
    );
    let _ = io::stdout().flush();

    let mut input = String::new();
    let _ = io::stdin().read_line(&mut input);
    let trimmed = input.trim();

    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("esc") {
        return SelectionResult::Cancelled;
    }

    if let Ok(num) = trimmed.parse::<usize>() {
        if num >= 1 && num <= options.len() {
            return SelectionResult::Selected {
                index: num - 1,
                text: options[num - 1].clone(),
            };
        } else if allow_custom && num == options.len() + 1 {
            return SelectionResult::Custom("Custom reply".to_string());
        }
    }

    if allow_custom {
        SelectionResult::Custom(trimmed.to_string())
    } else if let Some(choice) = options.first() {
        SelectionResult::Selected {
            index: 0,
            text: choice.clone(),
        }
    } else {
        SelectionResult::Cancelled
    }
}

#[cfg(test)]
mod tests {
    use axiom_core::AxiomConfig;

    use super::*;

    #[test]
    fn render_card_maintains_65_char_width_across_all_lines() {
        let config = AxiomConfig::default();
        let renderer = Renderer::from_config(&config);
        let options = vec![
            "velocity (Recommended: high speed)".to_string(),
            "full_machine (Unrestricted full system access)".to_string(),
            "strict (Zero-trust isolation, confirms every mutation)".to_string(),
        ];

        for selected_idx in 0..options.len() {
            let lines = render_card(
                "Select Permission Mode",
                &options,
                selected_idx,
                true,
                &renderer,
            );
            for (idx, line) in lines.iter().enumerate() {
                let vis = visible_width(line);
                assert_eq!(
                    vis, 65,
                    "Line {} for selected_idx {} has visible width {} instead of 65: '{}'",
                    idx, selected_idx, vis, line
                );
            }
        }
    }
}
