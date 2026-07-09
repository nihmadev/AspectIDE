pub fn segment_targets_root(normalized: &str) -> bool {
    normalized
        .split(' ')
        .any(|token| matches!(token, "/" | "/*"))
}

/// Extract the inner shell payload from `-c '…'` or `-Command '…'` style
/// interpreter invocations.
pub fn extract_interpreter_payload(normalized: &str) -> Option<String> {
    let mut arg_start = None;
    let mut cursor = 0usize;
    for token in normalized.split(' ') {
        let token_end = cursor + token.len();
        if matches!(token, "-c" | "-command" | "/c" | "/k" | "/command") {
            let mut idx = token_end;
            let bytes = normalized.as_bytes();
            while idx < bytes.len() && bytes[idx] == b' ' {
                idx += 1;
            }
            arg_start = Some(idx);
            break;
        }
        cursor = token_end + 1;
    }
    let start = arg_start?;
    let rest = normalized.get(start..)?.trim_start();
    if rest.is_empty() {
        return None;
    }

    let first = rest.as_bytes()[0];
    let payload = if first == b'\'' || first == b'"' {
        let quote = first as char;
        rest[1..]
            .find(quote)
            .map_or(&rest[1..], |close| &rest[1..=close])
    } else {
        rest
    };

    let payload = payload.trim();
    (!payload.is_empty()).then(|| payload.to_string())
}
