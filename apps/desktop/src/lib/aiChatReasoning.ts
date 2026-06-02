import type { AiChatMessage, AiMessageSegment } from "./aiChatTypes";

export function readVisibleReasoningFromProviderField(_value: unknown): string {
  return "";
}

export function normalizeVisibleReasoning(value: string | undefined): string | undefined {
  if (!value) return undefined;
  const normalized = value.replace(/\r\n?/g, "\n").trim();
  if (!normalized || looksLikePrivateReasoning(normalized) || looksLikeMojibake(normalized)) return undefined;
  const summary = stripExplicitSummaryPrefix(normalized);
  return summary.length > 0 && summary.length !== normalized.length ? summary : undefined;
}

export function normalizeAiMessageReasoning(message: AiChatMessage): AiChatMessage {
  const reasoning = normalizeVisibleReasoning(message.reasoning);
  const segments = normalizeReasoningSegments(message.segments);
  return {
    ...message,
    reasoning,
    segments,
  };
}

function normalizeReasoningSegments(segments: AiMessageSegment[] | undefined) {
  if (!segments) return undefined;
  const normalized = segments
    .map((segment) => {
      if (segment.kind !== "reasoning") return segment;
      const text = normalizeVisibleReasoning(segment.text);
      return text ? { ...segment, text } : null;
    })
    .filter((segment): segment is AiMessageSegment => Boolean(segment));
  return normalized.length > 0 ? normalized : undefined;
}

function looksLikePrivateReasoning(text: string) {
  const lower = text.toLowerCase();
  return [
    "the user says",
    "the user asked",
    "we need to",
    "i need to",
    "i should",
    "one approach",
    "thus,",
    "given the",
    "system says",
    "developer message",
    "tool call",
    "chain of thought",
  ].some((marker) => lower.includes(marker));
}

function looksLikeMojibake(text: string) {
  let hits = 0;
  for (const char of text) {
    const code = char.codePointAt(0);
    if (code === 0x00c2 || code === 0x00c3 || code === 0x00d0 || code === 0x00d1 || code === 0x00e2 || code === 0xfffd) {
      hits += 1;
    }
  }
  return hits >= 4;
}

function stripExplicitSummaryPrefix(text: string) {
  return text.replace(/^\s*(reasoning summary|summary|thought summary)\s*[:\-]\s*/i, "").trim();
}
