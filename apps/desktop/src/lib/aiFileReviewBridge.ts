export type FileReviewFocusRequest = {
  path: string;
  toolCallId?: string;
  hunkId?: string;
};

type FileReviewFocusListener = (request: FileReviewFocusRequest) => void;

const listeners = new Set<FileReviewFocusListener>();

export function subscribeFileReviewFocus(listener: FileReviewFocusListener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function requestFileReviewFocus(request: FileReviewFocusRequest) {
  for (const listener of listeners) listener(request);
}

export function extractReviewPathFromToolInput(tool: string, input?: string) {
  if (!input?.trim()) return null;
  try {
    const parsed = JSON.parse(input) as Record<string, unknown>;
    if (typeof parsed.path === "string" && parsed.path.trim()) return parsed.path.trim();
    if (tool === "PatchEngine" && Array.isArray(parsed.operations)) {
      const first = parsed.operations.find((entry) => isRecord(entry) && typeof entry.path === "string");
      if (first && isRecord(first) && typeof first.path === "string") return first.path;
    }
  } catch {
    const match = input.match(/"path"\s*:\s*"([^"]+)"/);
    if (match?.[1]) return match[1];
  }
  return null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}