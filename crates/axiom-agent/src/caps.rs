#[derive(Debug, Clone, PartialEq)]
pub struct AgentCaps {
    pub max_iterations: u32,
    pub max_tool_iterations: u32,
    pub max_tokens: u32,
    pub max_cost_usd: f64,
    pub max_wall_seconds: u64,
    pub max_consecutive_tool_errors: u32,
}

impl Default for AgentCaps {
    fn default() -> Self {
        Self {
            max_iterations: 12,
            max_tool_iterations: 20,
            max_tokens: 200_000,
            max_cost_usd: 1.0,
            max_wall_seconds: 300,
            max_consecutive_tool_errors: 3,
        }
    }
}
