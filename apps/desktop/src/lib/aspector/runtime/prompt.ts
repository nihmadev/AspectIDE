import { isGoalOrchestrationMessage } from "./../chat/goal-orchestration";
import { isCompactionCheckpointMessage } from "./../chat/context-compaction";
import type { AiChatAttachmentInput, AiChatMessage, AiChatSendInput } from "./../chat/types";
import type { ChatCompletionMessage, ChatContentPart } from "./../chat/transport";
import { normalizeVisibleReasoning } from "./../chat/reasoning";
import { buildGoalObjectiveBlock } from "./../session/goal/run-limits";
import { buildGoalRunPromptSection } from "./../session/goal/run-prompt";
import { getAiSessionGoal } from "./../session/goal/session-goal";
import { listAiSessionTodos } from "./../session/todos";
import { truncateText } from "./shared";
import { compactTerminalContext } from "./terminal";
import { isAutomaticSocialOnlyMessage } from "./../automatic/social-message";
import { loadProjectAgentsSnip } from "./../utils/project-agents-snip";
import { buildLuxIdeSystemPromptAsync } from "./../utils/system-prompt";
import { isTauriRuntime } from "./../../tauri/commands";

const contextPayloadBudgetChars = 120_000;
const maxHistoryMessages = 64;
const maxActiveDocumentChars = 24_000;
const maxAttachmentBudgetChars = 36_000;
const maxHistoryMessageChars = 8_000;
const maxHistoryToolOutputChars = 2_000;

export async function buildInitialMessages(input: AiChatSendInput): Promise<ChatCompletionMessage[]> {
  const projectAgentsSnip = isTauriRuntime() ? await loadProjectAgentsSnip(input) : "";
  const system = await buildLuxIdeSystemPromptAsync({
    preferences: input.preferences,
    provider: input.provider,
    globalInstructions: input.globalInstructions,
    projectInstructions: input.projectInstructions,
    projectAgentsSnip,
    runtimeToolsAvailable: isTauriRuntime(),
    agentBrowserEnabled: isTauriRuntime() && input.preferences.agentBrowserEnabled,
    selectedAgentInstructions: input.selectedAgentInstructions,
    selectedAgentName: input.selectedAgentName,
    selectedModel: input.selectedModel,
    workspace: input.workspace,
  });

  const messages: ChatCompletionMessage[] = [{ role: "system", content: system }];
  const currentUserContent = buildUserContent(input);
  const userContentLength = estimateMessageContentLength(currentUserContent);
  const budgetForHistory = Math.max(8_000, contextPayloadBudgetChars - system.length - userContentLength);
  messages.push(...compactHistoryMessages(input.history, budgetForHistory));
  messages.push({ role: "user", content: currentUserContent });
  return messages;
}

export function buildUserContent(input: AiChatSendInput): string | ChatContentPart[] {
  const requestText = input.message.trim() || (input.attachments.length > 0 ? "(Attachments only — no text message.)" : "");
  const sections = [`User request:\n${requestText}`];
  const sessionGoal = getAiSessionGoal(input.chatSessionId);
  if (sessionGoal) {
    sections.push(buildGoalObjectiveBlock(sessionGoal));
  }
  const goalRunSection = buildGoalRunPromptSection(input.chatSessionId);
  if (goalRunSection) sections.push(goalRunSection);
  const sessionTodos = listAiSessionTodos(input.chatSessionId);
  if (sessionTodos.length > 0) {
    const lines = sessionTodos.map((todo) => `- [${todo.status}] ${todo.content}`).join("\n");
    sections.push(`Active task list:\n${lines}`);
  }
  if (input.preferences.agentMode === "automatic") {
    sections.push(
      isAutomaticSocialOnlyMessage(requestText)
        ? "Runtime: Automatic mode is active, but this message is social-only (greeting/small talk). Reply briefly and warmly in 1–3 sentences. Do not call tools, scan the repo, or list project options until the user states a task."
        : "Runtime: Automatic mode is active. Execute autonomously with tools in this turn — inspect the workspace if needed, then implement. Do not ask clarifying questions unless execution is impossible without external credentials.",
    );
  }
  if (input.mentionHints?.codebase) {
    sections.push("Composer mention: @codebase — prioritize SemanticSearch, FastContext, and RepoMap before broad edits.");
  }
  if (input.mentionHints?.docs) {
    sections.push("Composer mention: @docs — load RulesContext and DocsContext before changing code.");
  }
  if (input.attachments.length > 0) {
    let remainingAttachmentChars = maxAttachmentBudgetChars;
    const compactAttachments = input.attachments.map((attachment) => {
      if (attachment.visionImageUrl) {
        const summary = truncateText(attachment.text, 800);
        remainingAttachmentChars = Math.max(0, remainingAttachmentChars - summary.length);
        return `### ${attachment.name} (${attachment.size} bytes)\n${summary}`;
      }
      const allowance = Math.max(0, remainingAttachmentChars);
      if (allowance <= 0) {
        return `### ${attachment.name} (${attachment.size} bytes)\n(omitted — attachment budget exhausted)`;
      }
      const text = truncateText(attachment.text, allowance);
      remainingAttachmentChars = Math.max(0, remainingAttachmentChars - text.length);
      return `### ${attachment.name} (${attachment.size} bytes)\n\`\`\`\n${text}\n\`\`\``;
    });
    sections.push(`Pinned context (explicit attachments and editor tabs dropped into chat):\n${compactAttachments.join("\n\n")}`);
  }
  const terminalSnapshot = compactTerminalContext(input, 1_600);
  if (terminalSnapshot.sessions.length > 0) {
    sections.push(`Integrated terminal snapshot:\n\`\`\`json\n${truncateText(JSON.stringify(terminalSnapshot, null, 2), 4_000)}\n\`\`\``);
  }
  const text = sections.join("\n\n");
  const visionParts = visionImageParts(input.attachments);
  if (visionParts.length === 0) return text;
  return [{ type: "text", text }, ...visionParts];
}

function visionImageParts(attachments: AiChatAttachmentInput[]): ChatContentPart[] {
  const parts: ChatContentPart[] = [];
  for (const attachment of attachments) {
    if (attachment.visionImageUrl) {
      parts.push({
        type: "image_url",
        image_url: { url: attachment.visionImageUrl, detail: "auto" },
      });
    }
    for (const frameUrl of attachment.visionFrameUrls ?? []) {
      parts.push({
        type: "image_url",
        image_url: { url: frameUrl, detail: "low" },
      });
    }
  }
  return parts;
}

function estimateMessageContentLength(content: string | ChatContentPart[]) {
  if (typeof content === "string") return content.length;
  return content.reduce((sum, part) => {
    if (part.type === "text") return sum + part.text.length;
    return sum + 256;
  }, 0);
}

function compactHistoryMessages(history: AiChatMessage[], budgetChars: number): ChatCompletionMessage[] {
  const selected: ChatCompletionMessage[] = [];
  let used = 0;
  const recent = history.slice(-maxHistoryMessages).reverse();
  let omitted = Math.max(0, history.length - recent.length);
  for (let i = 0; i < recent.length; i++) {
    const message = recent[i];
    const content = compactHistoryMessageContent(message);
    if (!content.trim()) continue;
    const cost = content.length + 64;
    if (selected.length > 0 && used + cost > budgetChars) {
      omitted += recent.length - i;
      break;
    }
    selected.push({ role: message.role, content });
    used += cost;
  }
  selected.reverse();
  if (omitted > 0) {
    selected.unshift({
      role: "system",
      content: `Earlier conversation compacted: ${omitted} earlier message(s) were omitted to keep the current request responsive. Use tools/context if exact details are needed.`,
    });
  }
  return selected;
}

function compactHistoryMessageContent(message: AiChatMessage) {
  if (isCompactionCheckpointMessage(message)) {
    return truncateText(message.content, maxHistoryMessageChars * 2);
  }
  if (isGoalOrchestrationMessage(message)) {
    return truncateText(`[goal orchestration — not shown in chat UI]\n${message.content}`, maxHistoryMessageChars);
  }
  const parts: string[] = [];
  const reasoning = normalizeVisibleReasoning(message.reasoning);
  if (reasoning) parts.push(`[reasoning summary]\n${truncateText(reasoning, 1_200)}`);
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

/**
 * Map a stored chat message to the `{role, content}` the native Rust turn-loop
 * hands to the model. Unlike a bare `content` copy (what the native path used to
 * send), this FOLDS the assistant's reasoning + tool results into the text — so a
 * turn that was stopped mid-work, whose progress lives only in `toolCalls`/
 * `segments` and not in `content`, still tells the model what it already did when
 * the user later says "continue"/"продолжи". Returns "" for a message with no
 * usable content (an empty assistant shell left by a hung/stopped turn); the
 * caller filters those out so they never poison the next turn's history.
 */
export function buildNativeHistoryContent(message: AiChatMessage): string {
  if (isCompactionCheckpointMessage(message)) return message.content;
  if (isGoalOrchestrationMessage(message)) return message.content;
  const parts: string[] = [];
  if (message.content.trim()) parts.push(truncateText(message.content, maxHistoryMessageChars));
  const toolCalls = message.toolCalls?.filter((call) => call.output || call.error) ?? [];
  if (toolCalls.length > 0) {
    parts.push(`Tool results:\n${toolCalls.slice(-12).map((call) => {
      const detail = call.error ? `error: ${call.error}` : call.output ?? "";
      return `- ${call.tool} (${call.status}): ${truncateText(detail, maxHistoryToolOutputChars)}`;
    }).join("\n")}`);
  }
  if (parts.length === 0) {
    const reasoning = normalizeVisibleReasoning(message.reasoning);
    if (reasoning) parts.push(`[reasoning]\n${truncateText(reasoning, 1_200)}`);
  }
  // A user turn that carried only attachments (no typed text) is stored with an
  // empty `content`. Never let it fold to "" — the caller drops empty entries, and
  // dropping a user turn loses the turn boundary (and can leave history starting
  // with an assistant message, which the Anthropic API rejects with a 400).
  if (parts.length === 0 && message.role === "user") {
    return (message.attachments?.length ?? 0) > 0 ? "(Attachments only — no text message.)" : "";
  }
  return parts.join("\n\n");
}

export const activeDocumentContextMaxChars = maxActiveDocumentChars;
