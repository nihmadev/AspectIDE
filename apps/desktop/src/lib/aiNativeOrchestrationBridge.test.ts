import { describe, expect, it } from "vitest";

import { bridgeNativeSubagentProgress, bridgeNativeToolCompleted } from "./aiNativeOrchestrationBridge";
import { getSubagentRun, registerSubagentRun } from "./aiSubagentRuns";

let seq = 0;
function freshRun(): string {
  seq += 1;
  const id = `call-${seq}`;
  registerSubagentRun({
    id,
    sessionId: `session-${seq}`,
    description: "test subagent",
    subagentType: "explorer",
    depth: 1,
    parentAgentId: null,
    abortController: new AbortController(),
  });
  return id;
}

describe("bridgeNativeSubagentProgress", () => {
  it("streams text snapshots into ONE transcript entry, replaced in place", () => {
    const id = freshRun();
    bridgeNativeSubagentProgress(id, "text", "Hello", "");
    bridgeNativeSubagentProgress(id, "text", "Hello world, longer snapshot", "");

    const run = getSubagentRun(id);
    expect(run?.transcript).toHaveLength(1);
    expect(run?.transcript[0].role).toBe("assistant");
    expect(run?.transcript[0].content).toBe("Hello world, longer snapshot");
  });

  it("appends tool lines as system entries and opens a fresh assistant entry after them", () => {
    const id = freshRun();
    bridgeNativeSubagentProgress(id, "text", "Round one narration", "");
    bridgeNativeSubagentProgress(id, "tool", '{"path":"src/a.rs"}', "Read");
    // Round two text must NOT rewrite round one — the last entry is a system
    // line, so the bridge appends a new assistant entry.
    bridgeNativeSubagentProgress(id, "text", "Round two narration", "");

    const run = getSubagentRun(id);
    expect(run?.transcript.map((entry) => entry.role)).toEqual(["assistant", "system", "assistant"]);
    expect(run?.transcript[0].content).toBe("Round one narration");
    expect(run?.transcript[1].content).toBe('→ Read {"path":"src/a.rs"}');
    expect(run?.transcript[2].content).toBe("Round two narration");
  });

  it("renders a tool line without arguments when the preview is empty", () => {
    const id = freshRun();
    bridgeNativeSubagentProgress(id, "tool", "", "TestHealth");
    expect(getSubagentRun(id)?.transcript[0].content).toBe("→ TestHealth");
  });

  it("ignores events for unknown runs and blank payloads", () => {
    // Unknown run id: must not throw or create a run.
    bridgeNativeSubagentProgress("missing-run", "text", "content", "");
    expect(getSubagentRun("missing-run")).toBeNull();

    const id = freshRun();
    bridgeNativeSubagentProgress(id, "text", "   ", "");
    bridgeNativeSubagentProgress(id, "tool", "preview", "   ");
    expect(getSubagentRun(id)?.transcript).toHaveLength(0);
  });

  it("settles the run on done/error stages (background-task completion path)", () => {
    const done = freshRun();
    bridgeNativeSubagentProgress(done, "done", "final summary", "");
    expect(getSubagentRun(done)?.status).toBe("completed");
    expect(getSubagentRun(done)?.summary).toBe("final summary");
    // A late event must not revive a settled run.
    bridgeNativeSubagentProgress(done, "text", "late stream", "");
    expect(getSubagentRun(done)?.transcript).toHaveLength(0);

    const failed = freshRun();
    bridgeNativeSubagentProgress(failed, "error", "provider exploded", "");
    expect(getSubagentRun(failed)?.status).toBe("failed");
    expect(getSubagentRun(failed)?.summary).toBe("provider exploded");
  });

  it("keeps a background Task's run row streaming when the tool call returns started", () => {
    const id = freshRun();
    bridgeNativeToolCompleted(
      getSubagentRun(id)!.sessionId,
      id,
      "Task",
      "success",
      JSON.stringify({ agentId: "explorer-1a2b3c4d", background: true, status: "started" }),
    );
    // Still running — the detached subagent settles it later via done/error.
    expect(getSubagentRun(id)?.status).toBe("running");
    // Foreground completion still settles immediately.
    bridgeNativeToolCompleted(
      getSubagentRun(id)!.sessionId,
      id,
      "Task",
      "success",
      JSON.stringify({ agentId: "explorer-1a2b3c4d", summary: "all good" }),
    );
    expect(getSubagentRun(id)?.status).toBe("completed");
    expect(getSubagentRun(id)?.summary).toBe("all good");
  });
});
