use axiom_llm::TokenUsage;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct UsagePricing {
    pub input_usd_per_million_tokens: Option<f64>,
    pub output_usd_per_million_tokens: Option<f64>,
}

impl UsagePricing {
    pub fn new(
        input_usd_per_million_tokens: Option<f64>,
        output_usd_per_million_tokens: Option<f64>,
    ) -> Self {
        Self {
            input_usd_per_million_tokens: valid_rate(input_usd_per_million_tokens),
            output_usd_per_million_tokens: valid_rate(output_usd_per_million_tokens),
        }
    }

    pub fn is_complete(self) -> bool {
        self.input_usd_per_million_tokens.is_some() && self.output_usd_per_million_tokens.is_some()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageLedger {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

impl UsageLedger {
    pub fn record(&mut self, usage: Option<&TokenUsage>) {
        let Some(usage) = usage else {
            return;
        };

        self.prompt_tokens = self
            .prompt_tokens
            .saturating_add(u64::from(usage.prompt_tokens));
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(u64::from(usage.completion_tokens));
        self.total_tokens = self
            .total_tokens
            .saturating_add(u64::from(usage.total_tokens));
    }

    pub fn merge(&mut self, other: &Self) {
        self.prompt_tokens = self.prompt_tokens.saturating_add(other.prompt_tokens);
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(other.completion_tokens);
        self.total_tokens = self.total_tokens.saturating_add(other.total_tokens);
    }

    /// Returns an estimated cost in micro-US-dollars. Both rates must be
    /// configured so a partial price is never presented as a complete cost.
    pub fn estimated_cost_microusd(&self, pricing: UsagePricing) -> Option<u64> {
        let input_rate = pricing.input_usd_per_million_tokens?;
        let output_rate = pricing.output_usd_per_million_tokens?;
        let micro_usd =
            self.prompt_tokens as f64 * input_rate + self.completion_tokens as f64 * output_rate;
        if !micro_usd.is_finite() || micro_usd < 0.0 {
            return None;
        }
        Some(micro_usd.round().min(u64::MAX as f64) as u64)
    }
}

fn valid_rate(rate: Option<f64>) -> Option<f64> {
    rate.filter(|value| value.is_finite() && *value >= 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_accumulates_provider_usage() {
        let mut ledger = UsageLedger::default();
        ledger.record(Some(&TokenUsage {
            prompt_tokens: 3,
            completion_tokens: 5,
            total_tokens: 8,
        }));

        assert_eq!(ledger.total_tokens, 8);
        assert_eq!(ledger.prompt_tokens, 3);
        assert_eq!(ledger.completion_tokens, 5);
    }

    #[test]
    fn ledger_merges_and_estimates_cost_when_both_rates_are_known() {
        let mut session = UsageLedger {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
        };
        session.merge(&UsageLedger {
            prompt_tokens: 50,
            completion_tokens: 10,
            total_tokens: 60,
        });

        let cost = session.estimated_cost_microusd(UsagePricing::new(Some(2.0), Some(8.0)));

        assert_eq!(session.total_tokens, 180);
        assert_eq!(cost, Some(540));
    }

    #[test]
    fn cost_is_unknown_when_pricing_is_partial_or_invalid() {
        let ledger = UsageLedger {
            prompt_tokens: 100,
            completion_tokens: 20,
            total_tokens: 120,
        };

        assert_eq!(
            ledger.estimated_cost_microusd(UsagePricing::new(Some(2.0), None)),
            None
        );
        assert!(!UsagePricing::new(Some(f64::NAN), Some(1.0)).is_complete());
    }
}
