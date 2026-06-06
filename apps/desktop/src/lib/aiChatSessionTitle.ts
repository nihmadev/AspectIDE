import { firstChoice, requestChatCompletion, type ChatCompletionMessage } from "./aiChatTransport";
import type { AiModelConfig, AiProviderConfig } from "./aiPreferences";
import { isTauriRuntime, luxCommands } from "./tauri";

export const DEFAULT_CHAT_SESSION_TITLE = "New chat";
export const MAX_CHAT_SESSION_TITLE_CHARS = 42;

const titleModelPattern = /haiku|mini|nano|flash|small|fast|lite|8b/i;

const inflightTitles = new Map<string, AbortController>();

export function isDefaultChatSessionTitle(title: string) {
  return title.trim() === DEFAULT_CHAT_SESSION_TITLE;
}

export function normalizeChatSessionTitle(value: string) {
  const normalized = value.replace(/\s+/g, " ").trim();
  if (!normalized) return DEFAULT_CHAT_SESSION_TITLE;
  const stripped = normalized.replace(/^["'`]+|["'`]+$/g, "").replace(/[.!?]+$/g, "").trim();
  if (!stripped) return DEFAULT_CHAT_SESSION_TITLE;
  return stripped.length > MAX_CHAT_SESSION_TITLE_CHARS
    ? `${stripped.slice(0, MAX_CHAT_SESSION_TITLE_CHARS).trimEnd()}...`
    : stripped;
}

export function heuristicChatSessionTitle(firstUserMessage: string) {
  return normalizeChatSessionTitle(firstUserMessage);
}

export function resolveTitleGenerationModel(provider: AiProviderConfig, activeModel: AiModelConfig) {
  const ranked = provider.models
    .map((model) => {
      const haystack = `${model.id} ${model.alias} ${model.name}`.toLowerCase();
      let score = 0;
      if (titleModelPattern.test(haystack)) score += 40;
      if (/haiku|flash/.test(haystack)) score += 20;
      if (/mini|nano/.test(haystack)) score += 15;
      if (model.id === activeModel.id) score += 5;
      return { model, score };
    })
    .filter((entry) => entry.score > 0)
    .sort((left, right) => right.score - left.score);
  return ranked[0]?.model ?? activeModel;
}

function parseGeneratedTitle(raw: string) {
  const line = raw.split("\n").map((entry) => entry.trim()).find(Boolean) ?? "";
  const withoutPrefix = line.replace(/^(?:title|chat title)\s*:\s*/i, "");
  return normalizeChatSessionTitle(withoutPrefix);
}

export async function generateChatSessionTitle(input: {
  firstUserMessage: string;
  provider: AiProviderConfig;
  model: AiModelConfig;
  selectedEffortId: string;
  abortSignal?: AbortSignal;
}): Promise<string> {
  const fallback = heuristicChatSessionTitle(input.firstUserMessage);
  const snippet = input.firstUserMessage.trim().slice(0, 1_200);
  if (!snippet) return fallback;

  // Native Rust path: title generation runs through the Rust transport.
  if (isTauriRuntime()) {
    try {
      const title = await luxCommands.aiGenerateSessionTitle({
        firstUserMessage: input.firstUserMessage,
        baseUrl: input.provider.baseUrl,
        apiKey: input.provider.apiKey || null,
        models: input.provider.models.map((model) => ({ id: model.id, alias: model.alias, name: model.name })),
        activeModelAlias: input.model.alias || input.model.id,
      });
      return title || fallback;
    } catch {
      return fallback;
    }
  }

  const titleModel = resolveTitleGenerationModel(input.provider, input.model);
  const system = [
    "You generate ultra-short IDE chat session titles.",
    "Return only the title (max 6 words), same language as the user message.",
    "No quotes, no markdown, no trailing punctuation.",
  ].join(" ");
  const messages: ChatCompletionMessage[] = [
    { role: "system", content: system },
    { role: "user", content: `First user message:\n${snippet}` },
  ];

  try {
    const response = await requestChatCompletion({
      abortSignal: input.abortSignal ?? new AbortController().signal,
      provider: input.provider,
      selectedEffortId: input.selectedEffortId,
      selectedModel: titleModel,
    }, messages, () => undefined, { toolsEnabled: false });
    const choice = firstChoice(response.body);
    const message = choice?.message;
    const raw = message && typeof message === "object" && "content" in message && typeof message.content === "string"
      ? message.content
      : "";
    const parsed = parseGeneratedTitle(raw);
    if (!parsed || isDefaultChatSessionTitle(parsed)) return fallback;
    return parsed;
  } catch {
    return fallback;
  }
}

export function shouldRefreshChatSessionTitle(session: {
  title: string;
  messages: Array<{ role: string; visibility?: string; content: string }>;
}) {
  const visibleUserMessages = session.messages.filter(
    (message) => message.role === "user" && message.visibility !== "internal" && message.content.trim(),
  );
  if (visibleUserMessages.length !== 1) return false;
  const heuristic = heuristicChatSessionTitle(visibleUserMessages[0].content);
  return isDefaultChatSessionTitle(session.title) || session.title === heuristic;
}

export function scheduleChatSessionTitleRefresh(params: {
  sessionId: string;
  firstUserMessage: string;
  provider: AiProviderConfig;
  model: AiModelConfig;
  selectedEffortId: string;
  rename: (sessionId: string, title: string) => void;
  readSession: (sessionId: string) => { title: string; messages: Array<{ role: string; visibility?: string; content: string }> } | null;
}) {
  const trimmed = params.firstUserMessage.trim();
  if (!trimmed) return;

  const existing = inflightTitles.get(params.sessionId);
  existing?.abort();
  const controller = new AbortController();
  inflightTitles.set(params.sessionId, controller);

  void (async () => {
    try {
      const title = await generateChatSessionTitle({
        firstUserMessage: trimmed,
        provider: params.provider,
        model: params.model,
        selectedEffortId: params.selectedEffortId,
        abortSignal: controller.signal,
      });
      if (controller.signal.aborted) return;
      const session = params.readSession(params.sessionId);
      if (!session || !shouldRefreshChatSessionTitle(session)) return;
      const heuristic = heuristicChatSessionTitle(trimmed);
      if (title === session.title || (title === heuristic && !isDefaultChatSessionTitle(session.title))) return;
      params.rename(params.sessionId, title);
    } finally {
      if (inflightTitles.get(params.sessionId) === controller) {
        inflightTitles.delete(params.sessionId);
      }
    }
  })();
}

export function cancelChatSessionTitleRefresh(sessionId: string) {
  inflightTitles.get(sessionId)?.abort();
  inflightTitles.delete(sessionId);
}