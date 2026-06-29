// Extension activation: drives the plan → report pipeline (runs each
// candidate's WASM init export and collects results).
use lux_core::{
    ExtensionActivated, ExtensionActivationFailed, ExtensionActivationPlan,
    ExtensionActivationReport,
};

use crate::runtime::{activation_failure_reason, ExtensionRuntime};

pub fn activate_extension_plan(
    plan: ExtensionActivationPlan,
) -> ExtensionActivationReport {
    let mut activated = Vec::new();
    let mut failed = Vec::new();

    for candidate in &plan.candidates {
        match activate_extension_candidate(candidate) {
            Ok(result) => activated.push(result),
            Err(error) => failed.push(ExtensionActivationFailed {
                id: candidate.id.clone(),
                name: candidate.name.clone(),
                version: candidate.version.clone(),
                root: candidate.root.clone(),
                wasm_module: candidate.wasm_module.clone(),
                reason: activation_failure_reason(&error),
            }),
        }
    }

    activated.sort_by(|l, r| l.id.cmp(&r.id));
    failed.sort_by(|l, r| l.id.cmp(&r.id));
    ExtensionActivationReport {
        plan,
        activated,
        failed,
    }
}

fn activate_extension_candidate(
    candidate: &lux_core::ExtensionActivationCandidate,
) -> Result<ExtensionActivated, lux_core::AppError> {
    let mut runtime = ExtensionRuntime::instantiate(candidate)
        .map_err(|failure| failure.error)?;
    let execution = runtime
        .call_activation(&candidate.host_contract.abi.entrypoint)
        .map_err(|failure| failure.error)?;
    Ok(ExtensionActivated {
        id: candidate.id.clone(),
        name: candidate.name.clone(),
        version: candidate.version.clone(),
        root: candidate.root.clone(),
        wasm_module: candidate.wasm_module.clone(),
        fuel_consumed: execution.fuel_consumed,
        fuel_remaining: execution.fuel_remaining,
    })
}
