// Command route building, deduplication, and execution.
use std::{collections::BTreeSet, time::Instant};

use lux_core::{
    AppResult, ExtensionActivationCandidate, ExtensionActivationPlan, ExtensionCommandExecution,
    ExtensionCommandExecutionPhase, ExtensionCommandExecutionStatus, ExtensionCommandRoute,
};

use crate::runtime::{ExtensionExportExecution, ExtensionRuntime, execution_failure_reason};

// ---------------------------------------------------------------------------
// Route building
// ---------------------------------------------------------------------------

#[must_use]
pub fn command_routes_for_activation_plan(
    plan: &ExtensionActivationPlan,
) -> Vec<ExtensionCommandRoute> {
    let mut routes = Vec::new();
    for candidate in &plan.candidates {
        routes.extend(
            candidate
                .commands
                .iter()
                .map(|command| ExtensionCommandRoute {
                    id: command.id.clone(),
                    title: command.title.clone(),
                    category: command.category.clone(),
                    handler: command.handler.clone(),
                    extension_id: candidate.id.clone(),
                    extension_name: candidate.name.clone(),
                    extension_version: candidate.version.clone(),
                }),
        );
    }
    routes.sort_by(|l, r| l.id.cmp(&r.id));
    routes
}

/// Validate that all route IDs across all candidates are unique.
/// Called by the public `extension_command_routes*` entry points only.
pub fn validate_unique_command_routes(routes: &[ExtensionCommandRoute]) -> AppResult<()> {
    let mut seen = BTreeSet::new();
    for route in routes {
        if !seen.insert(route.id.as_str()) {
            return Err(lux_core::AppError::Service(format!(
                "duplicate registered extension command id: {}",
                route.id
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Execution
// ---------------------------------------------------------------------------

/// F5 fix: global duplicate-ID validation is moved out of the hot
/// per-command path.  We now look up only the routes relevant to
/// `command_id`, so a conflict between two unrelated extensions does not
/// prevent execution of unrelated commands.
pub fn execute_extension_command_from_plan(
    plan: &ExtensionActivationPlan,
    command_id: &str,
) -> ExtensionCommandExecution {
    let started_at = Instant::now();

    // Build routes matching only this command_id (avoids full route rebuild).
    let matching_routes: Vec<ExtensionCommandRoute> = plan
        .candidates
        .iter()
        .flat_map(|candidate| {
            candidate
                .commands
                .iter()
                .filter(|cmd| cmd.id == command_id)
                .map(|command| ExtensionCommandRoute {
                    id: command.id.clone(),
                    title: command.title.clone(),
                    category: command.category.clone(),
                    handler: command.handler.clone(),
                    extension_id: candidate.id.clone(),
                    extension_name: candidate.name.clone(),
                    extension_version: candidate.version.clone(),
                })
        })
        .collect();

    if matching_routes.is_empty() {
        return failed_command_execution(
            command_id,
            None,
            ExtensionCommandExecutionPhase::Routing,
            format!("extension command is not registered: {command_id}"),
            started_at,
            None,
            None,
        );
    }

    // More than one route for the same command_id means conflicting extensions.
    // Block only this command; all other commands remain executable.
    if matching_routes.len() > 1 {
        let extensions: Vec<_> = matching_routes
            .iter()
            .map(|r| r.extension_id.as_str())
            .collect();
        return failed_command_execution(
            command_id,
            None,
            ExtensionCommandExecutionPhase::Routing,
            format!(
                "command id '{command_id}' is registered by multiple extensions: {}; \
                 resolve the conflict by disabling one of them",
                extensions.join(", ")
            ),
            started_at,
            None,
            None,
        );
    }

    let route = matching_routes.into_iter().next().expect("checked above");
    let Some(candidate) = plan
        .candidates
        .iter()
        .find(|c| c.id == route.extension_id)
    else {
        return failed_command_execution(
            command_id,
            Some(route),
            ExtensionCommandExecutionPhase::Routing,
            format!("extension command route lost activation candidate: {command_id}"),
            started_at,
            None,
            None,
        );
    };

    execute_extension_command_candidate(candidate, command_id, route, started_at)
}

fn execute_extension_command_candidate(
    candidate: &ExtensionActivationCandidate,
    command_id: &str,
    route: ExtensionCommandRoute,
    started_at: Instant,
) -> ExtensionCommandExecution {
    let mut runtime = match ExtensionRuntime::instantiate(candidate) {
        Ok(rt) => rt,
        Err(failure) => {
            return failed_command_execution(
                command_id,
                Some(route),
                ExtensionCommandExecutionPhase::Activation,
                execution_failure_reason(
                    &failure.error,
                    ExtensionCommandExecutionPhase::Activation,
                ),
                started_at,
                failure.execution,
                None,
            );
        }
    };

    let activation_execution =
        match runtime.call_activation(&candidate.host_contract.abi.entrypoint) {
            Ok(ex) => ex,
            Err(failure) => {
                return failed_command_execution(
                    command_id,
                    Some(route),
                    ExtensionCommandExecutionPhase::Activation,
                    execution_failure_reason(
                        &failure.error,
                        ExtensionCommandExecutionPhase::Activation,
                    ),
                    started_at,
                    failure.execution,
                    None,
                );
            }
        };

    // F8: call_handler refuels the store with EXTENSION_HOST_COMMAND_FUEL
    // before invoking the handler so handler budget is independent of
    // activation cost.
    match runtime.call_handler(&route.handler) {
        Ok(handler_execution) => succeeded_command_execution(
            command_id,
            route,
            activation_execution,
            handler_execution,
            started_at,
        ),
        Err(failure) => failed_command_execution(
            command_id,
            Some(route),
            ExtensionCommandExecutionPhase::Handler,
            execution_failure_reason(&failure.error, ExtensionCommandExecutionPhase::Handler),
            started_at,
            Some(activation_execution),
            failure.execution,
        ),
    }
}

// ---------------------------------------------------------------------------
// Result builders
// ---------------------------------------------------------------------------

fn succeeded_command_execution(
    command_id: &str,
    route: ExtensionCommandRoute,
    activation_execution: ExtensionExportExecution,
    handler_execution: ExtensionExportExecution,
    started_at: Instant,
) -> ExtensionCommandExecution {
    ExtensionCommandExecution {
        command_id: command_id.to_owned(),
        route: Some(route),
        status: ExtensionCommandExecutionStatus::Succeeded,
        phase: ExtensionCommandExecutionPhase::Handler,
        reason: None,
        duration_ms: elapsed_ms(started_at),
        activation_fuel_consumed: Some(activation_execution.fuel_consumed),
        activation_fuel_remaining: Some(activation_execution.fuel_remaining),
        handler_fuel_consumed: Some(handler_execution.fuel_consumed),
        handler_fuel_remaining: Some(handler_execution.fuel_remaining),
    }
}

fn failed_command_execution(
    command_id: &str,
    route: Option<ExtensionCommandRoute>,
    phase: ExtensionCommandExecutionPhase,
    reason: String,
    started_at: Instant,
    activation_execution: Option<ExtensionExportExecution>,
    handler_execution: Option<ExtensionExportExecution>,
) -> ExtensionCommandExecution {
    ExtensionCommandExecution {
        command_id: command_id.to_owned(),
        route,
        status: ExtensionCommandExecutionStatus::Failed,
        phase,
        reason: Some(reason),
        duration_ms: elapsed_ms(started_at),
        activation_fuel_consumed: activation_execution.as_ref().map(|e| e.fuel_consumed),
        activation_fuel_remaining: activation_execution.as_ref().map(|e| e.fuel_remaining),
        handler_fuel_consumed: handler_execution.as_ref().map(|e| e.fuel_consumed),
        handler_fuel_remaining: handler_execution.as_ref().map(|e| e.fuel_remaining),
    }
}

fn elapsed_ms(started_at: Instant) -> u64 {
    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX)
}
