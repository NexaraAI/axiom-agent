use std::sync::mpsc;

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::Terminal;
use unicode_width::UnicodeWidthStr as _;

use crate::chat::PlanDecision;
use crate::ui::out::LineKind;

use super::super::session::{PromptReply, PromptRequest, ToRenderer};
use super::super::text::{byte_offset, fit_header, truncate_chars, wrap_text, HeaderField};
use super::*;

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
