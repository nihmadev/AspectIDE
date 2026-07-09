export type InlineMentionSpan = {
  start: number;
  end: number;
  label: string;
  path?: string;
};

const mentionPattern = /@([^\s@][^\s@.,;:!?]{0,120})/g;

export function extractInlineMentionSpans(message: string): InlineMentionSpan[] {
  const spans: InlineMentionSpan[] = [];
  for (const match of message.matchAll(mentionPattern)) {
    const raw = match[1]?.trim();
    if (!raw) continue;
    const start = match.index ?? 0;
    spans.push({
      start,
      end: start + match[0].length,
      label: raw,
      path: raw.includes("/") || raw.includes("\\") ? raw : undefined,
    });
  }
  return spans;
}

export function stripMentionMarkersForVoice(message: string) {
  return message.replace(mentionPattern, (_, label: string) => label.trim());
}