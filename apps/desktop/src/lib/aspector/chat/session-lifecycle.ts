import { disposeChatTurnRuntimeSession } from "./turn-runtime";
import { clearAiRetryNotice } from "./../utils/retry-notice";
import { clearComposerSessionState } from "./composer-session";
import { clearAiSessionGoal } from "./../session/goal/session-goal";
import { disposeGoalRun } from "./../session/goal/session-goal-run";
import { clearAiSessionTodos } from "./../session/todos";
import { cancelAllSubagentRuns } from "./../subagents/runs";
import { removePendingFileReviewsForSession } from "./../utils/pending-file-review";
import { clearPendingQuestionsForSession } from "./../utils/pending-question";
import { clearPendingPlansForSession } from "./../utils/pending-plan";
import { luxCommands } from "./../../tauri/commands";

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