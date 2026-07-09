import { beforeEach, describe, expect, it } from "vitest";
import type { AiToolApprovalDecision } from "./types";
import {
  abortAiChatTurn,
  consumeStopAfterToolRound,
  finishAiChatTurn,
  getAiChatTurnRuntimeSnapshot,
  getTurnGeneration,
  requestAiToolApproval,
  requestStopAfterToolRound,
  resolveAiToolApproval,
  startAiChatTurn,
} from "./turn-runtime";

const SESSION_A = "session-a";
const SESSION_B = "session-b";

/** Observe whether an approval promise has settled without blocking the test. */
function track(promise: Promise<AiToolApprovalDecision>) {
  const state: { settled: boolean; value: AiToolApprovalDecision | null } = { settled: false, value: null };
  promise.then((value) => {
    state.settled = true;
    state.value = value;
  });
  return state;
}

const flush = () => Promise.resolve();

beforeEach(() => {
  // Wipe all module-level turn state between tests (clears controllers, generations,
  // stop flags, and pending approvals) so cases cannot leak into one another.
  abortAiChatTurn(null);
});

describe("stop-after-tool-round scoping", () => {
  it("does not let one session's stop flag be consumed by another session", () => {
    startAiChatTurn(SESSION_A, new AbortController());
    startAiChatTurn(SESSION_B, new AbortController());

    requestStopAfterToolRound(SESSION_A, getTurnGeneration(SESSION_A));

    // B never requested a stop: its loop must keep running.
    expect(consumeStopAfterToolRound(SESSION_B, getTurnGeneration(SESSION_B))).toBe(false);
    // A's flag is intact and consumable exactly once.
    expect(consumeStopAfterToolRound(SESSION_A, getTurnGeneration(SESSION_A))).toBe(true);
    expect(consumeStopAfterToolRound(SESSION_A, getTurnGeneration(SESSION_A))).toBe(false);
  });

  it("ignores a stop request left over from a previous generation", () => {
    startAiChatTurn(SESSION_A, new AbortController());
    const staleGeneration = getTurnGeneration(SESSION_A);
    requestStopAfterToolRound(SESSION_A, staleGeneration);

    // A new turn supersedes the old generation; the stale stop must not stop it.
    startAiChatTurn(SESSION_A, new AbortController());
    expect(consumeStopAfterToolRound(SESSION_A, getTurnGeneration(SESSION_A))).toBe(false);
  });

  it("mirrors only the active session's stop flag into the UI snapshot", () => {
    startAiChatTurn(SESSION_A, new AbortController());
    startAiChatTurn(SESSION_B, new AbortController()); // B becomes the active sending session

    requestStopAfterToolRound(SESSION_A, getTurnGeneration(SESSION_A));
    // The composer button reflects the active session (B), which has no stop pending.
    expect(getAiChatTurnRuntimeSnapshot().stopAfterToolRound).toBe(false);

    requestStopAfterToolRound(SESSION_B, getTurnGeneration(SESSION_B));
    expect(getAiChatTurnRuntimeSnapshot().stopAfterToolRound).toBe(true);
  });

  it("keeps the zero-arg UI path targeting the active session", () => {
    startAiChatTurn(SESSION_A, new AbortController());
    requestStopAfterToolRound(); // no args → active session
    expect(getAiChatTurnRuntimeSnapshot().stopAfterToolRound).toBe(true);
    expect(consumeStopAfterToolRound()).toBe(true);
    expect(getAiChatTurnRuntimeSnapshot().stopAfterToolRound).toBe(false);
  });
});

describe("tool-approval scoping", () => {
  it("rejects only the finishing session's approvals, leaving siblings pending", async () => {
    const controllerA = new AbortController();
    startAiChatTurn(SESSION_A, controllerA);
    startAiChatTurn(SESSION_B, new AbortController());

    const approvalA = track(requestAiToolApproval("approval-a", SESSION_A, getTurnGeneration(SESSION_A)));
    const approvalB = track(requestAiToolApproval("approval-b", SESSION_B, getTurnGeneration(SESSION_B)));

    finishAiChatTurn(SESSION_A, controllerA);
    await flush();

    // A's approval was force-rejected; B's is untouched and still awaiting a decision.
    expect(approvalA.settled).toBe(true);
    expect(approvalA.value).toBe("rejected");
    expect(approvalB.settled).toBe(false);

    resolveAiToolApproval("approval-b", "approved");
    await flush();
    expect(approvalB.settled).toBe(true);
    expect(approvalB.value).toBe("approved");
  });

  it("does not let aborting one session reject another session's approval", async () => {
    startAiChatTurn(SESSION_A, new AbortController());
    startAiChatTurn(SESSION_B, new AbortController());

    const approvalA = track(requestAiToolApproval("approval-a", SESSION_A, getTurnGeneration(SESSION_A)));
    const approvalB = track(requestAiToolApproval("approval-b", SESSION_B, getTurnGeneration(SESSION_B)));

    abortAiChatTurn(SESSION_A);
    await flush();

    expect(approvalA.value).toBe("rejected");
    expect(approvalB.settled).toBe(false);
  });
});
