/// Split a command line into independently-executed segments.
pub fn split_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let bytes = command.as_bytes();
    let mut index = 0;
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;

    while index < bytes.len() {
        let ch = bytes[index] as char;

        if escaped {
            escaped = false;
            current.push(ch);
            index += 1;
            continue;
        }

        match ch {
            '\\' if !in_single => {
                escaped = true;
                current.push(ch);
            }
            '\'' if !in_double && !cfg!(windows) => {
                in_single = !in_single;
                current.push(ch);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(ch);
            }
            _ if in_single || in_double => current.push(ch),
            '\n' => {
                segments.push(std::mem::take(&mut current));
            }
            ';' | '|' | '&' => {
                segments.push(std::mem::take(&mut current));
                while index + 1 < bytes.len() {
                    let next = bytes[index + 1] as char;
                    if next == '|' || next == '&' {
                        index += 1;
                    } else {
                        break;
                    }
                }
            }
            _ => current.push(ch),
        }
        index += 1;
    }

    if in_single || in_double || escaped {
        return vec![command.trim().to_string()];
    }

    if !current.trim().is_empty() {
        segments.push(current);
    }
    segments
        .into_iter()
        .map(|segment| segment.trim().to_string())
        .filter(|segment| !segment.is_empty())
        .collect()
}

/// Pull out the bodies of every command/process substitution.
pub fn extract_substitutions(command: &str) -> Vec<String> {
    let mut bodies = Vec::new();
    collect_substitutions(command, 0, &mut bodies);
    bodies
}

fn collect_substitutions(command: &str, depth: usize, out: &mut Vec<String>) {
    const MAX_DEPTH: usize = 8;
    if depth > MAX_DEPTH {
        return;
    }
    let bytes = command.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'`' => {
                if let Some(rel) = command[index + 1..].find('`') {
                    let body = &command[index + 1..index + 1 + rel];
                    out.push(body.to_string());
                    collect_substitutions(body, depth + 1, out);
                    index += 1 + rel + 1;
                    continue;
                }
            }
            b'$' | b'<' | b'>' if bytes.get(index + 1) == Some(&b'(') => {
                if let Some((body, end)) = capture_parenthesized(command, index + 2) {
                    out.push(body.to_string());
                    collect_substitutions(body, depth + 1, out);
                    index = end;
                    continue;
                }
            }
            _ => {}
        }
        index += 1;
    }
}

fn capture_parenthesized(command: &str, start: usize) -> Option<(&str, usize)> {
    let bytes = command.as_bytes();
    let mut depth = 1usize;
    let mut index = start;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((&command[start..index], index + 1));
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}
