export type TerminalOutputBuffer = {
  text: string;
  updatedAt: string | null;
  bytes: number;
  chunks: number;
  truncated: boolean;
};

/**
 * The read-only "Lux AI" terminal tab that live-mirrors the agent's Shell
 * commands. Virtual — not backed by a PTY session in Rust, so terminal
 * write/resize/close commands must be skipped for it (there is nothing to
 * receive them); it lives purely in the store + output buffer pipeline.
 */
export const AI_MIRROR_TERMINAL_ID = "lux-ai-shell-mirror";

export function isAiMirrorTerminal(sessionId: string | null | undefined) {
  return sessionId === AI_MIRROR_TERMINAL_ID;
}

/** Display name of the AI mirror tab (shell/session label in the panel). */
export const AI_MIRROR_TERMINAL_LABEL = "Lux AI";
