import { useEffect, useState } from "react";
import { estimateTokens } from "./aiChatContextCompaction";
import { useLuxStore } from "./store";
import type { AiChatMessage } from "./aiChatTypes";

/** Sampling cadence. Coarse on purpose: the readout is a vibe gauge, not telemetry. */
const TICK_MS = 600;
/** EMA smoothing — heavier weight on history so the number doesn't jitter per tick. */
const EMA_ALPHA = 0.35;

/**
 * Estimated output size of the streaming assistant tail: visible text, reasoning,
 * and streamed segments. Token counts arrive only at turn end (TurnUsage), so the
 * live readout derives speed from transcript growth through the same estimator the
 * context gauge uses — consistent units, no per-token event plumbing.
 */
function streamingTailTokens(message: AiChatMessage | undefined): number {
  if (!message || message.role !== "assistant") return 0;
  // The two transports fill different fields mid-stream (segments vs
  // content/reasoning), and at turn end one is derived from the other. Taking
  // the max of both views counts every path once and stays monotonic.
  const fromFields = estimateTokens(message.content) + estimateTokens(message.reasoning ?? "");
  let fromSegments = 0;
  for (const segment of message.segments ?? []) {
    if (segment.kind === "text" || segment.kind === "reasoning") {
      fromSegments += estimateTokens(segment.text);
    }
  }
  return Math.max(fromFields, fromSegments);
}

/**
 * The message actually being streamed. Turns patch one assistant message in place
 * (by id), while injected user messages ("recommendation" queue folds) append AFTER
 * it — so the streaming tail is the last assistant-role message, not necessarily
 * the last array element.
 */
export function latestAssistantMessage(messages: AiChatMessage[]): AiChatMessage | undefined {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    if (messages[i].role === "assistant") return messages[i];
  }
  return undefined;
}

/**
 * Live tokens-per-second of the active session's streaming answer, EMA-smoothed.
 * `null` whenever there is nothing to show (idle session, readout disabled, or no
 * measurable growth yet) — callers hide the chip on null.
 */
export function useLiveTokenSpeed(sessionId: string | null, enabled: boolean): number | null {
  const [speed, setSpeed] = useState<number | null>(null);

  useEffect(() => {
    if (!enabled || !sessionId) {
      setSpeed(null);
      return undefined;
    }

    let prevTokens: number | null = null;
    let prevAt = 0;
    let ema: number | null = null;

    const tick = () => {
      const state = useLuxStore.getState();
      const session = state.aiChatSessions.find((entry) => entry.id === sessionId);
      const busy = session && !session.closedAt
        && (session.status === "streaming" || session.status === "thinking"
          || session.status === "preparing" || session.status === "running-tools");
      if (!busy) {
        prevTokens = null;
        ema = null;
        setSpeed(null);
        return;
      }
      const tokens = streamingTailTokens(latestAssistantMessage(session.messages));
      const now = performance.now();
      if (prevTokens === null || tokens < prevTokens) {
        // First sample of a turn, or a new round reset the tail — re-anchor.
        prevTokens = tokens;
        prevAt = now;
        return;
      }
      const dtSeconds = (now - prevAt) / 1000;
      if (dtSeconds <= 0) return;
      const instant = (tokens - prevTokens) / dtSeconds;
      prevTokens = tokens;
      prevAt = now;
      ema = ema === null ? instant : ema * (1 - EMA_ALPHA) + instant * EMA_ALPHA;
      setSpeed(ema);
    };

    const handle = window.setInterval(tick, TICK_MS);
    tick();
    return () => {
      window.clearInterval(handle);
    };
  }, [enabled, sessionId]);

  return speed;
}

/** "28 tok/s" / "3.4 tok/s" — one decimal under 10, integer above. */
export function formatTokenSpeed(speed: number): string {
  const clamped = Math.max(0, speed);
  return clamped >= 10 ? `${Math.round(clamped)}` : clamped.toFixed(1);
}
