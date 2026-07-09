import { describe, expect, it } from "vitest";
import { listUnverifiedPathsInAssistantMessage, shouldShowPathEvidenceNotice } from "./path-evidence";
import type { AiChatMessage } from "./types";

function assistantMessage(content: string, toolOutput = ""): AiChatMessage {
  return {
    id: "m1",
    role: "assistant",
    content,
    timestamp: 0,
    toolCalls: [
      {
        id: "t1",
        tool: "Read",
        status: "success",
        input: "{}",
        output: toolOutput,
      },
    ],
  } as AiChatMessage;
}

const REPO_ROOTS: ReadonlySet<string> = new Set(["src", "crates", "apps", "docs"]);

describe("aiChatPathEvidence", () => {
  it("does not flag bare tool-name tokens or prose alternations as paths", () => {
    // The 1.0.12 report: "Some paths were not confirmed by tools in this turn:
    // read/, edit/, shell/, diff/ +11" — single-segment tokens must never be
    // treated as directory citations.
    const message = assistantMessage(
      "Проверил read/ edit/ shell/ diff/ группы; read/write доступ и edit/apply flow в порядке.",
    );
    expect(listUnverifiedPathsInAssistantMessage(message, REPO_ROOTS)).toEqual([]);
    expect(shouldShowPathEvidenceNotice(message, false, REPO_ROOTS)).toBe(false);
  });

  it("does not flag multi-segment prose slash-lists (the 1.0.13 report shapes)", () => {
    // The 1.0.13 report: "web/browser/MCP/SSH/, read/search/graph/,
    // edits/rollback/, replace/patch/read/ +11" — slash-separated word lists
    // whose first segment is not a real workspace directory are prose.
    const message = assistantMessage(
      "Подтверждено: web/browser/MCP/SSH/, read/search/graph/, edits/rollback/, replace/patch/read/ группы инструментов.",
    );
    expect(listUnverifiedPathsInAssistantMessage(message, REPO_ROOTS)).toEqual([]);
    expect(shouldShowPathEvidenceNotice(message, false, REPO_ROOTS)).toBe(false);
  });

  it("never flags directory citations when workspace roots are unknown", () => {
    const message = assistantMessage("Смотри src/components/settings/ и crates/lux-search/src/");
    expect(listUnverifiedPathsInAssistantMessage(message)).toEqual([]);
    expect(shouldShowPathEvidenceNotice(message, false)).toBe(false);
  });

  it("still flags real directory citations rooted in the workspace that no tool touched", () => {
    const message = assistantMessage("Смотри src/components/settings/ и crates/lux-search/src/");
    const unverified = listUnverifiedPathsInAssistantMessage(message, REPO_ROOTS);
    expect(unverified).toContain("src/components/settings/");
    expect(unverified).toContain("crates/lux-search/src/");
    expect(shouldShowPathEvidenceNotice(message, false, REPO_ROOTS)).toBe(true);
  });

  it("treats directories seen in tool output as verified", () => {
    const message = assistantMessage(
      "Файлы лежат в src/components/settings/",
      "listing: src/components/settings/SettingsControls.tsx",
    );
    expect(listUnverifiedPathsInAssistantMessage(message, REPO_ROOTS)).toEqual([]);
  });

  it("keeps file-path citations working without workspace roots", () => {
    const message = assistantMessage("Правь apps/desktop/src/lib/store.ts и Cargo.toml");
    const unverified = listUnverifiedPathsInAssistantMessage(message);
    expect(unverified).toContain("apps/desktop/src/lib/store.ts");
  });
});
