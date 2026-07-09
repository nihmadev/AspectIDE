use super::schema::{tool, req, opt, opt_int};

pub fn register(tools: &mut Vec<serde_json::Value>) {
    tools.push(tool(
        "SshConnect",
        "Open a non-interactive SSH session to a remote host and verify it. Use the host alias from ~/.ssh/config (see SshList), a hostname/IP, or user@host. Auth uses ssh-agent / default keys / an explicit identityFile \u{2014} never an interactive password (AspectIDE runs in BatchMode). Returns a sessionId for SshExec/SshTransfer plus the remote OS and home directory. This is the ONLY correct way to start SSH work; do not run `ssh` through Shell/TerminalWrite.",
        &[
            req("host", "string", "ssh_config alias, hostname/IP, or user@host."),
            opt("user", "string", "Login user (overrides host/config)."),
            opt_int("port", "Port (default 22 or per ssh_config).", 1, 65535),
            opt("identityFile", "string", "Path to a private key to use exclusively."),
            opt("label", "string", "Friendly label for the session."),
        ],
    ));
    tools.push(tool(
        "SshExec",
        "Run a command on an SSH session (from SshConnect) and return structured { exitCode, stdout, stderr, durationMs }. Runs in the session's sticky working directory; pass cwd to change it for this and following commands. Non-interactive and catastrophic-command guarded. Prefer this over Shell `ssh ...` for every remote command.",
        &[
            req("session", "string", "sessionId from SshConnect."),
            req("command", "string", "Remote command to run."),
            opt("cwd", "string", "Remote working directory (sticky for the session)."),
            opt_int("timeoutSecs", "Timeout in seconds, default 120, max 600.", 1, 600),
        ],
    ));
    tools.push(tool(
        "SshTransfer",
        "Copy a file or directory between the workspace and a remote host over scp, for an SSH session. The local path is confined to the workspace.",
        &[
            req("session", "string", "sessionId from SshConnect."),
            req("direction", "string", "\"upload\" (local\u{2192}remote) or \"download\" (remote\u{2192}local)."),
            req("localPath", "string", "Workspace-relative or absolute path inside the workspace."),
            req("remotePath", "string", "Absolute or login-relative path on the remote host."),
            opt("recursive", "boolean", "Copy directories recursively."),
        ],
    ));
    tools.push(tool(
        "SshDisconnect",
        "Close an SSH session (by sessionId) or every session (all=true).",
        &[
            opt("session", "string", "sessionId to close."),
            opt("all", "boolean", "Close all sessions."),
        ],
    ));
}
