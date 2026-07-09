/// Short `sudo`/`doas` flags that take a separate value argument.
pub const SUDO_VALUE_FLAGS: &[char] = &['u', 'g', 'h', 'p', 'C', 'r', 't', 'U', 'c', 'T', 'R'];

/// Skip leading flag tokens (`-n`, `--flag`, `-u root`) from a launcher tail,
/// returning the first non-flag token onward.
pub fn skip_flags(s: &str) -> &str {
    let mut rest = s.trim_start();
    while let Some(token_end) = rest.find(' ') {
        let token = &rest[..token_end];
        if !token.starts_with('-') {
            break;
        }
        rest = rest[token_end + 1..].trim_start();
        let is_value_flag = token.len() == 2
            && !token.starts_with("--")
            && SUDO_VALUE_FLAGS.contains(&(token.as_bytes()[1] as char));
        if is_value_flag {
            if let Some(next_end) = rest.find(' ') {
                let next = &rest[..next_end];
                if !next.starts_with('-') {
                    rest = rest[next_end + 1..].trim_start();
                }
            }
        }
    }
    rest
}

/// Skip `KEY=VALUE` environment variable assignments that prefix `env` payloads.
pub fn skip_env_assignments(s: &str) -> &str {
    let mut rest = s.trim_start();
    rest = skip_flags(rest);
    loop {
        match rest.split_once(' ') {
            Some((token, tail)) if token.contains('=') => rest = tail.trim_start(),
            _ => break,
        }
    }
    rest
}

/// Skip leading `/`-prefixed switch tokens of a `cmd` invocation.
pub fn skip_cmd_switches(s: &str) -> &str {
    let mut rest = s.trim_start();
    while let Some(token_end) = rest.find(' ') {
        let token = &rest[..token_end];
        if !token.starts_with('/') {
            break;
        }
        rest = rest[token_end + 1..].trim_start();
    }
    rest
}

/// Skip the optional leading (quoted) title and any `/`-prefixed switches of a
/// `start` invocation.
pub fn skip_start_args(s: &str) -> &str {
    let mut rest = s.trim_start();
    if rest.starts_with('"') {
        if let Some(close) = rest[1..].find('"') {
            rest = rest[1 + close + 1..].trim_start();
        }
    }
    while let Some(token_end) = rest.find(' ') {
        let token = &rest[..token_end];
        if !token.starts_with('/') {
            break;
        }
        let is_dir_flag = token.eq_ignore_ascii_case("/d");
        rest = rest[token_end + 1..].trim_start();
        if is_dir_flag {
            if let Some(next_end) = rest.find(' ') {
                let next = &rest[..next_end];
                if !next.starts_with('/') {
                    rest = rest[next_end + 1..].trim_start();
                }
            }
        }
    }
    rest
}
