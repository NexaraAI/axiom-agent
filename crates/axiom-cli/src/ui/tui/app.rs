use std::collections::VecDeque;
use std::sync::mpsc::Sender;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr as _;

use crate::ui::out::LineKind;

use crate::chat::{PlanDecision, TurnCancellation};

use super::{
    commands::{SlashCommandDef, SLASH_COMMANDS},
    session::{HeaderInfo, PromptReply, PromptRequest, ToRenderer},
    text::{
        byte_offset, draw_modal, fit_header, style_for, truncate_chars, wrap_text, HeaderField,
    },
    MAX_LINE_CHARS, MAX_TRANSCRIPT_LINES,
};

/// A modal question waiting for the user.
pub(super) enum Modal {
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
pub(super) struct TranscriptLine {
    kind: LineKind,
    text: String,
}

/// Everything the render thread knows.
pub(super) struct App {
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
    pub(super) quit: bool,
    /// Reaches the turn that is running, so Ctrl+C can cancel it.
    cancel: TurnCancellation,
    /// Highlighted index in slash command autocomplete popup.
    autocomplete_index: usize,
    /// Whether slash command autocomplete was dismissed by Esc for the current prefix.
    autocomplete_dismissed: bool,
}

impl App {
    pub(super) fn new(cancel: TurnCancellation) -> Self {
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
    pub(super) fn apply(&mut self, event: ToRenderer) {
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

    pub(super) fn on_key(
        &mut self,
        key: KeyEvent,
        input: &tokio::sync::mpsc::UnboundedSender<String>,
    ) {
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
    pub(super) fn on_paste(&mut self, text: &str) {
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

    pub(super) fn draw(&self, frame: &mut Frame) {
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

#[cfg(test)]
mod tests;
