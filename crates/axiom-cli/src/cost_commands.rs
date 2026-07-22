use std::path::{Path, PathBuf};

use anyhow::Result;
use axiom_core::{current_utc_month, usd_to_microusd, AxiomConfig, CostLedgerStore};

pub(crate) fn run() -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    let config = AxiomConfig::load_from_path(&config_path)?;
    let store = CostLedgerStore::new(cost_ledger_path(&config_path));
    let ledger = store.load()?;
    let month = current_utc_month();
    let month_spend = ledger.month_total_microusd(&month);

    println!("Axiom cost ledger");
    println!("month: {month}");
    println!("spent this month: {}", format_microusd(month_spend));
    match config.agent.monthly_budget_usd.and_then(usd_to_microusd) {
        Some(budget) => println!(
            "monthly budget: {} ({} remaining)",
            format_microusd(budget),
            format_microusd(budget.saturating_sub(month_spend))
        ),
        None => println!("monthly budget: not configured"),
    }
    match config.agent.session_budget_usd.and_then(usd_to_microusd) {
        Some(budget) => println!("per-session budget: {}", format_microusd(budget)),
        None => println!("per-session budget: not configured"),
    }

    let pricing_known = config.agent.input_cost_per_million_tokens.is_some()
        && config.agent.output_cost_per_million_tokens.is_some();
    if pricing_known {
        println!("pricing: configured; persistent budgets are enforceable");
    } else {
        println!(
            "pricing: unavailable; cost budget enforcement and new cost recording are unavailable"
        );
    }

    let sessions = ledger.session_totals_for_month(&month);
    if sessions.is_empty() {
        println!("sessions this month: none recorded");
    } else {
        println!("sessions this month:");
        for (session_id, total) in sessions {
            println!("  {session_id}: {}", format_microusd(total));
        }
    }
    println!("ledger: {}", store.path().display());
    Ok(())
}

pub(crate) fn cost_ledger_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("cost-ledger.json")
}

fn format_microusd(value: u64) -> String {
    format!("${:.6}", value as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_path_is_scoped_to_the_config_directory() {
        assert_eq!(
            cost_ledger_path(Path::new("state/config.toml")),
            PathBuf::from("state/cost-ledger.json")
        );
        assert_eq!(format_microusd(1_250_000), "$1.250000");
    }
}
