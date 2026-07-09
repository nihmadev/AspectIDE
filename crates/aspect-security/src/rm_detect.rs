use crate::normalize::normalize_path_operand;

pub fn is_rm_recursive_force(normalized: &str) -> bool {
    if crate::normalize::first_token(normalized) != "rm" {
        return false;
    }
    has_recursive_force_flags(normalized)
}

pub fn has_recursive_force_flags(normalized: &str) -> bool {
    let has_recursive = normalized.contains(" -r")
        || normalized.contains(" --recursive")
        || flag_cluster_has(normalized, 'r');
    let has_force = normalized.contains(" -f")
        || normalized.contains(" --force")
        || flag_cluster_has(normalized, 'f');
    has_recursive && has_force
}

pub fn flag_cluster_has(normalized: &str, needle: char) -> bool {
    normalized
        .split(' ')
        .any(|token| token.starts_with('-') && !token.starts_with("--") && token.contains(needle))
}

pub fn rm_targets(normalized: &str) -> Vec<String> {
    let mut flags_done = false;
    normalized
        .split(' ')
        .skip(1)
        .filter_map(|token| {
            if !flags_done && token == "--" {
                flags_done = true;
                return None;
            }
            if !flags_done && token.starts_with('-') {
                return None;
            }
            let normalized_target = normalize_path_operand(token);
            (!normalized_target.is_empty()).then_some(normalized_target)
        })
        .collect()
}

#[allow(clippy::literal_string_with_formatting_args)]
pub fn is_dangerous_root_target(target: &str) -> bool {
    for home_root in ["/home", "/users"] {
        if let Some(child) = target
            .strip_prefix(home_root)
            .and_then(|rest| rest.strip_prefix('/'))
        {
            if !child.is_empty() && !child.contains('/') {
                return true;
            }
        }
    }
    matches!(
        target,
        "/" | "/*"
            | "~"
            | "~/*"
            | "$home"
            | "${home}"
            | "$home/*"
            | "${home}/*"
            | "%userprofile%/*"
            | "$env:userprofile/*"
            | "/."
            | "/.*"
            | "/etc" | "/etc/*"
            | "/usr" | "/usr/*"
            | "/bin" | "/bin/*"
            | "/sbin" | "/sbin/*"
            | "/lib" | "/lib/*"
            | "/lib64" | "/lib64/*"
            | "/boot" | "/boot/*"
            | "/var" | "/var/*"
            | "/opt" | "/opt/*"
            | "/sys" | "/sys/*"
            | "/proc" | "/proc/*"
            | "/dev" | "/dev/*"
            | "/root" | "/root/*"
            | "/home" | "/home/*"
            | "/srv" | "/srv/*"
            | "/run" | "/run/*"
    )
}
