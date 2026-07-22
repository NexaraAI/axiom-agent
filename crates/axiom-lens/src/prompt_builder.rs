use axiom_engine::SkillCard;

pub fn build_skill_context_message(cards: &[SkillCard]) -> Option<String> {
    if cards.is_empty() {
        return None;
    }

    let mut message = String::from(
        "Axiom Skill Context:\n\
You are inside Axiom Agent.\n\
The following skills are relevant and available for this request.\n\
Use them as guidance. If a skill is executable, request it only when needed.\n\
Do not invent unavailable skills.\n\
When you need Axiom to execute a skill, respond exactly with this format and no extra text:\n\
```axiom-tool\n\
{\"skill_id\":\"file.read\",\"arguments\":{\"path\":\"README.md\"}}\n\
```\n\n\
Available skill cards:\n",
    );

    for (index, card) in cards.iter().enumerate() {
        message.push_str(&format!(
            "{}. {}\nSummary: {}\nInput: {}\nOutput: {}\nRisk: {}\n",
            index + 1,
            card.id,
            card.summary,
            card.input_contract,
            card.output_contract,
            card.risk_level
        ));

        if !card.when_to_use.is_empty() {
            message.push_str("When to use: ");
            message.push_str(&card.when_to_use.join("; "));
            message.push('\n');
        }

        message.push('\n');
    }

    Some(message)
}

#[cfg(test)]
mod tests {
    use axiom_engine::{Permission, RiskLevel, SkillCard};

    use super::*;

    #[test]
    fn prompt_builder_includes_selected_skill_cards() {
        let cards = vec![SkillCard {
            id: "python.write".to_string(),
            name: "Python Code Writer".to_string(),
            summary: "Use this when the user asks for Python code.".to_string(),
            when_to_use: vec!["User asks for Python code".to_string()],
            input_contract: "task".to_string(),
            output_contract: "code".to_string(),
            risk_level: RiskLevel::Low,
            permissions: vec![Permission::FileSystemRead],
            token_budget: 350,
        }];

        let prompt = build_skill_context_message(&cards).expect("context prompt");

        assert!(prompt.contains("Axiom Skill Context"));
        assert!(prompt.contains("python.write"));
        assert!(prompt.contains("Risk: low"));
        assert!(prompt.contains("```axiom-tool"));
    }

    #[test]
    fn prompt_builder_renders_cards_already_selected_within_the_lens_budget() {
        let cards = vec![
            card_with_budget("file.read", 1),
            card_with_budget("file.write", 1),
        ];

        let prompt = build_skill_context_message(&cards).expect("context prompt");

        assert!(prompt.contains("file.read"));
        assert!(prompt.contains("file.write"));
    }

    fn card_with_budget(id: &str, token_budget: u32) -> SkillCard {
        SkillCard {
            id: id.to_string(),
            name: id.to_string(),
            summary: "Test card".to_string(),
            when_to_use: Vec::new(),
            input_contract: "input".to_string(),
            output_contract: "output".to_string(),
            risk_level: RiskLevel::Low,
            permissions: vec![Permission::FileSystemRead],
            token_budget,
        }
    }
}
