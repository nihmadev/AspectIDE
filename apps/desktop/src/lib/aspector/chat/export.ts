import type { AiChatMessage } from "./types";
import type { AiChatSession } from "./../../store/index";
import { deriveSegmentContent, deriveSegmentToolCalls } from "./types";

export function exportChatSessionMarkdown(session: AiChatSession, workspaceRoot: string | null) {
  const title = session.title.trim() || "Lux chat";
  const lines: string[] = [
    `# ${title}`,
    "",
    `- Session: \`${session.id}\``,
    workspaceRoot ? `- Workspace: \`${workspaceRoot}\`` : "- Workspace: _(none)_",
    `- Exported: ${new Date().toISOString()}`,
    "",
  ];

  for (const message of session.messages) {
    lines.push(...formatExportMessage(message));
    lines.push("");
  }

  return lines.join("\n").trimEnd() + "\n";
}

function formatExportMessage(message: AiChatMessage) {
  const role = message.role === "user" ? "User" : "Assistant";
  const chunks: string[] = [`## ${role}`];
  const content = message.segments?.length
    ? deriveSegmentContent(message.segments)
    : message.content;
  if (content.trim()) chunks.push(content.trim());
  const tools = message.segments?.length
    ? deriveSegmentToolCalls(message.segments)
    : message.toolCalls ?? [];
  for (const call of tools) {
    chunks.push(`### Tool: ${call.tool} (${call.status})`);
    if (call.input?.trim()) chunks.push("```json\n" + call.input.trim() + "\n```");
    if (call.output?.trim()) chunks.push("```\n" + call.output.trim() + "\n```");
    if (call.error?.trim()) chunks.push(`> ${call.error.trim()}`);
  }
  if (message.turnUsage) {
    chunks.push(`_Tokens: in ${message.turnUsage.promptTokens}, out ${message.turnUsage.completionTokens}_`);
  }
  return chunks;
}