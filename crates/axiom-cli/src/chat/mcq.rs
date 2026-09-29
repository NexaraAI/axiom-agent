use anyhow::Result;
use axiom_core::AxiomConfig;
use axiom_engine::QuestionAnswer;

use crate::ui::out::{emitln_k, LineKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedMcq {
    pub question: String,
    pub options: Vec<String>,
}

pub(crate) fn extract_mcq_from_text(text: &str) -> Option<ParsedMcq> {
    let mut clean_lines = Vec::new();
    let mut in_code_block = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            continue;
        }
        if !in_code_block {
            clean_lines.push(trimmed);
        }
    }

    let mut options = Vec::new();
    let mut question_lines = Vec::new();
    let mut found_first_opt = false;

    for line in clean_lines {
        if line.is_empty() {
            continue;
        }
        if is_mcq_option_line(line, options.len()) {
            found_first_opt = true;
            options.push(line.to_string());
        } else if !found_first_opt {
            question_lines.push(line);
        }
    }

    if options.len() >= 2 {
        let filtered_q = question_lines
            .into_iter()
            .filter(|l| {
                !l.starts_with("**Multiple-Choice") && !l.starts_with("#") && !l.starts_with("---")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let question = if filtered_q.trim().is_empty() {
            "Multiple-Choice Question".to_string()
        } else {
            filtered_q.trim().to_string()
        };
        Some(ParsedMcq { question, options })
    } else {
        None
    }
}

fn is_mcq_option_line(line: &str, current_count: usize) -> bool {
    let stripped = line.trim_start_matches(['*', '-', ' ']).trim();
    let expected_letter = (b'A' + current_count as u8) as char;
    let expected_num = format!("{}.", current_count + 1);
    let expected_num_paren = format!("{})", current_count + 1);
    let expected_num_bracket = format!("[{}]", current_count + 1);

    if stripped.starts_with(&format!("{expected_letter}."))
        || stripped.starts_with(&format!("{expected_letter})"))
        || stripped.starts_with(&format!("[{expected_letter}]"))
        || stripped.starts_with(&format!("**{expected_letter}.**"))
        || stripped.starts_with(&format!("**{expected_letter})**"))
        || stripped.starts_with(&expected_num)
        || stripped.starts_with(&expected_num_paren)
        || stripped.starts_with(&expected_num_bracket)
    {
        return true;
    }

    false
}

pub(crate) fn render_interactive_mcq(
    question: &str,
    options: &[String],
    allow_custom: bool,
) -> Result<QuestionAnswer, String> {
    crate::ui::Spinner::clear_line();

    if options.is_empty() {
        let default_choice = question.to_string();
        return Ok(QuestionAnswer {
            selected: default_choice,
            index: Some(1),
            is_custom: false,
        });
    }

    let config = AxiomConfig::default();
    let renderer = crate::ui::Renderer::from_config(&config);

    emitln_k!(LineKind::Success);
    let result = crate::ui::interactive_select(question, options, 0, allow_custom, &renderer);

    match result {
        crate::ui::SelectionResult::Selected { index, text } => {
            emitln_k!(
                LineKind::Success,
                "{}\n",
                renderer.success(&format!("Selected: {text}"))
            );
            Ok(QuestionAnswer {
                selected: text,
                index: Some(index + 1),
                is_custom: false,
            })
        }
        crate::ui::SelectionResult::Custom(custom) => {
            emitln_k!(
                LineKind::Success,
                "{}\n",
                renderer.success(&format!("Custom reply: {custom}"))
            );
            Ok(QuestionAnswer {
                selected: custom,
                index: Some(options.len() + 1),
                is_custom: true,
            })
        }
        crate::ui::SelectionResult::Cancelled => {
            let first = options
                .first()
                .cloned()
                .unwrap_or_else(|| question.to_string());
            emitln_k!(
                LineKind::Success,
                "{}\n",
                renderer.success(&format!("Selected default: {first}"))
            );
            Ok(QuestionAnswer {
                selected: first,
                index: Some(1),
                is_custom: false,
            })
        }
    }
}
