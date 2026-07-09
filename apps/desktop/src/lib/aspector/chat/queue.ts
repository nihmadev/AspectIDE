import { useSyncExternalStore } from "react";

/**
 * Ephemeral per-session queue of user messages composed WHILE the agent is busy.
 *
 * Two modes (Codex-style, but richer):
 *  - "queued": a follow-up the user wants run after the current turn finishes. Sent
 *    verbatim as the next turn once the agent goes idle.
 *  - "recommendation": a mid-work note. Sent after the current turn too, but wrapped
 *    so the model treats it as a suggestion to fold in — NOT an instruction to abort
 *    the work in flight (it decides whether to act now or self-queue it).
 *
 * In-memory only: a queue entry is a live intent tied to the running session, never
 * persisted (a queued message that outlived a restart would target a dead turn).
 */
export type QueuedMessageMode = "queued" | "recommendation";

export type QueuedMessage = {
  id: string;
  sessionId: string;
  text: string;
  mode: QueuedMessageMode;
  createdAt: number;
  /**
   * The live turn this recommendation was already injected into (mid-work
   * fold-in). Stored HERE — in the module-level queue, not a component ref — so
   * a panel remount during the turn cannot re-inject the same text. Self-heals:
   * a dead turn's id never matches the next live turn, so an unconfirmed entry
   * becomes injectable again exactly when a new turn starts.
   */
  injectedTurnId?: string;
};

type Listener = () => void;

let queue: QueuedMessage[] = [];
const listeners = new Set<Listener>();

function emit() {
  for (const listener of listeners) listener();
}

function subscribe(listener: Listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function snapshot() {
  return queue;
}

/** Append a message to a session's queue. Returns the new entry id. */
export function enqueueChatMessage(sessionId: string, text: string, mode: QueuedMessageMode): string {
  const trimmed = text.trim();
  if (!trimmed) return "";
  const id = `q-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
  queue = [...queue, { id, sessionId, text: trimmed, mode, createdAt: Date.now() }];
  emit();
  return id;
}

export function updateQueuedMessage(id: string, patch: { text?: string; mode?: QueuedMessageMode }): void {
  let changed = false;
  queue = queue.map((entry) => {
    if (entry.id !== id) return entry;
    const text = patch.text !== undefined ? patch.text.trim() : entry.text;
    if (!text) return entry; // never blank a queued message via edit; delete it instead.
    changed = true;
    return { ...entry, text, mode: patch.mode ?? entry.mode };
  });
  if (changed) emit();
}

/** Record (or clear with `null`) the live turn an entry was injected into. */
export function setQueuedMessageInjectedTurn(id: string, turnId: string | null): void {
  let changed = false;
  queue = queue.map((entry) => {
    if (entry.id !== id) return entry;
    changed = true;
    return { ...entry, injectedTurnId: turnId ?? undefined };
  });
  if (changed) emit();
}

export function removeQueuedMessage(id: string): void {
  const before = queue.length;
  queue = queue.filter((entry) => entry.id !== id);
  if (queue.length !== before) emit();
}

export function clearQueuedMessagesForSession(sessionId: string): void {
  const before = queue.length;
  queue = queue.filter((entry) => entry.sessionId !== sessionId);
  if (queue.length !== before) emit();
}

/** Remove and return a session's oldest queued message (FIFO), or null if none.
 * Draining one-at-a-time lets each queued item run as its own follow-up turn. */
export function dequeueFirstForSession(sessionId: string): QueuedMessage | null {
  const index = queue.findIndex((entry) => entry.sessionId === sessionId);
  if (index === -1) return null;
  const [entry] = queue.splice(index, 1);
  queue = [...queue];
  emit();
  return entry;
}

export function getQueuedMessagesForSession(sessionId: string): QueuedMessage[] {
  return queue.filter((entry) => entry.sessionId === sessionId);
}

/** React hook: the live queued messages for one session (stable empty array when none). */
export function useQueuedMessages(sessionId: string | null): QueuedMessage[] {
  useSyncExternalStore(subscribe, snapshot, snapshot);
  if (!sessionId) return EMPTY;
  return queue.filter((entry) => entry.sessionId === sessionId);
}

/** React hook: the full live queue across all sessions (for cross-session draining). */
export function useAllQueuedMessages(): QueuedMessage[] {
  return useSyncExternalStore(subscribe, snapshot, snapshot);
}

const EMPTY: QueuedMessage[] = [];

/**
 * Wrap a "recommendation" message so the agent folds it in without abandoning the
 * task in flight. Plain "queued" messages are sent verbatim.
 */
export function buildQueuedMessagePayload(entry: QueuedMessage): string {
  if (entry.mode !== "recommendation") return entry.text;
  return [
    "[The user sent this WHILE you were working. Treat it as a recommendation to fold into your plan, not an order to stop. Do NOT abandon or undo the task currently in progress unless this message explicitly tells you to. If it is new scope, finish the current work first, then address it (or queue it as a follow-up task).]",
    "",
    entry.text,
  ].join("\n");
}
