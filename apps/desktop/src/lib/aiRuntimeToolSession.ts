import type { AiChatSendInput } from "./aiChatTypes";
import { applyGoalToolProgress } from "./aiSessionGoalRun";
import { getAiSessionGoal, setAiSessionGoal } from "./aiSessionGoal";
import { sanitizeSessionGoal, sanitizeSessionTodos } from "./aiSessionOrchestrationSanitize";
import { replaceAiSessionTodos } from "./aiSessionTodos";
import { spawnSubagent, resolveSubagentType } from "./aiSubagents";
import { countRunningSubagents } from "./aiSubagentRuns";
import { resolveMaxParallelSubagents } from "./aiSubagentPolicy";
import { booleanArg, clamp, isRecord, numberArg, stringArg, toolJson, topCounts, truncateText, type ToolResult, type UnknownRecord } from "./aiRuntimeShared";
import { luxCommands } from "./tauri";

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