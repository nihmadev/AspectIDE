import type { AiTurnQuestionOption } from "./tauri";

/**
 * Ephemeral store for an in-flight `AskUser` question.
 *
 * At most one question per chat session is pending at a time (the native turn
 * loop suspends on a single tool call). The store is in-memory only — questions
 * are transient UI prompts, never persisted: a question that outlived a restart
 * would point at a turn that is no longer running.
 *
 * Mirrors the subscribe/snapshot shape of `aiPendingFileReview` so the UI can
 * read it through `useSyncExternalStore`.
 */
export type PendingQuestion = {
  /** Stable id used to reconcile the resolve round-trip (the tool call id). */
  requestId: string;
  /** Turn id the question belongs to — used to resolve via the Rust command. */
  turnId: string;
  sessionId: string;
  question: string;
  detail: string;
  options: AiTurnQuestionOption[];
  multiSelect: boolean;
  allowCustom: boolean;
  /** Self-contained HTML5 document for a sandboxed preview, or "" when none. */
  htmlPreview: string;
  createdAt: number;
};

export type QuestionAnswer = { answer: string; cancelled: boolean };

type Listener = () => void;

let questions: PendingQuestion[] = [];
const listeners = new Set<Listener>();
/**
 * Promise resolvers for the browser/dev TS turn-loop, which (unlike the native
 * Rust path) has no oneshot channel to suspend on. `askUserTool` registers a
 * waiter here; `settlePendingQuestion` resolves it. The native path registers no
 * waiter — its suspension lives in Rust — so settle just clears the card there.
 */
const answerResolvers = new Map<string, (answer: QuestionAnswer) => void>();

function emit() {
  for (const listener of listeners) listener();
}

export function subscribePendingQuestions(listener: Listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getPendingQuestionsSnapshot() {
  return questions;
}

export function getPendingQuestionForSession(sessionId: string): PendingQuestion | null {
  return questions.find((entry) => entry.sessionId === sessionId) ?? null;
}

/**
 * Register a freshly-asked question. A new question for a session supersedes any
 * stale one (the prior turn cannot have two open prompts), so we replace rather
 * than stack — keeping the invariant of one pending question per session.
 */
export function registerPendingQuestion(entry: Omit<PendingQuestion, "createdAt">) {
  questions = [
    ...questions.filter((existing) => existing.sessionId !== entry.sessionId),
    { ...entry, createdAt: Date.now() },
  ];
  emit();
}

export function resolvePendingQuestion(requestId: string) {
  const before = questions.length;
  questions = questions.filter((entry) => entry.requestId !== requestId);
  if (questions.length !== before) emit();
}

/** Block on a human answer (browser/dev TS turn-loop only). */
export function waitForQuestionAnswer(requestId: string): Promise<QuestionAnswer> {
  return new Promise((resolve) => {
    answerResolvers.set(requestId, resolve);
  });
}

/**
 * Deliver an answer: resolve any dev-path waiter, then clear the card. Safe to
 * call on the native path (no waiter registered — it just clears the card while
 * Rust is unblocked separately via `aiResolveTurnQuestion`).
 */
export function settlePendingQuestion(requestId: string, answer: QuestionAnswer) {
  const resolver = answerResolvers.get(requestId);
  if (resolver) {
    answerResolvers.delete(requestId);
    resolver(answer);
  }
  resolvePendingQuestion(requestId);
}

export function clearPendingQuestionsForSession(sessionId: string) {
  const before = questions.length;
  const removed = questions.filter((entry) => entry.sessionId === sessionId);
  // Unblock any dev-path waiters for this session as cancelled so the turn-loop
  // tool call cannot hang after the session/turn is torn down.
  for (const entry of removed) {
    const resolver = answerResolvers.get(entry.requestId);
    if (resolver) {
      answerResolvers.delete(entry.requestId);
      resolver({ answer: "", cancelled: true });
    }
  }
  questions = questions.filter((entry) => entry.sessionId !== sessionId);
  if (questions.length !== before) emit();
}
