import {
  deriveSegmentContent,
  deriveSegmentReasoning,
  deriveSegmentToolCalls,
  type AiChatMessage,
  type AiChatToolCall,
  type AiMessageSegment,
} from "./aiChatTypes";

export type AiChatStreamProgress = {
  content: string;
  reasoning: string;
};

export function createTurnTimeline(emit: (patch: Partial<AiChatMessage>) => void) {
  const segments: AiMessageSegment[] = [];
  let activeReasoningId: string | null = null;
  let activeTextId: string | null = null;

  const find = (id: string | null) => (id ? segments.find((segment) => segment.id === id) ?? null : null);

  const flush = () => {
    emit(snapshotSegments(segments));
  };

  return {
    beginRound() {
      activeReasoningId = null;
      activeTextId = null;
    },
    setStreaming(progress: AiChatStreamProgress) {
      if (progress.reasoning.trim()) {
        if (!activeReasoningId) {
          activeReasoningId = crypto.randomUUID();
          segments.push({ kind: "reasoning", id: activeReasoningId, text: progress.reasoning });
        } else {
          const segment = find(activeReasoningId);
          if (segment && segment.kind === "reasoning") segment.text = progress.reasoning;
        }
      }
      if (progress.content) {
        if (!activeTextId) {
          activeTextId = crypto.randomUUID();
          segments.push({ kind: "text", id: activeTextId, text: progress.content });
        } else {
          const segment = find(activeTextId);
          if (segment && segment.kind === "text") segment.text = progress.content;
        }
      }
      flush();
    },
    commitRound(text: string, reasoning: string) {
      if (reasoning.trim()) {
        if (activeReasoningId) {
          const segment = find(activeReasoningId);
          if (segment && segment.kind === "reasoning") segment.text = reasoning;
        } else {
          activeReasoningId = crypto.randomUUID();
          segments.push({ kind: "reasoning", id: activeReasoningId, text: reasoning });
        }
      }
      if (text.trim()) {
        if (activeTextId) {
          const segment = find(activeTextId);
          if (segment && segment.kind === "text") segment.text = text;
        } else {
          activeTextId = crypto.randomUUID();
          segments.push({ kind: "text", id: activeTextId, text });
        }
      } else if (activeTextId) {
        const index = segments.findIndex((segment) => segment.id === activeTextId);
        if (index >= 0) segments.splice(index, 1);
        activeTextId = null;
      }
      flush();
    },
    appendText(text: string) {
      if (!text.trim()) return;
      activeTextId = crypto.randomUUID();
      segments.push({ kind: "text", id: activeTextId, text });
      flush();
    },
    addToolCalls(calls: AiChatToolCall[]) {
      for (const toolCall of calls) {
        segments.push({ kind: "tool", id: toolCall.id, toolCall });
      }
      flush();
    },
    updateToolCall(id: string, patch: Partial<AiChatToolCall>): AiChatToolCall | undefined {
      const segment = segments.find((entry) => entry.kind === "tool" && entry.toolCall.id === id);
      if (segment && segment.kind === "tool") {
        segment.toolCall = { ...segment.toolCall, ...patch };
        flush();
        return segment.toolCall;
      }
      flush();
      return undefined;
    },
    toolCalls() {
      return deriveSegmentToolCalls(segments);
    },
    snapshot(): Partial<AiChatMessage> {
      return snapshotSegments(segments);
    },
  };
}

export type TurnTimeline = ReturnType<typeof createTurnTimeline>;

function snapshotSegments(segments: AiMessageSegment[]): Partial<AiChatMessage> {
  return {
    segments: segments.map((segment) => ({ ...segment })),
    content: deriveSegmentContent(segments),
    reasoning: deriveSegmentReasoning(segments) || undefined,
    toolCalls: deriveSegmentToolCalls(segments),
  };
}
