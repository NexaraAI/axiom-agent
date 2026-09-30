#[derive(Debug, Clone, PartialEq)]
pub struct AgentCaps {
    pub max_iterations: u32,
    pub max_tool_iterations: u32,
    pub max_tokens: u32,
    pub max_cost_usd: f64,
    pub max_wall_seconds: u64,
    pub max_consecutive_tool_errors: u32,
}

/// Characters of a single tool result that may be folded into the model's
/// context.
///
/// Every stored observation is resent with every later model call in the
/// turn, and then again in every later turn, so an uncapped result is
/// multiplied by both. A `github.search` against a 15-repo organization
/// returns 15 full GitHub API objects at 83 fields each, about 110 kB, and
/// roughly 89% of that is boilerplate like `archive_url` and
/// `assignees_url`. Uncapped, one such call turned a 1 kB answer into
/// 79 kB of prompt tokens.
///
/// The budget bounds cost, not reach: the full result is still written to the
/// session's `outputs/` directory and remains viewable with `!show`, and the
/// truncation note tells the model how to narrow the next call.
pub const TOOL_RESULT_BUDGET_CHARS: usize = 8_000;

/// Hook output is advisory context for the next iteration, so it gets a much
/// smaller allowance than a tool result.
pub const HOOK_OUTPUT_BUDGET_CHARS: usize = 400;

impl Default for AgentCaps {
    fn default() -> Self {
        Self {
            max_iterations: 12,
            max_tool_iterations: 20,
            max_tokens: 200_000,
            max_cost_usd: 1.0,
            max_wall_seconds: 1800,
            max_consecutive_tool_errors: 3,
        }
    }
}
