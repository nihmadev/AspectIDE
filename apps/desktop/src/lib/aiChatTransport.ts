import type { AiModelConfig, AiProviderConfig } from "./aiPreferences";
import { isTauriRuntime, luxCommands, subscribeAiChatStream } from "./tauri";

export type ChatCompletionMessage = {
  role: "system" | "user" | "assistant" | "tool";
  content: string | null;
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
};

export async function requestChatCompletion(
  input: ChatCompletionTransportInput,
  messages: ChatCompletionMessage[],
  onStreamProgress: (progress: StreamProgress) => void,
  options: ChatCompletionTransportOptions = {},
): Promise<ChatCompletionResult> {
  throwIfAborted(input.abortSignal);
  const desktopRuntime = isTauriRuntime();
  const toolsEnabled = options.toolsEnabled ?? true;
  const payload = {
    model: input.selectedModel.alias || input.selectedModel.id,
    messages,
    temperature: 0.2,
    stream: false,
    ...reasoningPayload(input.selectedEffortId, input.provider),
    ...(desktopRuntime && toolsEnabled && options.tools?.length ? { tools: options.tools, tool_choice: "auto" } : {}),
  };

  if (desktopRuntime) {
    try {
      return await requestStreamingChatCompletion(input, payload, onStreamProgress);
    } catch (error) {
      throwIfAborted(input.abortSignal);
      if (!isStreamFallbackAllowed(error)) throw error;
    }
  }

  const startedAtMs = performance.now();
  const response = desktopRuntime
    ? await luxCommands.aiChatCompletion({
      baseUrl: input.provider.baseUrl,
      apiKey: input.provider.apiKey || null,
      payload,
    })
    : await requestBrowserChatCompletion(input, payload);
  throwIfAborted(input.abortSignal);
  return { body: response.body, streamed: false, timing: elapsedTiming(startedAtMs) };
}

export function firstChoice(value: unknown) {
  if (!isRecord(value) || !Array.isArray(value.choices)) return null;
  return value.choices.find(isRecord) ?? null;
}

export function readReasoningDelta(delta: UnknownRecord): string {
  if (typeof delta.reasoning_content === "string") return delta.reasoning_content;
  const reasoning = delta.reasoning;
  if (typeof reasoning === "string") return reasoning;
  if (isRecord(reasoning)) {
    if (typeof reasoning.content === "string") return reasoning.content;
    if (typeof reasoning.text === "string") return reasoning.text;
  }
  return "";
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
  if (!isTauriRuntime() && isLocalLoopbackUrl(url)) {
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
  };
}

function applyStreamChunk(accumulator: StreamAccumulator, data: unknown): StreamProgress {
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

function streamAccumulatorToCompletion(accumulator: StreamAccumulator) {
  return {
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

function isStreamFallbackAllowed(error: unknown) {
  return !hasStreamingStarted(error) && !isAbortErrorLike(error);
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

function hasStreamingStarted(error: unknown) {
  return error instanceof Error && Boolean((error as Error & { streamingStarted?: boolean }).streamingStarted);
}

function reasoningPayload(effortId: string, provider: AiProviderConfig) {
  if (!effortId) return {};
  const normalizedEffort = effortId === "xhigh" && provider.protocol !== "local-proxy" ? "high" : effortId;
  return {
    reasoning_effort: normalizedEffort,
    reasoning: { effort: normalizedEffort },
  };
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
