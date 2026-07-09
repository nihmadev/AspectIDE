import type { AiChatMessage } from "./types";

const checklistPattern = /^\s*(?:[-*]|\d+[.)])\s+/;

/**
 * Legacy "Run in Agent" handoff: detect a plan the model wrote as a markdown
 * checklist in its message text (no `PresentPlan` call). Superseded by the rich
 * `PresentPlan` card — so it now fires ONLY for a genuine multi-step checklist
 * (≥2 real list items). A prose answer, a single line, or a bare "Done." is NOT a
 * plan and must not surface a handoff card (that was the old false-positive: any
 * non-empty reply, even "Done.", became a 1-step "plan").
 */
export function extractPlanHandoffPayload(messages: AiChatMessage[]) {
  const lastAssistant = [...messages].reverse().find((message) => message.role === "assistant");
  if (!lastAssistant) return null;
  const content = lastAssistant.content.trim();
  if (!content) return null;
  const steps = content
    .split(/\r?\n/)
    .filter((line) => checklistPattern.test(line))
    .map((line) => line.replace(/^\s*(?:[-*]|\d+[.)])\s+/, "").trim())
    .filter(Boolean);
  // Require a real multi-step checklist; one item (or none) is not a plan.
  if (steps.length < 2) return null;
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