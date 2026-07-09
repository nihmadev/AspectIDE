use crate::normalize::first_token;

/// Risky-but-allowed commands worth flagging back to the model.
pub fn risky_warnings(normalized: &str) -> Vec<String> {
    let mut warnings = Vec::new();
    let ft = first_token(normalized);

    if normalized.starts_with("sudo ") || normalized.starts_with("doas ") {
        warnings.push("runs with elevated privileges (sudo/doas)".to_string());
    }
    if ft == "git" {
        if normalized.contains("push")
            && (normalized.contains("--force") || normalized.contains(" -f"))
        {
            warnings.push("git force-push can overwrite remote history".to_string());
        }
        if normalized.contains("reset --hard") {
            warnings.push("git reset --hard discards uncommitted changes".to_string());
        }
        if normalized.contains("clean ") && normalized.contains("-f") {
            warnings.push("git clean -f deletes untracked files".to_string());
        }
    }
    if ft == "chmod" && normalized.contains("777") {
        warnings.push("chmod 777 grants world-writable permissions".to_string());
    }
    if normalized.contains("publish") && matches!(ft, "npm" | "cargo" | "yarn" | "pnpm") {
        warnings.push("publishes a package to a public registry".to_string());
    }
    warnings
}
