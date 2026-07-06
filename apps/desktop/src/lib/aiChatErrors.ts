import { hasStreamingStarted } from "./aiChatTransport";
import type { TranslateFn } from "./i18n/useTranslation";

export type AiChatErrorKind =
  | "cancelled"
  | "timeout"
  | "rate-limit"
  | "context-overflow"
  | "auth"
  | "provider"
  | "invalid-json"
  | "tool-rejected"
  | "approval"
  | "workspace"
  | "file-not-found"
  | "stream"
  | "generic";

export type AiChatErrorPresentation = {
  kind: AiChatErrorKind;
  message: string;
  detail: string;
  canRetry: boolean;
  canRetryTools: boolean;
  canOpenSettings: boolean;
};

export function classifyAiChatError(error: unknown, t: TranslateFn): AiChatErrorPresentation {
  if (isAbortError(error)) {
    return {
      kind: "cancelled",
      message: t("aiChat.error.cancelled"),
      detail: "",
      canRetry: true,
      canRetryTools: false,
      canOpenSettings: false,
    };
  }

  const detail = readErrorMessage(error);
  const lower = detail.toLowerCase();

  // Context overflow: the accumulated history no longer fits the model's window.
  // This is RECOVERABLE by compacting the transcript, so the retry path force-runs
  // compaction before re-sending (see AiChatPanel). Match the common provider
  // phrasings across OpenAI/Anthropic/others.
  if (/context[ _-]?length|context_length_exceeded|maximum context|reduce the length|too many tokens|context window|prompt is too long|input is too long|exceeds? the (model'?s )?(maximum )?context|maximum.*tokens/.test(lower)) {
    return {
      kind: "context-overflow",
      message: t("aiChat.error.contextOverflow", { detail }),
      detail,
      canRetry: true,
      canRetryTools: false,
      canOpenSettings: false,
    };
  }
  // Rate limit (429): the backend already auto-retried the connection a couple of
  // times (with the live "retrying in Ns" notice), so reaching here means the limit
  // is still in effect. Surface a calm, recognizable message + retry instead of the
  // generic "AI request failed", and offer Settings to switch model/provider.
  if (/\b429\b|rate[ _-]?limit|too many requests|quota/.test(lower)) {
    return {
      kind: "rate-limit",
      message: t("aiChat.error.rateLimit", { detail }),
      detail,
      canRetry: true,
      canRetryTools: false,
      canOpenSettings: true,
    };
  }
  // Auth/permission (401/403, bad or missing API key). Not the provider being down —
  // retrying won't help until the key is fixed, so Automatic mode keeps retrying but
  // surfaces a precise "check your API key" message + Settings rather than the generic.
  if (/\b401\b|\b403\b|unauthorized|forbidden|invalid[ _-]?api[ _-]?key|incorrect api key|authentication_error|no api key|missing api key/.test(lower)) {
    return {
      kind: "auth",
      message: t("aiChat.error.auth", { detail }),
      detail,
      canRetry: true,
      canRetryTools: false,
      canOpenSettings: true,
    };
  }
  if (/timed out|timeout/.test(lower)) {
    return {
      kind: "timeout",
      message: t("aiChat.error.timeout", { detail }),
      detail,
      canRetry: true,
      canRetryTools: false,
      canOpenSettings: true,
    };
  }
  if (/failed to fetch|connection refused|connect|econnrefused|network/.test(lower)) {
    return {
      kind: "provider",
      message: t("aiChat.error.providerUnavailable", { detail }),
      detail,
      canRetry: true,
      canRetryTools: false,
      canOpenSettings: true,
    };
  }
  if (/non-json|json|expected value/.test(lower)) {
    return {
      kind: "invalid-json",
      message: t("aiChat.error.invalidJson", { detail }),
      detail,
      canRetry: true,
      canRetryTools: false,
      canOpenSettings: true,
    };
  }
  if (/rejected by the user|toolapprovalrejected/.test(lower)) {
    return {
      kind: "approval",
      message: t("aiChat.error.toolRejected", { detail }),
      detail,
      canRetry: true,
      canRetryTools: false,
      canOpenSettings: false,
    };
  }
  if (/unknown tool|tool call failed|tool execution|parallel subagent/.test(lower)) {
    return {
      kind: "tool-rejected",
      message: t("aiChat.error.tool", { detail }),
      detail,
      canRetry: false,
      canRetryTools: true,
      canOpenSettings: false,
    };
  }
  if (/workspace|no workspace is open/.test(lower)) {
    return {
      kind: "workspace",
      message: t("aiChat.error.workspace", { detail }),
      detail,
      canRetry: false,
      canRetryTools: false,
      canOpenSettings: false,
    };
  }
  if (/not found|file does not exist|cannot find/.test(lower)) {
    return {
      kind: "file-not-found",
      message: t("aiChat.error.fileNotFound", { detail }),
      detail,
      canRetry: true,
      canRetryTools: true,
      canOpenSettings: false,
    };
  }
  // "error decoding response body" is reqwest's opaque label for a streaming
  // body that dropped mid-flight. The backend now rewrites it to a "stream
  // interrupted" message, but match the raw phrase too so an unwrapped one still
  // routes to the retry-able stream branch instead of the generic catch-all.
  if (
    hasStreamingStarted(error)
    || /stream/.test(lower)
    || /error decoding response body|connection (was )?(closed|reset|dropped)|incomplete (message|chunked)/.test(lower)
  ) {
    return {
      kind: "stream",
      message: t("aiChat.error.stream", { detail }),
      detail,
      canRetry: true,
      canRetryTools: false,
      canOpenSettings: false,
    };
  }

  return {
    kind: "generic",
    message: t("aiChat.error.generic", { detail }),
    detail,
    canRetry: true,
    canRetryTools: false,
    canOpenSettings: false,
  };
}

export function formatAiError(error: unknown, t: TranslateFn) {
  return classifyAiChatError(error, t).message;
}

export function aiChatErrorFromMessage(message: string, t: TranslateFn): AiChatErrorPresentation {
  return classifyAiChatError(new Error(message), t);
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function readErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}