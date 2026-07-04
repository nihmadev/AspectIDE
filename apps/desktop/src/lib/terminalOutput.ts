import type { TerminalOutputBuffer } from "./terminalTypes";

/** Hard cap on retained per-terminal output so a runaway process can't grow the
 *  buffer without bound; older bytes are dropped from the front (tail-keep). */
export const MAX_TERMINAL_BUFFER_CHARS = 120_000;

export function emptyTerminalBuffer(): TerminalOutputBuffer {
  return { text: "", updatedAt: null, bytes: 0, chunks: 0, truncated: false };
}

/**
 * Fold one or more newly-received chunks into an existing buffer in a single pass.
 *
 * Taking an array (instead of one string per call) is what lets the coalescer
 * commit a whole frame's worth of PTY output with one O(buffer-size) concat+slice
 * instead of repeating that work for every byte the shell emits.
 */
export function appendTerminalChunks(current: TerminalOutputBuffer | undefined, chunks: readonly string[]): TerminalOutputBuffer {
  const previous = current ?? emptyTerminalBuffer();
  if (chunks.length === 0) return previous;

  const added = chunks.length === 1 ? chunks[0] : chunks.join("");
  if (!added) return previous;

  const combined = `${previous.text}${added}`;
  const overflow = combined.length > MAX_TERMINAL_BUFFER_CHARS;
  return {
    text: overflow ? combined.slice(combined.length - MAX_TERMINAL_BUFFER_CHARS) : combined,
    updatedAt: new Date().toISOString(),
    bytes: previous.bytes + added.length,
    chunks: previous.chunks + chunks.length,
    truncated: previous.truncated || overflow,
  };
}

/** Commits a batch of pending chunks (keyed by terminalId) into the store. */
export type TerminalOutputSink = (pending: ReadonlyMap<string, string[]>) => void;

/**
 * Batches high-frequency `appendTerminalOutput(terminalId, data)` calls so the
 * global store is written at most once per animation frame instead of once per
 * PTY chunk. Without this, a chatty shell or AI tool run produces hundreds of
 * store writes per second, each doing O(buffer-size) string work and waking every
 * unrelated Zustand subscriber (editor, chat, status bar).
 */
class TerminalOutputCoalescer {
  private pending = new Map<string, string[]>();
  private sink: TerminalOutputSink | null = null;
  private scheduled = false;

  /** Wire the store's commit function. Called once at store construction. */
  setSink(sink: TerminalOutputSink) {
    this.sink = sink;
  }

  enqueue(terminalId: string, data: string) {
    if (!terminalId || !data) return;
    const chunks = this.pending.get(terminalId);
    if (chunks) chunks.push(data);
    else this.pending.set(terminalId, [data]);
    this.schedule();
  }

  /** Drop not-yet-committed chunks so a clear/close can't be undone by a late
   *  flush. Pass no id to drop everything (workspace switch / close-all). */
  discard(terminalId?: string) {
    if (terminalId === undefined) this.pending.clear();
    else this.pending.delete(terminalId);
  }

  /** Force an immediate synchronous commit of everything queued. */
  flush() {
    if (this.pending.size === 0) return;
    // Swap the map out instead of clear(): `batch` must keep the queued chunks —
    // clearing the same reference before the sink reads it silently drops every
    // byte of terminal output (the v1.0.17 "terminal shows nothing" bug).
    const batch = this.pending;
    this.pending = new Map();
    this.sink?.(batch);
  }

  private schedule() {
    if (this.scheduled) return;
    this.scheduled = true;
    scheduleFrame(() => {
      this.scheduled = false;
      this.flush();
    });
  }
}

/** rAF in the browser; microtask fallback for tests/SSR where rAF is absent. */
function scheduleFrame(callback: () => void) {
  if (typeof requestAnimationFrame === "function") requestAnimationFrame(() => callback());
  else queueMicrotask(callback);
}

export const terminalOutputCoalescer = new TerminalOutputCoalescer();
