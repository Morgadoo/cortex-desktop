//! What an agent spent this month, and the budget that stops it.

use super::*;

/// What an agent's conversations cost this calendar month.
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
#[cfg_attr(
    feature = "typescript",
    derive(ts_rs::TS),
    ts(export, rename = "AgentSpend")
)]
pub struct Spend {
    pub runs: usize,
    #[cfg_attr(feature = "typescript", ts(type = "number"))]
    pub tokens: u64,
    pub cost: f64,
}

pub fn spend_this_month(config_dir: &Path) -> std::collections::HashMap<String, Spend> {
    use chrono::Datelike;
    let now = Utc::now();
    let mut out: std::collections::HashMap<String, Spend> = Default::default();
    for c in crate::chat::store::list_conversations(config_dir) {
        let Some(agent) = c.agent_id.clone() else {
            continue;
        };
        if c.updated_at.year() != now.year() || c.updated_at.month() != now.month() {
            continue;
        }
        let entry = out.entry(agent).or_default();
        entry.runs += 1;
        entry.tokens += c.tokens;
        entry.cost += c.cost;
    }
    out
}

/// Refuses a turn when the agent's monthly budget is used up.
pub fn check_budget(config_dir: &Path, agent: &Agent) -> Result<(), String> {
    let Some(budget) = agent.monthly_budget.filter(|b| *b > 0.0) else {
        return Ok(());
    };
    // The trace database knows each run's cost; before any run is traced this month, the
    // conversations are added up as before.
    let month_start = {
        use chrono::{Datelike, TimeZone};
        let now = Utc::now();
        Utc.with_ymd_and_hms(now.year(), now.month(), 1, 0, 0, 0)
            .single()
            .unwrap_or(now)
    };
    let spent =
        crate::chat::traces::agent_cost(config_dir, &agent.id, month_start).unwrap_or_else(|| {
            spend_this_month(config_dir)
                .get(&agent.id)
                .map_or(0.0, |s| s.cost)
        });
    if spent >= budget {
        return Err(format!(
            "{} has used its monthly budget (${spent:.2} of ${budget:.2}). Raise the budget on its page to continue.",
            agent.name
        ));
    }
    Ok(())
}
