import { requestChatCompletion } from "./aiChatTransport";
import type { AiChatMessage, AiChatToolCall } from "./aiChatTypes";
import type { AiModelConfig, AiProviderConfig } from "./aiPreferences";
import { normalizeVisibleReasoning } from "./aiChatReasoning";
import {
  clampContextAutoCompactThreshold,
  resolveContextCompactTriggerTokens,
  resolveModelContextTokens,
} from "./aiModelContext";
import { firstChoice, type ChatCompletionMessage } from "./aiChatTransport";
import { getAiSessionGoal } from "./aiSessionGoal";
import { listAiSessionTodos } from "./aiSessionTodos";
import { truncateText } from "./aiRuntimeShared";
import { isTauriRuntime, luxCommands } from "./tauri";

export const COMPACTION_CHECKPOINT_MARKER = "[Lux · context compacted]";
export const TOOL_OUTPUT_PRUNE_MARKER = "[Lux · tool output pruned";
export const PRESERVE_RECENT_MESSAGE_COUNT = 8;
export const MIN_MESSAGES_FOR_COMPACTION = 8;
/** OpenCode-style prune: keep full tool output only on the latest N assistant turns. */
export const PRESERVE_FULL_TOOL_OUTPUT_ASSISTANT_TURNS = 3;
const MIN_TOOL_OUTPUT_CHARS_TO_PRUNE = 320;
const MIN_TOKEN_REDUCTION_RATIO = 0.08;
const COMPACTION_COOLDOWN_MS = 12_000;
const MAX_TRANSCRIPT_CHARS = 48_000;
const MAX_SUMMARY_CHARS = 12_000;

export type ContextCompactionDroppedItem = {
  kind: "message" | "tool-output" | "reasoning";
  label: string;
  tokens: number;
};

export type ContextCompactionState = {
  generation: number;
  fingerprint: string;
  lastCompactedAt: number;
  tokensBefore: number;
  tokensAfter: number;
  droppedItems?: ContextCompactionDroppedItem[];
  droppedTokens?: number;
};

export type CompactChatHistoryInput = {
  chatSessionId?: string;
  messages: AiChatMessage[];
  compactionState: ContextCompactionState | null;
  model: AiModelConfig;
  provider: AiProviderConfig;
  selectedEffortId: string;
  threshold: number;
  autoCompactEnabled: boolean;
  force?: boolean;
  abortSignal?: AbortSignal;
};

export type CompactChatHistoryResult = {
  messages: AiChatMessage[];
  compactionState: ContextCompactionState | null;
  compacted: boolean;
  reason?: "below-threshold" | "too-few-messages" | "cooldown" | "no-reduction" | "same-fingerprint" | "already-checkpoint-only";
};

export function isCompactionCheckpointMessage(message: AiChatMessage) {
  return message.kind === "compaction-checkpoint" || message.content.trimStart().startsWith(COMPACTION_CHECKPOINT_MARKER);
}

export function estimateTokens(value: string) {
  const trimmed = value.trim();
  if (!trimmed) return 0;
  return Math.ceil(trimmed.length / 4);
}

export function estimateMessageTokens(message: AiChatMessage) {
  const reasoning = normalizeVisibleReasoning(message.reasoning) ?? "";
  const toolTokens = message.toolCalls?.reduce((sum, call) => sum
    + estimateTokens(call.tool)
    + estimateTokens(call.input ?? "")
    + estimateTokens(call.output ?? "")
    + estimateTokens(call.error ?? ""), 0) ?? 0;
  return estimateTokens(message.content) + estimateTokens(reasoning) + toolTokens;
}

export function estimateHistoryTokens(messages: AiChatMessage[]) {
  return messages.reduce((sum, message) => sum + estimateMessageTokens(message), 0);
}

export function shouldAutoCompactContext(params: {
  messages: AiChatMessage[];
  model: AiModelConfig;
  threshold: number;
  autoCompactEnabled: boolean;
  compactionState: ContextCompactionState | null;
}) {
  if (!params.autoCompactEnabled) return false;
  const nonCheckpoint = params.messages.filter((message) => !isCompactionCheckpointMessage(message));
  if (nonCheckpoint.length < MIN_MESSAGES_FOR_COMPACTION) return false;
  const tokens = estimateHistoryTokens(params.messages);
  const trigger = resolveContextCompactTriggerTokens(params.model, params.threshold);
  return tokens >= trigger;
}

export function buildCompactionFingerprint(messages: AiChatMessage[]) {
  const slice = messages.slice(0, Math.max(0, messages.length - PRESERVE_RECENT_MESSAGE_COUNT));
  return slice.map((message) => `${message.id}:${estimateMessageTokens(message)}`).join("|");
}

function pruneToolCallOutput(call: AiChatToolCall): AiChatToolCall {
  const output = call.output ?? "";
  if (!output || output.includes(TOOL_OUTPUT_PRUNE_MARKER) || output.length < MIN_TOOL_OUTPUT_CHARS_TO_PRUNE) {
    return call;
  }
  const tokens = estimateTokens(output);
  return {
    ...call,
    output: `${TOOL_OUTPUT_PRUNE_MARKER} · ~${tokens} tokens · re-run the tool to reload the full payload]`,
  };
}

function pruneAssistantMessageToolOutputs(message: AiChatMessage): AiChatMessage {
  const toolCalls = message.toolCalls?.map(pruneToolCallOutput);
  const segments = message.segments?.map((segment) => {
    if (segment.kind !== "tool") return segment;
    const pruned = pruneToolCallOutput(segment.toolCall);
    return pruned === segment.toolCall ? segment : { ...segment, toolCall: pruned };
  });
  const changedTools = toolCalls && toolCalls.some((call, index) => call !== message.toolCalls?.[index]);
  const changedSegments = segments && segments.some((segment, index) => segment !== message.segments?.[index]);
  if (!changedTools && !changedSegments) return message;
  return {
    ...message,
    ...(toolCalls ? { toolCalls } : {}),
    ...(segments ? { segments } : {}),
  };
}

/** Drop bulky tool outputs from older assistant turns (OpenCode `compaction.prune` pattern). */
export function pruneStaleToolOutputs(
  messages: AiChatMessage[],
  preserveRecentAssistantTurns = PRESERVE_FULL_TOOL_OUTPUT_ASSISTANT_TURNS,
): AiChatMessage[] {
  const assistantIndices: number[] = [];
  for (let index = 0; index < messages.length; index += 1) {
    if (messages[index]?.role === "assistant") assistantIndices.push(index);
  }
  if (assistantIndices.length <= preserveRecentAssistantTurns) return messages;
  const cutoffIndex = assistantIndices[assistantIndices.length - preserveRecentAssistantTurns - 1] ?? -1;
  if (cutoffIndex < 0) return messages;

  let changed = false;
  const next = messages.map((message, index) => {
    if (index > cutoffIndex || message.role !== "assistant") return message;
    const pruned = pruneAssistantMessageToolOutputs(message);
    if (pruned !== message) changed = true;
    return pruned;
  });
  return changed ? next : messages;
}

export function pruneReducedTokenEstimate(before: AiChatMessage[], after: AiChatMessage[]) {
  const left = estimateHistoryTokens(before);
  const right = estimateHistoryTokens(after);
  return left > 0 && right < left ? left - right : 0;
}

export async function compactChatHistory(input: CompactChatHistoryInput): Promise<CompactChatHistoryResult> {
  const prunedMessages = pruneStaleToolOutputs(input.messages);
  const threshold = clampContextAutoCompactThreshold(input.threshold);
  const triggerTokens = resolveContextCompactTriggerTokens(input.model, threshold);
  const tokensBefore = estimateHistoryTokens(prunedMessages);
  const force = input.force === true;
  const eligible = prunedMessages.filter((message) => !isCompactionCheckpointMessage(message));

  if (eligible.length < MIN_MESSAGES_FOR_COMPACTION) {
    const onlyPruned = prunedMessages !== input.messages;
    return {
      messages: onlyPruned ? prunedMessages : input.messages,
      compactionState: input.compactionState,
      compacted: onlyPruned,
      reason: onlyPruned ? undefined : "too-few-messages",
    };
  }

  if (!force && !input.autoCompactEnabled && tokensBefore < triggerTokens) {
    const onlyPruned = prunedMessages !== input.messages;
    return {
      messages: onlyPruned ? prunedMessages : input.messages,
      compactionState: input.compactionState,
      compacted: onlyPruned,
      reason: onlyPruned ? undefined : "below-threshold",
    };
  }

  if (!force && tokensBefore < triggerTokens) {
    const onlyPruned = prunedMessages !== input.messages;
    return {
      messages: onlyPruned ? prunedMessages : input.messages,
      compactionState: input.compactionState,
      compacted: onlyPruned,
      reason: onlyPruned ? undefined : "below-threshold",
    };
  }

  const preserveFrom = Math.max(0, prunedMessages.length - PRESERVE_RECENT_MESSAGE_COUNT);
  const older = prunedMessages.slice(0, preserveFrom).filter((message) => !isCompactionCheckpointMessage(message));
  const recent = prunedMessages.slice(preserveFrom);

  if (older.length === 0) {
    const onlyPruned = prunedMessages !== input.messages;
    return {
      messages: onlyPruned ? prunedMessages : input.messages,
      compactionState: input.compactionState,
      compacted: onlyPruned,
      reason: onlyPruned ? undefined : "already-checkpoint-only",
    };
  }

  const fingerprint = buildCompactionFingerprint(prunedMessages);
  const now = Date.now();
  if (!force && input.compactionState) {
    if (input.compactionState.fingerprint === fingerprint) {
      const onlyPruned = prunedMessages !== input.messages;
      return {
        messages: onlyPruned ? prunedMessages : input.messages,
        compactionState: input.compactionState,
        compacted: onlyPruned,
        reason: onlyPruned ? undefined : "same-fingerprint",
      };
    }
    if (now - input.compactionState.lastCompactedAt < COMPACTION_COOLDOWN_MS) {
      const onlyPruned = prunedMessages !== input.messages;
      return {
        messages: onlyPruned ? prunedMessages : input.messages,
        compactionState: input.compactionState,
        compacted: onlyPruned,
        reason: onlyPruned ? undefined : "cooldown",
      };
    }
  }

  const existingCheckpoint = prunedMessages.find(isCompactionCheckpointMessage);
  const previousSummary = existingCheckpoint
    ? extractCheckpointSummary(existingCheckpoint.content)
    : "";

  const transcript = buildCompactionTranscript(older);
  let summary = "";
  try {
    summary = await summarizeCompactionTranscript({
      transcript,
      previousSummary,
      sessionId: input.chatSessionId,
      provider: input.provider,
      model: input.model,
      selectedEffortId: input.selectedEffortId,
      abortSignal: input.abortSignal,
    });
  } catch {
    summary = buildDeterministicCompactionSummary(older, previousSummary, input.chatSessionId);
  }

  if (!summary.trim()) {
    summary = buildDeterministicCompactionSummary(older, previousSummary, input.chatSessionId);
  }

  const checkpoint: AiChatMessage = {
    id: crypto.randomUUID(),
    role: "user",
    kind: "compaction-checkpoint",
    content: formatCompactionCheckpointContent(summary, older.length + (existingCheckpoint ? estimateCheckpointCoveredCount(existingCheckpoint.content) : 0)),
    timestamp: Date.now(),
  };

  const nextMessages = [checkpoint, ...recent];
  const tokensAfter = estimateHistoryTokens(nextMessages);

  if (!force && tokensBefore > 0 && tokensAfter >= tokensBefore * (1 - MIN_TOKEN_REDUCTION_RATIO)) {
    const onlyPruned = prunedMessages !== input.messages;
    return {
      messages: onlyPruned ? prunedMessages : input.messages,
      compactionState: input.compactionState,
      compacted: onlyPruned,
      reason: onlyPruned ? undefined : "no-reduction",
    };
  }

  const droppedItems = buildCompactionDroppedReport(older);
  const droppedTokens = droppedItems.reduce((sum, item) => sum + item.tokens, 0);
  const compactionState: ContextCompactionState = {
    generation: (input.compactionState?.generation ?? 0) + 1,
    fingerprint: buildCompactionFingerprint(nextMessages),
    lastCompactedAt: now,
    tokensBefore,
    tokensAfter,
    droppedItems,
    droppedTokens,
  };

  return { messages: nextMessages, compactionState, compacted: true };
}

function formatCompactionCheckpointContent(summary: string, coveredCount: number) {
  return [
    COMPACTION_CHECKPOINT_MARKER,
    `covered_messages=${coveredCount}`,
    "Continue the same task from this checkpoint. Older turns were compressed; use tools if you need exact file contents or command output.",
    "",
    summary.trim(),
  ].join("\n");
}

function extractCheckpointSummary(content: string) {
  const lines = content.split("\n");
  const start = lines.findIndex((line) => line.trim() && !line.startsWith(COMPACTION_CHECKPOINT_MARKER) && !line.startsWith("covered_messages=") && !line.startsWith("Continue "));
  if (start < 0) return content.trim();
  return lines.slice(start).join("\n").trim();
}

function estimateCheckpointCoveredCount(content: string) {
  const match = content.match(/covered_messages=(\d+)/);
  return match ? Number(match[1]) : 0;
}

function buildCompactionTranscript(messages: AiChatMessage[]) {
  const parts: string[] = [];
  let used = 0;
  for (const message of messages) {
    const block = formatTranscriptMessage(message);
    if (used + block.length > MAX_TRANSCRIPT_CHARS) {
      parts.push("[... earlier turns truncated for summarization ...]");
      break;
    }
    parts.push(block);
    used += block.length;
  }
  return parts.join("\n\n");
}

function formatTranscriptMessage(message: AiChatMessage) {
  const role = message.role === "user" ? "User" : "Assistant";
  const reasoning = normalizeVisibleReasoning(message.reasoning);
  const chunks = [`### ${role}`];
  if (message.content.trim()) chunks.push(truncateText(message.content, 4_000));
  if (reasoning) chunks.push(`[reasoning]\n${truncateText(reasoning, 800)}`);
  const tools = message.toolCalls?.filter((call) => call.output || call.error).slice(-6) ?? [];
  if (tools.length > 0) {
    chunks.push(tools.map((call) => {
      const detail = call.error ? `error: ${truncateText(call.error, 400)}` : truncateText(call.output ?? "", 600);
      return `- ${call.tool} (${call.status}): ${detail}`;
    }).join("\n"));
  }
  return chunks.join("\n");
}

function buildDeterministicCompactionSummary(messages: AiChatMessage[], previousSummary: string, sessionId?: string) {
  const userMessages = messages.filter((message) => message.role === "user" && message.content.trim() && !isCompactionCheckpointMessage(message));
  const pinnedGoal = sessionId ? getAiSessionGoal(sessionId) : "";
  const taskGoal = pinnedGoal || (userMessages[0]?.content.trim() ?? "");
  const latestUser = userMessages[userMessages.length - 1]?.content.trim() ?? "";
  const tools = new Set<string>();
  const paths = new Set<string>();
  for (const message of messages) {
    for (const call of message.toolCalls ?? []) {
      tools.add(call.tool);
      extractPathsFromText(call.output ?? call.input ?? "").forEach((path) => paths.add(path));
    }
    extractPathsFromText(message.content).forEach((path) => paths.add(path));
  }
  const assistantNotes = messages
    .filter((message) => message.role === "assistant" && message.content.trim())
    .slice(-3)
    .map((message) => truncateText(message.content, 500));

  return [
    previousSummary ? `## Prior checkpoint\n${truncateText(previousSummary, 2_000)}` : "",
    "## Task goal",
    truncateText(taskGoal, 1_200),
    latestUser && latestUser !== taskGoal ? `## Latest user direction\n${truncateText(latestUser, 1_000)}` : "",
    "## Progress (recent assistant)",
    assistantNotes.length > 0 ? assistantNotes.join("\n---\n") : "(see preserved recent turns)",
    "## Tools used",
    [...tools].slice(0, 24).join(", ") || "(none recorded)",
    "## Paths touched",
    [...paths].slice(0, 32).join("\n") || "(none extracted)",
    sessionId && listAiSessionTodos(sessionId).length > 0
      ? `## Open tasks\n${listAiSessionTodos(sessionId).map((todo) => `- [${todo.status}] ${todo.content}`).join("\n")}`
      : "",
    "## Next step",
    "Resume from the preserved recent messages and complete the original task without re-discovering from scratch.",
  ].filter(Boolean).join("\n\n");
}

const pathPattern = /(?:^|[\s"'`(])([\w./\\-]+\.(?:ts|tsx|js|jsx|rs|py|go|java|css|html|md|json|yaml|yml|toml|sql|cs|cpp|h|vue|svelte))(?:$|[\s"'`,.)])/gi;

function extractPathsFromText(text: string) {
  const paths = new Set<string>();
  for (const match of text.matchAll(pathPattern)) {
    const path = match[1]?.replace(/\\/g, "/");
    if (path && path.length <= 180) paths.add(path);
  }
  return paths;
}

async function summarizeCompactionTranscript(input: {
  transcript: string;
  previousSummary: string;
  sessionId?: string;
  provider: AiProviderConfig;
  model: AiModelConfig;
  selectedEffortId: string;
  abortSignal?: AbortSignal;
}) {
  const system = [
    "You compress IDE pair-programming chat history into a durable checkpoint.",
    "Preserve: task goal, constraints, decisions, files/paths, tool outcomes, errors, and the exact next step.",
    "Do not invent facts. Do not add filler. Use markdown headings.",
    "Required sections: ## Task goal, ## Progress, ## Key decisions, ## Files and tools, ## Open items / next step",
    `Stay under ${Math.floor(MAX_SUMMARY_CHARS / 4)} tokens.`,
  ].join("\n");

  const pinnedGoal = input.sessionId ? getAiSessionGoal(input.sessionId) : "";
  const openTasks = input.sessionId ? listAiSessionTodos(input.sessionId) : [];

  // Native Rust path: summarization runs through the Rust transport.
  if (isTauriRuntime()) {
    const summary = await luxCommands.aiCompactionSummary({
      transcript: input.transcript,
      previousSummary: input.previousSummary,
      pinnedGoal,
      openTasks: openTasks.map((todo) => `[${todo.status}] ${todo.content}`),
      baseUrl: input.provider.baseUrl,
      apiKey: input.provider.apiKey || null,
      model: input.model.alias || input.model.id,
    });
    return truncateText(summary, MAX_SUMMARY_CHARS);
  }

  const userParts = [
    pinnedGoal ? `Pinned session goal:\n${truncateText(pinnedGoal, 2_000)}` : "",
    openTasks.length > 0 ? `Open tasks:\n${openTasks.map((todo) => `- [${todo.status}] ${todo.content}`).join("\n")}` : "",
    input.previousSummary ? `Previous checkpoint to merge:\n${truncateText(input.previousSummary, 4_000)}` : "",
    `Transcript (${input.transcript.length} chars):\n${truncateText(input.transcript, MAX_TRANSCRIPT_CHARS)}`,
  ].filter(Boolean);

  const messages: ChatCompletionMessage[] = [
    { role: "system", content: system },
    { role: "user", content: userParts.join("\n\n") },
  ];

  const abortSignal = input.abortSignal ?? new AbortController().signal;
  const response = await requestChatCompletion({
    abortSignal,
    provider: input.provider,
    selectedEffortId: input.selectedEffortId,
    selectedModel: input.model,
  }, messages, () => undefined, { toolsEnabled: false });

  const choice = firstChoice(response.body);
  const message = choice?.message;
  const rawContent = message && typeof message === "object" && "content" in message ? message.content : null;
  const content = typeof rawContent === "string" ? rawContent.trim() : "";
  if (!content) throw new Error("Compaction summary was empty.");
  return truncateText(content, MAX_SUMMARY_CHARS);
}

export function resolveContextUsageBudget(model: AiModelConfig | null | undefined) {
  return resolveModelContextTokens(model);
}

export function resolveAutoCompactThreshold(preferences: { contextAutoCompactThreshold: number }) {
  return clampContextAutoCompactThreshold(preferences.contextAutoCompactThreshold);
}

function buildCompactionDroppedReport(messages: AiChatMessage[]): ContextCompactionDroppedItem[] {
  const items: ContextCompactionDroppedItem[] = [];
  for (const message of messages) {
    const reasoning = normalizeVisibleReasoning(message.reasoning) ?? "";
    if (reasoning.trim()) {
      items.push({
        kind: "reasoning",
        label: `${message.role} reasoning`,
        tokens: estimateTokens(reasoning),
      });
    }
    const contentTokens = estimateTokens(message.content);
    if (contentTokens > 0) {
      items.push({
        kind: "message",
        label: `${message.role} message`,
        tokens: contentTokens,
      });
    }
    for (const call of message.toolCalls ?? []) {
      const toolText = [call.input, call.output, call.error].filter(Boolean).join("\n");
      const tokens = estimateTokens(toolText);
      if (tokens <= 0) continue;
      items.push({
        kind: "tool-output",
        label: `${call.tool} (${call.status})`,
        tokens,
      });
    }
  }
  return items
    .sort((left, right) => right.tokens - left.tokens)
    .slice(0, 12);
}