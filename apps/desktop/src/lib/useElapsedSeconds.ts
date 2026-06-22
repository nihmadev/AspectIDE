import { useEffect, useState } from "react";

/**
 * Live elapsed-time ticker for the chat "thinking" row. While `active`, counts up
 * from the moment of activation on a light interval; when `active` flips to false
 * the last measured value FREEZES (it is not reset), so the row can show "thought
 * for Ns" after reasoning ends. Restarts from zero on re-activation (a fresh
 * thinking round). Pure presentation — no timer runs while inactive.
 *
 * Returns elapsed milliseconds. The owning component drives its own re-render via
 * this hook's internal state, so it keeps ticking even when the parent message row
 * is memoized and bailing out per streamed token.
 */
export function useElapsedSeconds(active: boolean): number {
  const [elapsedMs, setElapsedMs] = useState(0);

  useEffect(() => {
    if (!active) return; // inactive → keep the last frozen value, run nothing.
    const start = Date.now();
    setElapsedMs(0);
    const id = window.setInterval(() => setElapsedMs(Date.now() - start), 250);
    return () => {
      window.clearInterval(id);
      // Freeze at the exact end instant rather than the last 250ms tick.
      setElapsedMs(Date.now() - start);
    };
  }, [active]);

  return elapsedMs;
}

/** Compact thinking elapsed as one number string (unit added via i18n): sub-second
 * as `0.N`, else whole seconds. */
export function formatThinkingElapsed(elapsedMs: number): string {
  const seconds = elapsedMs / 1000;
  if (seconds < 1) return seconds.toFixed(1);
  return String(Math.round(seconds));
}
