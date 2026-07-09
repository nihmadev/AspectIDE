import type { AiChatSession } from "./../../store/index";
import { exportPendingFileReviewsForPersistence } from "./../utils/pending-file-review";
import { getAiSessionGoal } from "./../session/goal/session-goal";
import { listAiSessionTodos } from "./../session/todos";

export function attachSessionExtrasForPersist(session: AiChatSession): AiChatSession {
  const sessionGoal = getAiSessionGoal(session.id);
  return {
    ...session,
    sessionGoal: sessionGoal || undefined,
    sessionTodos: listAiSessionTodos(session.id),
    pendingFileReviews: exportPendingFileReviewsForPersistence().filter((review) => review.sessionId === session.id),
  };
}

export function attachAllSessionExtrasForPersist(sessions: AiChatSession[]): AiChatSession[] {
  return sessions.map(attachSessionExtrasForPersist);
}