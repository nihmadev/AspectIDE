import type { AiChatMessage } from "./aiChatTypes";

const checklistPattern = /^\s*(?:[-*]|\d+[.)])\s+/m;

export function extractPlanHandoffPayload(messages: AiChatMessage[]) {
  const lastAssistant = [...messages].reverse().find((message) => message.role === "assistant");
  if (!lastAssistant) return null;
  const content = lastAssistant.content.trim();
  if (!content) return null;
  const lines = content.split(/\r?\n/);
  const checklistLines = lines.filter((line) => checklistPattern.test(line));
  const steps = checklistLines.length > 0
    ? checklistLines.map((line) => line.replace(/^\s*(?:[-*]|\d+[.)])\s+/, "").trim()).filter(Boolean)
    : lines
      .map((line) => line.trim())
      .filter((line) => line.length > 0 && !line.startsWith("#"))
      .slice(0, 12);
  if (steps.length === 0) return null;
  return {
    planMessageId: lastAssistant.id,
    steps,
    summary: content.slice(0, 2_000),
  };
}

export function buildPlanHandoffUserMessage(steps: string[]) {
  const numbered = steps.map((step, index) => `${index + 1}. ${step}`).join("\n");
  return [
    "Execute the approved plan below in Agent mode.",
    "Work through every step end-to-end, verify each outcome, and report what changed.",
    "Do not re-plan unless a step is blocked.",
    "",
    numbered,
  ].join("\n");
}