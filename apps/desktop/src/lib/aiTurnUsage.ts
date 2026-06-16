import type { AiChatMessage, AiChatTurnTokenUsage } from "./aiChatTypes";
import type { AiModelConfig } from "./aiPreferences";

export function extractTurnTokenUsage(body: unknown): AiChatTurnTokenUsage | null {
  const usage = pickUsageRecord(body);
  if (!usage) return null;
  const promptTokens = readTokenCount(usage, [
    "prompt_tokens",
    "input_tokens",
    "promptTokens",
    "inputTokens",
    "prompt_token_count",
  ]);
  const completionTokens = readTokenCount(usage, [
    "completion_tokens",
    "output_tokens",
    "completionTokens",
    "outputTokens",
    "candidates_token_count",
    "output_token_count",
  ]);
  const reasoningTokens = readTokenCount(usage, ["reasoning_tokens", "reasoningTokens"]);
  const completionWithReasoning = completionTokens + reasoningTokens;
  const totalTokens = readTokenCount(usage, ["total_tokens", "totalTokens"]) || promptTokens + completionWithReasoning;
  const cachedPromptTokens = readCachedPromptTokens(usage);
  if (promptTokens <= 0 && completionWithReasoning <= 0 && totalTokens <= 0) return null;
  return {
    promptTokens,
    completionTokens: completionWithReasoning,
    totalTokens: totalTokens > 0 ? totalTokens : promptTokens + completionWithReasoning,
    estimatedCostUsd: null,
    ...(cachedPromptTokens > 0 ? { cachedPromptTokens } : {}),
  };
}

/**
 * Cache-read prompt tokens across provider shapes:
 * - OpenAI / OpenRouter: `usage.prompt_tokens_details.cached_tokens`
 * - Anthropic (native / via OpenRouter): `usage.cache_read_input_tokens`
 * - Misc: top-level `cached_tokens`.
 */
function readCachedPromptTokens(usage: Record<string, unknown>): number {
  const direct = readTokenCount(usage, ["cache_read_input_tokens", "cached_tokens", "cachedTokens"]);
  if (direct > 0) return direct;
  const details = usage.prompt_tokens_details ?? usage.promptTokensDetails ?? usage.input_tokens_details;
  if (isRecord(details)) {
    return readTokenCount(details, ["cached_tokens", "cachedTokens", "cache_read_input_tokens"]);
  }
  return 0;
}

function pickUsageRecord(body: unknown): Record<string, unknown> | null {
  if (!isRecord(body)) return null;
  const candidates = [body.usage, body.usage_metadata];
  const choice = Array.isArray(body.choices) ? body.choices.find(isRecord) : null;
  if (isRecord(choice?.usage)) candidates.push(choice.usage);
  for (const candidate of candidates) {
    if (isRecord(candidate)) return candidate;
  }
  return null;
}

/** Fallback when the provider omits usage (common on streamed completions). */
export function estimateTurnUsageFromAssistant(message: AiChatMessage | null | undefined): AiChatTurnTokenUsage | null {
  if (!message) return null;
  let promptChars = 0;
  let completionChars = 0;

  const text = message.content?.trim() ?? "";
  completionChars += text.length;
  if (message.reasoning?.trim()) completionChars += message.reasoning.trim().length;

  for (const segment of message.segments ?? []) {
    if (segment.kind === "reasoning") completionChars += segment.text.length;
    if (segment.kind === "text") completionChars += segment.text.length;
  }

  for (const call of message.toolCalls ?? []) {
    promptChars += (call.input?.length ?? 0);
    completionChars += (call.output?.length ?? 0) + (call.error?.length ?? 0);
  }

  if (promptChars + completionChars <= 0) return null;

  const promptTokens = Math.max(1, Math.ceil(promptChars / 4));
  const completionTokens = Math.max(1, Math.ceil(completionChars / 4));
  return {
    promptTokens,
    completionTokens,
    totalTokens: promptTokens + completionTokens,
    estimatedCostUsd: null,
  };
}

export function resolveAssistantTurnUsage(
  usage: AiChatTurnTokenUsage | undefined,
  assistant: AiChatMessage | null | undefined,
): AiChatTurnTokenUsage | undefined {
  if (usage && (usage.promptTokens > 0 || usage.completionTokens > 0)) return usage;
  if (assistant?.turnUsage && (assistant.turnUsage.promptTokens > 0 || assistant.turnUsage.completionTokens > 0)) {
    return assistant.turnUsage;
  }
  return estimateTurnUsageFromAssistant(assistant) ?? undefined;
}

/**
 * Resolve effective per-million token rates for a model: a manual price set on the
 * model config wins (either field independently), otherwise the alias-based default.
 * Returns null only when there is no signal at all (never, since the fallback is total).
 */
export function resolveTurnCostRates(model: AiModelConfig): { inputPerMillion: number; outputPerMillion: number } {
  const fallback = resolveModelCostRates((model.alias || model.id).toLowerCase());
  const manualInput = positivePrice(model.inputPricePerMillion);
  const manualOutput = positivePrice(model.outputPricePerMillion);
  return {
    inputPerMillion: manualInput ?? fallback.inputPerMillion,
    outputPerMillion: manualOutput ?? fallback.outputPerMillion,
  };
}

/**
 * Populate `estimatedCostUsd` on a usage record. Uses the model's manual price when
 * set (Settings → Providers → model), else alias-based defaults. Cache-read prompt
 * tokens bill at a steep discount (~0.1x) across providers. Idempotent: recomputes
 * deterministically, so re-attaching is safe.
 */
export function attachTurnCostEstimate(usage: AiChatTurnTokenUsage, model: AiModelConfig): AiChatTurnTokenUsage {
  const rates = resolveTurnCostRates(model);
  const cached = Math.min(usage.cachedPromptTokens ?? 0, usage.promptTokens);
  const uncachedPrompt = Math.max(0, usage.promptTokens - cached);
  const inputCost = ((uncachedPrompt + cached * 0.1) / 1_000_000) * rates.inputPerMillion;
  const outputCost = (usage.completionTokens / 1_000_000) * rates.outputPerMillion;
  const estimatedCostUsd = Math.round((inputCost + outputCost) * 10_000) / 10_000;
  return { ...usage, estimatedCostUsd: estimatedCostUsd > 0 ? estimatedCostUsd : null };
}

function positivePrice(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? value : null;
}

export function mergeTurnTokenUsage(left: AiChatTurnTokenUsage | null, right: AiChatTurnTokenUsage | null): AiChatTurnTokenUsage | null {
  if (!left) return right;
  if (!right) return left;
  const estimatedCostUsd = (left.estimatedCostUsd ?? 0) + (right.estimatedCostUsd ?? 0);
  const cachedPromptTokens = (left.cachedPromptTokens ?? 0) + (right.cachedPromptTokens ?? 0);
  return {
    promptTokens: left.promptTokens + right.promptTokens,
    completionTokens: left.completionTokens + right.completionTokens,
    totalTokens: left.totalTokens + right.totalTokens,
    estimatedCostUsd: estimatedCostUsd > 0 ? Math.round(estimatedCostUsd * 10_000) / 10_000 : null,
    ...(cachedPromptTokens > 0 ? { cachedPromptTokens } : {}),
  };
}

function resolveModelCostRates(alias: string) {
  if (alias.includes("gpt-5") || alias.includes("openai/gpt-5")) return { inputPerMillion: 2.5, outputPerMillion: 10 };
  if (alias.includes("claude-sonnet") || alias.includes("anthropic/claude-sonnet")) return { inputPerMillion: 3, outputPerMillion: 15 };
  if (alias.includes("gemini-2.5-pro")) return { inputPerMillion: 1.25, outputPerMillion: 5 };
  if (alias.includes("deepseek")) return { inputPerMillion: 0.55, outputPerMillion: 2.19 };
  if (alias.includes("grok")) return { inputPerMillion: 2, outputPerMillion: 10 };
  return { inputPerMillion: 1.5, outputPerMillion: 6 };
}

function readTokenCount(usage: Record<string, unknown>, keys: string[]) {
  for (const key of keys) {
    const value = usage[key];
    if (typeof value === "number" && Number.isFinite(value) && value >= 0) return Math.round(value);
  }
  return 0;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}