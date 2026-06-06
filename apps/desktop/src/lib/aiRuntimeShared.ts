import type { AiChatToolCall } from "./aiChatTypes";

export type ToolResult = {
  title: string;
  content: string;
  stats?: AiChatToolCall["stats"];
  visionImageUrls?: string[];
  browserStreamPort?: number | null;
};

export type UnknownRecord = Record<string, unknown>;

export const maxToolOutputChars = 24_000;

export function toolJson(title: string, value: unknown, extras?: Partial<Pick<ToolResult, "visionImageUrls" | "browserStreamPort">>): ToolResult {
  const content = JSON.stringify(value, null, 2);
  return {
    title,
    content: truncateText(content, maxToolOutputChars),
    ...extras,
  };
}

export function stringArg(args: UnknownRecord, key: string, fallback = "") {
  const value = args[key];
  return typeof value === "string" ? value : fallback;
}

export function numberArg(args: UnknownRecord, key: string, fallback: number) {
  const value = args[key];
  const numeric = typeof value === "number" ? value : Number(value);
  return Number.isFinite(numeric) ? numeric : fallback;
}

export function booleanArg(args: UnknownRecord, key: string, fallback: boolean) {
  const value = args[key];
  return typeof value === "boolean" ? value : fallback;
}

export function stringArrayArg(args: UnknownRecord, key: string) {
  const value = args[key];
  return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
}

export function optionalPositiveNumberArg(args: UnknownRecord, key: string) {
  const value = args[key];
  if (value === undefined || value === null || value === "") return null;
  const numeric = typeof value === "number" ? value : Number(value);
  return Number.isFinite(numeric) && numeric > 0 ? Math.round(numeric) : null;
}

export function truncateText(text: string, maxChars: number) {
  if (text.length <= maxChars) return text;
  return `${text.slice(0, maxChars)}\n...[truncated ${text.length - maxChars} chars]`;
}

export function clamp(value: number, min: number, max: number) {
  if (!Number.isFinite(value)) return min;
  return Math.min(max, Math.max(min, Math.round(value)));
}

export function readErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

export function isRecord(value: unknown): value is UnknownRecord {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

export function normalizePathSlashes(path: string) {
  return path.replaceAll("\\", "/");
}

export function topCounts(values: string[], limit: number) {
  const counts = new Map<string, number>();
  for (const value of values) counts.set(value, (counts.get(value) ?? 0) + 1);
  return Array.from(counts.entries())
    .sort((left, right) => right[1] - left[1] || left[0].localeCompare(right[0]))
    .slice(0, limit)
    .map(([name, count]) => ({ name, count }));
}
