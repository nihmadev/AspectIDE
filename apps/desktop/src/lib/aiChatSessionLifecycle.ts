import { disposeChatTurnRuntimeSession } from "./aiChatTurnRuntime";
import { clearAiRetryNotice } from "./aiRetryNotice";
import { clearComposerSessionState } from "./aiChatComposerSession";
import { clearAiSessionGoal } from "./aiSessionGoal";
import { disposeGoalRun } from "./aiSessionGoalRun";
import { clearAiSessionTodos } from "./aiSessionTodos";
import { cancelAllSubagentRuns } from "./aiSubagentRuns";
import { removePendingFileReviewsForSession } from "./aiPendingFileReview";
import { clearPendingQuestionsForSession } from "./aiPendingQuestion";
import { clearPendingPlansForSession } from "./aiPendingPlan";
import { luxCommands } from "./tauri";

/** Tear down in-memory AI chat side state when a session is deleted. */
export function disposeAiChatSessionSideState(sessionId: string) {
  disposeChatTurnRuntimeSession(sessionId);
  cancelAllSubagentRuns(sessionId);
  clearAiSessionTodos(sessionId);
  clearAiSessionGoal(sessionId);
  disposeGoalRun(sessionId);

  removePendingFileReviewsForSession(sessionId);
  clearPendingQuestionsForSession(sessionId);
  clearPendingPlansForSession(sessionId);
  clearComposerSessionState(sessionId);
  clearAiRetryNotice(sessionId);

  // Release the matching native (Rust) per-session maps too — the clears above
  // only touch in-memory JS state. Fire-and-forget.
  void luxCommands.aiSessionDispose(sessionId);
}