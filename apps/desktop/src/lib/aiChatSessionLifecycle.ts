import { disposeChatTurnRuntimeSession } from "./aiChatTurnRuntime";
import { clearComposerSessionState } from "./aiChatComposerSession";
import { clearAiSessionGoal } from "./aiSessionGoal";
import { clearAiSessionTodos } from "./aiSessionTodos";
import { cancelAllSubagentRuns } from "./aiSubagentRuns";
import { removePendingFileReviewsForSession } from "./aiPendingFileReview";

/** Tear down in-memory AI chat side state when a session is deleted. */
export function disposeAiChatSessionSideState(sessionId: string) {
  disposeChatTurnRuntimeSession(sessionId);
  cancelAllSubagentRuns(sessionId);
  clearAiSessionTodos(sessionId);
  clearAiSessionGoal(sessionId);

  removePendingFileReviewsForSession(sessionId);
  clearComposerSessionState(sessionId);
}