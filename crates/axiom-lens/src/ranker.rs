use std::collections::{BTreeMap, BTreeSet};

use axiom_engine::{current_axiom_version, InstalledSkill, Platform, SkillCard, SkillType};

use crate::analyze_intent;

pub const DEFAULT_CARD_TOKEN_BUDGET: u32 = 1_200;

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
    select_relevant_skills_with_budget(
        prompt,
        installed_skills,
        max_cards,
        DEFAULT_CARD_TOKEN_BUDGET,
    )
}

pub fn select_relevant_skills_with_budget(
    prompt: &str,
    installed_skills: &[InstalledSkill],
    max_cards: usize,
    max_card_token_budget: u32,
) -> Vec<SkillCard> {
    let intent = analyze_intent(prompt);
    let platform = Platform::current();
    let prompt_lower = prompt.to_ascii_lowercase();
    let available = installed_skills
        .iter()
        .filter(|skill| skill.record.is_selectable())
        .filter(|skill| skill.manifest.is_platform_compatible(&platform))
        .filter(|skill| skill.manifest.min_axiom_version <= current_axiom_version())
        .collect::<Vec<_>>();
    let available_by_id = available
        .iter()
        .map(|skill| (skill.manifest.id.as_str(), *skill))
        .collect::<BTreeMap<_, _>>();
    let mut scored = available
        .iter()
        .filter_map(|skill| {
            let score = score_skill(skill, &intent.candidate_skill_ids, &prompt_lower);
            (score > 0).then_some((score, *skill))
        })
        .collect::<Vec<_>>();

    scored.sort_by(|(left_score, left_skill), (right_score, right_skill)| {
        right_score
            .cmp(left_score)
            .then_with(|| left_skill.manifest.id.cmp(&right_skill.manifest.id))
    });

    let mut total_budget: u32 = 0;
    let mut selected = Vec::new();
    let mut selected_ids = BTreeSet::new();
    for (_, skill) in scored {
        if selected.len() >= max_cards {
            break;
        }
        let mut dependency_order = Vec::new();
        if !resolve_dependencies(
            skill,
            &available_by_id,
            &mut BTreeSet::new(),
            &mut BTreeSet::new(),
            &mut dependency_order,
        ) {
            continue;
        }
        let cards = dependency_order
            .into_iter()
            .filter(|candidate| !selected_ids.contains(candidate.manifest.id.as_str()))
            .map(|candidate| candidate.manifest.to_skill_card())
            .collect::<Vec<_>>();
        let added_budget = cards
            .iter()
            .fold(0_u32, |sum, card| sum.saturating_add(card.token_budget));
        if selected.len().saturating_add(cards.len()) > max_cards
            || total_budget.saturating_add(added_budget) > max_card_token_budget
        {
            continue;
        }
        for card in cards {
            total_budget = total_budget.saturating_add(card.token_budget);
            selected_ids.insert(card.id.clone());
            selected.push(card);
        }
    }
    selected
}

fn resolve_dependencies<'a>(
    skill: &'a InstalledSkill,
    available_by_id: &BTreeMap<&str, &'a InstalledSkill>,
    visiting: &mut BTreeSet<String>,
    visited: &mut BTreeSet<String>,
    ordered: &mut Vec<&'a InstalledSkill>,
) -> bool {
    if visited.contains(&skill.manifest.id) {
        return true;
    }
    if !visiting.insert(skill.manifest.id.clone()) {
        return false;
    }
    for requirement in &skill.manifest.depends_on {
        let dependency = available_by_id
            .get(requirement.as_str())
            .copied()
            .or_else(|| {
                available_by_id
                    .values()
                    .copied()
                    .find(|candidate| candidate.manifest.provides.contains(requirement))
            });
        let Some(dependency) = dependency else {
            return false;
        };
        if !resolve_dependencies(dependency, available_by_id, visiting, visited, ordered) {
            return false;
        }
    }
    visiting.remove(&skill.manifest.id);
    visited.insert(skill.manifest.id.clone());
    ordered.push(skill);
    true
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

    for keyword in &skill.manifest.keywords {
        if prompt_contains_phrase(prompt_lower, keyword) {
            score += 30;
        }
    }

    for example in &skill.manifest.examples {
        let matching_words = example
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| word.len() >= 4)
            .filter(|word| prompt_contains_phrase(prompt_lower, word))
            .count();
        score += (matching_words as u32).saturating_mul(4);
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

fn prompt_contains_phrase(prompt_lower: &str, phrase: &str) -> bool {
    let phrase = phrase.trim().to_ascii_lowercase();
    !phrase.is_empty()
        && prompt_lower
            .split(|character: char| !character.is_alphanumeric())
            .any(|word| word == phrase)
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
        let (manifest, expected_skill_id) = match std::env::consts::OS {
            "windows" => (
                include_str!(
                    "../../../fixtures/skill-registry/skills/shell.powershell.safe/skill.toml"
                ),
                "shell.powershell.safe",
            ),
            "macos" => (
                include_str!("../../../fixtures/skill-registry/skills/shell.zsh.safe/skill.toml"),
                "shell.zsh.safe",
            ),
            _ => (
                include_str!("../../../fixtures/skill-registry/skills/shell.bash.safe/skill.toml"),
                "shell.bash.safe",
            ),
        };
        let skills = vec![installed_skill(manifest)];

        let cards = select_relevant_skills("run this command in terminal", &skills, 5);

        assert_eq!(
            cards.first().map(|card| card.id.as_str()),
            Some(expected_skill_id)
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

    #[test]
    fn manifest_keywords_select_skills_without_hardcoded_intent_candidates() {
        let mut skill = installed_skill(include_str!(
            "../../../fixtures/skill-registry/skills/file.read/skill.toml"
        ));
        skill.manifest.keywords = vec!["blueprint".to_string()];

        let cards = select_relevant_skills("explain this blueprint", &[skill], 5);

        assert_eq!(
            cards.first().map(|card| card.id.as_str()),
            Some("file.read")
        );
    }

    #[test]
    fn card_budget_truncates_selected_context() {
        let mut first = installed_skill(include_str!(
            "../../../fixtures/skill-registry/skills/file.read/skill.toml"
        ));
        first.manifest.keywords = vec!["blueprint".to_string()];
        first
            .manifest
            .llm_card
            .as_mut()
            .expect("fixture card")
            .token_budget = 700;
        let mut second = installed_skill(include_str!(
            "../../../fixtures/skill-registry/skills/file.write/skill.toml"
        ));
        second.manifest.keywords = vec!["blueprint".to_string()];
        second
            .manifest
            .llm_card
            .as_mut()
            .expect("fixture card")
            .token_budget = 700;

        let cards = select_relevant_skills_with_budget("blueprint", &[first, second], 5, 1_000);

        assert_eq!(cards.len(), 1);
    }

    #[test]
    fn co_selects_dependencies_before_the_matching_skill() {
        let dependency = installed_skill(include_str!(
            "../../../fixtures/skill-registry/skills/file.read/skill.toml"
        ));
        let mut primary = installed_skill(include_str!(
            "../../../fixtures/skill-registry/skills/file.write/skill.toml"
        ));
        primary.manifest.keywords = vec!["blueprint".to_string()];
        primary.manifest.depends_on = vec!["file.read".to_string()];

        let cards = select_relevant_skills("blueprint", &[primary, dependency], 5);

        assert_eq!(
            cards
                .iter()
                .map(|card| card.id.as_str())
                .collect::<Vec<_>>(),
            vec!["file.read", "file.write"]
        );
    }

    #[test]
    fn skips_a_skill_with_missing_or_cyclic_dependencies() {
        let mut primary = installed_skill(include_str!(
            "../../../fixtures/skill-registry/skills/file.write/skill.toml"
        ));
        primary.manifest.keywords = vec!["blueprint".to_string()];
        primary.manifest.depends_on = vec!["file.read".to_string()];
        assert!(select_relevant_skills("blueprint", &[primary.clone()], 5).is_empty());

        let mut dependency = installed_skill(include_str!(
            "../../../fixtures/skill-registry/skills/file.read/skill.toml"
        ));
        dependency.manifest.depends_on = vec!["file.write".to_string()];
        assert!(select_relevant_skills("blueprint", &[primary, dependency], 5).is_empty());
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
