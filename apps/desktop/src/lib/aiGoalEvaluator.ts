import type { AiChatMessage } from "./aiChatTypes";
import { filterVisibleChatMessages } from "./aiChatGoalOrchestration";
import { requestChatCompletion, type ChatCompletionMessage } from "./aiChatTransport";
import type { AiAgentMode, AiModelConfig, AiProviderConfig } from "./aiPreferences";
import { truncateText } from "./aiRuntimeShared";
import { listAiSessionTodos } from "./aiSessionTodos";
import {
  applyGoalEvaluatorVerdict,
  evaluateGoalRunContinuation,
  getActiveGoalRun,
  goalEvaluationToContinuation,
  lastAssistantMessage,
  setGoalRunEvaluatorPending,
  type GoalRunContinuationDecision,
} from "./aiSessionGoalRun";

export type GoalEvaluatorVerdict = {
  satisfied: boolean;
  blocked: boolean;
  reason: string;
  source: "model" | "heuristic" | "marker";
};

export function buildGoalEvaluatorTranscript(messages: AiChatMessage[], maxChars = 12_000) {
  const visible = filterVisibleChatMessages(messages);
  const assistant = lastAssistantMessage(visible) ?? lastAssistantMessage(messages);
  if (!assistant) return "No assistant turn yet.";
  const parts: string[] = [];
  if (assistant.content.trim()) {
    parts.push(`Assistant reply:\n${truncateText(assistant.content.trim(), 6_000)}`);
  }
  const tools = assistant.toolCalls?.filter((call) => call.output || call.error) ?? [];
  if (tools.length > 0) {
    parts.push(`Tool results (${tools.length}):\n${tools.slice(-12).map((call) => {
      const detail = call.error ? `error: ${truncateText(call.error, 400)}` : truncateText(call.output ?? "", 600);
      return `- ${call.tool} (${call.status}): ${detail}`;
    }).join("\n")}`);
  }
  const tail = visible.slice(-4, -1).filter((entry) => entry.role === "user" && entry.content.trim());
  if (tail.length > 0) {
    parts.push(`Recent visible user context:\n${tail.map((entry) => `- ${truncateText(entry.content.trim(), 240)}`).join("\n")}`);
  }
  return truncateText(parts.join("\n\n"), maxChars);
}

export async function requestGoalEvaluatorVerdict(input: {
  condition: string;
  messages: AiChatMessage[];
  provider: AiProviderConfig;
  model: AiModelConfig;
  selectedEffortId: string;
  abortSignal: AbortSignal;
  openTodoSummaries?: string[];
}): Promise<GoalEvaluatorVerdict | null> {
  const transcript = buildGoalEvaluatorTranscript(input.messages);
  const todoBlock = input.openTodoSummaries?.length
    ? `Open TodoWrite tasks:\n${input.openTodoSummaries.slice(0, 8).map((line) => `- ${line}`).join("\n")}`
    : "Open TodoWrite tasks: none";
  const system = [
    "You evaluate whether a software agent completion condition is satisfied.",
    "You do NOT execute tools. Judge only from the transcript evidence the agent already surfaced.",
    "Return strict JSON with keys: satisfied (boolean), blocked (boolean), reason (string).",
    "satisfied=true only when the condition is fully met with evidence in the transcript.",
    "For smoke/test goals (words like test, smoke, demo, тест, демо): satisfied=true once the agent ran tools and reported verification — not full product delivery.",
    "blocked=true when user credentials, product decisions, or external input is required before continuing.",
    "If satisfied and blocked are both false, the worker should continue.",
    "Keep reason under 220 characters.",
  ].join("\n");
  const user = [
    `Completion condition:\n${input.condition.trim()}`,
    "",
    todoBlock,
    "",
    "Transcript:",
    transcript,
  ].join("\n");
  const messages: ChatCompletionMessage[] = [
    { role: "system", content: system },
    { role: "user", content: user },
  ];
  try {
    const response = await requestChatCompletion(
      {
        abortSignal: input.abortSignal,
        provider: input.provider,
        selectedEffortId: input.selectedEffortId,
        selectedModel: input.model,
      },
      messages,
      () => undefined,
      { toolsEnabled: false },
    );
    const content = extractAssistantText(response.body);
    const parsed = parseEvaluatorJson(content);
    if (!parsed) return null;
    return {
      satisfied: parsed.satisfied,
      blocked: parsed.blocked,
      reason: parsed.reason,
      source: "model",
    };
  } catch {
    return null;
  }
}

/** Sync heuristics first, then the same chat model judges whether to continue (Claude Code / Codex style). */
export async function evaluateGoalRunContinuationAfterTurn(input: {
  sessionId: string;
  messages: AiChatMessage[];
  agentMode: AiAgentMode;
  provider: AiProviderConfig | null | undefined;
  selectedModel: AiModelConfig | null | undefined;
  selectedEffortId: string;
  abortSignal?: AbortSignal;
}): Promise<GoalRunContinuationDecision> {
  const sync = evaluateGoalRunContinuation(input.sessionId, input.messages, input.agentMode);
  if (!sync.continue) return sync;
  if (!input.provider || !input.selectedModel || input.abortSignal?.aborted) return sync;

  const run = getActiveGoalRun(input.sessionId);
  if (!run) return sync;

  setGoalRunEvaluatorPending(input.sessionId, true);
  const openTodoSummaries = listAiSessionTodos(input.sessionId)
    .filter((todo) => todo.status !== "completed" && todo.status !== "cancelled")
    .slice(0, 8)
    .map((todo) => todo.content.trim())
    .filter(Boolean);

  const verdict = await requestGoalEvaluatorVerdict({
    condition: run.goal,
    messages: input.messages,
    provider: input.provider,
    model: input.selectedModel,
    selectedEffortId: input.selectedEffortId,
    abortSignal: input.abortSignal ?? new AbortController().signal,
    openTodoSummaries,
  });

  if (!getActiveGoalRun(input.sessionId)) return sync;
  if (input.abortSignal?.aborted) return sync;
  if (!verdict) return sync;

  return goalEvaluationToContinuation(applyGoalEvaluatorVerdict(input.sessionId, verdict));
}

function extractAssistantText(body: unknown) {
  if (!body || typeof body !== "object") return "";
  const choices = (body as { choices?: Array<{ message?: { content?: unknown } }> }).choices;
  const content = choices?.[0]?.message?.content;
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content.map((part) => {
      if (part && typeof part === "object" && "text" in part && typeof part.text === "string") return part.text;
      return "";
    }).join("\n");
  }
  return "";
}

function parseEvaluatorJson(content: string): { satisfied: boolean; blocked: boolean; reason: string } | null {
  const trimmed = content.trim();
  const fenced = trimmed.match(/```(?:json)?\s*([\s\S]*?)```/i)?.[1]?.trim() ?? trimmed;
  const start = fenced.indexOf("{");
  const end = fenced.lastIndexOf("}");
  if (start === -1 || end <= start) return null;
  try {
    const parsed = JSON.parse(fenced.slice(start, end + 1)) as Record<string, unknown>;
    const satisfied = parsed.satisfied === true;
    const blocked = parsed.blocked === true;
    const reason = typeof parsed.reason === "string" && parsed.reason.trim()
      ? parsed.reason.trim().slice(0, 280)
      : satisfied
        ? "Completion condition satisfied."
        : blocked
          ? "Blocked — needs user input."
          : "Condition not satisfied yet.";
    return { satisfied, blocked, reason };
  } catch {
    return null;
  }
}