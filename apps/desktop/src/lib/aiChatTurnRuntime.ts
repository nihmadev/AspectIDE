import type { AiToolApprovalDecision } from "./aiChatTypes";

export type AiChatTurnRuntimeSnapshot = {
  sendingSessionId: string | null;
};

const listeners = new Set<() => void>();
const abortControllersBySession = new Map<string, AbortController>();
const approvalResolversById = new Map<string, (decision: AiToolApprovalDecision) => void>();

let snapshot: AiChatTurnRuntimeSnapshot = {
  sendingSessionId: null,
};

export function subscribeAiChatTurnRuntime(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getAiChatTurnRuntimeSnapshot() {
  return snapshot;
}

export function startAiChatTurn(sessionId: string, abortController: AbortController) {
  abortControllersBySession.set(sessionId, abortController);
  setSendingSessionId(sessionId);
}

export function finishAiChatTurn(sessionId: string, abortController: AbortController) {
  if (abortControllersBySession.get(sessionId) === abortController) {
    abortControllersBySession.delete(sessionId);
  }
  rejectAllAiToolApprovals();
  if (snapshot.sendingSessionId === sessionId) setSendingSessionId(null);
}

export function abortAiChatTurn(sessionId: string | null) {
  if (sessionId) {
    abortControllersBySession.get(sessionId)?.abort();
  } else {
    for (const controller of abortControllersBySession.values()) controller.abort();
  }
  rejectAllAiToolApprovals();
}

export function requestAiToolApproval(approvalId: string) {
  return new Promise<AiToolApprovalDecision>((resolve) => {
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
  snapshot = { sendingSessionId };
  for (const listener of listeners) listener();
}
