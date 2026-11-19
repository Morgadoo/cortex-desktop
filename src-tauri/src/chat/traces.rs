//! Run traces, feedback and the artifact registry: the `cortex-traces` crate, with what the engine
//! passes in rig's terms turned into plain numbers.

pub use cortex_traces::*;

use std::path::Path;

/// One model call, from rig's usage report.
#[allow(clippy::too_many_arguments)]
pub fn model_call(
    config_dir: &Path,
    run: &str,
    step: usize,
    model: &str,
    millis: u64,
    usage: Option<&rig_core::completion::Usage>,
    error: Option<&str>,
) {
    let spent = usage.map(|u| Spent {
        input_tokens: u.input_tokens.unwrap_or(0),
        output_tokens: u.output_tokens.unwrap_or(0),
        cost: u.cost.map_or(0.0, |c| c.total),
    });
    cortex_traces::model_call(config_dir, run, step, model, millis, spent, error);
}
