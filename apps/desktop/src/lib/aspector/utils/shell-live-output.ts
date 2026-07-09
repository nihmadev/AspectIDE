/**
 * Live output buffers for running AI `Shell` tool calls, keyed by tool-call id.
 *
 * The Rust shell mirror streams `LuxEvent::AiShellOutput { data, tool_call_id }`
 * while a command runs. The "Lux AI" terminal tab consumes the raw stream; this
 * store keeps a bounded, ANSI-stripped tail per tool call so the chat panel can
 * expand a RUNNING Shell row and watch the output live (previously the row only
 * became expandable after completion, when `output` arrived).
 */

type Listener = () => void;

/** Tail cap per tool call — enough to read live progress, bounded for memory. */
const MAX_CHARS_PER_CALL = 16_000;
/** Retain at most this many buffers; oldest are evicted (a finished call's row
 *  switches to the tool's final `output`, so losing its live tail is harmless). */
const MAX_TRACKED_CALLS = 24;

// CSI sequences (colors/cursor) + OSC sequences (window titles), both ESC-led.
// Built via fromCharCode so the source file contains no raw control characters.
const ESC = String.fromCharCode(27);
const BEL = String.fromCharCode(7);
const ANSI_PATTERN = new RegExp(
  `${ESC}\\[[0-9;?]*[ -/]*[@-~]|${ESC}\\][^${BEL}${ESC}]*(?:${BEL}|${ESC}\\\\)?`,
  "g",
);

const buffers = new Map<string, string>();
const listeners = new Set<Listener>();
/** Bumped on every append; snapshot identity for useSyncExternalStore. */
let version = 0;

export function subscribeAiShellLiveOutput(listener: Listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Cheap snapshot token — components re-read via getAiShellLiveOutput on change. */
export function getAiShellLiveOutputVersion() {
  return version;
}

export function getAiShellLiveOutput(toolCallId: string): string {
  return buffers.get(toolCallId) ?? "";
}

export function hasAiShellLiveOutput(toolCallId: string): boolean {
  return (buffers.get(toolCallId)?.length ?? 0) > 0;
}

export function appendAiShellLiveOutput(toolCallId: string, data: string) {
  // Normalize for chat display: strip ANSI color/cursor sequences and CRs the
  // terminal tab renders natively but a <pre> would show as garbage.
  const cleaned = data.replace(ANSI_PATTERN, "").replace(/\r+\n/g, "\n").replace(/\r/g, "\n");
  if (!cleaned) return;
  const existing = buffers.get(toolCallId);
  if (existing === undefined && buffers.size >= MAX_TRACKED_CALLS) {
    const oldest = buffers.keys().next().value;
    if (oldest !== undefined) buffers.delete(oldest);
  }
  let next = (existing ?? "") + cleaned;
  if (next.length > MAX_CHARS_PER_CALL) {
    next = next.slice(next.length - MAX_CHARS_PER_CALL);
  }
  buffers.set(toolCallId, next);
  version += 1;
  for (const listener of listeners) listener();
}
