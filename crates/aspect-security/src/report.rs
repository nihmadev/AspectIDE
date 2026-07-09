use crate::{
    catastrophic::whole_command_catastrophic_rm,
    catastrophic::whole_command_catastrophic_windows, normalize::{normalize, squeezed},
    read_only::is_read_only_segment, risky::risky_warnings,
    splitter::{extract_substitutions, split_segments},
};

use crate::catastrophic::catastrophic_reason;

/// Result of classifying a shell command line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellSafetyReport {
    /// Set when the command is catastrophic and must not run. Human-readable.
    pub blocked: Option<String>,
    /// Non-fatal risk notices surfaced to the model alongside the result.
    pub warnings: Vec<String>,
    /// True when every segment is a known read-only inspection command.
    pub read_only: bool,
}

/// Classify a full command line (may contain `;`, `&&`, `||`, `|`, newlines).
#[must_use]
pub fn classify_shell_command(command: &str) -> ShellSafetyReport {
    let full = normalize(command);
    let full_squeezed = squeezed(&full);
    let mut report = ShellSafetyReport::default();

    if full_squeezed.contains(":(){:|:&};:") || full_squeezed.contains(":(){:|:&};") {
        report.blocked = Some("fork bomb detected".to_string());
        return report;
    }

    if let Some(reason) = whole_command_catastrophic_rm(&full) {
        report.blocked = Some(reason);
        return report;
    }

    if let Some(reason) = whole_command_catastrophic_windows(&full) {
        report.blocked = Some(reason);
        return report;
    }

    let segments = split_segments(command);
    report.read_only = !segments.is_empty();

    let substitution_bodies = extract_substitutions(command);
    if !substitution_bodies.is_empty() {
        report.read_only = false;
    }
    let inner_segments: Vec<String> = substitution_bodies
        .iter()
        .flat_map(|body| split_segments(body))
        .collect();

    if (full.contains("curl ") || full.contains("wget "))
        && (full.contains("| sh")
            || full.contains("|sh")
            || full.contains("| bash")
            || full.contains("|bash"))
    {
        report
            .warnings
            .push("piping a download straight into a shell executes remote code".to_string());
    }

    for segment in segments.iter().chain(inner_segments.iter()) {
        let normalized = normalize(segment);
        if normalized.is_empty() {
            continue;
        }

        if let Some(reason) = catastrophic_reason(&normalized) {
            report.blocked = Some(reason);
            report.read_only = false;
            report.warnings.clear();
            return report;
        }

        for warning in risky_warnings(&normalized) {
            if !report.warnings.contains(&warning) {
                report.warnings.push(warning);
            }
        }

        if !is_read_only_segment(&normalized) {
            report.read_only = false;
        }
    }

    report
}
