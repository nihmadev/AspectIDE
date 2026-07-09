import type { AiChatSendInput } from "./types";
import type { AiModelConfig, AiPreferences, AiProviderConfig } from "./../utils/preferences";
import type { Locale } from "../../i18n";
import type { DocumentSnapshot, TerminalSessionInfo, WorkspaceInfo } from "./../../types/index";
import type { TerminalOutputBuffer } from "./../../terminal/types";

export function buildCheckpointSendInput(params: {
  activeDocument: DocumentSnapshot | null;
  aiPreferences: AiPreferences;
  locale: Locale;
  openDocuments: DocumentSnapshot[];
  selectedModel: AiModelConfig;
  selectedProvider: AiProviderConfig;
  terminal: TerminalSessionInfo | null;
  terminalOutputBuffers: Record<string, TerminalOutputBuffer>;
  terminalSessions: TerminalSessionInfo[];
  workspace: WorkspaceInfo;
}): AiChatSendInput {
  return {
    abortSignal: new AbortController().signal,
    activeDocument: params.activeDocument,
    attachments: [],
    chatSessionId: "checkpoint",
    history: [],
    locale: params.locale,
    message: "",
    onAssistantMessage: () => undefined,
    onAssistantMessageUpdate: () => undefined,
    onToolApproval: async () => "approved",
    openDocuments: params.openDocuments,
    preferences: params.aiPreferences,
    projectInstructions: "",
    globalInstructions: "",
    provider: params.selectedProvider,
    selectedAgentInstructions: "",
    selectedAgentName: "",
    selectedModel: params.selectedModel,
    terminal: params.terminal,
    terminalContext: {
      activeTerminalId: params.terminal?.id ?? null,
      outputBuffers: params.terminalOutputBuffers,
      sessions: params.terminalSessions,
    },
    workspace: params.workspace,
  };
}