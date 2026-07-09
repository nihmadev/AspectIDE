use super::schema::{tool, req, opt, opt_int};

pub fn register(tools: &mut Vec<serde_json::Value>) {
    tools.push(tool(
        "Shell",
        "Run ONE shell command and get {exitCode, stdout, stderr, timedOut, stdoutTruncated, stderrTruncated}. Runs non-interactively in the workspace root (override with cwd); default timeout 120s (max 600). Output over ~24k chars is head+tail truncated (see the *Truncated flags). On Windows it runs via cmd.exe /C as a SINGLE line \u{2014} chain steps with `&&`, never a newline; use cmd syntax (dir/type, %VAR%, backslash paths). On Unix it is /bin/sh. Catastrophic commands are refused. For long commands (builds, test suites, installs) pass background:true \u{2014} you get {jobId, status:\"started\"} back IMMEDIATELY, keep working, then fetch the result with ShellOutput. Never sit idle waiting for a foreground command you could have backgrounded.",
        &[
            req("command", "string", "The command line (single line; chain with && or ;)."),
            opt("cwd", "string", "Working directory (workspace-relative); defaults to the workspace root."),
            opt_int("timeoutSecs", "Timeout in seconds (default 120).", 1, 600),
            opt("background", "boolean", "true = run detached: returns {jobId} at once; collect with ShellOutput. Use for anything slow (builds, tests, installs)."),
        ],
    ));
    tools.push(tool(
        "ShellOutput",
        "Fetch a background Shell job's result (from Shell background:true). wait:true (default) blocks until the job finishes or timeoutSecs passes; wait:false returns the current status instantly. Returns {jobId, status: running|done|failed, result} where result is the full Shell response JSON.",
        &[
            req("jobId", "string", "Job id returned by Shell background:true."),
            opt("wait", "boolean", "Block until finished (default true)."),
            opt_int("timeoutSecs", "Max seconds to wait (default 600).", 5, 1800),
        ],
    ));
    tools.push(tool(
        "TerminalContext",
        "Terminal sessions and output.",
        &[
            opt("sessionId", "string", ""),
            opt_int("maxChars", "", 1, 100_000),
        ],
    ));
    tools.push(tool(
        "TerminalWrite",
        "Write to a terminal.",
        &[
            req("data", "string", "Text."),
            opt("sessionId", "string", ""),
        ],
    ));
}
