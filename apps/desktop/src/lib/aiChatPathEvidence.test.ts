import { describe, expect, it } from "vitest";
import { listUnverifiedPathsInAssistantMessage, shouldShowPathEvidenceNotice } from "./aiChatPathEvidence";
import type { AiChatMessage } from "./aiChatTypes";

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

describe("aiChatPathEvidence", () => {
  it("does not flag bare tool-name tokens or prose alternations as paths", () => {
    // The 1.0.12 report: "Some paths were not confirmed by tools in this turn:
    // read/, edit/, shell/, diff/ +11" — single-segment tokens must never be
    // treated as directory citations.
    const message = assistantMessage(
      "Проверил read/ edit/ shell/ diff/ группы; read/write доступ и edit/apply flow в порядке.",
    );
    expect(listUnverifiedPathsInAssistantMessage(message)).toEqual([]);
    expect(shouldShowPathEvidenceNotice(message, false)).toBe(false);
  });

  it("still flags real multi-segment directory citations that no tool touched", () => {
    const message = assistantMessage("Смотри src/components/settings/ и crates/lux-search/src/");
    const unverified = listUnverifiedPathsInAssistantMessage(message);
    expect(unverified).toContain("src/components/settings/");
    expect(unverified).toContain("crates/lux-search/src/");
    expect(shouldShowPathEvidenceNotice(message, false)).toBe(true);
  });

  it("treats directories seen in tool output as verified", () => {
    const message = assistantMessage(
      "Файлы лежат в src/components/settings/",
      "listing: src/components/settings/SettingsControls.tsx",
    );
    expect(listUnverifiedPathsInAssistantMessage(message)).toEqual([]);
  });

  it("keeps file-path citations working unchanged", () => {
    const message = assistantMessage("Правь apps/desktop/src/lib/store.ts и Cargo.toml");
    const unverified = listUnverifiedPathsInAssistantMessage(message);
    expect(unverified).toContain("apps/desktop/src/lib/store.ts");
  });
});
