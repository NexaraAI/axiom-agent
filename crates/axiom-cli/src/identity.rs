/// Builds the system context that defines Axiom independently from the skills
/// selected for an individual request.
pub(crate) fn system_message(agent_name: &str, installed_skill_ids: &[String]) -> String {
    let mut message = format!(
        "You are {agent_name}, a terminal-first coding and automation agent.\n\
Your identity is Axiom Agent; installed skills are capabilities, not the sum of your identity.\n\
Answer questions about who you are, what you can do, and how to use Axiom directly without requesting a tool.\n\
Do not claim a capability is installed when it is not listed below, and do not invoke a tool unless it is needed for the user's request.\n\n\
Installed and currently available skill IDs:\n"
    );

    if installed_skill_ids.is_empty() {
        message.push_str("- none\n");
    } else {
        for (index, skill_id) in installed_skill_ids.iter().enumerate() {
            message.push_str(&format!("{}. {skill_id}\n", index + 1));
        }
    }

    message
}

#[cfg(test)]
mod tests {
    use super::system_message;

    #[test]
    fn identity_message_names_axiom_and_all_available_skills() {
        let message = system_message(
            "Axiom Agent",
            &["file.read".to_string(), "git.status".to_string()],
        );

        assert!(message.contains("You are Axiom Agent"));
        assert!(message.contains("1. file.read"));
        assert!(message.contains("2. git.status"));
        assert!(message.contains("without requesting a tool"));
    }

    #[test]
    fn identity_message_handles_an_empty_skill_set() {
        let message = system_message("Axiom Agent", &[]);

        assert!(message.contains("- none"));
    }
}
