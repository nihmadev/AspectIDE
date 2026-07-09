import type { AiChatToolCall } from "./../chat/types";
import type { OpenAiToolCall } from "./../chat/transport";
import { isRecord, normalizePathSlashes, truncateText, type ToolResult, type UnknownRecord } from "./shared";

export function parseToolArguments(value: string | undefined): UnknownRecord {
  if (!value) return {};
  try {
    const parsed = JSON.parse(value) as unknown;
    return isRecord(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

export function formatToolOutput(result: ToolResult): string {
  const title = result.title?.trim() ?? "";
  const body = result.content?.trim() ?? "";
  if (!body) return title;
  if (!title || body.startsWith(title)) return body;
  return `${title}\n\n${body}`;
}

export function collectChangedPathsFromToolResult(content: string, sink: string[]) {
  try {
    const parsed = JSON.parse(content) as { changedPaths?: unknown };
    if (!Array.isArray(parsed.changedPaths)) return;
    for (const path of parsed.changedPaths) {
      if (typeof path === "string" && path.trim()) sink.push(path);
    }
  } catch {
    // ignore malformed tool payloads
  }
}

export function createRunningToolCall(call: OpenAiToolCall): AiChatToolCall {
  const name = call.function?.name || "Tool";
  return {
    id: call.id ?? crypto.randomUUID(),
    tool: name,
    status: "running",
    input: summarizeToolInput(call.function?.arguments),
    startTime: Date.now(),
  };
}

function summarizeToolInput(value: string | undefined) {
  if (!value) return "";
  try {
    const parsed = JSON.parse(value) as unknown;
    if (isRecord(parsed)) {
      const primaryKey = ["path", "query", "command", "pattern", "url", "cwd"].find((key) => typeof parsed[key] === "string" && String(parsed[key]).trim());
      if (primaryKey) return sanitizeToolSummary(String(parsed[primaryKey]));
      return truncateText(Object.entries(parsed).map(([key, entry]) => `${key}: ${formatToolInputValue(entry)}`).join(", "), 180);
    }
  } catch {
    return sanitizeToolSummary(truncateText(value, 180));
  }
  return sanitizeToolSummary(truncateText(value, 180));
}

function formatToolInputValue(value: unknown) {
  if (typeof value === "string") return sanitizeToolSummary(value);
  if (typeof value === "number" || typeof value === "boolean") return String(value);
  if (Array.isArray(value)) return `${value.length} item${value.length === 1 ? "" : "s"}`;
  if (isRecord(value)) return "object";
  return String(value ?? "");
}

function sanitizeToolSummary(value: string) {
  return truncateText(normalizePathSlashes(value.trim()).replace(/^\/\/\?\//, "").replace(/^\/\?\//, ""), 180);
}