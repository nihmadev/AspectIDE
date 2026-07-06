import { reasoningPayload, requestChatCompletion } from "./aiChatTransport";
import type { AiChatMessage, AiChatToolCall } from "./aiChatTypes";
import { resolveModelProtocol, type AiModelConfig, type AiProviderConfig } from "./aiPreferences";
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
export const PRESERVE_RECENT_MESSAGE_COUNT = 10;
export const MIN_MESSAGES_FOR_COMPACTION = 10;
/**
 * Manual ("Compact now" / `/compact`) compaction must work at any moment, even on
 * a one-message chat. Forced runs use a floor of 1 and an ADAPTIVE preserve window
 * (never larger than `eligible - 1`), so there is always at least one older message
 * to summarize instead of refusing with "too few messages" — or, the subtler cliff,
 * refusing with "already-checkpoint-only" because a fixed preserve window swallowed
 * the whole short transcript. Auto-compaction keeps the larger, conservative values
 * above.
 */
export const FORCED_PRESERVE_RECENT_MESSAGE_COUNT = 5;
export const FORCED_MIN_MESSAGES_FOR_COMPACTION = 1;
/** OpenCode-style prune: keep full tool output only on the latest N assistant turns. */
export const PRESERVE_FULL_TOOL_OUTPUT_ASSISTANT_TURNS = 3;
const MIN_TOOL_OUTPUT_CHARS_TO_PRUNE = 320;
const MIN_TOKEN_REDUCTION_RATIO = 0.08;
const COMPACTION_COOLDOWN_MS = 12_000;
// The preserved recent window is bounded by BOTH a message count and a token
// budget (this fraction of the trigger). Without the token bound a few large
// recent messages (big tool outputs, pasted files) stay in the preserved window
// and defeat compaction — the whole point is to summarize the bulk, so we shrink
// the window from the front until it fits the budget (never below the floor).
const PRESERVE_TOKEN_BUDGET_RATIO = 0.35;
const MIN_PRESERVED_MESSAGES = 2;
const MAX_TRANSCRIPT_CHARS = 84_000;
const MAX_SUMMARY_CHARS = 18_000;
const MAX_PREVIOUS_SUMMARY_CHARS = 6_000;
const MAX_PINNED_GOAL_CHARS = 3_000;
const MAX_MESSAGE_CONTENT_CHARS = 8_000;
const MAX_REASONING_CHARS = 1_200;
const MAX_TOOL_OUTPUT_CHARS = 1_200;
const MAX_TOOL_INPUT_CHARS = 900;
const MAX_CRITICAL_ITEMS = 80;
const COMPACTION_CHECKPOINT_INSTRUCTION =
  "Continue the same task from this checkpoint. Older turns were compressed losslessly for intent/state; use tools if exact file contents or full command output are needed.";
const LATIN_CHARS_PER_TOKEN = 4.0;
const CYRILLIC_CHARS_PER_TOKEN = 2.7;
const NUMBER_CHARS_PER_TOKEN = 3.0;
const SYMBOL_CHARS_PER_TOKEN = 2.0;
const CJK_CHARS_PER_TOKEN = 1.0;
const TOKEN_ESTIMATE_SAFETY_MULTIPLIER = 1.08;
const TOKEN_ESTIMATE_MESSAGE_OVERHEAD = 4;

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

type CompactionMemory = {
  goal: string;
  latestUserDirection: string;
  openTasks: string[];
  criticalItems: string[];
  paths: string[];
  tools: string[];
  errors: string[];
  decisions: string[];
  progress: string[];
};

export function isCompactionCheckpointMessage(message: AiChatMessage) {
  return message.kind === "compaction-checkpoint" || message.content.trimStart().startsWith(COMPACTION_CHECKPOINT_MARKER);
}

export function estimateTokens(value: string) {
  const text = value.trim();
  if (!text) return 0;

  // `chars / 4` was too crude for real IDE context: Cyrillic, JSON/code symbols,
  // paths and logs have very different token density. This lightweight estimator
  // mirrors tokenizer behavior better without bundling a model-specific tokenizer.
  const latin = text.match(/[A-Za-z_]+/g)?.join("").length ?? 0;
  const cyrillic = text.match(/[\u0400-\u04FF]+/g)?.join("").length ?? 0;
  const cjk = text.match(/[\u3040-\u30FF\u3400-\u9FFF\uAC00-\uD7AF]/g)?.length ?? 0;
  const numbers = text.match(/\d+/g)?.join("").length ?? 0;
  const whitespace = text.match(/\s+/g)?.join("").length ?? 0;
  const knownChars = latin + cyrillic + cjk + numbers + whitespace;
  const symbols = Math.max(0, text.length - knownChars);
  const rawTokens = (latin / LATIN_CHARS_PER_TOKEN)
    + (cyrillic / CYRILLIC_CHARS_PER_TOKEN)
    + (cjk / CJK_CHARS_PER_TOKEN)
    + (numbers / NUMBER_CHARS_PER_TOKEN)
    + (symbols / SYMBOL_CHARS_PER_TOKEN);

  return Math.max(1, Math.ceil(rawTokens * TOKEN_ESTIMATE_SAFETY_MULTIPLIER));
}

const messageTokenCache = new WeakMap<AiChatMessage, number>();

export function estimateMessageTokens(message: AiChatMessage) {
  const cached = messageTokenCache.get(message);
  if (cached !== undefined) return cached;
  const reasoning = normalizeVisibleReasoning(message.reasoning) ?? "";
  const toolTokens = message.toolCalls?.reduce((sum, call) => sum
    + estimateTokens(call.tool)
    + estimateTokens(call.input ?? "")
    + estimateTokens(call.output ?? "")
    + estimateTokens(call.error ?? ""), 0) ?? 0;
  const total = TOKEN_ESTIMATE_MESSAGE_OVERHEAD + estimateTokens(message.content) + estimateTokens(reasoning) + toolTokens;
  messageTokenCache.set(message, total);
  return total;
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
  const tokens = estimateHistoryTokens(pruneStaleToolOutputs(params.messages));
  const trigger = resolveContextCompactTriggerTokens(params.model, params.threshold);
  return tokens >= trigger;
}

export function buildCompactionFingerprint(messages: AiChatMessage[], preserveCount = PRESERVE_RECENT_MESSAGE_COUNT) {
  const slice = messages.slice(0, Math.max(0, messages.length - preserveCount));
  return slice.map((message) => `${message.id}:${estimateMessageTokens(message)}`).join("|");
}

function pruneToolCallOutput(call: AiChatToolCall): AiChatToolCall {
  const output = call.output ?? "";
  if (!output || output.includes(TOOL_OUTPUT_PRUNE_MARKER) || output.length < MIN_TOOL_OUTPUT_CHARS_TO_PRUNE) {
    return call;
  }
  const tokens = estimateTokens(output);
  const head = truncateText(output.trim(), 700);
  return {
    ...call,
    output: `${TOOL_OUTPUT_PRUNE_MARKER} · ~${tokens} tokens · preview:\n${head}\n[re-run the tool to reload the full payload]`,
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

/**
 * Index to preserve FROM: keeps the most recent messages bounded by both
 * `maxCount` and `tokenBudget`. Walks backward from the end, always keeping at
 * least `min(maxCount, MIN_PRESERVED_MESSAGES)`, then stops once adding the next
 * older message would exceed the token budget. This guarantees the older bulk is
 * summarized even when recent messages are individually huge.
 */
export function resolvePreserveWindow(messages: AiChatMessage[], maxCount: number, tokenBudget: number): number {
  const floor = Math.min(maxCount, MIN_PRESERVED_MESSAGES);
  let count = 0;
  let tokens = 0;
  let index = messages.length;
  while (index > 0 && count < maxCount) {
    const messageTokens = estimateMessageTokens(messages[index - 1]);
    if (count >= floor && tokens + messageTokens > tokenBudget) break;
    tokens += messageTokens;
    count += 1;
    index -= 1;
  }
  return Math.max(0, index);
}

export async function compactChatHistory(input: CompactChatHistoryInput): Promise<CompactChatHistoryResult> {
  const prunedMessages = pruneStaleToolOutputs(input.messages);
  const threshold = clampContextAutoCompactThreshold(input.threshold);
  const triggerTokens = resolveContextCompactTriggerTokens(input.model, threshold);
  const tokensBefore = estimateHistoryTokens(prunedMessages);
  const force = input.force === true;
  const minMessages = force ? FORCED_MIN_MESSAGES_FOR_COMPACTION : MIN_MESSAGES_FOR_COMPACTION;
  const eligible = prunedMessages.filter((message) => !isCompactionCheckpointMessage(message));
  // Forced runs shrink the preserve window so at least one eligible message always
  // lands in the summarized slice — a fixed window on a short chat preserved
  // everything and bailed with "already-checkpoint-only" instead of compacting.
  const preserveCount = force
    ? Math.min(FORCED_PRESERVE_RECENT_MESSAGE_COUNT, Math.max(0, eligible.length - 1))
    : PRESERVE_RECENT_MESSAGE_COUNT;

  if (eligible.length < minMessages) {
    const onlyPruned = prunedMessages !== input.messages;
    return {
      messages: onlyPruned ? prunedMessages : input.messages,
      compactionState: input.compactionState,
      compacted: onlyPruned,
      reason: onlyPruned ? undefined : "too-few-messages",
    };
  }

  // Auto-compaction disabled: a non-forced run must never summarize, even when the
  // transcript is over the trigger threshold. The previous `&& tokensBefore < triggerTokens`
  // condition let an over-threshold disabled session fall straight through and compact
  // anyway. Cheap tool-output pruning still applies; only model summarization is gated.
  if (!force && !input.autoCompactEnabled) {
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

  // Bound the preserved window by tokens as well as count, so large recent
  // messages can't keep the bulk of the context out of the summarized slice.
  const preserveBudget = Math.max(1, Math.floor(triggerTokens * PRESERVE_TOKEN_BUDGET_RATIO));
  const preserveFrom = resolvePreserveWindow(prunedMessages, preserveCount, preserveBudget);
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

  const fingerprint = buildCompactionFingerprint(prunedMessages, preserveCount);
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

  const memory = buildCompactionMemory(older, previousSummary, input.chatSessionId);
  const transcript = buildCompactionTranscript(older, memory);
  let summary = "";
  try {
    summary = await summarizeCompactionTranscript({
      transcript,
      previousSummary,
      memory,
      sessionId: input.chatSessionId,
      provider: input.provider,
      model: input.model,
      selectedEffortId: input.selectedEffortId,
      abortSignal: input.abortSignal,
    });
  } catch (err) {
    if (input.abortSignal?.aborted) throw err;
    summary = buildDeterministicCompactionSummary(older, previousSummary, input.chatSessionId, memory);
  }

  summary = mergeSummaryWithDeterministicSafetyNet(summary, older, previousSummary, input.chatSessionId, memory);

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
    const throttledState: ContextCompactionState = input.compactionState
      ? { ...input.compactionState, fingerprint, lastCompactedAt: now, tokensBefore, tokensAfter }
      : { generation: 0, fingerprint, lastCompactedAt: now, tokensBefore, tokensAfter };
    return {
      messages: onlyPruned ? prunedMessages : input.messages,
      compactionState: throttledState,
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
    COMPACTION_CHECKPOINT_INSTRUCTION,
    "",
    summary.trim(),
  ].join("\n");
}

function extractCheckpointSummary(content: string) {
  const lines = content.split("\n");
  const start = lines.findIndex((line) => line.trim() && !line.startsWith(COMPACTION_CHECKPOINT_MARKER) && !line.startsWith("covered_messages=") && line.trim() !== COMPACTION_CHECKPOINT_INSTRUCTION);
  if (start < 0) return content.trim();
  return lines.slice(start).join("\n").trim();
}

function estimateCheckpointCoveredCount(content: string) {
  const match = content.match(/covered_messages=(\d+)/);
  return match ? Number(match[1]) : 0;
}

function buildCompactionTranscript(messages: AiChatMessage[], memory: CompactionMemory) {
  const header = [
    "# Durable compaction source",
    "The summary must preserve all actionable state below. Prefer exact bullets over vague prose.",
    "",
    "## Extracted critical state",
    formatMemory(memory),
    "",
    "## Chronological transcript tail",
  ].join("\n");
  const parts: string[] = [];
  let used = header.length;
  for (const message of [...messages].reverse()) {
    const block = formatTranscriptMessage(message);
    if (used + block.length > MAX_TRANSCRIPT_CHARS) {
      parts.unshift("[... older low-signal turns omitted; critical state above was extracted before omission ...]");
      break;
    }
    parts.unshift(block);
    used += block.length;
  }
  return [header, parts.join("\n\n")].filter(Boolean).join("\n\n");
}

function formatTranscriptMessage(message: AiChatMessage) {
  const role = message.role === "user" ? "User" : "Assistant";
  const reasoning = normalizeVisibleReasoning(message.reasoning);
  const chunks = [`### ${role} · ${message.id}`];
  if (message.content.trim()) chunks.push(truncateText(message.content, MAX_MESSAGE_CONTENT_CHARS));
  if (reasoning) chunks.push(`[visible reasoning]\n${truncateText(reasoning, MAX_REASONING_CHARS)}`);
  const tools = message.toolCalls?.filter((call) => call.output || call.error || call.input).slice(-10) ?? [];
  if (tools.length > 0) {
    chunks.push("[tools]\n" + tools.map((call) => {
      const input = call.input ? ` input=${truncateText(call.input, MAX_TOOL_INPUT_CHARS)}` : "";
      const detail = call.error
        ? `error=${truncateText(call.error, MAX_TOOL_OUTPUT_CHARS)}`
        : `output=${truncateText(call.output ?? "", MAX_TOOL_OUTPUT_CHARS)}`;
      return `- ${call.tool} (${call.status})${input}: ${detail}`;
    }).join("\n"));
  }
  return chunks.join("\n");
}

function buildCompactionMemory(messages: AiChatMessage[], previousSummary: string, sessionId?: string): CompactionMemory {
  const userMessages = messages.filter((message) => message.role === "user" && message.content.trim() && !isCompactionCheckpointMessage(message));
  const pinnedGoal = sessionId ? getAiSessionGoal(sessionId) : "";
  const openTasks = sessionId ? listAiSessionTodos(sessionId).map((todo) => `[${todo.status}] ${todo.content}`) : [];
  const criticalItems = new Set<string>();
  const paths = new Set<string>();
  const tools = new Set<string>();
  const errors = new Set<string>();
  const decisions = new Set<string>();
  const progress = new Set<string>();

  if (previousSummary.trim()) criticalItems.add(`Previous checkpoint: ${truncateText(previousSummary.trim(), MAX_PREVIOUS_SUMMARY_CHARS)}`);
  if (pinnedGoal.trim()) criticalItems.add(`Pinned goal: ${truncateText(pinnedGoal.trim(), MAX_PINNED_GOAL_CHARS)}`);

  for (const message of messages) {
    const content = message.content.trim();
    extractPathsFromText(content).forEach((path) => paths.add(path));
    extractImportantLines(content).forEach((line) => {
      criticalItems.add(line);
      if (looksLikeDecision(line)) decisions.add(line);
      if (looksLikeError(line)) errors.add(line);
      if (looksLikeProgress(line)) progress.add(line);
    });
    for (const call of message.toolCalls ?? []) {
      tools.add(`${call.tool} (${call.status})`);
      extractPathsFromText(call.input ?? "").forEach((path) => paths.add(path));
      extractPathsFromText(call.output ?? "").forEach((path) => paths.add(path));
      if (call.error?.trim()) errors.add(`${call.tool}: ${truncateText(call.error.trim(), 700)}`);
      if (call.output?.trim()) {
        extractImportantLines(call.output).forEach((line) => {
          criticalItems.add(line);
          if (looksLikeError(line)) errors.add(line);
        });
      }
    }
  }

  return {
    goal: pinnedGoal || userMessages[0]?.content.trim() || "",
    latestUserDirection: userMessages[userMessages.length - 1]?.content.trim() || "",
    openTasks,
    criticalItems: [...criticalItems].slice(0, MAX_CRITICAL_ITEMS),
    paths: [...paths].slice(0, 80),
    tools: [...tools].slice(0, 60),
    errors: [...errors].slice(0, 40),
    decisions: [...decisions].slice(0, 40),
    progress: [...progress].slice(0, 40),
  };
}

function formatMemory(memory: CompactionMemory) {
  return [
    "### Goal",
    memory.goal || "(not recorded)",
    "### Latest user direction",
    memory.latestUserDirection || "(not recorded)",
    "### Open tasks",
    memory.openTasks.length ? memory.openTasks.map((item) => `- ${item}`).join("\n") : "(none recorded)",
    "### Decisions",
    formatBulletList(memory.decisions),
    "### Progress / completed facts",
    formatBulletList(memory.progress),
    "### Errors / blockers",
    formatBulletList(memory.errors),
    "### Files / paths",
    formatBulletList(memory.paths),
    "### Tools",
    formatBulletList(memory.tools),
    "### Other critical lines",
    formatBulletList(memory.criticalItems),
  ].join("\n");
}

function formatBulletList(items: string[]) {
  return items.length ? items.map((item) => `- ${truncateText(item, 1_200)}`).join("\n") : "(none recorded)";
}

function buildDeterministicCompactionSummary(messages: AiChatMessage[], previousSummary: string, sessionId?: string, memory = buildCompactionMemory(messages, previousSummary, sessionId)) {
  const assistantNotes = messages
    .filter((message) => message.role === "assistant" && message.content.trim())
    .slice(-6)
    .map((message) => truncateText(message.content, 900));

  return [
    "## Task goal",
    truncateText(memory.goal || "Continue the active user task.", 1_800),
    memory.latestUserDirection && memory.latestUserDirection !== memory.goal ? `## Latest user direction\n${truncateText(memory.latestUserDirection, 1_500)}` : "",
    previousSummary ? `## Prior checkpoint merged\n${truncateText(previousSummary, MAX_PREVIOUS_SUMMARY_CHARS)}` : "",
    "## Open tasks",
    memory.openTasks.length > 0 ? memory.openTasks.map((item) => `- ${item}`).join("\n") : "- Continue from preserved recent turns; no explicit task list was stored.",
    "## Progress",
    [formatBulletList(memory.progress), assistantNotes.length > 0 ? assistantNotes.map((note) => `- ${note}`).join("\n") : ""].filter(Boolean).join("\n"),
    "## Key decisions / constraints",
    formatBulletList(memory.decisions),
    "## Files and tools",
    [`Files:\n${formatBulletList(memory.paths)}`, `Tools:\n${formatBulletList(memory.tools)}`].join("\n\n"),
    "## Errors / blockers",
    formatBulletList(memory.errors),
    "## Critical preserved facts",
    formatBulletList(memory.criticalItems),
    "## Open items / next step",
    "Resume from the preserved recent messages, trust this checkpoint for older state, and use read/search tools for exact file contents before editing.",
  ].filter(Boolean).join("\n\n");
}

function mergeSummaryWithDeterministicSafetyNet(
  summary: string,
  messages: AiChatMessage[],
  previousSummary: string,
  sessionId: string | undefined,
  memory: CompactionMemory,
) {
  const trimmed = summary.trim();
  const fallback = buildDeterministicCompactionSummary(messages, previousSummary, sessionId, memory);
  if (!trimmed) return truncateText(fallback, MAX_SUMMARY_CHARS);

  const requiredHeadings = ["## Task goal", "## Open tasks", "## Progress", "## Files and tools", "## Errors / blockers", "## Critical preserved facts", "## Open items / next step"];
  const missing = requiredHeadings.filter((heading) => !trimmed.includes(heading));
  const hasTasks = memory.openTasks.length === 0 || memory.openTasks.some((task) => trimmed.includes(task.slice(0, 80)));
  const hasLatestDirection = !memory.latestUserDirection || trimmed.includes(memory.latestUserDirection.slice(0, 80));
  if (missing.length === 0 && hasTasks && hasLatestDirection) return truncateText(trimmed, MAX_SUMMARY_CHARS);

  return truncateText([
    trimmed,
    "",
    "## Deterministic safety net",
    "The model summary above missed required durable state; preserve the following exact extracted state too.",
    fallback,
  ].join("\n"), MAX_SUMMARY_CHARS);
}

const pathPattern = /(?:^|[\s"'`(])([\w./\\! -]+\.(?:ts|tsx|js|jsx|rs|py|go|java|css|html|md|json|yaml|yml|toml|sql|cs|cpp|h|vue|svelte))(?:$|[\s"'`,.)])/gi;
const importantLinePattern = /(todo|task|goal|fix|bug|error|fail|failed|exception|panic|blocked|must|нужно|надо|почини|ошибка|баг|сделай|готово|исправ|решен|completed|pending|in_progress|decision|decided|changed|updated|wrote|created|verified|провер)/i;
const decisionPattern = /(decided|decision|выбрал|решил|теперь|должен|must|нельзя|запрещ|только|only|avoid|preserve|keep)/i;
const errorPattern = /(error|failed|failure|exception|panic|timeout|unavailable|dropped|interrupted|ошибка|упал|слом|не работает|баг)/i;
const progressPattern = /(completed|done|fixed|implemented|created|updated|verified|готово|сделал|исправил|добавил|обновил|проверил|записан|создан)/i;

function extractPathsFromText(text: string) {
  const paths = new Set<string>();
  for (const match of text.matchAll(pathPattern)) {
    const path = match[1]?.replace(/\\/g, "/").trim();
    if (path && path.length <= 220) paths.add(path);
  }
  return paths;
}

function extractImportantLines(text: string) {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim().replace(/^[-*]\s+/, ""))
    .filter((line) => line.length >= 12 && line.length <= 1_500 && importantLinePattern.test(line))
    .slice(0, 40);
}

function looksLikeDecision(line: string) {
  return decisionPattern.test(line);
}

function looksLikeError(line: string) {
  return errorPattern.test(line);
}

function looksLikeProgress(line: string) {
  return progressPattern.test(line);
}

async function summarizeCompactionTranscript(input: {
  transcript: string;
  previousSummary: string;
  memory: CompactionMemory;
  sessionId?: string;
  provider: AiProviderConfig;
  model: AiModelConfig;
  selectedEffortId: string;
  abortSignal?: AbortSignal;
}) {
  const system = [
    "You compress IDE pair-programming chat history into a durable checkpoint for a coding agent.",
    "This is not a casual summary. It is the only memory the agent will have for older turns.",
    "Preserve exact task goal, latest user direction, active tasks with status, constraints, decisions, files/paths, tool outcomes, errors/blockers, verification results, and the next step.",
    "Never replace concrete facts with vague prose. Do not invent facts. Do not omit unresolved bugs/tasks. Do not say 'see above'.",
    "If there is conflict between transcript and extracted critical state, include both and mark the conflict.",
    "Required markdown sections exactly: ## Task goal, ## Latest user direction, ## Open tasks, ## Progress, ## Key decisions / constraints, ## Files and tools, ## Errors / blockers, ## Critical preserved facts, ## Open items / next step.",
    `Stay under ${Math.floor(MAX_SUMMARY_CHARS / 4)} tokens, but prefer preserving facts over being short.`,
  ].join("\n");

  const pinnedGoal = input.sessionId ? getAiSessionGoal(input.sessionId) : "";
  const openTasks = input.sessionId ? listAiSessionTodos(input.sessionId) : [];

  if (isTauriRuntime()) {
    const summary = await luxCommands.aiCompactionSummary({
      transcript: input.transcript,
      previousSummary: input.previousSummary,
      pinnedGoal,
      openTasks: openTasks.map((todo) => `[${todo.status}] ${todo.content}`),
      baseUrl: input.provider.baseUrl,
      apiKey: input.provider.apiKey || null,
      model: input.model.alias || input.model.id,
      protocol: resolveModelProtocol(input.provider, input.model),
      reasoning: reasoningPayload(input.selectedEffortId, input.provider, input.model),
    });
    return truncateText(summary, MAX_SUMMARY_CHARS);
  }

  const userParts = [
    pinnedGoal ? `Pinned session goal:\n${truncateText(pinnedGoal, MAX_PINNED_GOAL_CHARS)}` : "",
    openTasks.length > 0 ? `Open tasks:\n${openTasks.map((todo) => `- [${todo.status}] ${todo.content}`).join("\n")}` : "",
    `Extracted critical state:\n${formatMemory(input.memory)}`,
    input.previousSummary ? `Previous checkpoint to merge:\n${truncateText(input.previousSummary, MAX_PREVIOUS_SUMMARY_CHARS)}` : "",
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

/**
 * Reconcile a compaction result with the session's LIVE messages at apply time.
 *
 * The summarization await takes seconds; a send committed during that window
 * (composer, queued "Send now", goal continuation) appends messages the
 * compaction snapshot never saw. Blindly replacing with the compacted list
 * would delete the new user message and the in-flight assistant shell — every
 * subsequent stream update then no-ops on the missing id and the whole turn
 * vanishes. Appends everything committed after the snapshot to the compacted
 * result and reports whether such divergence happened (callers should skip the
 * replace entirely when a turn is live — it was built from the uncompacted
 * history anyway, so the checkpoint buys nothing and only risks divergence).
 */
export function reconcileCompactedMessages(
  snapshot: readonly AiChatMessage[],
  compacted: AiChatMessage[],
  live: readonly AiChatMessage[],
): { messages: AiChatMessage[]; divergedDuringCompaction: boolean } {
  const snapshotIds = new Set(snapshot.map((message) => message.id));
  const appended = live.filter((message) => !snapshotIds.has(message.id));
  if (appended.length === 0) {
    return { messages: compacted, divergedDuringCompaction: false };
  }
  return { messages: [...compacted, ...appended], divergedDuringCompaction: true };
}
