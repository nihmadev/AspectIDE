import type { AiModelConfig, AiProviderConfig } from "./aiPreferences";
import { readVisibleReasoningFromProviderField } from "./aiChatReasoning";
import { createDesktopRuntimeError, isBrowserPreviewRuntime, isTauriRuntime, luxCommands, subscribeAiChatStream } from "./tauri";

export type ChatContentPart =
  | { type: "text"; text: string; cache_control?: { type: "ephemeral" } }
  | { type: "image_url"; image_url: { url: string; detail?: "low" | "high" | "auto" } };

export type ChatCompletionMessage = {
  role: "system" | "user" | "assistant" | "tool";
  content: string | ChatContentPart[] | null;
  name?: string;
  tool_call_id?: string;
  tool_calls?: OpenAiToolCall[];
};

export type OpenAiToolCall = {
  index?: number;
  id?: string;
  type?: "function";
  function?: {
    name?: string;
    arguments?: string;
  };
};

export type ChatCompletionResult = {
  body: unknown;
  timing: ChatCompletionTiming;
  streamed: boolean;
};

export type ChatCompletionTiming = {
  durationMs: number;
  firstTokenMs: number | null;
  streamMs: number | null;
};

export type StreamProgress = {
  content: string;
  reasoning: string;
};

export type ChatCompletionTransportInput = {
  abortSignal: AbortSignal;
  provider: AiProviderConfig;
  selectedEffortId: string;
  selectedModel: AiModelConfig;
};

export type ChatCompletionTransportOptions = {
  tools?: unknown[];
  toolsEnabled?: boolean;
};

type UnknownRecord = Record<string, unknown>;

type StreamAccumulator = {
  content: string;
  reasoning: string;
  role: string;
  toolCalls: OpenAiToolCall[];
  finishReason: string | null;
  usage: UnknownRecord | null;
};

export async function requestChatCompletion(
  input: ChatCompletionTransportInput,
  messages: ChatCompletionMessage[],
  onStreamProgress: (progress: StreamProgress) => void,
  options: ChatCompletionTransportOptions = {},
): Promise<ChatCompletionResult> {
  throwIfAborted(input.abortSignal);
  const desktopRuntime = isTauriRuntime();
  const browserPreviewRuntime = isBrowserPreviewRuntime();
  if (!desktopRuntime && !browserPreviewRuntime) throw createDesktopRuntimeError("AI chat completion");

  const toolsEnabled = options.toolsEnabled ?? true;
  const reasoning = reasoningPayload(input.selectedEffortId, input.provider);
  const payload = {
    model: input.selectedModel.alias || input.selectedModel.id,
    messages: applyPromptCacheBreakpoints(messages, input.selectedModel),
    stream: false,
    // Reasoning models reject an explicit temperature (OpenAI o-series / gpt-5
    // return HTTP 400) — send it only for standard models.
    ...(Object.keys(reasoning).length === 0 ? { temperature: 0.2 } : {}),
    ...reasoning,
    ...(desktopRuntime && toolsEnabled && options.tools?.length ? { tools: options.tools, tool_choice: "auto" } : {}),
  };

  if (desktopRuntime) {
    return requestChatCompletionWithRetry(
      () => requestStreamingChatCompletion(input, payload, onStreamProgress),
      input.abortSignal,
    );
  }

  const startedAtMs = performance.now();
  const response = await requestBrowserChatCompletion(input, payload);
  throwIfAborted(input.abortSignal);
  return { body: response.body, streamed: false, timing: elapsedTiming(startedAtMs) };
}

export function firstChoice(value: unknown) {
  if (!isRecord(value) || !Array.isArray(value.choices)) return null;
  return value.choices.find(isRecord) ?? null;
}

export function readReasoningDelta(delta: UnknownRecord): string {
  return readVisibleReasoningFromProviderField(delta);
}

async function requestBrowserChatCompletion(input: ChatCompletionTransportInput, payload: UnknownRecord) {
  const endpoint = chatCompletionEndpoint(input.provider.baseUrl);
  const headers: Record<string, string> = {
    "Accept": "application/json",
    "Content-Type": "application/json",
  };
  const apiKey = input.provider.apiKey.trim();
  if (apiKey) headers.Authorization = `Bearer ${apiKey}`;

  const response = await fetch(endpoint, {
    body: JSON.stringify({ ...payload, stream: false }),
    headers,
    method: "POST",
    signal: input.abortSignal,
  });
  const body = await response.json().catch(async () => ({ error: { message: await response.text().catch(() => "AI provider returned a non-JSON response") } }));
  if (!response.ok) throw new Error(aiResponseError(response.status, body));
  return { status: response.status, body };
}

function chatCompletionEndpoint(baseUrl: string) {
  const trimmed = baseUrl.trim().replace(/\/+$/g, "");
  if (!trimmed) throw new Error("AI provider base URL is empty");
  const url = parseProviderBaseUrl(trimmed);
  if (isBrowserPreviewRuntime() && isLocalLoopbackUrl(url)) {
    const proxyPath = url.pathname.replace(/\/+$/g, "");
    const chatPath = proxyPath.endsWith("/chat/completions") ? proxyPath : `${proxyPath}/chat/completions`;
    return `/__lux_ai_proxy${chatPath}`;
  }
  return trimmed.endsWith("/chat/completions") ? trimmed : `${trimmed}/chat/completions`;
}

function parseProviderBaseUrl(value: string) {
  try {
    const url = new URL(value);
    if (url.protocol !== "http:" && url.protocol !== "https:") throw new Error(`Unsupported AI provider URL scheme: ${url.protocol.replace(/:$/g, "")}`);
    return url;
  } catch (error) {
    throw new Error(error instanceof Error ? error.message : `Invalid AI provider URL: ${value}`);
  }
}

function isLocalLoopbackUrl(url: URL) {
  return url.protocol === "http:" && ["127.0.0.1", "localhost", "::1", "[::1]"].includes(url.hostname);
}

const maxProviderRetries = 2;

async function requestChatCompletionWithRetry(
  run: () => Promise<ChatCompletionResult>,
  abortSignal: AbortSignal,
): Promise<ChatCompletionResult> {
  let lastError: unknown;
  for (let attempt = 0; attempt <= maxProviderRetries; attempt += 1) {
    throwIfAborted(abortSignal);
    try {
      return await run();
    } catch (error) {
      lastError = error;
      if (isAbortErrorLike(error) || hasStreamingStarted(error) || attempt >= maxProviderRetries || !isRetryableProviderError(error)) {
        throw enrichProviderRetryError(error, attempt);
      }
      await abortableDelay(350 * (attempt + 1), abortSignal);
    }
  }
  throw enrichProviderRetryError(lastError, maxProviderRetries);
}

function enrichProviderRetryError(error: unknown, attempts: number) {
  if (!(error instanceof Error) || attempts <= 0) return error;
  const message = error.message.includes("retried")
    ? error.message
    : `${error.message} (retried ${attempts} time${attempts === 1 ? "" : "s"} on the same provider — no model fallback)`;
  return new Error(message);
}

function isRetryableProviderError(error: unknown) {
  if (!(error instanceof Error)) return false;
  const text = error.message.toLowerCase();
  return text.includes("429")
    || text.includes("408")
    || text.includes("500")
    || text.includes("502")
    || text.includes("503")
    || text.includes("504")
    || text.includes("timeout")
    || text.includes("temporarily")
    || text.includes("overloaded");
}

function abortableDelay(ms: number, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    if (signal.aborted) {
      reject(new DOMException("AI request was cancelled", "AbortError"));
      return;
    }
    let timer = 0;
    const onAbort = () => {
      window.clearTimeout(timer);
      reject(new DOMException("AI request was cancelled", "AbortError"));
    };
    timer = window.setTimeout(() => {
      signal.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    signal.addEventListener("abort", onAbort, { once: true });
  });
}

function aiResponseError(status: number, body: unknown) {
  const fallback = `AI provider returned HTTP ${status}`;
  if (!isRecord(body)) return fallback;
  if (isRecord(body.error)) {
    const message = typeof body.error.message === "string" ? body.error.message : null;
    return message ? `AI provider error ${status}: ${message}` : fallback;
  }
  const message = typeof body.message === "string" ? body.message : null;
  return message ? `AI provider error ${status}: ${message}` : fallback;
}

async function requestStreamingChatCompletion(input: ChatCompletionTransportInput, payload: UnknownRecord, onStreamProgress: (progress: StreamProgress) => void): Promise<ChatCompletionResult> {
  const streamId = crypto.randomUUID();
  const startedAtMs = performance.now();
  let firstTokenAtMs: number | null = null;
  let started = false;
  let cleanup: (() => void) | undefined;
  let abortListener: (() => void) | undefined;

  try {
    const result = await new Promise<ChatCompletionResult>((resolve, reject) => {
      const accumulator = createStreamAccumulator();
      let settled = false;

      const settle = (callback: () => void) => {
        if (settled) return;
        settled = true;
        callback();
      };

      const abort = () => {
        void luxCommands.aiChatCompletionStreamCancel(streamId).catch(() => undefined);
        settle(() => reject(new DOMException("AI request was cancelled", "AbortError")));
      };

      if (input.abortSignal.aborted) {
        abort();
        return;
      }

      abortListener = () => abort();
      input.abortSignal.addEventListener("abort", abortListener, { once: true });

      const startStream = () => {
        void luxCommands.aiChatCompletionStream({
          baseUrl: input.provider.baseUrl,
          apiKey: input.provider.apiKey || null,
          payload: { ...payload, stream: true },
          streamId,
        }).catch((error) => {
          settle(() => reject(error));
        });
      };

      void subscribeAiChatStream((event) => {
        if (event.streamId !== streamId || settled) return;
        if (event.kind === "chunk") {
          started = true;
          firstTokenAtMs ??= performance.now();
          try {
            const progress = applyStreamChunk(accumulator, event.data);
            if (progress.content || progress.reasoning) onStreamProgress(progress);
          } catch (error) {
            void luxCommands.aiChatCompletionStreamCancel(streamId).catch(() => undefined);
            settle(() => reject(markStreamingStarted(error)));
          }
          return;
        }
        if (event.kind === "done") {
          started = true;
          const timing = streamTiming(startedAtMs, firstTokenAtMs, performance.now());
          settle(() => resolve({ body: streamAccumulatorToCompletion(accumulator), streamed: true, timing }));
          return;
        }
        if (event.kind === "cancelled") {
          settle(() => reject(new DOMException("AI request was cancelled", "AbortError")));
          return;
        }
        if (event.kind === "error") {
          const error = new Error(event.error || "AI stream failed");
          settle(() => reject(started ? markStreamingStarted(error) : error));
        }
      }).then((unlisten) => {
        cleanup = unlisten;
        if (settled) cleanup?.();
        else startStream();
      }).catch((error) => {
        settle(() => reject(error));
      });
    });
    throwIfAborted(input.abortSignal);
    return result;
  } catch (error) {
    if (started && !isAbortErrorLike(error)) throw markStreamingStarted(error);
    throw error;
  } finally {
    cleanup?.();
    if (abortListener) input.abortSignal.removeEventListener("abort", abortListener);
  }
}

function elapsedTiming(startedAtMs: number): ChatCompletionTiming {
  return {
    durationMs: Math.max(0, Math.round(performance.now() - startedAtMs)),
    firstTokenMs: null,
    streamMs: null,
  };
}

function streamTiming(startedAtMs: number, firstTokenAtMs: number | null, finishedAtMs: number): ChatCompletionTiming {
  const firstTokenMs = firstTokenAtMs === null ? null : Math.max(0, Math.round(firstTokenAtMs - startedAtMs));
  return {
    durationMs: Math.max(0, Math.round(finishedAtMs - startedAtMs)),
    firstTokenMs,
    streamMs: firstTokenAtMs === null ? null : Math.max(0, Math.round(finishedAtMs - firstTokenAtMs)),
  };
}

function createStreamAccumulator(): StreamAccumulator {
  return {
    content: "",
    reasoning: "",
    role: "assistant",
    toolCalls: [],
    finishReason: null,
    usage: null,
  };
}

function applyStreamChunk(accumulator: StreamAccumulator, data: unknown): StreamProgress {
  const usage = pickStreamUsageRecord(data);
  if (usage) accumulator.usage = usage;
  const choice = firstChoice(data);
  if (!choice) return streamProgress(accumulator);
  if (typeof choice.finish_reason === "string") accumulator.finishReason = choice.finish_reason;
  const delta = isRecord(choice.delta) ? choice.delta : null;
  if (!delta) return streamProgress(accumulator);
  if (typeof delta.role === "string") accumulator.role = delta.role;
  if (typeof delta.content === "string") accumulator.content += delta.content;
  const reasoningChunk = readReasoningDelta(delta);
  if (reasoningChunk) accumulator.reasoning += reasoningChunk;
  applyToolCallDeltas(accumulator, delta.tool_calls);
  return streamProgress(accumulator);
}

function streamProgress(accumulator: StreamAccumulator): StreamProgress {
  return { content: accumulator.content, reasoning: accumulator.reasoning };
}

function applyToolCallDeltas(accumulator: StreamAccumulator, value: unknown) {
  if (!Array.isArray(value)) return;
  value.filter(isRecord).forEach((delta, fallbackIndex) => {
    const index = clamp(numberArg(delta, "index", fallbackIndex), 0, 128);
    const existing = accumulator.toolCalls[index] ?? { type: "function", function: { name: "", arguments: "" } };
    const next: OpenAiToolCall = {
      ...existing,
      index,
      type: "function",
      id: typeof delta.id === "string" ? delta.id : existing.id,
      function: {
        name: existing.function?.name ?? "",
        arguments: existing.function?.arguments ?? "",
      },
    };
    if (isRecord(delta.function)) {
      if (typeof delta.function.name === "string") {
        next.function = { ...next.function, name: `${next.function?.name ?? ""}${delta.function.name}` };
      }
      if (typeof delta.function.arguments === "string") {
        next.function = { ...next.function, arguments: `${next.function?.arguments ?? ""}${delta.function.arguments}` };
      }
    }
    accumulator.toolCalls[index] = next;
  });
}

function pickStreamUsageRecord(data: unknown): UnknownRecord | null {
  if (!isRecord(data)) return null;
  const candidates = [data.usage, data.usage_metadata, firstChoice(data)?.usage];
  for (const candidate of candidates) {
    if (isRecord(candidate)) return candidate;
  }
  return null;
}

function streamAccumulatorToCompletion(accumulator: StreamAccumulator) {
  return {
    ...(accumulator.usage ? { usage: accumulator.usage } : {}),
    choices: [{
      index: 0,
      finish_reason: accumulator.finishReason,
      message: {
        role: accumulator.role,
        content: accumulator.content,
        reasoning_content: accumulator.reasoning,
        tool_calls: accumulator.toolCalls.filter(Boolean),
      },
    }],
  };
}

function markStreamingStarted(error: unknown) {
  if (error instanceof Error) {
    (error as Error & { streamingStarted?: boolean }).streamingStarted = true;
    return error;
  }
  const wrapped = new Error(String(error));
  (wrapped as Error & { streamingStarted?: boolean }).streamingStarted = true;
  return wrapped;
}

export function hasStreamingStarted(error: unknown) {
  return error instanceof Error && Boolean((error as Error & { streamingStarted?: boolean }).streamingStarted);
}

/**
 * Anthropic prompt caching: mark the (stable) system prompt with a `cache_control`
 * breakpoint so Anthropic-family models cache it and re-read it cheaply on every
 * subsequent turn. Only applied for Anthropic models (claude/anthropic) — other
 * providers (OpenAI, DeepSeek, …) cache automatically and may reject the field,
 * so their payload is left untouched.
 */
function applyPromptCacheBreakpoints(
  messages: ChatCompletionMessage[],
  model: AiModelConfig,
): ChatCompletionMessage[] {
  if (!isAnthropicCacheModel(model)) return messages;
  let applied = false;
  return messages.map((message) => {
    if (applied || message.role !== "system" || typeof message.content !== "string" || !message.content.trim()) {
      return message;
    }
    applied = true;
    return {
      ...message,
      content: [{ type: "text", text: message.content, cache_control: { type: "ephemeral" } }],
    };
  });
}

export function isAnthropicCacheModel(model: AiModelConfig): boolean {
  const id = `${model.alias ?? ""} ${model.id ?? ""}`.toLowerCase();
  return id.includes("claude") || id.includes("anthropic");
}

export function reasoningPayload(effortId: string, provider: AiProviderConfig): Record<string, unknown> {
  if (!effortId) return {};
  const normalizedEffort = effortId === "xhigh" && provider.protocol !== "local-proxy" ? "high" : effortId;
  // Send the single field the provider expects, not both. OpenRouter's unified API
  // takes `reasoning: { effort }`; every other OpenAI-compatible provider takes the
  // OpenAI-standard `reasoning_effort` string. Sending the unknown `reasoning`
  // object to a strict provider (OpenAI, DeepSeek, …) can 400.
  if (provider.providerType === "openrouter") {
    return { reasoning: { effort: normalizedEffort } };
  }
  return { reasoning_effort: normalizedEffort };
}

function throwIfAborted(signal: AbortSignal) {
  if (signal.aborted) throw new DOMException("AI request was cancelled", "AbortError");
}

function isAbortErrorLike(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function numberArg(args: UnknownRecord, key: string, fallback: number) {
  const value = args[key];
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function clamp(value: number, min: number, max: number) {
  return Math.min(max, Math.max(min, value));
}

function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
