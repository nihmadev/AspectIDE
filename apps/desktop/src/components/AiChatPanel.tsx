import { Bot, Brain, CornerDownLeft, Mic, PanelRightClose, Paperclip, SendHorizontal, ShieldCheck, Sparkles, Square, Wrench, X } from "lucide-react";
import type { ChangeEvent, ClipboardEvent, DragEvent, KeyboardEvent } from "react";
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import { CompactDropdown } from "./CompactDropdown";
import { AiToolCallsGroup } from "./AiToolCall";
import { AiToolsView } from "./AiToolsView";
import { documentDisplayPath } from "../lib/documents";
import { useTranslation, type TranslateFn } from "../lib/i18n/useTranslation";
import { AI_PREFERENCES_KEY, getAiModel, getAiProvider, mergeAiPreferences, type AiPreferences, type AiToolApprovalMode } from "../lib/aiPreferences";
import { readChatAttachment, sendAiChatMessage, type AiChatAttachmentInput, type AiChatMessage, type AiToolApprovalDecision, type AiToolApprovalRequest } from "../lib/aiChatRuntime";
import { useLuxStore } from "../lib/store";
import { luxCommands } from "../lib/tauri";
import { useVoiceInput } from "../lib/useVoiceInput";

type ChatAttachment = {
  file: File;
  id: string;
  name: string;
  size: number;
};

type ContextUsageRow = {
  color: string;
  detail: string;
  id: string;
  label: string;
  percent: number;
  tokens: number;
};

type ContextUsageSummary = {
  percent: number;
  rows: ContextUsageRow[];
  tokenBudget: number;
  totalTokens: number;
};

const contextTokenBudget = 200_000;

const contextRowColors = {
  agent: "#9aa0a6",
  model: "#8fb5d9",
  index: "#57c178",
  files: "#ffc46b",
  conversation: "#6d8589",
} as const;

export function AiChatPanel() {
  const activeDocumentId = useLuxStore((state) => state.activeDocumentId);
  const aiIndex = useLuxStore((state) => state.aiIndex);
  const aiPreferences = useLuxStore((state) => state.aiPreferences);
  const setAiPreferences = useLuxStore((state) => state.setAiPreferences);
  const openDocuments = useLuxStore((state) => state.openDocuments);
  const setAiChatOpen = useLuxStore((state) => state.setAiChatOpen);
  const terminal = useLuxStore((state) => state.terminal);
  const workspace = useLuxStore((state) => state.workspace);
  const { t } = useTranslation();
  const [message, setMessage] = useState("");
  const [messages, setMessages] = useState<AiChatMessage[]>([]);
  const [attachments, setAttachments] = useState<ChatAttachment[]>([]);
  const [contextOpen, setContextOpen] = useState(false);
  const [toolsViewOpen, setToolsViewOpen] = useState(false);
  const [draggingFiles, setDraggingFiles] = useState(false);
  const [sending, setSending] = useState(false);
  const [sendError, setSendError] = useState<string | null>(null);
  const abortControllerRef = useRef<AbortController | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const messagesEndRef = useRef<HTMLDivElement | null>(null);
  const approvalResolversRef = useRef(new Map<string, (decision: AiToolApprovalDecision) => void>());
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  const activeDocument = useMemo(
    () => openDocuments.find((document) => document.id === activeDocumentId) ?? null,
    [activeDocumentId, openDocuments],
  );

  const selectedProvider = getAiProvider(aiPreferences.providers, aiPreferences.selectedProviderId) ?? aiPreferences.providers[0] ?? null;
  const selectedModel = getAiModel(selectedProvider, aiPreferences.selectedModelId) ?? selectedProvider?.models[0] ?? null;
  const selectedAgent = aiPreferences.agentProfiles.find((profile) => profile.id === aiPreferences.selectedAgentId) ?? aiPreferences.agentProfiles[0] ?? null;
  const modelSupportsEffort = Boolean(selectedModel?.effortLevels.length);
  const agentOptions = aiPreferences.agentProfiles.map((profile) => ({ label: profile.name, value: profile.id }));
  const modelOptions = selectedProvider?.models.map((model) => ({ label: model.name, value: model.id })) ?? [];
  const effortOptions = selectedModel?.effortLevels.map((effort) => ({ label: effort.label, value: effort.id })) ?? [];
  const toolApprovalOptions: Array<{ label: string; value: AiToolApprovalMode }> = [
    { label: t("aiChat.toolApproval.default"), value: "default" },
    { label: t("aiChat.toolApproval.fullAccess"), value: "full-access" },
  ];
  const contextUsage = useMemo(() => buildContextUsageSummary({
    activeDocumentPath: activeDocument ? documentDisplayPath(activeDocument) : null,
    aiIndexStatus: aiIndex.status,
    agentInstruction: selectedAgent?.instructions ?? "",
    agentName: selectedAgent?.name ?? "",
    attachments,
    message,
    preferences: aiPreferences,
    selectedModelAlias: selectedModel?.alias ?? selectedModel?.id ?? "",
    t,
  }), [activeDocument, aiIndex.status, aiPreferences, attachments, message, selectedAgent, selectedModel, t]);
  const contextLabel = t("aiChat.context.percentBadge", { percent: contextUsage.percent });
  const contextTitle = t("aiChat.context.tooltip", {
    percent: contextUsage.percent,
    totalTokens: formatCompactTokens(contextUsage.totalTokens),
    tokenBudget: formatCompactTokens(contextUsage.tokenBudget),
  });
  const canSend = Boolean(selectedProvider && selectedModel && message.trim()) && !sending;

  const updateAiPreference = useCallback((patch: Partial<AiPreferences>) => {
    const nextPreferences = mergeAiPreferences(aiPreferences, patch);
    setAiPreferences(nextPreferences);
    void luxCommands.settingsSet("user", AI_PREFERENCES_KEY, nextPreferences).catch(() => undefined);
  }, [aiPreferences, setAiPreferences]);

  const updateModel = useCallback((selectedModelId: string) => {
    updateAiPreference({ selectedModelId });
  }, [updateAiPreference]);

  const resizeComposerTextarea = useCallback((target?: HTMLTextAreaElement | null) => {
    const textarea = target ?? textareaRef.current;
    if (!textarea) return;
    const maxHeight = 132;
    textarea.style.height = "auto";
    const nextHeight = Math.min(maxHeight, Math.max(24, textarea.scrollHeight));
    textarea.style.height = `${nextHeight}px`;
    textarea.style.overflowY = textarea.scrollHeight > maxHeight ? "auto" : "hidden";
  }, []);

  useLayoutEffect(() => {
    resizeComposerTextarea();
  }, [message]);

  useLayoutEffect(() => {
    messagesEndRef.current?.scrollIntoView({ block: "end" });
  }, [messages, toolsViewOpen]);

  const updateMessage = useCallback((nextMessage: string) => {
    setMessage(nextMessage);
    requestAnimationFrame(() => resizeComposerTextarea());
  }, [resizeComposerTextarea]);

  const handleMessageChange = useCallback((event: ChangeEvent<HTMLTextAreaElement>) => {
    resizeComposerTextarea(event.currentTarget);
    updateMessage(event.currentTarget.value);
  }, [resizeComposerTextarea, updateMessage]);
  const voiceInput = useVoiceInput({ message, preferences: aiPreferences, updateMessage });

  const attachFiles = (files: FileList | File[] | null) => {
    if (!files || files.length === 0) return;
    setAttachments((current) => {
      const byId = new Map(current.map((attachment) => [attachment.id, attachment]));
      for (const file of Array.from(files)) byId.set(attachmentId(file), { file, id: attachmentId(file), name: file.name, size: file.size });
      return [...byId.values()];
    });
  };

  const removeAttachment = (id: string) => {
    setAttachments((current) => current.filter((attachment) => attachment.id !== id));
  };

  const handleComposerDragOver = (event: DragEvent<HTMLDivElement>) => {
    if (!event.dataTransfer.types.includes("Files")) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setDraggingFiles(true);
  };

  const handleComposerDrop = (event: DragEvent<HTMLDivElement>) => {
    if (!event.dataTransfer.types.includes("Files")) return;
    event.preventDefault();
    setDraggingFiles(false);
    attachFiles(event.dataTransfer.files);
  };

  const handleComposerPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(event.clipboardData.files);
    if (files.length === 0) return;
    event.preventDefault();
    attachFiles(files);
  };

  const handleCancelSend = useCallback(() => {
    abortControllerRef.current?.abort();
    resolveAllToolApprovals("rejected");
  }, []);

  const appendAssistantMessage = useCallback((assistantMessage: AiChatMessage) => {
    setMessages((current) => current.some((candidate) => candidate.id === assistantMessage.id) ? current : [...current, assistantMessage]);
  }, []);

  const updateAssistantMessage = useCallback((messageId: string, patch: Partial<AiChatMessage>) => {
    setMessages((current) => current.map((candidate) => candidate.id === messageId ? { ...candidate, ...patch } : candidate));
  }, []);

  const requestToolApproval = useCallback((request: AiToolApprovalRequest) => {
    return new Promise<AiToolApprovalDecision>((resolve) => {
      approvalResolversRef.current.set(request.id, resolve);
    });
  }, []);

  const resolveToolApproval = useCallback((approvalId: string, decision: AiToolApprovalDecision) => {
    const resolver = approvalResolversRef.current.get(approvalId);
    if (!resolver) return;
    approvalResolversRef.current.delete(approvalId);
    resolver(decision);
  }, []);

  const resolveAllToolApprovals = (decision: AiToolApprovalDecision) => {
    const resolvers = [...approvalResolversRef.current.values()];
    approvalResolversRef.current.clear();
    for (const resolve of resolvers) resolve(decision);
  };

  const handleSend = useCallback(async () => {
    if (!selectedProvider || !selectedModel || !message.trim() || sending) return;
    const currentMessage = message.trim();
    const currentAttachments = attachments;
    const userMessage: AiChatMessage = {
      id: crypto.randomUUID(),
      role: "user",
      content: currentMessage,
      timestamp: Date.now(),
    };
    const history = messages;
    const abortController = new AbortController();
    abortControllerRef.current = abortController;
    setMessages((current) => [...current, userMessage]);
    setMessage("");
    setAttachments([]);
    setSendError(null);
    setSending(true);

    try {
      const runtimeAttachments: AiChatAttachmentInput[] = await Promise.all(currentAttachments.map((attachment) => readChatAttachment(attachment.file)));
      await sendAiChatMessage({
        abortSignal: abortController.signal,
        activeDocument,
        attachments: runtimeAttachments,
        history,
        message: currentMessage,
        openDocuments,
        preferences: aiPreferences,
        provider: selectedProvider,
        selectedAgentInstructions: selectedAgent?.instructions ?? "",
        selectedAgentName: selectedAgent?.name ?? "",
        selectedModel,
        terminal,
        workspace,
        onAssistantMessage: appendAssistantMessage,
        onAssistantMessageUpdate: updateAssistantMessage,
        onToolApproval: requestToolApproval,
      });
    } catch (error) {
      const errorMessage = isAbortError(error) ? "Request cancelled." : readErrorMessage(error);
      const assistantError: AiChatMessage = {
        id: crypto.randomUUID(),
        role: "assistant",
        content: errorMessage,
        timestamp: Date.now(),
      };
      setMessages((current) => replaceEmptyAssistantTail(current, assistantError));
      if (isAbortError(error)) {
        setSendError(errorMessage);
      } else {
        setSendError(errorMessage);
      }
    } finally {
      if (abortControllerRef.current === abortController) abortControllerRef.current = null;
      resolveAllToolApprovals("rejected");
      setSending(false);
      requestAnimationFrame(() => resizeComposerTextarea());
    }
  }, [activeDocument, aiPreferences, appendAssistantMessage, attachments, message, messages, openDocuments, requestToolApproval, resizeComposerTextarea, selectedAgent, selectedModel, selectedProvider, sending, terminal, updateAssistantMessage, workspace]);

  const handleComposerKeyDown = useCallback((event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key !== "Enter" || event.shiftKey) return;
    event.preventDefault();
    void handleSend();
  }, [handleSend]);

  return (
    <aside className="ai-chat-panel" aria-label={t("aiChat.panel.aria")}>
      <header className="ai-chat-header">
        <div className="ai-chat-title">
          <Sparkles size={15} />
          <span>{t("aiChat.title")}</span>
        </div>
        <div className="ai-chat-header-actions">
          <button
            className="icon-button compact"
            type="button"
            aria-label="Tools"
            title="Available Tools"
            data-active={toolsViewOpen}
            onClick={() => setToolsViewOpen(!toolsViewOpen)}
          >
            <Wrench size={15} />
          </button>
          <button className="icon-button compact" type="button" aria-label={t("aiChat.closeChat")} title={t("aiChat.closeChat")} onClick={() => setAiChatOpen(false)}>
            <PanelRightClose size={15} />
          </button>
        </div>
      </header>

      <div className="ai-chat-body">
        {toolsViewOpen ? (
          <AiToolsView />
        ) : messages.length > 0 ? (
          <section className="ai-chat-thread" aria-live="polite">
            {messages.map((chatMessage) => (
              <article className="ai-chat-message" data-role={chatMessage.role} key={chatMessage.id}>
                <div className="ai-chat-message-meta">
                  <span>{chatMessage.role === "user" ? "You" : "Lux AI"}</span>
                  <time>{formatMessageTime(chatMessage.timestamp)}</time>
                </div>
                {chatMessage.content && <div className="ai-chat-message-content">{renderChatContent(chatMessage.content)}</div>}
                {chatMessage.toolCalls && chatMessage.toolCalls.length > 0 && <AiToolCallsGroup onApprovalDecision={resolveToolApproval} toolCalls={chatMessage.toolCalls} />}
              </article>
            ))}
            {sendError && <div className="ai-chat-error" role="status">{sendError}</div>}
            <div ref={messagesEndRef} />
          </section>
        ) : (
          <section className="ai-chat-empty">
            <div className="ai-chat-mark"><Sparkles size={22} /></div>
            <h2>{t("aiChat.empty.title")}</h2>
            <div className="ai-chat-suggestions" aria-label={t("aiChat.suggestions.aria")}>
              <button type="button" onClick={() => updateMessage(t("aiChat.suggestion.explainSelectedCode.prompt"))}>{t("aiChat.suggestion.explainSelectedCode.button")}</button>
              <button type="button" onClick={() => updateMessage(t("aiChat.suggestion.fixCompileErrors.prompt"))}>{t("aiChat.suggestion.fixCompileErrors.button")}</button>
              <button type="button" onClick={() => updateMessage(t("aiChat.suggestion.generateTests.prompt"))}>{t("aiChat.suggestion.generateTests.button")}</button>
            </div>
          </section>
        )}
      </div>

      <footer className="ai-chat-composer-shell">
        <div
          className="ai-chat-composer"
          data-dragging-files={draggingFiles}
          onDragLeave={(event) => {
            if (event.currentTarget.contains(event.relatedTarget as Node | null)) return;
            setDraggingFiles(false);
          }}
          onDragOver={handleComposerDragOver}
          onDrop={handleComposerDrop}
        >
          {contextOpen && (
            <div className="ai-context-popover">
              <div className="ai-context-popover-head">
                <div>
                  <span>{t("aiChat.context.label")}</span>
                  <strong>{t("aiChat.context.full", { percent: contextUsage.percent })}</strong>
                </div>
                <button type="button" aria-label={t("aiChat.context.closeAria")} title={t("common.close")} onClick={() => setContextOpen(false)}>
                  <X size={13} />
                </button>
              </div>
              <div className="ai-context-token-row">
                <span>{t("aiChat.context.estimatedUsage")}</span>
                <strong>{t("aiChat.context.tokenUsage", { totalTokens: formatCompactTokens(contextUsage.totalTokens), tokenBudget: formatCompactTokens(contextUsage.tokenBudget) })}</strong>
              </div>
              <div className="ai-context-meter" aria-hidden="true">
                {contextUsage.rows.map((row) => (
                  <span key={row.id} style={{ width: `${row.percent}%`, background: row.color }} />
                ))}
              </div>
              <dl className="ai-context-breakdown">
                {contextUsage.rows.map((row) => (
                  <div key={row.id}>
                    <dt><span style={{ background: row.color }} />{row.label}</dt>
                    <dd>{formatContextValue(row)}</dd>
                  </div>
                ))}
              </dl>
            </div>
          )}
          <textarea
            ref={textareaRef}
            value={message}
            onChange={handleMessageChange}
            onKeyDown={handleComposerKeyDown}
            onPaste={handleComposerPaste}
            placeholder={t("aiChat.composer.placeholder")}
            rows={1}
          />
          {attachments.length > 0 && (
            <div className="ai-attachment-list" aria-label={t("aiChat.attachments.aria")}>
              {attachments.map((attachment) => (
                <span className="ai-attachment-chip" key={attachment.id} title={t("aiChat.attachment.tooltip", { name: attachment.name, size: formatBytes(attachment.size, t) })}>
                  <span>{attachment.name}</span>
                  <small>{formatBytes(attachment.size, t)}</small>
                  <button type="button" aria-label={t("aiChat.attachment.removeAria", { name: attachment.name })} title={t("common.remove")} onClick={() => removeAttachment(attachment.id)}>
                    <X size={12} />
                  </button>
                </span>
              ))}
            </div>
          )}
          <div className="ai-composer-actions">
            <div className="ai-composer-left-actions">
              <input
                ref={fileInputRef}
                className="sr-only"
                type="file"
                multiple
                onChange={(event) => {
                  attachFiles(event.currentTarget.files);
                  event.currentTarget.value = "";
                }}
              />
              <button className="icon-button compact" type="button" aria-label={t("aiChat.attachFiles")} title={t("aiChat.attachFiles")} onClick={() => fileInputRef.current?.click()}>
                <Paperclip size={15} />
              </button>
              <CompactDropdown
                className="ai-composer-select"
                icon={<Bot size={13} />}
                label={t("aiChat.mode.agent")}
                value={aiPreferences.selectedAgentId}
                options={agentOptions}
                onChange={(selectedAgentId) => updateAiPreference({ selectedAgentId })}
              />
              <CompactDropdown
                className="ai-composer-select"
                icon={<Sparkles size={13} />}
                label={t("aiChat.model.label")}
                value={selectedModel?.id ?? aiPreferences.selectedModelId}
                options={modelOptions}
                onChange={updateModel}
              />
              {modelSupportsEffort && (
                <CompactDropdown
                  className="ai-composer-select"
                  icon={<Brain size={13} />}
                  label={t("aiChat.reasoningEffort.label")}
                  value={aiPreferences.selectedEffortId}
                  options={effortOptions}
                  onChange={(selectedEffortId) => updateAiPreference({ selectedEffortId })}
                />
              )}
              <CompactDropdown<AiToolApprovalMode>
                className="ai-composer-select ai-composer-tool-approval"
                icon={<ShieldCheck size={13} />}
                label={t("aiChat.toolApproval.label")}
                value={aiPreferences.toolApprovalMode}
                options={toolApprovalOptions}
                onChange={(toolApprovalMode) => updateAiPreference({ toolApprovalMode })}
              />
              {attachments.length > 0 && <span className="ai-attachment-count">{attachments.length}</span>}
            </div>
            <div className="ai-composer-right-actions">
              <button className="ai-context-circle" type="button" aria-label={t("aiChat.context.label")} title={contextTitle} data-active={contextOpen} onClick={() => setContextOpen((open) => !open)}>
                {contextLabel}
              </button>
              <button
                className="ai-voice-button"
                type="button"
                aria-label={t("aiChat.voiceInput.aria")}
                title={voiceInput.voiceTitle}
                data-recording={voiceInput.voiceMode === "recording" || voiceInput.listening}
                data-transcribing={voiceInput.voiceMode === "transcribing"}
                disabled={!voiceInput.canUseVoice || voiceInput.voiceMode === "transcribing"}
                onClick={voiceInput.toggleVoiceInput}
              >
                <Mic size={14} />
              </button>
              <button
                className="ai-send-button"
                type="button"
                aria-label={t("aiChat.send.aria")}
                title={sending ? "Stop generation" : selectedProvider && selectedModel ? t("aiChat.send.aria") : t("aiChat.send.disabledTooltip")}
                disabled={!sending && !canSend}
                onClick={sending ? handleCancelSend : () => void handleSend()}
              >
                {sending ? <Square size={13} /> : message.trim() ? <SendHorizontal size={15} /> : <CornerDownLeft size={15} />}
              </button>
            </div>
          </div>
        </div>
      </footer>
    </aside>
  );
}

function replaceEmptyAssistantTail(messages: AiChatMessage[], assistantError: AiChatMessage) {
  const last = messages[messages.length - 1];
  if (last?.role === "assistant" && !last.content.trim() && (!last.toolCalls || last.toolCalls.length === 0)) {
    return [...messages.slice(0, -1), { ...last, content: assistantError.content, timestamp: assistantError.timestamp }];
  }
  return [...messages, assistantError];
}

function attachmentId(file: File) {
  return `${file.name}-${file.lastModified}-${file.size}`;
}

function buildContextUsageSummary({
  activeDocumentPath,
  aiIndexStatus,
  agentInstruction,
  agentName,
  attachments,
  message,
  preferences,
  selectedModelAlias,
  t,
}: {
  activeDocumentPath: string | null;
  aiIndexStatus: string;
  agentInstruction: string;
  agentName: string;
  attachments: ChatAttachment[];
  message: string;
  preferences: AiPreferences;
  selectedModelAlias: string;
  t: TranslateFn;
}): ContextUsageSummary {
  const agentTokens = estimateTokens([agentName, preferences.agentMode, agentInstruction].join(" "));
  const modelTokens = estimateTokens(selectedModelAlias);
  const indexTokens = preferences.projectIndexingEnabled && aiIndexStatus !== "disabled" ? estimateTokens(aiIndexStatus) : 0;
  const filesTokens = activeDocumentPath ? estimateTokens(activeDocumentPath) : 0;
  const conversationTokens = Math.max(estimateTokens(message), message.trim() ? 80 : 0)
    + attachments.reduce((sum, attachment) => sum + estimateAttachmentTokens(attachment), 0);
  const rawRows: Omit<ContextUsageRow, "percent">[] = [
    { color: contextRowColors.agent, detail: agentName || preferences.agentMode, id: "agent", label: t("aiChat.context.agent"), tokens: agentTokens },
    { color: contextRowColors.model, detail: selectedModelAlias, id: "model", label: t("aiChat.model.label"), tokens: modelTokens },
    { color: contextRowColors.index, detail: preferences.projectIndexingEnabled ? aiIndexStatus : t("common.off"), id: "index", label: t("aiChat.context.index"), tokens: indexTokens },
    { color: contextRowColors.files, detail: activeDocumentPath ?? "", id: "files", label: t("aiChat.context.file"), tokens: filesTokens },
    { color: contextRowColors.conversation, detail: attachments.length > 0 ? t("aiChat.attachment.count", { count: attachments.length }) : "", id: "conversation", label: t("aiChat.context.message"), tokens: conversationTokens },
  ].filter((row) => row.tokens > 0 || row.detail);
  const totalTokens = rawRows.reduce((sum, row) => sum + row.tokens, 0);
  const rows = rawRows.map((row) => ({
    ...row,
    percent: totalTokens > 0 ? Math.max(0.8, (row.tokens / totalTokens) * 100) : 0,
  }));

  return {
    percent: Math.min(100, Math.round((totalTokens / contextTokenBudget) * 100)),
    rows,
    tokenBudget: contextTokenBudget,
    totalTokens,
  };
}

function estimateAttachmentTokens(attachment: ChatAttachment) {
  return estimateTokens(attachment.name) + Math.max(1, Math.ceil(attachment.size / 1024));
}

function estimateTokens(value: string) {
  const trimmed = value.trim();
  if (!trimmed) return 0;
  return Math.ceil(trimmed.length / 4);
}

function formatCompactTokens(tokens: number) {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(tokens >= 10_000_000 ? 0 : 1)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(tokens >= 10_000 ? 0 : 1)}K`;
  return String(tokens);
}

function formatContextValue(row: ContextUsageRow) {
  const tokens = formatCompactTokens(row.tokens);
  return row.detail ? `${tokens} - ${row.detail}` : tokens;
}

function formatBytes(bytes: number, t: TranslateFn) {
  if (bytes < 1024) return t("common.fileSize.bytes", { bytes });
  const kilobytes = bytes / 1024;
  if (kilobytes < 1024) return t("common.fileSize.kilobytes", { kilobytes: kilobytes.toFixed(kilobytes >= 10 ? 0 : 1) });
  const megabytes = kilobytes / 1024;
  return t("common.fileSize.megabytes", { megabytes: megabytes.toFixed(megabytes >= 10 ? 0 : 1) });
}

function renderChatContent(content: string) {
  const parts = content.split(/```/g);
  if (parts.length === 1) return content;
  return parts.map((part, index) => {
    if (index % 2 === 0) return <span key={index}>{part}</span>;
    const firstLineBreak = part.indexOf("\n");
    const language = firstLineBreak > 0 ? part.slice(0, firstLineBreak).trim() : "";
    const code = firstLineBreak > 0 ? part.slice(firstLineBreak + 1) : part;
    return (
      <pre className="ai-chat-code-block" key={index} data-language={language || undefined}>
        <code>{code.trim()}</code>
      </pre>
    );
  });
}

function formatMessageTime(timestamp: number) {
  return new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" }).format(timestamp);
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function readErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
