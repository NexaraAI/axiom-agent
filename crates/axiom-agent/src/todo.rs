use serde::{Deserialize, Serialize};

const TODO_BLOCK_START: &str = "```axiom-todo";
const TODO_BLOCK_END: &str = "```";
const MAX_TODO_ITEMS: usize = 64;
const MAX_TODO_TITLE_CHARS: usize = 240;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoList {
    pub items: Vec<TodoItem>,
}

impl TodoList {
    pub fn prompt_context(&self) -> String {
        let state = if self.items.is_empty() {
            "Todo list: none.".to_string()
        } else {
            let items = self
                .items
                .iter()
                .map(|item| format!("- [{}] {}", item.status, item.title))
                .collect::<Vec<_>>()
                .join("\n");
            format!("Todo list:\n{items}")
        };
        format!(
            "{state}\nWhen a multi-step task benefits from an explicit plan, update the complete list with exactly one fenced block: ```axiom-todo\n{{\"items\":[{{\"title\":\"step\",\"status\":\"pending\"}}]}}\n```. Valid statuses are pending, in_progress, completed, and blocked. The block is control data and is hidden from the user."
        )
    }

    pub fn remaining_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| matches!(item.status, TodoStatus::Pending | TodoStatus::InProgress))
            .count()
    }

    pub fn completed_count(&self) -> usize {
        self.items
            .iter()
            .filter(|item| item.status == TodoStatus::Completed)
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedTodoUpdate {
    pub todo: TodoList,
    pub visible_content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoUpdateError(String);

impl std::fmt::Display for TodoUpdateError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for TodoUpdateError {}

pub fn parse_todo_update(content: &str) -> Result<Option<ParsedTodoUpdate>, TodoUpdateError> {
    let Some(block_start) = content.find(TODO_BLOCK_START) else {
        return Ok(None);
    };
    let json_start = block_start + TODO_BLOCK_START.len();
    let after_start = &content[json_start..];
    let Some(relative_end) = after_start.find(TODO_BLOCK_END) else {
        return Err(TodoUpdateError(
            "todo block is missing its closing fence".to_string(),
        ));
    };
    let block_end = json_start + relative_end;
    let json = content[json_start..block_end].trim();
    let todo: TodoList = serde_json::from_str(json)
        .map_err(|error| TodoUpdateError(format!("todo JSON is invalid: {error}")))?;
    validate_todo(&todo)?;

    let visible_content = format!(
        "{}{}",
        &content[..block_start],
        &content[block_end + TODO_BLOCK_END.len()..]
    )
    .trim()
    .to_string();
    Ok(Some(ParsedTodoUpdate {
        todo,
        visible_content,
    }))
}

fn validate_todo(todo: &TodoList) -> Result<(), TodoUpdateError> {
    if todo.items.len() > MAX_TODO_ITEMS {
        return Err(TodoUpdateError(format!(
            "todo list exceeds the {MAX_TODO_ITEMS}-item limit"
        )));
    }
    let mut in_progress = 0;
    for item in &todo.items {
        let title = item.title.trim();
        if title.is_empty() {
            return Err(TodoUpdateError(
                "todo item titles cannot be empty".to_string(),
            ));
        }
        if title.chars().count() > MAX_TODO_TITLE_CHARS {
            return Err(TodoUpdateError(format!(
                "todo item title exceeds {MAX_TODO_TITLE_CHARS} characters"
            )));
        }
        if item.status == TodoStatus::InProgress {
            in_progress += 1;
        }
    }
    if in_progress > 1 {
        return Err(TodoUpdateError(
            "only one todo item may be in_progress".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoItem {
    pub title: String,
    pub status: TodoStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Blocked,
}

impl std::fmt::Display for TodoStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Pending => "pending",
            Self::InProgress => "in progress",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
        };
        formatter.write_str(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn todo_context_lists_every_item_and_status() {
        let list = TodoList {
            items: vec![TodoItem {
                title: "Run tests".to_string(),
                status: TodoStatus::InProgress,
            }],
        };

        let context = list.prompt_context();
        assert!(context.starts_with("Todo list:\n- [in progress] Run tests"));
        assert!(context.contains("```axiom-todo"));
    }

    #[test]
    fn parses_and_hides_a_valid_todo_update() {
        let parsed = parse_todo_update(
            "Working.\n```axiom-todo\n{\"items\":[{\"title\":\"Run tests\",\"status\":\"in_progress\"}]}\n```\nContinuing.",
        )
        .expect("valid update")
        .expect("todo block");

        assert_eq!(parsed.todo.items.len(), 1);
        assert_eq!(parsed.todo.remaining_count(), 1);
        assert_eq!(parsed.visible_content, "Working.\n\nContinuing.");
    }

    #[test]
    fn rejects_invalid_or_ambiguous_todo_updates() {
        let malformed =
            parse_todo_update("```axiom-todo\n{nope}\n```").expect_err("invalid JSON should fail");
        assert!(malformed.to_string().contains("invalid"));

        let ambiguous = parse_todo_update(
            "```axiom-todo\n{\"items\":[{\"title\":\"A\",\"status\":\"in_progress\"},{\"title\":\"B\",\"status\":\"in_progress\"}]}\n```",
        )
        .expect_err("two active items should fail");
        assert!(ambiguous.to_string().contains("only one"));
    }
}
