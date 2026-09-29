use axiom_engine::ExternalToolSource;

use crate::UsageLedger;

use super::{native_tool_name, AgentLoop};

impl<'a> AgentLoop<'a> {
    pub(super) fn cost_limit_reached(&self, ledger: &UsageLedger) -> bool {
        let max_cost = self.caps.max_cost_usd;
        if !max_cost.is_finite() || max_cost < 0.0 {
            return false;
        }
        let Some(cost_microusd) = ledger.estimated_cost_microusd(self.pricing) else {
            return false;
        };
        let cap_microusd = (max_cost * 1_000_000.0).round().clamp(0.0, u64::MAX as f64) as u64;
        cost_microusd >= cap_microusd
    }

    pub(super) fn skill_id_for_native_tool(&self, name: &str) -> Option<String> {
        let trimmed = name.trim();
        // Some models emit their internal recipient format instead of the
        // bare wire name (for example GLM's `functions.axiom_github_search`).
        // Strip the namespace wrapper before matching.
        let cleaned = trimmed
            .strip_prefix("functions.")
            .or_else(|| trimmed.strip_prefix("tools."))
            .unwrap_or(trimmed);
        let unprefix = cleaned
            .strip_prefix("axiom_")
            .or_else(|| cleaned.strip_prefix("axiom."))
            .unwrap_or(cleaned);

        if let Some(skill_id) = self
            .installed_skills
            .iter()
            .map(|skill| skill.manifest.id.as_str())
            .find(|skill_id| {
                *skill_id == cleaned
                    || native_tool_name(skill_id) == cleaned
                    || *skill_id == unprefix
                    || skill_id.replace('.', "_") == cleaned
                    || skill_id.replace('.', "_") == unprefix
            })
        {
            return Some(skill_id.to_string());
        }

        if let Some(definition) = self
            .external_tools
            .into_iter()
            .flat_map(ExternalToolSource::definitions)
            .find(|definition| {
                definition.id == cleaned
                    || native_tool_name(&definition.id) == cleaned
                    || definition.id == unprefix
                    || definition.id.replace('.', "_") == cleaned
                    || definition.id.replace('.', "_") == unprefix
            })
        {
            return Some(definition.id);
        }

        const CORE_BUILTIN_IDS: &[&str] = &[
            "file.read",
            "file.write",
            "project.scan",
            "web.fetch",
            "shell.powershell.safe",
            "shell.bash.safe",
            "shell.zsh.safe",
            "shell.run",
            "python.run",
            "git.status",
            "git.diff",
            "skill.create",
            "question.ask",
        ];
        CORE_BUILTIN_IDS
            .iter()
            .copied()
            .find(|builtin| {
                *builtin == cleaned
                    || *builtin == unprefix
                    || native_tool_name(builtin) == cleaned
                    || builtin.replace('.', "_") == cleaned
                    || builtin.replace('.', "_") == unprefix
            })
            .map(ToString::to_string)
    }
}
