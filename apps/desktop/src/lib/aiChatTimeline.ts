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

// rAF/cancel resolved once. Falls back to a 16ms timer in any non-DOM context
// (e.g. a headless test harness) so the timeline never throws there.
const scheduleFrame: (cb: () => void) => number =
  typeof requestAnimationFrame === "function"
    ? (cb) => requestAnimationFrame(cb)
    : (cb) => setTimeout(cb, 16) as unknown as number;
const cancelFrame: (id: number) => void =
  typeof cancelAnimationFrame === "function"
    ? (id) => cancelAnimationFrame(id)
    : (id) => clearTimeout(id);

export function createTurnTimeline(emit: (patch: Partial<AiChatMessage>) => void) {
  const segments: AiMessageSegment[] = [];
  let activeReasoningId: string | null = null;
  let activeTextId: string | null = null;
  let frameId: number | null = null;
  let disposed = false;

  const find = (id: string | null) => (id ? segments.find((segment) => segment.id === id) ?? null : null);

  // Two flush cadences. `flushNow` emits synchronously and cancels any pending
  // frame — used by every NON-streaming transition (round commit, tool start/
  // update, append) so ordering vs tool calls and the final settle is exact.
  // `flushStreaming` coalesces the per-token deltas to at most one emit per
  // animation frame: during a response the Rust loop fires one StreamDelta per
  // SSE token (tens-hundreds/sec), and without this each one rebuilt the store +
  // re-lexed the whole markdown answer. Lossless because the full round text is
  // re-accumulated in `roundContent`/`roundReasoning` on every delta regardless
  // of when we flush.
  const cancelFlush = () => {
    if (frameId !== null) {
      cancelFrame(frameId);
      frameId = null;
    }
  };
  const flushNow = () => {
    cancelFlush();
    if (!disposed) emit(snapshotSegments(segments));
  };
  const flushStreaming = () => {
    if (disposed || frameId !== null) return;
    frameId = scheduleFrame(() => {
      frameId = null;
      if (!disposed) emit(snapshotSegments(segments));
    });
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
      flushStreaming();
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
        // Seal it: a later empty commit in the same round (e.g. a second parallel
        // tool call) must not re-touch this finalized block.
        activeReasoningId = null;
      }
      if (text.trim()) {
        if (activeTextId) {
          const segment = find(activeTextId);
          if (segment && segment.kind === "text") segment.text = text;
        } else {
          activeTextId = crypto.randomUUID();
          segments.push({ kind: "text", id: activeTextId, text });
        }
        // Seal the committed text so the NEXT commitRound("") this round (the native
        // loop fires one per tool call) can't fall into the delete branch below and
        // splice out already-shown narration. A new round opens a fresh segment.
        activeTextId = null;
      } else if (activeTextId) {
        // Only drop a placeholder that never received real text — never delete a
        // committed narration line.
        const segment = find(activeTextId);
        if (segment && segment.kind === "text" && !segment.text.trim()) {
          const index = segments.findIndex((entry) => entry.id === activeTextId);
          if (index >= 0) segments.splice(index, 1);
        }
        activeTextId = null;
      }
      flushNow();
    },
    appendText(text: string) {
      if (!text.trim()) return;
      activeTextId = crypto.randomUUID();
      segments.push({ kind: "text", id: activeTextId, text });
      flushNow();
    },
    addToolCalls(calls: AiChatToolCall[]) {
      for (const toolCall of calls) {
        segments.push({ kind: "tool", id: toolCall.id, toolCall });
      }
      flushNow();
    },
    updateToolCall(id: string, patch: Partial<AiChatToolCall>): AiChatToolCall | undefined {
      const segment = segments.find((entry) => entry.kind === "tool" && entry.toolCall.id === id);
      if (segment && segment.kind === "tool") {
        segment.toolCall = { ...segment.toolCall, ...patch };
        flushNow();
        return segment.toolCall;
      }
      flushNow();
      return undefined;
    },
    toolCalls() {
      return deriveSegmentToolCalls(segments);
    },
    snapshot(): Partial<AiChatMessage> {
      // Drop any pending streaming frame: the caller emits the authoritative
      // final patch right after this, so a trailing rAF must not fire afterward.
      cancelFlush();
      return snapshotSegments(segments);
    },
    // Stop the timeline for good (turn settled, errored, or aborted). Cancels a
    // pending streaming frame so no emit lands after the turn's `isActiveTurn`
    // guard would have closed.
    dispose() {
      disposed = true;
      cancelFlush();
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
