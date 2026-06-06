import type { AiToolApprovalDecision } from "./aiChatTypes";
import { clearAiTurnActivity, deleteAiTurnActivity } from "./aiTurnActivity";

export type AiChatTurnRuntimeSnapshot = {
  sendingSessionId: string | null;
  /** When true, the active turn stops after the current tool round completes. */
  stopAfterToolRound: boolean;
  /** Bumped when a browser tool enables or updates the live preview stream. */
  browserStreamRefreshToken: number;
};

const listeners = new Set<() => void>();
const abortControllersBySession = new Map<string, AbortController>();
const turnGenerationBySession = new Map<string, number>();
const approvalResolversById = new Map<string, (decision: AiToolApprovalDecision) => void>();

let snapshot: AiChatTurnRuntimeSnapshot = {
  sendingSessionId: null,
  stopAfterToolRound: false,
  browserStreamRefreshToken: 0,
};

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
  snapshot = { ...snapshot, stopAfterToolRound: false };
  setSendingSessionId(sessionId);
}

export function requestStopAfterToolRound() {
  if (!snapshot.sendingSessionId) return;
  snapshot = { ...snapshot, stopAfterToolRound: true };
  for (const listener of listeners) listener();
}

export function consumeStopAfterToolRound() {
  if (!snapshot.stopAfterToolRound) return false;
  snapshot = { ...snapshot, stopAfterToolRound: false };
  for (const listener of listeners) listener();
  return true;
}

export function finishAiChatTurn(sessionId: string, abortController: AbortController) {
  if (abortControllersBySession.get(sessionId) !== abortController) return;
  abortControllersBySession.delete(sessionId);
  rejectAllAiToolApprovals();
  clearAiTurnActivity(sessionId);
  if (snapshot.sendingSessionId === sessionId) {
    snapshot = { ...snapshot, stopAfterToolRound: false };
    setSendingSessionId(null);
  }
}

export function abortAiChatTurn(sessionId: string | null) {
  if (sessionId) {
    const controller = abortControllersBySession.get(sessionId);
    controller?.abort();
    abortControllersBySession.delete(sessionId);
    turnGenerationBySession.set(sessionId, (turnGenerationBySession.get(sessionId) ?? 0) + 1);
    clearAiTurnActivity(sessionId);
    if (snapshot.sendingSessionId === sessionId) {
      snapshot = { ...snapshot, stopAfterToolRound: false };
      setSendingSessionId(null);
    }
  } else {
    for (const controller of abortControllersBySession.values()) controller.abort();
    abortControllersBySession.clear();
    turnGenerationBySession.clear();
    snapshot = { ...snapshot, stopAfterToolRound: false, sendingSessionId: null };
  }
  rejectAllAiToolApprovals();
}

export function disposeChatTurnRuntimeSession(sessionId: string) {
  abortAiChatTurn(sessionId);
  turnGenerationBySession.delete(sessionId);
  deleteAiTurnActivity(sessionId);
}

export function requestAiToolApproval(approvalId: string) {
  return new Promise<AiToolApprovalDecision>((resolve) => {
    const previous = approvalResolversById.get(approvalId);
    if (previous) previous("rejected");
    approvalResolversById.set(approvalId, resolve);
  });
}

export function resolveAiToolApproval(approvalId: string, decision: AiToolApprovalDecision) {
  const resolver = approvalResolversById.get(approvalId);
  if (!resolver) return;
  approvalResolversById.delete(approvalId);
  resolver(decision);
}

function rejectAllAiToolApprovals() {
  const resolvers = [...approvalResolversById.values()];
  approvalResolversById.clear();
  for (const resolve of resolvers) resolve("rejected");
}

function setSendingSessionId(sendingSessionId: string | null) {
  if (snapshot.sendingSessionId === sendingSessionId) return;
  snapshot = { ...snapshot, sendingSessionId };
  for (const listener of listeners) listener();
}

export function bumpBrowserStreamRefresh() {
  snapshot = {
    ...snapshot,
    browserStreamRefreshToken: snapshot.browserStreamRefreshToken + 1,
  };
  for (const listener of listeners) listener();
}