use super::schema::{tool, req, opt, opt_int, req_arr_items, opt_str_arr, patch_operation_schema};

pub fn register(tools: &mut Vec<serde_json::Value>) {
    tools.push(tool(
        "Write",
        "Create or rewrite a file.",
        &[
            req("path", "string", "File path."),
            req("text", "string", "Contents."),
            opt("overwrite", "boolean", ""),
            opt("saveToDisk", "boolean", ""),
        ],
    ));
    tools.push(tool(
        "StrReplace",
        "Replace exact text in a file. Line endings are matched tolerantly (LF vs CRLF), so `\\n`-only oldText matches a Windows `\\r\\n` file. Re-issuing an insert that is already present is a safe no-op, not a duplicate.",
        &[
            req("path", "string", "File path."),
            req("oldText", "string", "Exact text to find (whitespace-significant; EOL-tolerant)."),
            req("newText", "string", "Replacement text; empty string deletes the matched text."),
            opt_int("expectedReplacements", "", 1, 1000),
            opt("saveToDisk", "boolean", ""),
        ],
    ));
    tools.push(tool(
        "PatchEngine",
        "Atomic multi-file batch: ALL operations apply or NONE \u{2014} one failed op rejects the whole batch. Use ONLY when edits must land together (cross-file rename/refactor where a half-applied state would be broken); for independent edits \u{2014} even several in one file \u{2014} use StrReplace/Write/Delete instead, so one failure cannot discard the rest. Each op needs `action` and `path`; a failing op is reported by its index. StrReplace-style ops are EOL-tolerant.",
        &[
            req_arr_items("operations", "Patch ops.", patch_operation_schema()),
            opt("saveToDisk", "boolean", ""),
            opt("dryRun", "boolean", ""),
        ],
    ));
    tools.push(tool(
        "Delete",
        "Delete a file.",
        &[req("path", "string", "")],
    ));
    tools.push(tool(
        "Checkpoint",
        "Snapshot file contents so risky edits can be rolled back. action=create captures the given paths (or all open editor files when paths is omitted); list/diff/delete/restore manage them.",
        &[
            req("action", "string", "create/list/diff/delete/restore."),
            opt("id", "string", "Checkpoint id (for diff/delete/restore)."),
            opt("label", "string", "Human label for a created checkpoint."),
            opt_str_arr("paths", "Array of workspace file paths to snapshot on create. Omit to snapshot all open editor files."),
            opt("saveToDisk", "boolean", ""),
            opt("dryRun", "boolean", ""),
        ],
    ));
}
