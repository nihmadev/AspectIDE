import type { AiTurnPlanStep } from "./tauri";

/**
 * Ephemeral store for a proposed `PresentPlan` awaiting the user's "Start".
 *
 * One pending plan per chat session. In Plan/Agent mode the card stays until the
 * user starts it (or a newer plan/turn supersedes it); in Automatic mode the
 * card is still shown for transparency but execution auto-starts, so it is
 * cleared as soon as the turn moves on. In-memory only — a plan is a live
 * proposal tied to the running turn, not durable session state.
 */
export type PendingPlan = {
  planId: string;
  turnId: string;
  sessionId: string;
  title: string;
  summary: string;
  steps: AiTurnPlanStep[];
  /** True when the agent auto-starts execution (Automatic mode) — no Start button. */
  autoStart: boolean;
  createdAt: number;
};

type Listener = () => void;

let plans: PendingPlan[] = [];
const listeners = new Set<Listener>();

function emit() {
  for (const listener of listeners) listener();
}

export function subscribePendingPlans(listener: Listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getPendingPlansSnapshot() {
  return plans;
}

export function getPendingPlanForSession(sessionId: string): PendingPlan | null {
  return plans.find((entry) => entry.sessionId === sessionId) ?? null;
}

/** Register a freshly-proposed plan, superseding any prior plan for the session. */
export function registerPendingPlan(entry: Omit<PendingPlan, "createdAt">) {
  plans = [
    ...plans.filter((existing) => existing.sessionId !== entry.sessionId),
    { ...entry, createdAt: Date.now() },
  ];
  emit();
}

export function clearPendingPlan(planId: string) {
  const before = plans.length;
  plans = plans.filter((entry) => entry.planId !== planId);
  if (plans.length !== before) emit();
}

export function clearPendingPlansForSession(sessionId: string) {
  const before = plans.length;
  plans = plans.filter((entry) => entry.sessionId !== sessionId);
  if (plans.length !== before) emit();
}
