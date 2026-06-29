//! Shell command safety validation for the AI `Shell` tool.
//!
//! Concept ported from clawd-code (MIT) `bash_validation` submodules —
//! `readOnlyValidation`, `destructiveCommandWarning`, `commandSemantics`,
//! `pathValidation`. Re-implemented as a self-contained, testable Rust module so
//! the safety boundary lives in the Rust runtime where the shell actually runs,
//! not in the TypeScript orchestration layer.
//!
//! Two outcomes:
//! - `blocked`: catastrophic, system-destroying commands are refused outright.
//! - `warnings`: risky-but-legitimate commands run but are flagged back to the
//!   model so it can reconsider or explain the risk.
//!
//! `read_only` classification is also computed (no writes, no redirections) so a
//! future auto-approval path can skip the prompt for safe inspection commands.

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

    // Whole-command catastrophic checks that must survive segment splitting
    // (the fork bomb is itself built from `;`, `|`, `&` separators).
    if full_squeezed.contains(":(){:|:&};:") || full_squeezed.contains(":(){:|:&};") {
        report.blocked = Some("fork bomb detected".to_string());
        return report;
    }

    let segments = split_segments(command);
    report.read_only = !segments.is_empty();

    // Commands hidden inside substitutions — `$(…)`, backticks, `<(…)`/`>(…)` —
    // execute too, yet never surface as a segment's first token, so
    // `cat "$(rm -rf ~)"` would otherwise be judged solely on its outer `cat`.
    // Classify their bodies as well: an inner catastrophic command blocks the
    // whole line, and the mere presence of a substitution forbids read-only
    // auto-approval (a prompt is still possible — this never blocks on its own).
    let substitution_bodies = extract_substitutions(command);
    if !substitution_bodies.is_empty() {
        report.read_only = false;
    }
    let inner_segments: Vec<String> = substitution_bodies
        .iter()
        .flat_map(|body| split_segments(body))
        .collect();

    // Whole-command risk: piping a download straight into a shell (the `|`
    // boundary would otherwise hide the `curl … | sh` combination).
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
            // First catastrophic hit wins; no point evaluating the rest.
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

/// Split a command line into independently-executed segments.
///
/// SECURITY: The splitter tracks escape characters (`\`) inside double-quoted
/// strings so an escaped quote cannot close the current quote context and
/// expose a subsequent separator as a boundary. Without this, `"foo\" ; rm -rf /"
/// would see the `\` "escape" the `"` and then the `;` would split a second
/// segment containing the destructive command. On escape ambiguity we fail
/// **closed**: the whole command is returned as a single opaque segment so the
/// catastrophic/risky classifiers still see it (and `read_only` stays false).
fn split_segments(command: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let bytes = command.as_bytes();
    let mut index = 0;
    let mut in_single = false;
    let mut in_double = false;
    // Track escape state inside double-quoted strings. A backslash inside
    // double quotes escapes the next character (including `"`), so we must
    // consume it before toggling the quote flag.
    let mut escaped = false;

    while index < bytes.len() {
        let ch = bytes[index] as char;

        if escaped {
            // The previous character was `\` inside a double-quoted string;
            // this byte is consumed literally regardless of what it is.
            escaped = false;
            current.push(ch);
            index += 1;
            continue;
        }

        match ch {
            // A backslash inside a double-quoted string escapes the next char.
            '\\' if in_double => {
                escaped = true;
                current.push(ch);
            }
            '\'' if !in_double => {
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
                // Treat &&, ||, | and ; all as segment boundaries; collapse runs.
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

    // Unclosed quotes: fail closed — return the whole command as one segment so
    // classifiers still see it. `escaped` staying true at EOF is similarly
    // ambiguous (truncated escape sequence).
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

/// Pull out the bodies of every command/process substitution so the commands
/// they hide are themselves classified. Handles `$( … )`, `` ` … ` ``,
/// `<( … )` and `>( … )`, recursing into nested substitutions (depth-bounded
/// against a pathological input). The bodies are returned verbatim; the caller
/// re-splits and classifies them.
fn extract_substitutions(command: &str) -> Vec<String> {
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
            // `` `…` `` backtick substitution.
            b'`' => {
                if let Some(rel) = command[index + 1..].find('`') {
                    let body = &command[index + 1..index + 1 + rel];
                    out.push(body.to_string());
                    collect_substitutions(body, depth + 1, out);
                    index += 1 + rel + 1;
                    continue;
                }
            }
            // `$( … )` command/arithmetic substitution and `<( … )` / `>( … )`
            // process substitution — all open with a sigil followed by `(`.
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

/// Starting just after an opening `(`, return the balanced-paren body and the
/// byte index just past its closing `)`. `None` if the parens never balance.
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

/// Lowercase + whitespace-collapsed copy for matching.
fn normalize(segment: &str) -> String {
    segment
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A whitespace-free copy for fork-bomb / glued-token detection.
fn squeezed(normalized: &str) -> String {
    normalized.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Return the effective command token after stripping known launcher prefixes.
///
/// SECURITY: Destructive commands wrapped in launchers such as
/// `sudo -n rm -rf /`, `env rm -rf /`, `command rm -rf /`, or
/// `doas -n rm -rf /` must not bypass first-token classification.
/// We recursively unwrap known launchers (with any flags between the launcher
/// and the payload command) so the returned token is the innermost executable.
fn first_token(normalized: &str) -> &str {
    let mut rest = normalized.trim();

    // Up to 4 wrapper layers (e.g. `sudo env command rm …`).
    for _ in 0..4 {
        let mut tokens = rest.splitn(2, ' ');
        let head = tokens.next().unwrap_or("").trim_end_matches(|c: char| !c.is_alphanumeric() && c != '_' && c != '-');
        let tail = tokens.next().unwrap_or("").trim_start();

        match head {
            // Pure launchers that unconditionally forward to the next token.
            "sudo" | "doas" | "command" | "exec" => {
                // Skip any flag arguments (e.g. `sudo -n`, `sudo -u root`).
                rest = skip_flags(tail);
            }
            // `env` forwards to the next non-assignment token.
            "env" => {
                rest = skip_env_assignments(tail);
            }
            // `time` is a shell built-in that runs its argument.
            "time" => {
                rest = tail;
            }
            _ => break,
        }
        if rest.is_empty() {
            break;
        }
    }

    rest.split(' ').next().unwrap_or("")
}

/// Skip leading flag tokens (`-n`, `--flag`, `-u root`) from a command tail,
/// returning the first non-flag token onward. Used to unwrap `sudo -n cmd`.
fn skip_flags(s: &str) -> &str {
    let mut rest = s.trim_start();
    while let Some(token_end) = rest.find(' ') {
        let token = &rest[..token_end];
        if token.starts_with('-') {
            // A flag may consume the next token as its argument (e.g. `-u root`).
            // We conservatively skip one extra token for single-char flags with args.
            rest = rest[token_end + 1..].trim_start();
            // If this was a value-consuming flag like `-u`, skip one more token.
            if token.len() == 2 && token.starts_with('-') && token.as_bytes()[1].is_ascii_alphabetic() {
                if let Some(next_end) = rest.find(' ') {
                    let next = &rest[..next_end];
                    if !next.starts_with('-') {
                        rest = rest[next_end + 1..].trim_start();
                    }
                }
            }
        } else {
            break;
        }
    }
    rest
}

/// Skip `KEY=VALUE` environment variable assignments that prefix `env` payloads,
/// returning the remainder starting with the actual command name.
fn skip_env_assignments(s: &str) -> &str {
    let mut rest = s.trim_start();
    // Also skip any leading flags to `env` itself (e.g. `env -i`).
    rest = skip_flags(rest);
    loop {
        match rest.split_once(' ') {
            Some((token, tail)) if token.contains('=') => rest = tail.trim_start(),
            _ => break,
        }
    }
    rest
}

/// Returns a reason when the command is catastrophic and must be refused.
fn catastrophic_reason(normalized: &str) -> Option<String> {
    // Disabling the rm root guard is never legitimate from an agent.
    if normalized.contains("--no-preserve-root") {
        return Some("rm --no-preserve-root targets the entire filesystem".to_string());
    }

    // SECURITY: interpreter wrappers (`bash -c '…'`, `sh -c '…'`, `python -c '…'`,
    // `powershell -Command '…'`) can hide an inner `rm -rf /` from the first-token
    // check. Classify the inner payload as an additional segment — the caller's
    // `inner_segments` path already handles `$(…)` substitutions, but `-c` payloads
    // are plain string arguments, not substitutions, so we handle them here.
    // We match normalized (lowercased, whitespace-collapsed) to keep it tight.
    let ft = first_token(normalized);
    if matches!(ft, "bash" | "sh" | "zsh" | "fish" | "dash" | "ksh" | "csh" | "tcsh") {
        if let Some(inner) = extract_interpreter_payload(normalized) {
            let inner_norm = normalize(&inner);
            if let Some(reason) = catastrophic_reason(&inner_norm) {
                return Some(reason);
            }
        }
    }
    if matches!(ft, "python" | "python3" | "ruby" | "perl" | "node")
        && normalized.contains(" -c ")
    {
        if let Some(inner) = extract_interpreter_payload(normalized) {
            let inner_norm = normalize(&inner);
            if let Some(reason) = catastrophic_reason(&inner_norm) {
                return Some(reason);
            }
        }
    }
    // PowerShell `-Command` or `-EncodedCommand` can similarly hide payloads.
    if matches!(ft, "powershell" | "pwsh") {
        if let Some(inner) = extract_interpreter_payload(normalized) {
            let inner_norm = normalize(&inner);
            if let Some(reason) = catastrophic_reason(&inner_norm) {
                return Some(reason);
            }
        }
    }

    // rm -rf against filesystem root / home / a protected system directory.
    // Every operand is checked (not just the last), after quote/slash/`$HOME`
    // normalization, so `rm -rf "/"`, `rm -rf "$HOME"` and `rm -rf /etc /usr`
    // can't slip through.
    if is_rm_recursive_force(normalized) {
        for target in rm_targets(normalized) {
            if is_dangerous_root_target(&target) {
                return Some(format!(
                    "recursive force delete of a protected path: {target}"
                ));
            }
        }
    }

    // Filesystem creation / raw device writes.
    if first_token(normalized) == "mkfs" || normalized.starts_with("mkfs.") {
        return Some("mkfs would format a filesystem".to_string());
    }
    if first_token(normalized) == "dd" && writes_to_block_device(normalized) {
        return Some("dd writes directly to a block device".to_string());
    }
    if redirects_to_block_device(normalized) {
        return Some("redirect targets a raw block device".to_string());
    }

    // chmod/chown -R against filesystem root.
    if (first_token(normalized) == "chmod" || first_token(normalized) == "chown")
        && normalized.contains(" -r")
        && segment_targets_root(normalized)
    {
        return Some("recursive permission/ownership change at filesystem root".to_string());
    }

    // Windows catastrophic equivalents.
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

fn is_rm_recursive_force(normalized: &str) -> bool {
    if first_token(normalized) != "rm" {
        return false;
    }
    let has_recursive = normalized.contains(" -r")
        || normalized.contains(" --recursive")
        || flag_cluster_has(normalized, 'r');
    let has_force = normalized.contains(" -f")
        || normalized.contains(" --force")
        || flag_cluster_has(normalized, 'f');
    has_recursive && has_force
}

/// Detect a clustered short flag like `-rf`/`-fr` containing `needle`.
fn flag_cluster_has(normalized: &str, needle: char) -> bool {
    normalized
        .split(' ')
        .any(|token| token.starts_with('-') && !token.starts_with("--") && token.contains(needle))
}

/// Every non-flag operand of an `rm` command, normalized for comparison
/// (quotes stripped, a single trailing slash dropped, `$HOME`/`%USERPROFILE%`
/// folded to `~`). `--` ends flag parsing so `rm -rf -- "/"` is still caught.
fn rm_targets(normalized: &str) -> Vec<String> {
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

/// Strip surrounding quotes, collapse a trailing slash, and fold the home
/// directory env-vars to `~` so quoted / decorated forms compare equal to the
/// bare dangerous targets.
// The `${home}` / `${home}/*` literals are shell syntax matched verbatim, not
// format placeholders.
#[allow(clippy::literal_string_with_formatting_args)]
fn normalize_path_operand(token: &str) -> String {
    let mut path = token.trim_matches(|c| c == '"' || c == '\'').to_string();
    // Fold home-directory references to a single canonical form (`~` or `~/*`).
    // SECURITY (finding #4): also fold `$HOME/*` / `${HOME}/*` / `%USERPROFILE%/*`
    // glob forms so they are caught by `is_dangerous_root_target`.
    for home in ["$home", "${home}", "%userprofile%", "$env:userprofile"] {
        if path == home {
            path = "~".to_string();
            break;
        }
        // Glob wipe: `$HOME/*` → `~/*`
        let glob = format!("{home}/*");
        if path == glob {
            path = "~/*".to_string();
            break;
        }
        // Backslash variant for Windows: `%USERPROFILE%\*` → `~/*`
        let bslash_glob = format!("{home}\\*");
        if path == bslash_glob {
            path = "~/*".to_string();
            break;
        }
    }
    // Drop a single trailing slash (but keep "/" itself intact).
    if path.len() > 1 {
        if let Some(stripped) = path.strip_suffix('/') {
            path = stripped.to_string();
        }
    }
    path
}

// The `${home}` literals are shell syntax matched verbatim, not format args.
#[allow(clippy::literal_string_with_formatting_args)]
fn is_dangerous_root_target(target: &str) -> bool {
    // A direct child of a multi-user home root wipes an entire user profile,
    // e.g. `rm -rf /home/alice` / `/users/bob`. (`/root` is matched whole below.)
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
            // SECURITY (finding #4): `~/*` wipes home contents after shell glob expansion.
            | "~/*"
            | "$home"
            | "${home}"
            // Raw un-normalized glob forms (double-safety for any quoting variant).
            | "$home/*"
            | "${home}/*"
            | "%userprofile%/*"
            | "$env:userprofile/*"
            | "/."
            | "/.*"
            // Protected top-level system directories (and their glob form).
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

/// Extract the inner shell payload from `-c '…'` or `-Command '…'` style
/// interpreter invocations. Returns `None` if no recognizable `-c`/`-command`
/// argument is found. The returned string is unquoted one level.
fn extract_interpreter_payload(normalized: &str) -> Option<String> {
    // Look for `-c` or `-command` flag followed by the payload token.
    let tokens: Vec<&str> = normalized.split(' ').collect();
    for (i, &token) in tokens.iter().enumerate() {
        if matches!(token, "-c" | "-command" | "/c" | "/command") {
            if let Some(&payload) = tokens.get(i + 1) {
                // Strip surrounding quotes from the payload.
                let unquoted = payload.trim_matches(|c| c == '\'' || c == '"');
                if !unquoted.is_empty() {
                    return Some(unquoted.to_string());
                }
            }
        }
    }
    None
}

fn writes_to_block_device(normalized: &str) -> bool {
    normalized
        .split(' ')
        .any(|token| token.strip_prefix("of=").is_some_and(is_block_device_path))
}

fn redirects_to_block_device(normalized: &str) -> bool {
    // Look for `>` / `>>` followed by a device path.
    let mut tokens = normalized.split(' ').peekable();
    while let Some(token) = tokens.next() {
        if token == ">" || token == ">>" {
            if let Some(next) = tokens.peek() {
                if is_block_device_path(next) {
                    return true;
                }
            }
        } else if let Some(rest) = token.strip_prefix(">>") {
            if is_block_device_path(rest) {
                return true;
            }
        } else if let Some(rest) = token.strip_prefix('>') {
            if is_block_device_path(rest) {
                return true;
            }
        }
    }
    false
}

fn is_block_device_path(path: &str) -> bool {
    let path = path.trim_matches(|c| c == '"' || c == '\'');
    path.starts_with("/dev/sd")
        || path.starts_with("/dev/nvme")
        || path.starts_with("/dev/hd")
        || path.starts_with("/dev/disk")
        || path.starts_with("/dev/vd")
        || path == "/dev/mem"
}

fn segment_targets_root(normalized: &str) -> bool {
    normalized
        .split(' ')
        .any(|token| matches!(token, "/" | "/*"))
}

fn mentions_windows_drive_root(normalized: &str) -> bool {
    // c:, c:\, c:/ for any drive letter.
    normalized.split([' ', '"']).any(|token| {
        let token = token.trim();
        let bytes = token.as_bytes();
        bytes.len() >= 2
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (token.len() == 2 || matches!(bytes.get(2), Some(b'\\' | b'/')))
    })
}

/// Risky-but-allowed commands worth flagging back to the model.
fn risky_warnings(normalized: &str) -> Vec<String> {
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

/// Commands unconditionally safe to auto-approve (no write, no exec, no delete).
/// SECURITY (finding #5): `find` is intentionally excluded — `find . -delete`,
/// `find . -exec rm …`, `-fprint`, `-ok`, `-execdir` are destructive or exec-capable.
/// `fd` is similarly excluded because it supports `--exec`. Both are handled by
/// the `find`/`fd` branch in `is_read_only_segment` instead.
const READ_ONLY_COMMANDS: &[&str] = &[
    "ls", "dir", "pwd", "cat", "type", "echo", "printenv", "whoami", "hostname", "id",
    "date", "uname", "which", "where", "head", "tail", "wc", "grep", "rg", "tree",
    "stat", "file", "du", "df", "ps", "less", "more", "sort", "uniq", "cut", "jq", "yq", "diff",
    "basename", "dirname", "realpath", "readlink", "sleep", "true", "test",
];

/// Flags in `find` that make it destructive or capable of arbitrary execution.
const FIND_DANGEROUS_FLAGS: &[&str] = &[
    "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprint0",
    "-fprintf", "-ls",
];

/// True when the segment cannot write or mutate state.
fn is_read_only_segment(normalized: &str) -> bool {
    // Any output redirection makes it a write.
    if normalized.contains('>') {
        return false;
    }
    let ft = first_token(normalized);
    if READ_ONLY_COMMANDS.contains(&ft) {
        return true;
    }
    // Read-only subcommands of common VCS/toolchains.
    match ft {
        "git" => is_git_read_only(normalized),

        // SECURITY (finding #5): `find` is only read-only when it uses no
        // destructive or execution predicates. Fail closed: unknown = not read-only.
        "find" => {
            !FIND_DANGEROUS_FLAGS
                .iter()
                .any(|flag| normalized.contains(flag))
        }

        // `fd` with `--exec` / `--exec-batch` executes commands — not read-only.
        "fd" => {
            !normalized.contains(" --exec")
                && !normalized.contains(" -x ")
                && !normalized.contains(" --exec-batch")
                && !normalized.contains(" -X ")
        }

        // `env` alone (with or without `-i`) can print environment (read-only);
        // but `env COMMAND …` executes — handled by the first_token unwrap already.
        "env" => normalized.trim() == "env" || normalized.starts_with("env -"),

        "node" | "npm" | "pnpm" | "yarn" | "cargo" | "python" | "python3" | "rustc" | "go"
        | "deno" | "bun" => {
            normalized.contains("--version")
                || normalized.contains(" -v")
                || normalized.contains(" version")
                || normalized.contains(" list")
                || normalized.contains(" ls")
                || normalized.contains(" view")
                || normalized.contains(" outdated")
                || normalized.contains(" tree")
                || normalized.contains(" metadata")
        }
        "sed" => normalized.contains("-n") && !normalized.contains("-i"),
        _ => false,
    }
}

/// Classify a `git …` command as read-only.
///
/// SECURITY (finding #6): The previous implementation whitelisted entire
/// subcommands like `branch`, `remote`, and `tag` without checking flags.
/// `git branch -D feature`, `git tag -d v1`, and `git remote set-url origin …`
/// all mutate state. We now only allow the pure read forms of each subcommand.
fn is_git_read_only(normalized: &str) -> bool {
    let rest = normalized.strip_prefix("git ").unwrap_or("");
    let sub = rest.split(' ').find(|t| !t.starts_with('-')).unwrap_or("");
    match sub {
        // Unconditionally read-only subcommands.
        "status" | "log" | "diff" | "show" | "describe" | "rev-parse" | "blame"
        | "ls-files" | "shortlog" | "cat-file" | "ls-tree" | "for-each-ref"
        | "count-objects" | "fsck" | "verify-pack" | "stash" => {
            // `git stash list/show` are reads; `git stash apply/pop/drop` are writes.
            if sub == "stash" {
                rest.contains("list") || rest.contains("show")
            } else {
                true
            }
        }

        // `git branch`: only list/show forms are read-only.
        // `-D`, `-d`, `-m`, `-M`, `-c`, `-C`, `--delete`, `--move`, `--copy`,
        // `--set-upstream-to` all mutate state.
        "branch" => {
            !rest.contains(" -d")
                && !rest.contains(" -D")
                && !rest.contains(" -m")
                && !rest.contains(" -M")
                && !rest.contains(" -c")
                && !rest.contains(" -C")
                && !rest.contains("--delete")
                && !rest.contains("--move")
                && !rest.contains("--copy")
                && !rest.contains("--set-upstream")
                && !rest.contains("--unset-upstream")
                && !rest.contains("--edit-description")
        }

        // `git remote`: only list/show/get-url are read-only.
        // `add`, `remove`, `set-url`, `rename`, `set-head`, `set-branches`,
        // `set-branches`, `prune` all mutate state.
        "remote" => {
            let sub2 = rest
                .split_whitespace()
                .nth(1) // the word after "remote"
                .unwrap_or("");
            matches!(sub2, "" | "-v" | "--verbose" | "show" | "get-url")
        }

        // `git tag`: only list/verify forms are read-only.
        // `-d`/`--delete`, `-m` (annotated create), creating a new tag, `-f`,
        // `--sign`, `--local-user` all mutate state.
        "tag" => {
            !rest.contains(" -d")
                && !rest.contains(" -D")
                && !rest.contains("--delete")
                && !rest.contains(" -m ")
                && !rest.contains(" -a ")
                && !rest.contains("--annotate")
                && !rest.contains(" -s")
                && !rest.contains(" -f")
                && !rest.contains("--force")
                // A bare `git tag NAME` without `-l`/`--list` creates a tag.
                // Only allow the explicit list form.
                && (rest.contains(" -l")
                    || rest.contains("--list")
                    || rest.split_whitespace().count() <= 1)
        }

        // `git config` is NOT read-only by default: a write persists a hook.
        "config" => {
            (rest.contains("--get") || rest.contains("--list") || rest.contains(" -l"))
                && !rest.contains("--unset")
                && !rest.contains("--replace-all")
                && !rest.contains("--add")
                && !rest.contains("--edit")
        }

        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocked(command: &str) -> bool {
        classify_shell_command(command).blocked.is_some()
    }

    #[test]
    fn blocks_rm_rf_root() {
        assert!(blocked("rm -rf /"));
        assert!(blocked("rm -rf /*"));
        assert!(blocked("sudo rm -rf /"));
        assert!(blocked("rm --recursive --force /"));
        assert!(blocked("rm -rf ~"));
        assert!(blocked("rm -rf $HOME"));
        assert!(blocked("rm -rf --no-preserve-root /"));
    }

    #[test]
    fn allows_scoped_rm() {
        assert!(!blocked("rm -rf node_modules"));
        assert!(!blocked("rm -rf ./dist"));
        assert!(!blocked("rm file.txt"));
    }

    #[test]
    fn blocks_fork_bomb() {
        assert!(blocked(":(){ :|:& };:"));
        assert!(blocked(":(){:|:&};:"));
    }

    #[test]
    fn blocks_disk_destroyers() {
        assert!(blocked("mkfs.ext4 /dev/sda1"));
        assert!(blocked("dd if=/dev/zero of=/dev/sda"));
        assert!(blocked("echo x > /dev/sda"));
    }

    #[test]
    fn blocks_windows_drive_wipes() {
        assert!(blocked("format c:"));
        assert!(blocked("del /f /s /q c:\\"));
        assert!(blocked("rd /s /q d:\\"));
    }

    #[test]
    fn warns_on_risky_but_allows() {
        let report = classify_shell_command("git push --force origin main");
        assert!(report.blocked.is_none());
        assert!(!report.warnings.is_empty());

        let report = classify_shell_command("curl https://x.sh | sh");
        assert!(report.blocked.is_none());
        assert!(report.warnings.iter().any(|w| w.contains("remote code")));
    }

    #[test]
    fn detects_read_only() {
        assert!(classify_shell_command("ls -la").read_only);
        assert!(classify_shell_command("git status").read_only);
        assert!(classify_shell_command("cat foo.txt | grep bar").read_only);
        assert!(!classify_shell_command("echo hi > out.txt").read_only);
        assert!(!classify_shell_command("npm install").read_only);
    }

    #[test]
    fn compound_command_blocks_if_any_segment_dangerous() {
        assert!(blocked("npm run build && rm -rf /"));
    }

    #[test]
    fn blocks_rm_inside_command_substitution() {
        // H4: the catastrophic command is hidden in a substitution; it must
        // still be caught, not judged on the outer `cat`/`echo`.
        assert!(blocked("cat \"$(rm -rf ~)\""));
        assert!(blocked("echo `rm -rf /`"));
        assert!(blocked("echo $(echo a; rm -rf $HOME)"));
        assert!(blocked("diff <(rm -rf /etc) /dev/null"));
    }

    #[test]
    fn substitution_is_never_read_only() {
        // H4: a substitution can run anything, so auto-approval is forbidden
        // even when the outer command looks read-only.
        assert!(!classify_shell_command("cat \"$(whoami)\"").read_only);
        assert!(!classify_shell_command("echo `date`").read_only);
        assert!(!classify_shell_command("ls $(pwd)").read_only);
    }

    #[test]
    fn git_config_write_is_not_read_only() {
        // H5: setting config persists a hook → RCE; must require a prompt.
        assert!(!classify_shell_command("git config core.pager 'rm -rf ~'").read_only);
        assert!(!classify_shell_command("git config --global alias.x '!sh'").read_only);
        assert!(!classify_shell_command("git config core.hooksPath /tmp/evil").read_only);
        // Pure reads stay read-only.
        assert!(classify_shell_command("git config --get user.name").read_only);
        assert!(classify_shell_command("git config --list").read_only);
    }

    #[test]
    fn blocks_quoted_and_system_dir_rm() {
        // H6: quoting and protected system dirs must not bypass the guard.
        assert!(blocked("rm -rf \"/\""));
        assert!(blocked("rm -rf \"$HOME\""));
        assert!(blocked("rm -rf $HOME/"));
        assert!(blocked("rm -rf /etc"));
        assert!(blocked("rm -rf /usr /bin"));
        assert!(blocked("rm -rf /boot/*"));
        assert!(blocked("rm -rf /home/alice"));
        assert!(blocked("rm -rf -- \"/\""));
        // Scoped deletes still allowed.
        assert!(!blocked("rm -rf ./build"));
        assert!(!blocked("rm -rf target/debug"));
    }

    // --- Regression tests for security findings ---

    #[test]
    fn escaped_quote_does_not_hide_segment() {
        // Finding #2: an escaped `"` inside double-quotes must not close the
        // quoting context, exposing the subsequent `;` as a real separator.
        // The whole command has an unclosed outer quote so we fail-closed
        // (return one segment), meaning the catastrophic rm is still visible.
        assert!(blocked("echo \"foo\\\" ; rm -rf /"));
    }

    #[test]
    fn blocks_rm_via_wrapper_launchers() {
        // Finding #3: common launcher wrappers must not bypass first_token.
        assert!(blocked("env rm -rf /"));
        assert!(blocked("command rm -rf /"));
        assert!(blocked("sudo -n rm -rf /"));
        assert!(blocked("doas -n rm -rf /"));
        assert!(blocked("sudo -u root rm -rf /"));
        assert!(blocked("sudo env rm -rf /"));
    }

    #[test]
    fn blocks_rm_via_interpreter_wrapper() {
        // Finding #3 extension: `bash -c 'rm -rf /'` must be blocked.
        assert!(blocked("bash -c 'rm -rf /'"));
        assert!(blocked("sh -c 'rm -rf /'"));
        assert!(blocked("zsh -c 'rm -rf /'"));
        // Scoped inner payload is still allowed.
        assert!(!blocked("bash -c 'ls -la'"));
    }

    #[test]
    fn blocks_home_glob_wipe() {
        // Finding #4: `rm -rf $HOME/*` wipes home contents after expansion.
        assert!(blocked("rm -rf $HOME/*"));
        assert!(blocked("rm -rf ${HOME}/*"));
        assert!(blocked("rm -rf $home/*"));
        assert!(blocked("rm -rf \"$HOME/*\""));
        // Scoped paths under home sub-dirs are still allowed.
        assert!(!blocked("rm -rf $HOME/project/dist"));
    }

    #[test]
    fn find_with_destructive_flags_is_not_read_only() {
        // Finding #5: `find` with delete/exec predicates must not be auto-approved.
        assert!(!classify_shell_command("find . -delete").read_only);
        assert!(!classify_shell_command("find . -exec rm -rf {} +").read_only);
        assert!(!classify_shell_command("find . -execdir sh -c 'rm {}' \\;").read_only);
        assert!(!classify_shell_command("find . -ok rm {} \\;").read_only);
        assert!(!classify_shell_command("find . -fprint /tmp/out").read_only);
        // Safe predicates stay read-only.
        assert!(classify_shell_command("find . -name '*.rs' -type f").read_only);
        assert!(classify_shell_command("find /src -maxdepth 2").read_only);
    }

    #[test]
    fn git_branch_destructive_is_not_read_only() {
        // Finding #6: mutating git branch/tag/remote must require approval.
        assert!(!classify_shell_command("git branch -D feature").read_only);
        assert!(!classify_shell_command("git branch -d old").read_only);
        assert!(!classify_shell_command("git branch -m old new").read_only);
        assert!(!classify_shell_command("git tag -d v1.0").read_only);
        assert!(!classify_shell_command("git remote set-url origin https://evil.com").read_only);
        assert!(!classify_shell_command("git remote add upstream https://foo.com").read_only);
        assert!(!classify_shell_command("git remote remove upstream").read_only);
        // Read-only forms stay approved.
        assert!(classify_shell_command("git branch").read_only);
        assert!(classify_shell_command("git branch --show-current").read_only);
        assert!(classify_shell_command("git tag -l").read_only);
        assert!(classify_shell_command("git remote -v").read_only);
        assert!(classify_shell_command("git remote show origin").read_only);
    }
}
