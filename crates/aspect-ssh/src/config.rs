//! Minimal, read-only parser for `~/.ssh/config`, used purely for *discovery* —
//! surfacing the user's named hosts to the agent and Settings UI so it can pick a
//! target. It is not a full OpenSSH config evaluator (no `Include`, `Match`, or
//! pattern expansion); OpenSSH itself still does the real resolution at connect
//! time. Only non-secret routing fields are extracted.

use crate::model::SshConfigHost;

/// Parse SSH config text into the concrete host aliases it defines.
///
/// Wildcard blocks like `Host *` set defaults in OpenSSH but are not connectable
/// targets, so they're skipped here. Per OpenSSH semantics the first value seen
/// for a key within a block wins.
#[must_use]
pub fn parse_ssh_config(text: &str) -> Vec<SshConfigHost> {
    let mut hosts: Vec<SshConfigHost> = Vec::new();
    let mut current: Option<SshConfigHost> = None;

    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        let Some((keyword, value)) = split_keyword(line) else {
            continue;
        };
        let key = keyword.to_ascii_lowercase();
        match key.as_str() {
            "host" => {
                flush(&mut hosts, current.take());
                if let Some(alias) = first_concrete_pattern(value) {
                    current = Some(SshConfigHost {
                        alias,
                        hostname: None,
                        user: None,
                        port: None,
                        identity_file: None,
                    });
                }
            }
            // A `Match` block can't be reduced to one alias — close the current
            // host so its options don't bleed into unrelated entries.
            "match" => flush(&mut hosts, current.take()),
            "hostname" => set_once(&mut current, |h| &mut h.hostname, unquote(value)),
            "user" => set_once(&mut current, |h| &mut h.user, unquote(value)),
            "identityfile" => {
                set_once(&mut current, |h| &mut h.identity_file, unquote(value));
            }
            "port" => {
                if let Some(host) = current.as_mut() {
                    if host.port.is_none() {
                        if let Ok(port) = unquote(value).parse::<u16>() {
                            host.port = Some(port);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    flush(&mut hosts, current.take());
    hosts
}

/// Push a finished host block if it carries a usable alias.
fn flush(hosts: &mut Vec<SshConfigHost>, host: Option<SshConfigHost>) {
    if let Some(host) = host {
        if !host.alias.is_empty() {
            hosts.push(host);
        }
    }
}

/// Set a string field only if currently unset (first-wins) and the value is
/// non-empty, on the open host block.
fn set_once(
    current: &mut Option<SshConfigHost>,
    field: impl FnOnce(&mut SshConfigHost) -> &mut Option<String>,
    value: String,
) {
    if value.is_empty() {
        return;
    }
    if let Some(host) = current.as_mut() {
        let slot = field(host);
        if slot.is_none() {
            *slot = Some(value);
        }
    }
}

/// Drop a trailing `#` comment. OpenSSH treats `#` as a comment only when it is
/// outside quotes, so a value like `IdentityFile "~/keys/my#key"` keeps its `#`.
fn strip_comment(line: &str) -> &str {
    let mut in_single = false;
    let mut in_double = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '\'' if !in_double => in_single = !in_single,
            '"' if !in_single => in_double = !in_double,
            '#' if !in_single && !in_double => return &line[..index],
            _ => {}
        }
    }
    line
}

/// Split a config line into `(keyword, value)`. The separator is whitespace
/// and/or a single `=`, per OpenSSH's `Key Value` / `Key=Value` / `Key = Value`.
fn split_keyword(line: &str) -> Option<(&str, &str)> {
    let end = line.find(|c: char| c.is_whitespace() || c == '=')?;
    let keyword = &line[..end];
    let rest = line[end..].trim_start_matches(|c: char| c.is_whitespace() || c == '=');
    if keyword.is_empty() {
        None
    } else {
        Some((keyword, rest.trim()))
    }
}

/// The first whitespace-separated pattern that contains no `*`/`?`/`!` wildcard.
fn first_concrete_pattern(value: &str) -> Option<String> {
    value
        .split_whitespace()
        .map(unquote)
        .find(|pattern| !pattern.is_empty() && !pattern.contains(['*', '?', '!']))
}

/// Strip a single layer of surrounding double or single quotes.
fn unquote(value: &str) -> String {
    let trimmed = value.trim();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

