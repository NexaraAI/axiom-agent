use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr as _;

use crate::ui::out::LineKind;

use super::app::Modal;

/// One field of the header line, in priority order.
pub(super) struct HeaderField {
    pub(super) text: String,
    pub(super) style: Style,
    /// Survives a narrow terminal: identity and mode beat configuration detail.
    pub(super) essential: bool,
}

/// Lay out as many header fields as fit, dropping the least important first and truncating what
/// is left, so a narrow terminal never silently loses which model is answering.
pub(super) fn fit_header(fields: Vec<HeaderField>, width: u16) -> Vec<Span<'static>> {
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
pub(super) fn truncate_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
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

pub(super) fn style_for(kind: LineKind) -> Style {
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
pub(super) fn byte_offset(text: &str, nth: usize) -> usize {
    text.char_indices()
        .nth(nth)
        .map(|(index, _)| index)
        .unwrap_or(text.len())
}

pub(super) fn truncate_chars(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{kept}…")
}

/// Soft-wrap a single line without embedded newlines to `width` display columns.
pub(super) fn wrap_single_line(line: &str, width: usize) -> Vec<String> {
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
pub(super) fn wrap_text(text: &str, width: usize) -> Vec<String> {
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

pub(super) fn draw_modal(
    frame: &mut Frame,
    area: Rect,
    modal: &Modal,
    input: &str,
    plan: &[String],
) {
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
