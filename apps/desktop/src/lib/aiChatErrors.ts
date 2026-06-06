import { hasStreamingStarted } from "./aiChatTransport";
import type { TranslateFn } from "./i18n/useTranslation";

export type AiChatErrorKind =
  | "cancelled"
  | "timeout"
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
  if (hasStreamingStarted(error) || /stream/.test(lower)) {
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