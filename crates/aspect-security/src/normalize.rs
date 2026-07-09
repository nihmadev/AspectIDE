/// Lowercase + whitespace-collapsed copy for matching.
pub fn normalize(segment: &str) -> String {
    segment
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A whitespace-free copy for fork-bomb / glued-token detection.
pub fn squeezed(normalized: &str) -> String {
    normalized.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Return the effective command token after stripping known launcher prefixes.
pub fn first_token(normalized: &str) -> &str {
    let mut rest = normalized.trim();

    for _ in 0..4 {
        let mut tokens = rest.splitn(2, ' ');
        let head = tokens
            .next()
            .unwrap_or("")
            .trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
        let tail = tokens.next().unwrap_or("").trim_start();

        match head {
            "sudo" | "doas" | "command" | "exec" => {
                rest = skip_flags(tail);
            }
            "env" => {
                rest = skip_env_assignments(tail);
            }
            "time" | "call" => {
                rest = tail;
            }
            "cmd" => {
                let next = skip_cmd_switches(tail);
                if next.starts_with(['"', '\'']) {
                    break;
                }
                rest = next;
            }
            "start" => {
                let next = skip_start_args(tail);
                if next.starts_with(['"', '\'']) {
                    break;
                }
                rest = next;
            }
            _ => break,
        }
        if rest.is_empty() {
            break;
        }
    }

    rest.split(' ').next().unwrap_or("")
}

use crate::launcher::{skip_cmd_switches, skip_env_assignments, skip_flags, skip_start_args};

/// Strip surrounding quotes, collapse a trailing slash, and fold the home
/// directory env-vars to `~` so quoted / decorated forms compare equal to the
/// bare dangerous targets.
#[allow(clippy::literal_string_with_formatting_args)]
pub fn normalize_path_operand(token: &str) -> String {
    let mut path = token.trim_matches(|c| c == '"' || c == '\'').to_string();
    for home in ["$home", "${home}", "%userprofile%", "$env:userprofile"] {
        if path == home {
            path = "~".to_string();
            break;
        }
        let glob = format!("{home}/*");
        if path == glob {
            path = "~/*".to_string();
            break;
        }
        let bslash_glob = format!("{home}\\*");
        if path == bslash_glob {
            path = "~/*".to_string();
            break;
        }
    }
    if path.len() > 1 {
        if let Some(stripped) = path.strip_suffix('/') {
            path = stripped.to_string();
        }
    }
    path
}
