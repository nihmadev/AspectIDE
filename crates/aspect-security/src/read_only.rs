use crate::normalize::first_token;

/// Commands unconditionally safe to auto-approve (no write, no exec, no delete).
pub const READ_ONLY_COMMANDS: &[&str] = &[
    "ls", "dir", "pwd", "cat", "type", "echo", "printenv", "whoami", "hostname", "id", "date",
    "uname", "which", "where", "head", "tail", "wc", "grep", "rg", "tree", "stat", "file", "du",
    "df", "ps", "less", "more", "sort", "uniq", "cut", "jq", "yq", "diff", "basename", "dirname",
    "realpath", "readlink", "sleep", "true", "test",
];

/// Flags in `find` that make it destructive or capable of arbitrary execution.
pub const FIND_DANGEROUS_FLAGS: &[&str] = &[
    "-delete", "-exec", "-execdir", "-ok", "-okdir", "-fprint", "-fprint0", "-fprintf", "-ls",
];

/// True when the segment cannot write or mutate state.
pub fn is_read_only_segment(normalized: &str) -> bool {
    if normalized.contains('>') {
        return false;
    }
    let ft = first_token(normalized);
    if READ_ONLY_COMMANDS.contains(&ft) {
        return true;
    }
    match ft {
        "git" => is_git_read_only(normalized),

        "find" => !FIND_DANGEROUS_FLAGS
            .iter()
            .any(|flag| normalized.contains(flag)),

        "fd" => {
            !normalized.contains(" --exec")
                && !normalized.contains(" -x ")
                && !normalized.contains(" --exec-batch")
                && !normalized.contains(" -X ")
        }

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
fn is_git_read_only(normalized: &str) -> bool {
    let rest = normalized.strip_prefix("git ").unwrap_or("");
    let sub = rest.split(' ').find(|t| !t.starts_with('-')).unwrap_or("");
    match sub {
        "status" | "log" | "diff" | "show" | "describe" | "rev-parse" | "blame" | "ls-files"
        | "shortlog" | "cat-file" | "ls-tree" | "for-each-ref" | "count-objects" | "fsck"
        | "verify-pack" | "stash" => {
            if sub == "stash" {
                rest.contains("list") || rest.contains("show")
            } else {
                true
            }
        }

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

        "remote" => {
            let sub2 = rest
                .split_whitespace()
                .nth(1)
                .unwrap_or("");
            matches!(sub2, "" | "-v" | "--verbose" | "show" | "get-url")
        }

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
                && (rest.contains(" -l")
                    || rest.contains("--list")
                    || rest.split_whitespace().count() <= 1)
        }

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
