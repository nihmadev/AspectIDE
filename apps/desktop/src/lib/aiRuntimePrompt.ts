import type { AiChatMessage, AiChatSendInput } from "./aiChatTypes";
import type { ChatCompletionMessage } from "./aiChatTransport";
import { truncateText } from "./aiRuntimeShared";
import { compactTerminalContext } from "./aiRuntimeTerminal";
import { buildLuxIdeSystemPrompt } from "./aiSystemPrompt";
import { isTauriRuntime } from "./tauri";

const contextPayloadBudgetChars = 120_000;
const maxHistoryMessages = 64;
const maxActiveDocumentChars = 24_000;
const maxAttachmentBudgetChars = 36_000;
const maxHistoryMessageChars = 8_000;
const maxHistoryToolOutputChars = 2_000;

export function buildInitialMessages(input: AiChatSendInput): ChatCompletionMessage[] {
  const system = buildLuxIdeSystemPrompt({
    preferences: input.preferences,
    provider: input.provider,
    runtimeToolsAvailable: isTauriRuntime(),
    selectedAgentInstructions: input.selectedAgentInstructions,
    selectedAgentName: input.selectedAgentName,
    selectedModel: input.selectedModel,
    workspace: input.workspace,
  });

  const messages: ChatCompletionMessage[] = [{ role: "system", content: system }];
  const currentUserContent = buildUserContent(input);
  const budgetForHistory = Math.max(8_000, contextPayloadBudgetChars - system.length - currentUserContent.length);
  messages.push(...compactHistoryMessages(input.history, budgetForHistory));
  messages.push({ role: "user", content: currentUserContent });
  return messages;
}

export function buildUserContent(input: AiChatSendInput) {
  const sections = [`User request:\n${input.message.trim()}`];
  if (input.activeDocument) {
    const path = input.activeDocument.path ?? input.activeDocument.title;
    sections.push(`Active document (${path}, ${input.activeDocument.language_id}, dirty=${input.activeDocument.is_dirty}):\n\`\`\`${input.activeDocument.language_id}\n${truncateText(input.activeDocument.text, maxActiveDocumentChars)}\n\`\`\``);
  }
  if (input.attachments.length > 0) {
    let remainingAttachmentChars = maxAttachmentBudgetChars;
    const compactAttachments = input.attachments.map((attachment) => {
      const text = truncateText(attachment.text, Math.max(1_000, remainingAttachmentChars));
      remainingAttachmentChars = Math.max(0, remainingAttachmentChars - text.length);
      return `### ${attachment.name} (${attachment.size} bytes)\n\`\`\`\n${text}\n\`\`\``;
    });
    sections.push(`Attachments:\n${compactAttachments.join("\n\n")}`);
  }
  const terminalSnapshot = compactTerminalContext(input, 1_600);
  if (terminalSnapshot.sessions.length > 0) {
    sections.push(`Integrated terminal snapshot:\n\`\`\`json\n${truncateText(JSON.stringify(terminalSnapshot, null, 2), 4_000)}\n\`\`\``);
  }
  return sections.join("\n\n");
}

function compactHistoryMessages(history: AiChatMessage[], budgetChars: number): ChatCompletionMessage[] {
  const selected: ChatCompletionMessage[] = [];
  let used = 0;
  const recent = history.slice(-maxHistoryMessages).reverse();
  for (const message of recent) {
    const content = compactHistoryMessageContent(message);
    if (!content.trim()) continue;
    const cost = content.length + 64;
    if (selected.length > 0 && used + cost > budgetChars) break;
    selected.push({ role: message.role, content });
    used += cost;
  }
  selected.reverse();
  if (history.length > selected.length) {
    selected.unshift({
      role: "system",
      content: `Earlier conversation compacted: ${history.length - selected.length} older message(s) were omitted to keep the current request responsive. Use tools/context if exact older details are needed.`,
    });
  }
  return selected;
}

function compactHistoryMessageContent(message: AiChatMessage) {
  const parts: string[] = [];
  if (message.reasoning?.trim()) parts.push(`[reasoning summary]\n${truncateText(message.reasoning, 1_200)}`);
  if (message.content.trim()) parts.push(truncateText(message.content, maxHistoryMessageChars));
  const toolCalls = message.toolCalls?.filter((call) => call.output || call.error).slice(-8) ?? [];
  if (toolCalls.length > 0) {
    parts.push(`Tool results:\n${toolCalls.map((call) => {
      const detail = call.error ? `error: ${call.error}` : call.output ?? "";
      return `- ${call.tool} (${call.status}): ${truncateText(detail, maxHistoryToolOutputChars)}`;
    }).join("\n")}`);
  }
  return parts.join("\n\n");
}

export const activeDocumentContextMaxChars = maxActiveDocumentChars;
