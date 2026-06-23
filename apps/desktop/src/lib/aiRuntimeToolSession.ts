import type { AiChatSendInput } from "./aiChatTypes";
import { applyGoalToolProgress } from "./aiSessionGoalRun";
import { getAiSessionGoal, setAiSessionGoal } from "./aiSessionGoal";
import { sanitizeSessionGoal, sanitizeSessionTodos } from "./aiSessionOrchestrationSanitize";
import { replaceAiSessionTodos } from "./aiSessionTodos";
import { spawnSubagent, resolveSubagentType } from "./aiSubagents";
import { countRunningSubagents } from "./aiSubagentRuns";
import { resolveMaxParallelSubagents } from "./aiSubagentPolicy";
import { booleanArg, clamp, isRecord, numberArg, stringArg, stringArrayArg, toolJson, topCounts, truncateText, type ToolResult, type UnknownRecord } from "./aiRuntimeShared";
import { luxCommands, type McpServerConfig } from "./tauri";

export type SessionTodoStatus = "pending" | "in_progress" | "completed" | "blocked" | "cancelled";

export type SessionTodoPriority = "low" | "medium" | "high";

export type SessionTodo = {
  id: string;
  content: string;
  status: SessionTodoStatus;
  priority: SessionTodoPriority;
  notes?: string;
};

export type RuntimeToolSession = {
  todos: SessionTodo[];
  subagentDepth: number;
  parentAgentId: string | null;
};

export function todoWrite(args: UnknownRecord, session: RuntimeToolSession, input: AiChatSendInput): ToolResult {
  const rawTodos = args.todos;
  if (!Array.isArray(rawTodos)) throw new Error("TodoWrite requires a todos array.");
  const todos = rawTodos.map(normalizeSessionTodo).filter((todo): todo is SessionTodo => Boolean(todo));
  if (todos.length === 0) throw new Error("TodoWrite requires at least one valid todo item.");
  const sanitized = sanitizeSessionTodos(todos.map((todo) => ({
    id: todo.id,
    content: todo.content,
    status: todo.status,
    priority: todo.priority,
    notes: todo.notes,
  })));
  if (!sanitized.ok) throw new Error(sanitized.reason);
  const acceptedTodos = sanitized.value.map((todo, index) => ({
    id: todo.id || `todo-${index + 1}`,
    content: todo.content,
    status: normalizeSessionTodoStatus(todo.status),
    priority: normalizeSessionTodoPriority(todo.priority),
    notes: todo.notes,
  }));
  session.todos = acceptedTodos;
  replaceAiSessionTodos(input.chatSessionId, acceptedTodos.map((todo) => ({
    id: todo.id,
    content: todo.content,
    status: todo.status,
    priority: todo.priority,
    source: "agent" as const,
    notes: todo.notes,
  })));
  const statusCounts = topCounts(acceptedTodos.map((todo) => todo.status), 8);
  const dropped = todos.length - acceptedTodos.length;
  return toolJson("TodoWrite", {
    count: acceptedTodos.length,
    statusCounts,
    todos: acceptedTodos,
    notes: [
      "This task list is scoped to the current AI response and does not modify workspace files.",
      ...(dropped > 0 ? [`Ignored ${dropped} invalid or duplicate item(s).`] : []),
    ],
  });
}

export function goalWrite(args: UnknownRecord, input: AiChatSendInput): ToolResult {
  const rawGoal = stringArg(args, "goal", "").trim();
  const rawStatus = stringArg(args, "status", "").trim().toLowerCase();
  const rawProgress = numberArg(args, "progress", Number.NaN);
  const summary = stringArg(args, "summary", "").trim();
  const previousGoal = getAiSessionGoal(input.chatSessionId);
  const hasProgress = Number.isFinite(rawProgress);
  const progress = hasProgress ? Math.min(100, Math.max(0, Math.round(rawProgress))) : undefined;

  if (rawGoal) {
    const sanitized = sanitizeSessionGoal(rawGoal);
    if (!sanitized.ok) {
      return toolJson("Goal", {
        goal: previousGoal || null,
        rejected: rawGoal,
        reason: sanitized.reason,
        notes: [
          "Session goal was not updated.",
          "Pin a short user-facing objective (1–2 sentences), not review headings or checklist rows.",
        ],
      });
    }
    setAiSessionGoal(input.chatSessionId, sanitized.value);
    applyGoalToolProgress(input.chatSessionId, {
      goal: sanitized.value,
      progress,
      status: rawStatus || undefined,
      summary: summary || undefined,
    });
    return toolJson("Goal", {
      goal: sanitized.value,
      progress: progress ?? null,
      status: rawStatus || null,
      summary: summary || null,
      notes: ["Session goal updated. Report progress each turn during /goal runs."],
    });
  }

  if (!hasProgress && !rawStatus) {
    return toolJson("Goal", {
      goal: previousGoal || null,
      notes: ["Provide goal and/or progress (0–100) and optional status completed."],
    });
  }

  applyGoalToolProgress(input.chatSessionId, {
    progress,
    status: rawStatus || undefined,
    summary: summary || undefined,
  });

  return toolJson("Goal", {
    goal: getAiSessionGoal(input.chatSessionId) || null,
    progress: progress ?? null,
    status: rawStatus || null,
    summary: summary || null,
    notes: ["Goal progress recorded for the orchestration rail."],
  });
}

/** Normalize an options/steps entry that may be a bare string or a structured object. */
function normalizeLabeledEntry(value: unknown, labelKey: string): { label: string; description: string; file: string } | null {
  if (typeof value === "string") {
    const label = value.trim();
    return label ? { label, description: "", file: "" } : null;
  }
  if (!isRecord(value)) return null;
  const label = stringArg(value, labelKey, "").trim();
  if (!label) return null;
  return {
    label,
    description: stringArg(value, "description", "").trim() || stringArg(value, "detail", "").trim(),
    file: stringArg(value, "file", "").trim(),
  };
}

/** Normalize a PresentPlan `alternatives` entry: a bare string or { option, tradeoff }. */
function normalizePlanDecision(value: unknown): { option: string; tradeoff: string } | null {
  if (typeof value === "string") {
    const option = value.trim();
    return option ? { option, tradeoff: "" } : null;
  }
  if (!isRecord(value)) return null;
  const option = stringArg(value, "option", "").trim();
  if (!option) return null;
  return { option, tradeoff: stringArg(value, "tradeoff", "").trim() };
}

/**
 * AskUser (browser/dev TS turn-loop). Registers the question card and suspends on
 * a resolver until the user answers or dismisses. In Automatic mode it never
 * blocks — it returns immediately telling the model to self-decide, matching the
 * native Rust behavior.
 */
export async function askUserTool(args: UnknownRecord, input: AiChatSendInput, callId: string): Promise<ToolResult> {
  const question = stringArg(args, "question", "").trim();
  if (!question) throw new Error("AskUser requires a non-empty question.");
  const detail = stringArg(args, "detail", "").trim();
  const htmlPreview = stringArg(args, "htmlPreview", "");
  const multiSelect = booleanArg(args, "multiSelect", false);
  const allowCustom = booleanArg(args, "allowCustom", true);
  const options = (Array.isArray(args.options) ? args.options : [])
    .map((entry) => normalizeLabeledEntry(entry, "label"))
    .filter((entry): entry is { label: string; description: string; file: string } => Boolean(entry))
    .slice(0, 10)
    .map((entry) => ({ label: entry.label, description: entry.description }));

  if (input.preferences.agentMode === "automatic") {
    const list = options.length > 0
      ? `\nOptions:\n${options.map((o) => (o.description ? `- ${o.label} — ${o.description}` : `- ${o.label}`)).join("\n")}`
      : "";
    return toolJson("AskUser", {
      autoAnswered: true,
      answer: `Automatic mode: no user is available to answer. Pick the best option for this repository, state the choice as an assumption, and continue.${list}`,
    });
  }

  const { registerPendingQuestion, waitForQuestionAnswer } = await import("./aiPendingQuestion");
  registerPendingQuestion({
    requestId: callId,
    turnId: callId,
    sessionId: input.chatSessionId,
    question,
    detail,
    options,
    multiSelect,
    allowCustom,
    htmlPreview,
  });
  const result = await waitForQuestionAnswer(callId);
  if (result.cancelled || !result.answer.trim()) {
    return toolJson("AskUser", {
      answer: "",
      dismissed: true,
      note: "User dismissed the question without answering. Proceed with your best judgment or ask again only if truly blocked.",
    });
  }
  return toolJson("AskUser", { answer: result.answer });
}

/**
 * PresentPlan (browser/dev TS turn-loop). Pins the plan as goal + task list and
 * registers the plan card. In Plan/Agent mode the user presses Start to execute;
 * in Automatic mode the card auto-starts (the model proceeds immediately).
 */
export async function presentPlanTool(args: UnknownRecord, input: AiChatSendInput, callId: string): Promise<ToolResult> {
  const title = stringArg(args, "title", "").trim() || "Plan";
  const summary = stringArg(args, "summary", "").trim();
  const steps = (Array.isArray(args.steps) ? args.steps : [])
    .map((entry) => normalizeLabeledEntry(entry, "title"))
    .filter((entry): entry is { label: string; description: string; file: string } => Boolean(entry))
    .slice(0, 40)
    .map((entry) => ({ title: entry.label, detail: entry.description, file: entry.file }));
  if (steps.length === 0) {
    throw new Error("PresentPlan requires at least one step (array of strings or { title, detail, file }).");
  }
  // Structured reasoning phases (think-mcp parity): key decision(s), failure
  // modes, and verification checks. Each is optional and accepts strings or objects.
  const alternatives = (Array.isArray(args.alternatives) ? args.alternatives : [])
    .map((entry) => normalizePlanDecision(entry))
    .filter((entry): entry is { option: string; tradeoff: string } => Boolean(entry))
    .slice(0, 8);
  const risks = stringArrayArg(args, "risks").map((r) => r.trim()).filter(Boolean).slice(0, 12);
  const verification = stringArrayArg(args, "verification").map((v) => v.trim()).filter(Boolean).slice(0, 12);

  setAiSessionGoal(input.chatSessionId, summary || title);
  replaceAiSessionTodos(input.chatSessionId, steps.map((step, index) => ({
    id: `plan-${index + 1}`,
    content: step.title,
    status: index === 0 ? "in_progress" as const : "pending" as const,
    priority: "medium" as const,
    source: "agent" as const,
    notes: step.detail || undefined,
  })));

  const { quality, coaching } = assessPlanQuality(title, summary, steps, alternatives, risks, verification);
  const autoStart = input.preferences.agentMode === "automatic";
  const { registerPendingPlan } = await import("./aiPendingPlan");
  registerPendingPlan({
    planId: `plan-${callId}`,
    turnId: callId,
    sessionId: input.chatSessionId,
    title,
    summary,
    steps,
    alternatives,
    risks,
    verification,
    quality,
    coaching,
    autoStart,
  });
  const baseGuidance = autoStart
    ? "Plan presented and auto-started (Automatic mode). Begin executing step 1 now; do not wait for confirmation."
    : "Plan presented to the user. Stop here and wait — the user will press Start to hand the plan to Agent mode. Do not begin editing yet.";
  const guidance = coaching.length === 0
    ? baseGuidance
    : `Plan quality ${quality.toFixed(2)}/1.0 — strengthen before/while executing: ${coaching.join(" ")}\n${baseGuidance}`;
  return toolJson("PresentPlan", { stepCount: steps.length, autoStart, quality, coaching, guidance });
}

/**
 * McpManage (browser/dev TS turn-loop). Mirrors the Rust dispatch: manages MCP
 * server configs, connects/disconnects, toggles enable, and reports live status.
 */
export async function mcpManageTool(args: UnknownRecord, _input: AiChatSendInput): Promise<ToolResult> {
  const action = stringArg(args, "action", "").trim().toLowerCase() || "list";
  const id = stringArg(args, "id", "").trim();
  const { isTauriRuntime, luxCommands, MCP_SERVERS_KEY } = await import("./tauri");

  if (!isTauriRuntime()) {
    return toolJson("McpManage", { error: "MCP management requires the desktop runtime." });
  }

  const readConfigs = async (): Promise<McpServerConfig[]> => {
    const value = await luxCommands.settingsGet("user", MCP_SERVERS_KEY).catch(() => null);
    return Array.isArray(value?.value) ? (value.value as McpServerConfig[]) : [];
  };

  switch (action) {
    case "list": {
      const [configured, live] = await Promise.all([readConfigs(), luxCommands.mcpStatus()]);
      return toolJson("McpManage", { configured, live });
    }
    case "add": {
      if (!id) throw new Error("McpManage add requires 'id'.");
      const command = stringArg(args, "command", "").trim();
      if (!command) throw new Error("McpManage add requires 'command'.");
      const serverArgs = stringArrayArg(args, "args").map((a) => a.trim()).filter(Boolean).slice(0, 64);
      const env = isRecord(args.env)
        ? (Object.fromEntries(Object.entries(args.env).filter(([, v]) => typeof v === "string")) as Record<string, string>)
        : {};
      const enabled = typeof args.enabled === "boolean" ? args.enabled : true;
      const name = stringArg(args, "name", "").trim() || id;
      const status = await luxCommands.mcpAdd({ id, name, command, args: serverArgs, env, enabled });
      return toolJson("McpManage", status);
    }
    case "connect":
    case "restart": {
      if (!id) throw new Error(`McpManage ${action} requires 'id'.`);
      const config = (await readConfigs()).find((c) => c.id === id);
      if (!config) throw new Error(`MCP server '${id}' not found in settings.`);
      const status = await luxCommands.mcpConnect(config);
      return toolJson("McpManage", status);
    }
    case "disconnect": {
      if (!id) throw new Error("McpManage disconnect requires 'id'.");
      await luxCommands.mcpDisconnect(id);
      return toolJson("McpManage", { id, state: "disconnected" });
    }
    case "enable":
    case "disable": {
      if (!id) throw new Error(`McpManage ${action} requires 'id'.`);
      await luxCommands.mcpEnable(id, action === "enable");
      return toolJson("McpManage", { id, enabled: action === "enable" });
    }
    case "remove": {
      if (!id) throw new Error("McpManage remove requires 'id'.");
      await luxCommands.mcpRemove(id);
      return toolJson("McpManage", { id, removed: true });
    }
    default:
      throw new Error(`Unknown McpManage action '${action}'. Use list|add|connect|restart|disconnect|enable|disable|remove.`);
  }
}

const PLAN_RISK_MARKERS = [
  "security", "secure", "auth", "password", "token", "payment", "billing",
  "concurren", "migrat", "schema", "distributed", "performance", "rollback",
  "delete", "destructive", "public api", "breaking",
];
const PLAN_VAGUE_LABELS = [
  "set up the project", "set up project", "setup", "implement business logic",
  "implement logic", "implement the feature", "add documentation", "write docs",
  "do the rest", "finish up", "wire everything", "make it work", "clean up",
  "testing", "polish",
];

/**
 * Deterministic, advisory plan-quality gate — mirror of the Rust `assess_plan_quality`
 * (itself a port of think-mcp's cycle gate). Scores the five reasoning phases
 * (decompose · alternative · critique · synthesis · verification) into a `[0,1]`
 * score plus concrete coaching nudges. Alternatives + critique are only expected on
 * non-trivial/risky work. Never blocks execution.
 */
function assessPlanQuality(
  title: string,
  summary: string,
  steps: { title: string; detail: string; file: string }[],
  alternatives: { option: string; tradeoff: string }[],
  risks: string[],
  verification: string[],
): { quality: number; coaching: string[] } {
  const coaching: string[] = [];
  const haystack = [
    title,
    summary,
    ...steps.flatMap((s) => [s.title, s.detail]),
    ...alternatives.flatMap((a) => [a.option, a.tradeoff]),
    ...risks,
    ...verification,
  ].join("\n").toLowerCase();
  const riskHits = PLAN_RISK_MARKERS.filter((m) => haystack.includes(m)).length;
  const requiredSteps = Math.min(3 + riskHits, 8);
  const expectsAlternatives = riskHits >= 1 || steps.length >= 5;
  const expectsCritique = riskHits >= 1 || steps.length >= 4;
  let score = 1.0;

  // 1. Decompose.
  if (steps.length < requiredSteps) {
    score -= 0.2;
    coaching.push(`Decompose further — ${steps.length} step(s) for ${riskHits > 0 ? "higher" : "this"}-risk work; aim for ~${requiredSteps}, each a concrete action on a named file/module.`);
  }

  // 2. Concreteness.
  const vague = steps.filter((s) => {
    const t = s.title.toLowerCase();
    return PLAN_VAGUE_LABELS.some((v) => t === v || t.startsWith(v));
  }).length;
  const withAnchor = steps.filter((s) => s.file !== "" || [...s.detail].length >= 24).length;
  if (vague > 0) {
    score -= 0.15;
    coaching.push(`Replace ${vague} vague step label(s) (e.g. 'implement logic', 'add documentation') with a specific action + its acceptance check.`);
  }
  if (steps.length >= 3 && withAnchor * 2 < steps.length) {
    score -= 0.1;
    coaching.push("Most steps lack a file or concrete detail — name the file/module each step touches and what proves it done.");
  }

  // 3. Alternative + synthesis — the key decision and why it wins.
  const hasDecision = alternatives.some((a) => a.option.trim() !== "")
    || ["instead of", "rather than", "trade-off", "tradeoff", "alternative", " vs ", "chose ", "chosen ", "decided ", "вместо", "альтернатив", "компромисс"].some((kw) => haystack.includes(kw));
  if (expectsAlternatives && !hasDecision) {
    score -= 0.2;
    coaching.push("Name the key decision: the approach you chose and why it wins over the alternative(s) (the tradeoff). A plan that weighs options beats one that charges ahead with its first idea.");
  }

  // 4. Critique — failure modes / hidden assumptions of the riskiest step.
  const hasCritique = risks.some((r) => r.trim() !== "")
    || ["risk", "fail", "assumption", "assume", "edge case", "race", "breaks if", "could break", "fallback", "риск", "провал", "допущен", "сломает"].some((kw) => haystack.includes(kw));
  if (expectsCritique && !hasCritique) {
    score -= 0.2;
    coaching.push("Critique the riskiest step: list its failure modes and hidden assumptions — what breaks, under what input/timing, and how you'd catch it.");
  }

  // 5. Verification.
  const hasVerification = verification.some((v) => v.trim() !== "")
    || steps.some((s) => {
      const t = `${s.title} ${s.detail}`.toLowerCase();
      return ["test", "verif", "build", "typecheck", "lint", "run ", "check", "assert", "validate"].some((kw) => t.includes(kw));
    });
  if (!hasVerification) {
    score -= 0.25;
    coaching.push("Add explicit verification: the tests/build/checks that prove it works (plus a rollback trigger for risky changes).");
  }

  // Rollback awareness for genuinely risky work.
  if (riskHits >= 2) {
    const hasRollback = ["rollback", "revert", "checkpoint", "backup"].some((kw) => haystack.includes(kw));
    if (!hasRollback) {
      score -= 0.1;
      coaching.push("High-risk plan: name a rollback/recovery path (Checkpoint, revert, or backup) for the riskiest step.");
    }
  }

  return { quality: Math.max(0, Math.min(1, score)), coaching };
}

export async function taskSubagentTool(args: UnknownRecord, input: AiChatSendInput, session: RuntimeToolSession): Promise<ToolResult> {
  const description = stringArg(args, "description", "").trim();
  const prompt = stringArg(args, "prompt", "").trim();
  if (!description || !prompt) throw new Error("Task requires description and prompt.");
  const maxParallel = resolveMaxParallelSubagents(input.preferences);
  if (countRunningSubagents(input.chatSessionId) >= maxParallel) {
    throw new Error(`Parallel subagent limit reached (${maxParallel}). Wait for a running Task to finish or cancel it.`);
  }
  const subagentType = resolveSubagentType(stringArg(args, "subagent_type", "generalPurpose"));
  const result = await spawnSubagent({
    parentInput: input,
    description,
    prompt,
    subagentType,
    model: stringArg(args, "model", "").trim() || undefined,
    depth: session.subagentDepth,
    parentAgentId: session.parentAgentId ?? undefined,
  });
  return toolJson("Task", {
    agentId: result.agentId,
    subagentType: result.subagentType,
    depth: result.depth,
    parentAgentId: result.parentAgentId,
    childAgentIds: result.childAgentIds,
    summary: result.summary,
    resume: stringArg(args, "resume", "").trim() || null,
  });
}

/**
 * Agent-to-agent (A2A) coordination tool. The main agent and its subagents share
 * one per-session blackboard (in the Rust runtime) so they can hand off findings
 * and decisions without routing everything through the parent's context window.
 */
export async function agentMessageTool(
  args: UnknownRecord,
  input: AiChatSendInput,
  session: RuntimeToolSession,
): Promise<ToolResult> {
  const sessionId = input.chatSessionId;
  if (!sessionId) throw new Error("AgentMessage requires an active chat session.");
  const action = stringArg(args, "action", "post").trim().toLowerCase();

  if (action === "read") {
    const topic = stringArg(args, "topic", "").trim();
    const limit = clamp(numberArg(args, "limit", 30), 1, 200);
    const entries = await luxCommands.aiBlackboardRead(sessionId, topic || null, limit);
    return toolJson("AgentMessage", {
      action: "read",
      topic: topic || null,
      count: entries.length,
      messages: entries.map((entry) => ({
        author: entry.author,
        topic: entry.topic,
        content: entry.content,
        timestampMs: entry.timestampMs,
      })),
      notes: entries.length === 0
        ? ["No agent messages on this board yet for the requested topic."]
        : undefined,
    });
  }

  if (action && action !== "post") {
    throw new Error(`Unknown AgentMessage action: ${action}. Use "post" or "read".`);
  }

  const content = stringArg(args, "content", "").trim();
  if (!content) throw new Error("AgentMessage post requires non-empty content.");
  const topic = stringArg(args, "topic", "general").trim() || "general";
  const author = agentAuthorLabel(input, session);
  const entry = await luxCommands.aiBlackboardPost(sessionId, author, topic, content);
  return toolJson("AgentMessage", {
    action: "post",
    posted: { id: entry.id, author: entry.author, topic: entry.topic, timestampMs: entry.timestampMs },
    notes: ["Other agents in this session can read this via AgentMessage with action=read."],
  });
}

function agentAuthorLabel(input: AiChatSendInput, session: RuntimeToolSession): string {
  const base = input.selectedAgentName?.trim() || "main agent";
  if (session.subagentDepth > 0) {
    const tag = session.parentAgentId ? session.parentAgentId.slice(-6) : String(session.subagentDepth);
    return `${base} · sub#${tag}`;
  }
  return base;
}

function normalizeSessionTodo(value: unknown, index: number): SessionTodo | null {
  if (!isRecord(value)) return null;
  const content = typeof value.content === "string" ? value.content.trim() : "";
  if (!content) return null;
  const id = typeof value.id === "string" && value.id.trim() ? value.id.trim() : `todo-${index + 1}`;
  const status = normalizeSessionTodoStatus(value.status);
  const priority = normalizeSessionTodoPriority(value.priority);
  const notes = typeof value.notes === "string" && value.notes.trim() ? truncateText(value.notes.trim(), 500) : undefined;
  return { id, content: truncateText(content, 500), status, priority, notes };
}

function normalizeSessionTodoStatus(value: unknown): SessionTodoStatus {
  const normalized = typeof value === "string" ? value.toLowerCase().replace(/[-\s]+/g, "_") : "";
  switch (normalized) {
    case "in_progress":
    case "completed":
    case "blocked":
    case "cancelled":
      return normalized;
    default:
      return "pending";
  }
}

function normalizeSessionTodoPriority(value: unknown): SessionTodoPriority {
  const normalized = typeof value === "string" ? value.toLowerCase() : "";
  switch (normalized) {
    case "low":
    case "high":
      return normalized;
    default:
      return "medium";
  }
}