// Contribution registry: maps activated extensions to their contribution
// points, and collects unavailable contributions from failed/blocked ones.
use aspect_core::{
    ExtensionActivationReport, ExtensionContributionRegistration, ExtensionContributionRegistry,
    ExtensionContributionUnavailable,
};

#[must_use]
pub fn build_contribution_registry(
    activation: ExtensionActivationReport,
) -> ExtensionContributionRegistry {
    let mut registered = Vec::new();
    let mut unavailable = Vec::new();

    for activated in &activation.activated {
        if let Some(candidate) = activation
            .plan
            .candidates
            .iter()
            .find(|c| c.id == activated.id)
        {
            registered.extend(
                candidate
                    .contribution_points
                    .iter()
                    .cloned()
                    .map(|contribution| ExtensionContributionRegistration {
                        extension_id: candidate.id.clone(),
                        extension_name: candidate.name.clone(),
                        extension_version: candidate.version.clone(),
                        contribution,
                    }),
            );
        }
    }

    for failed in &activation.failed {
        if let Some(candidate) = activation
            .plan
            .candidates
            .iter()
            .find(|c| c.id == failed.id)
        {
            unavailable.extend(
                candidate
                    .contribution_points
                    .iter()
                    .cloned()
                    .map(|contribution| ExtensionContributionUnavailable {
                        extension_id: candidate.id.clone(),
                        extension_name: candidate.name.clone(),
                        extension_version: candidate.version.clone(),
                        contribution,
                        reason: failed.reason.clone(),
                    }),
            );
        }
    }

    for blocked in &activation.plan.blocked {
        unavailable.extend(
            blocked
                .contribution_points
                .iter()
                .cloned()
                .map(|contribution| ExtensionContributionUnavailable {
                    extension_id: blocked.id.clone(),
                    extension_name: blocked.name.clone(),
                    extension_version: blocked.version.clone(),
                    contribution,
                    reason: blocked.reason.clone(),
                }),
        );
    }

    registered.sort_by(|l, r| {
        l.contribution
            .id
            .cmp(&r.contribution.id)
            .then_with(|| l.extension_id.cmp(&r.extension_id))
    });
    unavailable.sort_by(|l, r| {
        l.contribution
            .id
            .cmp(&r.contribution.id)
            .then_with(|| l.extension_id.cmp(&r.extension_id))
    });

    ExtensionContributionRegistry {
        activation,
        registered,
        unavailable,
    }
}
