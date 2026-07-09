use crate::{
    block_device::{mentions_windows_drive_root, redirects_to_block_device},
    interpreter::{extract_interpreter_payload, segment_targets_root},
    normalize::first_token,
    rm_detect::{
        has_recursive_force_flags, is_dangerous_root_target, is_rm_recursive_force, rm_targets,
    },
};

/// Returns a reason when the command is catastrophic and must be refused.
pub fn catastrophic_reason(normalized: &str) -> Option<String> {
    if normalized.contains("--no-preserve-root") {
        return Some("rm --no-preserve-root targets the entire filesystem".to_string());
    }

    let ft = first_token(normalized);
    if matches!(
        ft,
        "bash" | "sh" | "zsh" | "fish" | "dash" | "ksh" | "csh" | "tcsh"
    ) {
        if let Some(inner) = extract_interpreter_payload(normalized) {
            let inner_norm = crate::normalize::normalize(&inner);
            if let Some(reason) = catastrophic_reason(&inner_norm) {
                return Some(reason);
            }
        }
    }
    if matches!(ft, "python" | "python3" | "ruby" | "perl" | "node") && normalized.contains(" -c ")
    {
        if let Some(inner) = extract_interpreter_payload(normalized) {
            let inner_norm = crate::normalize::normalize(&inner);
            if let Some(reason) = catastrophic_reason(&inner_norm) {
                return Some(reason);
            }
        }
    }
    if matches!(ft, "powershell" | "pwsh") {
        if let Some(inner) = extract_interpreter_payload(normalized) {
            let inner_norm = crate::normalize::normalize(&inner);
            if let Some(reason) = catastrophic_reason(&inner_norm) {
                return Some(reason);
            }
        }
    }
    if matches!(ft, "cmd" | "cmd.exe") {
        if let Some(inner) = extract_interpreter_payload(normalized) {
            let inner_norm = crate::normalize::normalize(&inner);
            if let Some(reason) = catastrophic_reason(&inner_norm) {
                return Some(reason);
            }
        }
    }

    if is_rm_recursive_force(normalized) {
        for target in rm_targets(normalized) {
            if is_dangerous_root_target(&target) {
                return Some(format!(
                    "recursive force delete of a protected path: {target}"
                ));
            }
        }
    }

    if first_token(normalized) == "mkfs" || normalized.starts_with("mkfs.") {
        return Some("mkfs would format a filesystem".to_string());
    }
    if first_token(normalized) == "dd" && crate::block_device::writes_to_block_device(normalized) {
        return Some("dd writes directly to a block device".to_string());
    }
    if redirects_to_block_device(normalized) {
        return Some("redirect targets a raw block device".to_string());
    }

    if (first_token(normalized) == "chmod" || first_token(normalized) == "chown")
        && normalized.contains(" -r")
        && segment_targets_root(normalized)
    {
        return Some(
            "recursive permission/ownership change at filesystem root".to_string(),
        );
    }

    let ft = first_token(normalized);
    if ft == "format" && mentions_windows_drive_root(normalized) {
        return Some("format would erase a Windows drive".to_string());
    }
    if (ft == "del" || ft == "rd" || ft == "rmdir")
        && normalized.contains("/s")
        && mentions_windows_drive_root(normalized)
    {
        return Some("recursive delete of a Windows drive root".to_string());
    }
    if ft == "diskpart" || normalized.starts_with("cipher /w") {
        return Some("low-level disk operation".to_string());
    }

    None
}

/// Position-independent catastrophic `rm` detector for the whole command line.
pub fn whole_command_catastrophic_rm(normalized: &str) -> Option<String> {
    let first = normalized.split(' ').next().unwrap_or("");
    if first == "rm" {
        return None;
    }
    let tokens: Vec<&str> = normalized.split(' ').collect();
    for (i, &token) in tokens.iter().enumerate() {
        if token != "rm" {
            continue;
        }
        let tail = tokens[i..].join(" ");
        if has_recursive_force_flags(&tail) {
            for target in rm_targets(&tail) {
                if is_dangerous_root_target(&target) {
                    return Some(format!(
                        "recursive force delete of a protected path: {target}"
                    ));
                }
            }
        }
    }
    None
}

/// Position-independent catastrophic Windows-verb detector for the whole line.
pub fn whole_command_catastrophic_windows(normalized: &str) -> Option<String> {
    let tokens: Vec<&str> = normalized.split(' ').collect();
    let first = tokens.first().copied().unwrap_or("");

    if first != "diskpart" && tokens.contains(&"diskpart") {
        return Some("low-level disk operation".to_string());
    }

    if first != "format" && tokens.contains(&"format") && mentions_windows_drive_root(normalized) {
        return Some("format would erase a Windows drive".to_string());
    }

    let has_recursive_verb = tokens.iter().any(|&t| matches!(t, "del" | "rd" | "rmdir"));
    let verb_is_leading = matches!(first, "del" | "rd" | "rmdir");
    if has_recursive_verb
        && !verb_is_leading
        && tokens.contains(&"/s")
        && mentions_windows_drive_root(normalized)
    {
        return Some("recursive delete of a Windows drive root".to_string());
    }

    None
}
