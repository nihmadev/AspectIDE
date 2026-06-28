// Per-session detector for tool-call loops — the model hammering the same tool
// with the same input over and over (a flood / stuck loop). Kept outside the chat
// store (like aiRetryNotice / aiAutomaticRetry) so high-frequency turns never churn
// the persisted session list.
//
// A short ring buffer of recent tool-call signatures is scanned after every turn.
// When one signature dominates the window, the goal-run evaluator escalates: first a
// corrective nudge, then a hard pause/stop so an autonomous run cannot spin forever.

import type { AiChatToolCall } from "./aiChatTypes";

/** How many recent tool-call signatures to remember per session. */
const RECENT_SIGNATURE_LIMIT = 14;
/** Repeats within the window that warrant a corrective nudge. */
export const TOOL_LOOP_WARN_COUNT = 3;
/** Repeats within the window that warrant a hard stop/pause. */
export const TOOL_LOOP_STOP_COUNT = 5;
/** Input prefix length used when fingerprinting a call (full input is overkill). */
const SIGNATURE_INPUT_CHARS = 400;

const signaturesBySession = new Map<string, string[]>();

export type ToolLoopReport = {
  /** True once a single signature repeats at least TOOL_LOOP_WARN_COUNT times. */
  looping: boolean;
  /** True once it repeats at least TOOL_LOOP_STOP_COUNT times. */
  critical: boolean;
  /** The dominant repeated signature, when looping. */
  signature: string | null;
  /** Occurrence count of the dominant signature within the window. */
  count: number;
};

function toolSignature(call: AiChatToolCall): string {
  const input = (call.input ?? "").trim().slice(0, SIGNATURE_INPUT_CHARS);
  return `${call.tool}::${input}`;
}

const EMPTY_REPORT: ToolLoopReport = { looping: false, critical: false, signature: null, count: 0 };

/**
 * Record this turn's tool-call signatures into the session's ring buffer and report
 * whether one signature now dominates the recent window. Call once per finalized turn.
 */
export function recordTurnToolSignatures(sessionId: string, toolCalls: AiChatToolCall[] | undefined): ToolLoopReport {
  if (!toolCalls || toolCalls.length === 0) return EMPTY_REPORT;
  const buffer = signaturesBySession.get(sessionId) ?? [];
  for (const call of toolCalls) buffer.push(toolSignature(call));
  while (buffer.length > RECENT_SIGNATURE_LIMIT) buffer.shift();
  signaturesBySession.set(sessionId, buffer);

  const counts = new Map<string, number>();
  let dominant: string | null = null;
  let dominantCount = 0;
  for (const signature of buffer) {
    const next = (counts.get(signature) ?? 0) + 1;
    counts.set(signature, next);
    if (next > dominantCount) {
      dominantCount = next;
      dominant = signature;
    }
  }

  const looping = dominantCount >= TOOL_LOOP_WARN_COUNT;
  return {
    looping,
    critical: dominantCount >= TOOL_LOOP_STOP_COUNT,
    signature: looping ? dominant : null,
    count: dominantCount,
  };
}

/** Short human label for a signature (tool name + a hint of the input). */
export function describeLoopSignature(signature: string | null): string {
  if (!signature) return "the same tool call";
  const [tool, input = ""] = signature.split("::");
  const hint = input.replace(/\s+/g, " ").trim().slice(0, 80);
  return hint ? `${tool} (${hint}…)` : tool;
}

/** Clear the loop window — called on goal-run start/stop and successful recovery. */
export function resetToolLoopDetector(sessionId: string): void {
  signaturesBySession.delete(sessionId);
}
