import type { AiChatSession } from "./store";
import { exportPendingFileReviewsForPersistence } from "./aiPendingFileReview";
import { getAiSessionGoal } from "./aiSessionGoal";
import { listAiSessionTodos } from "./aiSessionTodos";

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