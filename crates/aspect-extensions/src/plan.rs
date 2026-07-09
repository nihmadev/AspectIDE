// Activation planning: converts discovered ExtensionInfo list into a validated
// ExtensionActivationPlan (candidates + blocked entries).
use aspect_core::{
    ExtensionActivationCandidate, ExtensionActivationPlan, ExtensionContributionKind,
    ExtensionInfo, ExtensionStatus,
};

use crate::{
    discovery::blocked_extension,
    wasm_preflight::{validate_wasm_host_contract, validate_wasm_preflight},
};

#[must_use]
pub fn build_activation_plan(extensions: Vec<ExtensionInfo>) -> ExtensionActivationPlan {
    let mut candidates = Vec::new();
    let mut blocked = Vec::new();

    for extension in extensions {
        if extension.status != ExtensionStatus::Discovered {
            blocked.push(blocked_extension(
                &extension,
                extension
                    .error
                    .as_deref()
                    .unwrap_or("extension manifest is not valid"),
            ));
            continue;
        }

        let unknown_contributions = extension
            .contribution_points
            .iter()
            .filter(|p| p.kind == ExtensionContributionKind::Unknown)
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>();
        if !unknown_contributions.is_empty() {
            blocked.push(blocked_extension(
                &extension,
                &format!(
                    "unsupported contribution points: {}",
                    unknown_contributions.join(", ")
                ),
            ));
            continue;
        }

        let wasm_preflight = match validate_wasm_preflight(&extension) {
            Ok(p) => p,
            Err(e) => {
                blocked.push(blocked_extension(&extension, &e.to_string()));
                continue;
            }
        };
        let host_contract = match validate_wasm_host_contract(&extension, &wasm_preflight) {
            Ok(c) => c,
            Err(e) => {
                blocked.push(blocked_extension(&extension, &e.to_string()));
                continue;
            }
        };

        candidates.push(ExtensionActivationCandidate {
            id: extension.id,
            name: extension.name,
            version: extension.version,
            root: extension.root,
            manifest_path: extension.manifest_path,
            wasm_module: extension.wasm_module,
            contribution_points: extension.contribution_points,
            commands: extension.commands,
            wasm_preflight,
            host_contract,
        });
    }

    candidates.sort_by(|l, r| l.id.cmp(&r.id));
    blocked.sort_by(|l, r| l.id.cmp(&r.id));
    ExtensionActivationPlan {
        candidates,
        blocked,
    }
}
