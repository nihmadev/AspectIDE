/// Hard ceiling on cumulative tool-output bytes appended across the turn.
pub const TURN_OUTPUT_BYTE_BUDGET: usize = 600_000;
/// Hard ceiling on total tool calls across the turn — a backstop against a model
/// that calls tools without converging.
pub const TURN_TOOL_CALL_BUDGET: usize = 200;
/// Per-tool output character limit before truncation.
pub const TOOL_OUTPUT_CHAR_LIMIT: usize = 32_000;

/// Max tool rounds per turn (clamped).
pub const MAX_TOOL_ROUNDS: usize = 128;
/// Default tool round limit when none specified.
pub const DEFAULT_TOOL_ROUND_LIMIT: u32 = 32;

/// Number of parallel subagent slots.
pub const MAX_PARALLEL_NATIVE_SUBAGENTS: usize = 4;

/// Subagent round limits.
pub const SUBAGENT_MAX_ROUNDS_NORMAL: usize = 16;
pub const SUBAGENT_MAX_ROUNDS_AUTOMATIC: usize = 24;

/// Max chars for subagent summary.
pub const SUBAGENT_SUMMARY_CAP: usize = 12_000;

/// Presentation caps.
pub const PLAN_STEP_CAP: usize = 40;
pub const PLAN_ALTERNATIVE_CAP: usize = 8;
pub const PLAN_RISK_CAP: usize = 12;
pub const PLAN_VERIFICATION_CAP: usize = 12;

/// Approval timeout.
pub const APPROVAL_TIMEOUT_SECS: u64 = 300;
/// Question timeout.
pub const QUESTION_TIMEOUT_SECS: u64 = 300;

/// Tool output truncation notice.
pub const TRUNCATION_NOTICE: &str = "[Tool output truncated: {} chars total, showing first {TOOL_OUTPUT_CHAR_LIMIT}. Use targeted follow-up queries to retrieve specific sections.]";

/// Budget exceeded message injected as user message.
pub const BUDGET_EXCEEDED_NOTICE: &str =
    "[Tool budget reached for this turn: {turn_tool_calls} tool calls, \
     ~{turn_output_bytes} bytes of tool output. No more tools will run this \
     turn — synthesize a final answer from the results already gathered.]";

/// Empty-turn placeholder.
pub const EMPTY_TURN_PLACEHOLDER: &str =
    "The turn produced no answer. Press **Retry** or rephrase your request.";

/// Browser tool timeouts (seconds).
pub const BROWSER_TIMEOUT_INSTALL: u64 = 600;
pub const BROWSER_TIMEOUT_OPEN_ACT_CHAT: u64 = 120;
pub const BROWSER_TIMEOUT_SCREENSHOT: u64 = 90;
pub const BROWSER_TIMEOUT_DOCTOR_QUICK: u64 = 25;
pub const BROWSER_TIMEOUT_DOCTOR_FULL: u64 = 180;
pub const BROWSER_TIMEOUT_DEFAULT: u64 = 60;

/// Active text character limit for ActiveContext.
pub const ACTIVE_TEXT_CHAR_LIMIT: usize = 20_000;

/// ContextBudgeter defaults.
pub const BUDGETER_DEFAULT_TARGET_CHARS: usize = 16_000;
pub const BUDGETER_MIN_CHARS: usize = 2_000;
pub const BUDGETER_MAX_CHARS: usize = 22_000;
pub const BUDGETER_DEFAULT_MAX_ITEMS: usize = 28;
pub const BUDGETER_ACTIVE_SCORE: i64 = 90;
pub const BUDGETER_OPEN_DOC_SCORE: i64 = 70;
pub const BUDGETER_RULE_SCORE: i64 = 60;
pub const BUDGETER_MEMORY_SCORE: i64 = 55;
pub const BUDGETER_DIAG_SCORE: i64 = 50;
pub const BUDGETER_PER_ITEM_CHARS: usize = 1800;
pub const BUDGETER_ACTIVE_CHARS: usize = 4_000;
pub const BUDGETER_OPEN_DOC_CHARS: usize = 2_000;

/// Batch read-before-edit guard label.
pub const F6_BLOCKED_MESSAGE: &str =
    "{} blocked (F6): the Read of {} was issued in the same response as this edit. \
     The model could not have seen the file contents. Read the file in a prior turn, \
     then retry the edit.";

/// Read-before-edit guard label.
pub const READ_BEFORE_EDIT_MESSAGE: &str =
    "{} blocked: read {raw_path} before editing it. Call Read (or InspectFile) on this file first, then retry the edit so the change is based on its current contents.";

/// Max injections per turn.
pub const MAX_INJECTIONS_PER_TURN: usize = 16;

/// Subagent progress throttle (ms).
pub const SUBAGENT_PROGRESS_THROTTLE_MS: u64 = 300;

/// Background shell job cap.
pub const SHELL_BACKGROUND_TIMEOUT_INTERACTIVE: u64 = 1800;
pub const SHELL_BACKGROUND_TIMEOUT_SUBAGENT: u64 = 300;
pub const SHELL_DEFAULT_TIMEOUT: u64 = 600;
pub const SHELL_MIN_TIMEOUT: u64 = 5;

/// TaskWait timeout.
pub const TASK_WAIT_DEFAULT_TIMEOUT: u64 = 600;
pub const TASK_WAIT_MIN_TIMEOUT: u64 = 5;
pub const TASK_WAIT_MAX_TIMEOUT: u64 = 1800;

/// Poll interval for background operations.
pub const POLL_INTERVAL_MS: u64 = 300;

/// Browser output cap.
pub const BROWSER_MAX_OUTPUT: usize = 24_000;

/// ImpactAnalysis ceiling.
pub const IMPACT_MAX_AFFECTED_FILES: usize = 120;

/// Context source defaults.
pub const CONTEXT_DEFAULT_MAX_RESULTS: usize = 24;
pub const CONTEXT_DEFAULT_MAX_FILES: usize = 5000;
