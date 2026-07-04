use axiom_engine::{current_axiom_version, InstalledSkill, Platform, SkillCard, SkillType};

use crate::analyze_intent;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankedSkill {
    pub skill_id: String,
    pub score: u32,
}

impl RankedSkill {
    pub fn new(skill_id: impl Into<String>, score: u32) -> Self {
        Self {
            skill_id: skill_id.into(),
            score,
        }
    }
}

pub fn select_relevant_skills(
    prompt: &str,
    installed_skills: &[InstalledSkill],
    max_cards: usize,
) -> Vec<SkillCard> {
    let intent = analyze_intent(prompt);
    let platform = Platform::current();
    let prompt_lower = prompt.to_ascii_lowercase();
    let mut scored = installed_skills
        .iter()
        .filter(|skill| skill.record.is_selectable())
        .filter(|skill| skill.manifest.is_platform_compatible(&platform))
        .filter(|skill| skill.manifest.min_axiom_version <= current_axiom_version())
        .filter_map(|skill| {
            let score = score_skill(skill, &intent.candidate_skill_ids, &prompt_lower);
            (score > 0).then_some((score, skill.manifest.to_skill_card()))
        })
        .collect::<Vec<_>>();

    scored.sort_by(|(left_score, left_card), (right_score, right_card)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_card.id.cmp(&right_card.id))
    });

    scored
        .into_iter()
        .take(max_cards)
        .map(|(_, card)| card)
        .collect()
}

fn score_skill(skill: &InstalledSkill, candidate_skill_ids: &[String], prompt_lower: &str) -> u32 {
    let mut score = 0;
    if candidate_skill_ids
        .iter()
        .any(|candidate| candidate == &skill.manifest.id)
    {
        score += 100;
    }

    for part in skill.manifest.id.split('.') {
        if prompt_lower.contains(part) {
            score += 8;
        }
    }

    if prompt_lower.contains(&skill.manifest.category.to_ascii_lowercase()) {
        score += 6;
    }

    let card = skill.manifest.to_skill_card();
    for phrase in card
        .when_to_use
        .iter()
        .chain(std::iter::once(&card.summary))
    {
        for word in phrase
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| word.len() >= 4)
        {
            if prompt_lower.contains(&word.to_ascii_lowercase()) {
                score += 1;
            }
        }
    }

    if matches!(skill.manifest.skill_type, SkillType::Guard) && prompt_lower.contains("delete") {
        score += 40;
    }

    score
}

#[cfg(test)]
mod tests {
    use axiom_engine::{
        InstalledSkillRecord, RiskLevel, SkillLifecycleState, SkillManifest, TrustLevel,
    };

    use super::*;

    #[test]
    fn selects_python_write_for_python_prompt() {
        let skills = vec![
            installed_skill(include_str!(
                "../../../fixtures/skill-registry/skills/python.write/skill.toml"
            )),
            installed_skill(include_str!(
                "../../../fixtures/skill-registry/skills/web.fetch/skill.toml"
            )),
        ];

        let cards = select_relevant_skills("write a python script", &skills, 5);

        assert_eq!(
            cards.first().map(|card| card.id.as_str()),
            Some("python.write")
        );
    }

    #[test]
    fn selects_web_fetch_for_url_prompt() {
        let skills = vec![installed_skill(include_str!(
            "../../../fixtures/skill-registry/skills/web.fetch/skill.toml"
        ))];

        let cards = select_relevant_skills("fetch https://example.com", &skills, 5);

        assert_eq!(
            cards.first().map(|card| card.id.as_str()),
            Some("web.fetch")
        );
    }

    #[test]
    fn selects_shell_safe_for_terminal_prompt() {
        let skills = vec![installed_skill(include_str!(
            "../../../fixtures/skill-registry/skills/shell.powershell.safe/skill.toml"
        ))];

        let cards = select_relevant_skills("run this command in terminal", &skills, 5);

        assert_eq!(
            cards.first().map(|card| card.id.as_str()),
            Some("shell.powershell.safe")
        );
    }

    #[test]
    fn disabled_skills_are_not_selected() {
        let mut skill = installed_skill(include_str!(
            "../../../fixtures/skill-registry/skills/python.write/skill.toml"
        ));
        skill.record.enabled = false;

        let cards = select_relevant_skills("write python", &[skill], 5);

        assert!(cards.is_empty());
    }

    #[test]
    fn quarantined_skills_are_not_selected() {
        let mut skill = installed_skill(include_str!(
            "../../../fixtures/skill-registry/skills/python.write/skill.toml"
        ));
        skill.record.enabled = false;
        skill.record.state = SkillLifecycleState::Quarantined;

        let cards = select_relevant_skills("write python", &[skill], 5);

        assert!(cards.is_empty());
    }

    #[test]
    fn max_cards_limit_is_respected() {
        let skills = vec![
            installed_skill(include_str!(
                "../../../fixtures/skill-registry/skills/file.read/skill.toml"
            )),
            installed_skill(include_str!(
                "../../../fixtures/skill-registry/skills/file.write/skill.toml"
            )),
        ];

        let cards = select_relevant_skills("read and write files", &skills, 1);

        assert_eq!(cards.len(), 1);
    }

    fn installed_skill(manifest: &str) -> InstalledSkill {
        let manifest = SkillManifest::parse_toml(manifest).expect("manifest parses");
        InstalledSkill {
            record: InstalledSkillRecord {
                id: manifest.id.clone(),
                version: manifest.version.clone(),
                installed_at: "test".to_string(),
                updated_at: None,
                source: "test".to_string(),
                registry_url: None,
                manifest_url: None,
                checksum: None,
                enabled: true,
                state: SkillLifecycleState::Enabled,
                trust_level: TrustLevel::Trusted,
                last_checked_at: None,
                last_update_error: None,
                last_runtime_error: None,
                success_count: 0,
                failure_count: 0,
                last_used_at: None,
                average_latency_ms: None,
            },
            manifest,
        }
    }

    #[test]
    fn risk_display_is_stable_for_prompt_context() {
        assert_eq!(RiskLevel::Low.to_string(), "low");
    }
}
