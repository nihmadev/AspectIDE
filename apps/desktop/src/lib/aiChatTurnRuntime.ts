import type { AiToolApprovalDecision } from "./aiChatTypes";
import { clearAiTurnActivity, deleteAiTurnActivity } from "./aiTurnActivity";

export type AiChatTurnRuntimeSnapshot = {
  sendingSessionId: string | null;
  /** When true, the active turn stops after the current tool round completes. */
  stopAfterToolRound: boolean;
  /** Bumped when a browser tool enables or updates the live preview stream. */
  browserStreamRefreshToken: number;
};

/**
 * Pending tool-approval, scoped to the `{ sessionId, generation }` of the turn that
 * requested it. Scoping is what prevents one chat session (or a parallel agent run)
 * from rejecting/consuming an approval that belongs to a different active turn.
 */
type ApprovalEntry = {
  sessionId: string | null;
  generation: number;
  resolve: (decision: AiToolApprovalDecision) => void;
};

const listeners = new Set<() => void>();
const abortControllersBySession = new Map<string, AbortController>();
const turnGenerationBySession = new Map<string, number>();
const approvalEntriesById = new Map<string, ApprovalEntry>();
/** sessionId -> turn generation for which a "stop after tool round" was requested. */
const stopGenerationBySession = new Map<string, number>();

let snapshot: AiChatTurnRuntimeSnapshot = {
  sendingSessionId: null,
  stopAfterToolRound: false,
  browserStreamRefreshToken: 0,
};

function notify() {
  for (const listener of listeners) listener();
}

export function subscribeAiChatTurnRuntime(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getAiChatTurnRuntimeSnapshot() {
  return snapshot;
}

export function getTurnGeneration(sessionId: string) {
  return turnGenerationBySession.get(sessionId) ?? 0;
}

export function isActiveChatTurn(sessionId: string, generation: number, abortController: AbortController) {
  return turnGenerationBySession.get(sessionId) === generation
    && abortControllersBySession.get(sessionId) === abortController;
}

export function startAiChatTurn(sessionId: string, abortController: AbortController) {
  const previous = abortControllersBySession.get(sessionId);
  if (previous && previous !== abortController) {
    previous.abort();
  }
  turnGenerationBySession.set(sessionId, (turnGenerationBySession.get(sessionId) ?? 0) + 1);
  abortControllersBySession.set(sessionId, abortController);
  // A fresh turn must never inherit a stale stop request from the previous generation.
  stopGenerationBySession.delete(sessionId);
  setSendingSessionId(sessionId);
}

/**
 * Request "stop after the current tool round" for a specific turn. Defaults to the
 * active sending session / its current generation so the existing zero-arg UI call
 * keeps working, but callers in a parallel turn loop should pass their own
 * `{ sessionId, generation }` so the flag cannot bleed into another session.
 */
export function requestStopAfterToolRound(sessionId?: string, generation?: number) {
  const targetSession = sessionId ?? snapshot.sendingSessionId;
  if (!targetSession) return;
  const targetGeneration = generation ?? getTurnGeneration(targetSession);
  stopGenerationBySession.set(targetSession, targetGeneration);
  syncStopMirror();
}

/**
 * Consume (clear and read) the stop request for a specific turn. Only the matching
 * `{ sessionId, generation }` consumes it, so a stale request from an older turn or a
 * different session is ignored instead of silently stopping the wrong loop.
 */
export function consumeStopAfterToolRound(sessionId?: string, generation?: number) {
  const targetSession = sessionId ?? snapshot.sendingSessionId;
  if (!targetSession) return false;
  const pendingGeneration = stopGenerationBySession.get(targetSession);
  if (pendingGeneration === undefined) return false;
  const targetGeneration = generation ?? getTurnGeneration(targetSession);
  if (pendingGeneration !== targetGeneration) return false;
  stopGenerationBySession.delete(targetSession);
  syncStopMirror();
  return true;
}

export function finishAiChatTurn(sessionId: string, abortController: AbortController) {
  if (abortControllersBySession.get(sessionId) !== abortController) return;
  abortControllersBySession.delete(sessionId);
  stopGenerationBySession.delete(sessionId);
  // Reject only this session's pending approvals — a sibling session's turn may still be live.
  rejectAiToolApprovalsForSession(sessionId);
  clearAiTurnActivity(sessionId);
  if (snapshot.sendingSessionId === sessionId) {
    setSendingSessionId(null);
  } else {
    syncStopMirror();
  }
}

export function abortAiChatTurn(sessionId: string | null) {
  if (sessionId) {
    const controller = abortControllersBySession.get(sessionId);
    controller?.abort();
    abortControllersBySession.delete(sessionId);
    turnGenerationBySession.set(sessionId, (turnGenerationBySession.get(sessionId) ?? 0) + 1);
    stopGenerationBySession.delete(sessionId);
    clearAiTurnActivity(sessionId);
    rejectAiToolApprovalsForSession(sessionId);
    if (snapshot.sendingSessionId === sessionId) {
      setSendingSessionId(null);
    } else {
      syncStopMirror();
    }
  } else {
    for (const controller of abortControllersBySession.values()) controller.abort();
    abortControllersBySession.clear();
    turnGenerationBySession.clear();
    stopGenerationBySession.clear();
    rejectAllAiToolApprovals();
    snapshot = { ...snapshot, stopAfterToolRound: false, sendingSessionId: null };
    notify();
  }
}

export function disposeChatTurnRuntimeSession(sessionId: string) {
  abortAiChatTurn(sessionId);
  turnGenerationBySession.delete(sessionId);
  stopGenerationBySession.delete(sessionId);
  deleteAiTurnActivity(sessionId);
}

/**
 * Register a pending approval owned by a turn. `sessionId`/`generation` default to the
 * active sending session so the legacy single-arg call site keeps tagging approvals
 * with the right owner; pass them explicitly from parallel turn loops.
 */
export function requestAiToolApproval(approvalId: string, sessionId?: string, generation?: number) {
  return new Promise<AiToolApprovalDecision>((resolve) => {
    const previous = approvalEntriesById.get(approvalId);
    if (previous) previous.resolve("rejected");
    const ownerSession = sessionId ?? snapshot.sendingSessionId;
    const ownerGeneration = generation ?? (ownerSession ? getTurnGeneration(ownerSession) : 0);
    approvalEntriesById.set(approvalId, { sessionId: ownerSession, generation: ownerGeneration, resolve });
  });
}

export function resolveAiToolApproval(approvalId: string, decision: AiToolApprovalDecision) {
  const entry = approvalEntriesById.get(approvalId);
  if (!entry) return;
  approvalEntriesById.delete(approvalId);
  entry.resolve(decision);
}

/** Reject every approval owned by a session (all of its generations). */
export function rejectAiToolApprovalsForSession(sessionId: string) {
  for (const [approvalId, entry] of [...approvalEntriesById.entries()]) {
    if (entry.sessionId !== sessionId) continue;
    approvalEntriesById.delete(approvalId);
    entry.resolve("rejected");
  }
}

function rejectAllAiToolApprovals() {
  const entries = [...approvalEntriesById.values()];
  approvalEntriesById.clear();
  for (const entry of entries) entry.resolve("rejected");
}

/** True when the session has a stop request pending for its current generation. */
function hasPendingStop(sessionId: string | null) {
  if (!sessionId) return false;
  const pendingGeneration = stopGenerationBySession.get(sessionId);
  return pendingGeneration !== undefined && pendingGeneration === getTurnGeneration(sessionId);
}

/** Keep the snapshot's UI mirror aligned with the active session's pending-stop state. */
function syncStopMirror() {
  const stopAfterToolRound = hasPendingStop(snapshot.sendingSessionId);
  if (snapshot.stopAfterToolRound === stopAfterToolRound) return;
  snapshot = { ...snapshot, stopAfterToolRound };
  notify();
}

function setSendingSessionId(sendingSessionId: string | null) {
  const stopAfterToolRound = hasPendingStop(sendingSessionId);
  if (snapshot.sendingSessionId === sendingSessionId && snapshot.stopAfterToolRound === stopAfterToolRound) return;
  snapshot = { ...snapshot, sendingSessionId, stopAfterToolRound };
  notify();
}

export function bumpBrowserStreamRefresh() {
  snapshot = {
    ...snapshot,
    browserStreamRefreshToken: snapshot.browserStreamRefreshToken + 1,
  };
  notify();
}
