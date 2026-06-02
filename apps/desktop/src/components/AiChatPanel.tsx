import { ArrowDown, Brain, MessageSquarePlus, PanelRightClose, Plus, RotateCcw, Sparkles, Wifi, X } from "lucide-react";
import type { ChangeEvent, ClipboardEvent, DragEvent, KeyboardEvent } from "react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { AiChatComposer } from "./ai-chat/AiChatComposer";
import { AiChatMessages } from "./ai-chat/AiChatMessages";
import { buildAiChatContextUsageSummary, formatCompactTokens } from "../lib/aiChatContextUsage";
import { loadAiChatHistory, saveAiChatHistory } from "../lib/aiChatHistory";
import { aiChatSessionTitle, aiChatStatusLabel } from "../lib/aiChatPresentation";
import { documentDisplayPath } from "../lib/documents";
import { useTranslation, type TranslateFn } from "../lib/i18n/useTranslation";
import { AI_PREFERENCES_KEY, getAiModel, getAiProvider, mergeAiPreferences, type AiPreferences } from "../lib/aiPreferences";
import { readChatAttachment, sendAiChatMessage } from "../lib/aiChatRuntime";
import type { AiChatAttachmentInput, AiChatMessage, AiToolApprovalDecision, AiToolApprovalRequest } from "../lib/aiChatTypes";
import { selectActiveAiChatSession, useLuxStore, type AiChatSessionStatus } from "../lib/store";
import { luxCommands, type AiProviderDiagnosticResponse } from "../lib/tauri";
import { useVoiceInput } from "../lib/useVoiceInput";

type ChatAttachment = {
  file: File;
  id: string;
  name: string;
  size: number;
};

type AiChatPanelProps = {
  embedded?: boolean;
  presentation?: "panel" | "agent";
  showCloseButton?: boolean;
};

export function AiChatPanel({ embedded = false, presentation = "panel", showCloseButton = true }: AiChatPanelProps) {
  const activeDocumentId = useLuxStore((state) => state.activeDocumentId);
  const aiIndex = useLuxStore((state) => state.aiIndex);
  const aiPreferences = useLuxStore((state) => state.aiPreferences);
  const aiChatSessions = useLuxStore((state) => state.aiChatSessions);
  const activeChatSession = useLuxStore(selectActiveAiChatSession);
  const activeAiChatSessionId = useLuxStore((state) => state.activeAiChatSessionId);
  const appendAiChatMessage = useLuxStore((state) => state.appendAiChatMessage);
  const createAiChatSession = useLuxStore((state) => state.createAiChatSession);
  const replaceAiChatMessages = useLuxStore((state) => state.replaceAiChatMessages);
  const restoreAiChatSession = useLuxStore((state) => state.restoreAiChatSession);
  const setAiChatSessions = useLuxStore((state) => state.setAiChatSessions);
  const setAiPreferences = useLuxStore((state) => state.setAiPreferences);
  const setAiChatSessionStatus = useLuxStore((state) => state.setAiChatSessionStatus);
  const updateAiChatMessage = useLuxStore((state) => state.updateAiChatMessage);
  const openDocuments = useLuxStore((state) => state.openDocuments);
  const setAiChatOpen = useLuxStore((state) => state.setAiChatOpen);
  const terminal = useLuxStore((state) => state.terminal);
  const terminalSessions = useLuxStore((state) => state.terminalSessions);
  const activeTerminalId = useLuxStore((state) => state.activeTerminalId);
  const terminalOutputBuffers = useLuxStore((state) => state.terminalOutputBuffers);
  const workspace = useLuxStore((state) => state.workspace);
  const { locale, t } = useTranslation();
  const [message, setMessage] = useState("");
  const [attachments, setAttachments] = useState<ChatAttachment[]>([]);
  const [contextOpen, setContextOpen] = useState(false);
  const [draggingFiles, setDraggingFiles] = useState(false);
  const [sendingSessionId, setSendingSessionId] = useState<string | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const [lastUserDraft, setLastUserDraft] = useState<string | null>(null);
  const [providerDiagnostic, setProviderDiagnostic] = useState<AiProviderDiagnosticResponse | null>(null);
  const [providerDiagnosticRunning, setProviderDiagnosticRunning] = useState(false);
  const [showScrollDown, setShowScrollDown] = useState(false);
  const abortControllerRef = useRef<AbortController | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const pinnedToBottomRef = useRef(true);
  const approvalResolversRef = useRef(new Map<string, (decision: AiToolApprovalDecision) => void>());
  const persistedSessionsLoadedRef = useRef(false);
  const skipNextSessionPersistRef = useRef(true);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const messages = activeChatSession?.messages ?? [];
  const activeStatus = activeChatSession?.status ?? "idle";
  const activeLastError = activeChatSession?.lastError ?? null;
  const activeSessionClosed = Boolean(activeChatSession?.closedAt);
  const sending = sendingSessionId !== null;
  const activeSessionSending = sendingSessionId === activeAiChatSessionId;
  // The last assistant message is "live" while this session is generating, so its
  // reasoning block auto-expands and collapses once the turn settles.
  const streamingMessageId = activeSessionSending
    ? [...messages].reverse().find((entry) => entry.role === "assistant")?.id ?? null
    : null;
  const isAgentHome = presentation === "agent" && messages.length === 0;

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
  const contextUsage = useMemo(() => buildAiChatContextUsageSummary({
    activeDocumentPath: activeDocument ? documentDisplayPath(activeDocument) : null,
    aiIndexStatus: aiIndex.status,
    agentInstruction: selectedAgent?.instructions ?? "",
    agentName: selectedAgent?.name ?? "",
    attachments,
    conversation: messages,
    message,
    preferences: aiPreferences,
    selectedModelAlias: selectedModel?.alias ?? selectedModel?.id ?? "",
    t,
  }), [activeDocument, aiIndex.status, aiPreferences, attachments, message, messages, selectedAgent, selectedModel, t]);
  const contextLabel = t("aiChat.context.percentBadge", { percent: contextUsage.percent });
  const contextTitle = t("aiChat.context.tooltip", {
    percent: contextUsage.percent,
    totalTokens: formatCompactTokens(contextUsage.totalTokens),
    tokenBudget: formatCompactTokens(contextUsage.tokenBudget),
  });
  const canSend = Boolean(selectedProvider && selectedModel && message.trim()) && !sending && !activeSessionClosed;
  const providerStatusLabel = providerDiagnosticRunning
    ? t("aiChat.provider.checking")
    : providerDiagnostic
      ? providerDiagnostic.ok
        ? t("aiChat.provider.ok", { latency: providerDiagnostic.latencyMs })
        : t("aiChat.provider.failed")
      : t("aiChat.provider.ready");
  const providerStatusTitle = providerDiagnostic
    ? providerDiagnostic.ok
      ? t("aiChat.provider.okDetail", { status: providerDiagnostic.status ?? "-", latency: providerDiagnostic.latencyMs })
      : providerDiagnostic.error ?? t("aiChat.provider.failed")
    : selectedProvider
      ? `${selectedProvider.name} / ${selectedModel?.name ?? aiPreferences.selectedModelId}`
      : t("aiChat.send.disabledTooltip");
  const renderComposerContent = () => (
    <AiChatComposer
      activeSessionSending={activeSessionSending}
      disabled={activeSessionClosed}
      agentOptions={agentOptions}
      attachments={attachments}
      attachFiles={attachFiles}
      canSend={canSend}
      contextLabel={contextLabel}
      contextOpen={contextOpen}
      contextTitle={contextTitle}
      contextUsage={contextUsage}
      draggingFiles={draggingFiles}
      effortOptions={effortOptions}
      fileInputRef={fileInputRef}
      handleCancelSend={handleCancelSend}
      handleComposerDragOver={handleComposerDragOver}
      handleComposerDrop={handleComposerDrop}
      handleComposerKeyDown={handleComposerKeyDown}
      handleComposerPaste={handleComposerPaste}
      handleMessageChange={handleMessageChange}
      handleSend={() => void handleSend()}
      isAgentHome={isAgentHome}
      message={message}
      modelOptions={modelOptions}
      modelSupportsEffort={modelSupportsEffort}
      preferences={aiPreferences}
      removeAttachment={removeAttachment}
      selectedModelId={selectedModel?.id ?? aiPreferences.selectedModelId}
      selectedProviderReady={Boolean(selectedProvider && selectedModel)}
      setContextOpen={setContextOpen}
      setDraggingFiles={setDraggingFiles}
      t={t}
      textareaRef={textareaRef}
      updateAiPreference={updateAiPreference}
      updateModel={updateModel}
      voiceInput={voiceInput}
    />
  );

  useEffect(() => {
    if (persistedSessionsLoadedRef.current) return;
    persistedSessionsLoadedRef.current = true;
    void loadAiChatHistory().then((history) => {
      if (history && history.sessions.length > 0) setAiChatSessions(history);
    }).finally(() => {
      skipNextSessionPersistRef.current = false;
    });
  }, [setAiChatSessions]);

  useEffect(() => {
    if (!persistedSessionsLoadedRef.current) return;
    if (skipNextSessionPersistRef.current) {
      return;
    }
    void saveAiChatHistory({
      activeSessionId: activeAiChatSessionId,
      sessions: aiChatSessions,
    }).catch(() => undefined);
  }, [activeAiChatSessionId, aiChatSessions]);

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

  const scrollToBottom = useCallback((behavior: ScrollBehavior = "auto") => {
    const element = scrollRef.current;
    if (!element) return;
    pinnedToBottomRef.current = true;
    setShowScrollDown(false);
    element.scrollTo({ top: element.scrollHeight, behavior });
  }, []);

  const handleBodyScroll = useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    const distanceFromBottom = element.scrollHeight - element.scrollTop - element.clientHeight;
    const pinned = distanceFromBottom <= 28;
    pinnedToBottomRef.current = pinned;
    setShowScrollDown(!pinned && element.scrollHeight - element.clientHeight > 48);
  }, []);

  // Keep the view pinned to the latest message while the user is already at the bottom.
  // When they have scrolled up to read, new content streams in without yanking the viewport.
  useLayoutEffect(() => {
    if (!pinnedToBottomRef.current) return;
    const element = scrollRef.current;
    if (!element) return;
    element.scrollTop = element.scrollHeight;
  }, [messages]);

  useEffect(() => {
    pinnedToBottomRef.current = true;
    setShowScrollDown(false);
    const element = scrollRef.current;
    if (element) element.scrollTop = element.scrollHeight;
  }, [activeAiChatSessionId]);

  useEffect(() => {
    setSendError(null);
  }, [activeAiChatSessionId]);

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

  const runProviderDiagnostic = useCallback(async () => {
    if (!selectedProvider || !selectedModel || providerDiagnosticRunning) return;
    setProviderDiagnosticRunning(true);
    setProviderDiagnostic(null);
    try {
      const result = await luxCommands.aiProviderDiagnostic({
        baseUrl: selectedProvider.baseUrl,
        apiKey: selectedProvider.apiKey || null,
        payload: {
          model: selectedModel.alias || selectedModel.id,
          messages: [{ role: "user", content: "Reply with OK." }],
          max_tokens: 8,
          stream: false,
          temperature: 0,
        },
      });
      setProviderDiagnostic(result);
    } catch (error) {
      setProviderDiagnostic({
        ok: false,
        status: null,
        latencyMs: 0,
        error: formatAiError(error, t),
        model: selectedModel.alias || selectedModel.id,
        baseUrl: selectedProvider.baseUrl,
      });
    } finally {
      setProviderDiagnosticRunning(false);
    }
  }, [providerDiagnosticRunning, selectedModel, selectedProvider, t]);

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

  const handleSend = useCallback(async (overrideMessage?: string, overrideHistory?: AiChatMessage[]) => {
    const nextMessage = (overrideMessage ?? message).trim();
    if (!selectedProvider || !selectedModel || !nextMessage || sending || activeSessionClosed) return;
    const sessionId = activeChatSession?.id ?? createAiChatSession(workspace?.root ?? null);
    const currentMessage = nextMessage;
    const currentAttachments = overrideMessage ? [] : attachments;
    const userMessage: AiChatMessage = {
      id: crypto.randomUUID(),
      role: "user",
      content: currentMessage,
      timestamp: Date.now(),
    };
    const history = overrideHistory ?? useLuxStore.getState().aiChatSessions.find((session) => session.id === sessionId)?.messages ?? [];
    const abortController = new AbortController();
    abortControllerRef.current = abortController;
    appendAiChatMessage(sessionId, userMessage);
    setLastUserDraft(currentMessage);
    pinnedToBottomRef.current = true;
    setShowScrollDown(false);
    setMessage("");
    setAttachments([]);
    setSendError(null);
    setSendingSessionId(sessionId);
    setAiChatSessionStatus(sessionId, "thinking");

    try {
      const runtimeAttachments: AiChatAttachmentInput[] = await Promise.all(currentAttachments.map((attachment) => readChatAttachment(attachment.file)));
      await sendAiChatMessage({
        abortSignal: abortController.signal,
        activeDocument,
        attachments: runtimeAttachments,
        history,
        locale,
        message: currentMessage,
        openDocuments,
        preferences: aiPreferences,
        provider: selectedProvider,
        selectedAgentInstructions: selectedAgent?.instructions ?? "",
        selectedAgentName: selectedAgent?.name ?? "",
        selectedModel,
        terminal,
        terminalContext: { activeTerminalId, outputBuffers: terminalOutputBuffers, sessions: terminalSessions },
        workspace,
        onAssistantMessage: (assistantMessage) => {
          const session = useLuxStore.getState().aiChatSessions.find((candidate) => candidate.id === sessionId);
          if (session?.messages.some((candidate) => candidate.id === assistantMessage.id)) return;
          appendAiChatMessage(sessionId, assistantMessage);
        },
        onAssistantMessageUpdate: (messageId, patch) => updateAiChatMessage(sessionId, messageId, patch),
        onStatusChange: (status) => setAiChatSessionStatus(sessionId, statusToSessionStatus(status)),
        onToolApproval: requestToolApproval,
      });
      setAiChatSessionStatus(sessionId, "idle");
    } catch (error) {
      const errorMessage = formatAiError(error, t);
      const assistantError: AiChatMessage = {
        id: crypto.randomUUID(),
        role: "assistant",
        content: errorMessage,
        timestamp: Date.now(),
      };
      replaceAiChatMessages(sessionId, replaceEmptyAssistantTail(useLuxStore.getState().aiChatSessions.find((session) => session.id === sessionId)?.messages ?? [], assistantError));
      setAiChatSessionStatus(sessionId, isAbortError(error) ? "idle" : "error", errorMessage);
      if (isAbortError(error)) {
        setSendError(errorMessage);
      } else {
        setSendError(errorMessage);
      }
    } finally {
      if (abortControllerRef.current === abortController) abortControllerRef.current = null;
      resolveAllToolApprovals("rejected");
      setSendingSessionId((currentSessionId) => currentSessionId === sessionId ? null : currentSessionId);
      requestAnimationFrame(() => resizeComposerTextarea());
    }
  }, [activeChatSession?.id, activeDocument, activeSessionClosed, activeTerminalId, aiPreferences, appendAiChatMessage, attachments, createAiChatSession, locale, message, openDocuments, replaceAiChatMessages, requestToolApproval, resizeComposerTextarea, selectedAgent, selectedModel, selectedProvider, sending, setAiChatSessionStatus, t, terminal, terminalOutputBuffers, terminalSessions, updateAiChatMessage, workspace]);

  const retryLastRequest = useCallback(() => {
    const draft = lastUserDraft ?? [...messages].reverse().find((entry) => entry.role === "user")?.content ?? "";
    if (!draft.trim() || sending || activeSessionClosed) return;
    if (activeChatSession) {
      const lastUserIndex = findLastUserMessageIndex(activeChatSession.messages);
      if (lastUserIndex >= 0) {
        const nextHistory = activeChatSession.messages.slice(0, lastUserIndex);
        replaceAiChatMessages(activeChatSession.id, nextHistory);
        void handleSend(draft, nextHistory);
        return;
      }
    }
    void handleSend(draft);
  }, [activeChatSession, activeSessionClosed, handleSend, lastUserDraft, messages, replaceAiChatMessages, sending]);

  const regenerateLastResponse = useCallback(() => {
    if (!activeChatSession || sending || activeSessionClosed) return;
    const lastUserIndex = findLastUserMessageIndex(activeChatSession.messages);
    if (lastUserIndex < 0) return;
    const draft = activeChatSession.messages[lastUserIndex].content;
    const nextHistory = activeChatSession.messages.slice(0, lastUserIndex);
    replaceAiChatMessages(activeChatSession.id, nextHistory);
    void handleSend(draft, nextHistory);
  }, [activeChatSession, activeSessionClosed, handleSend, replaceAiChatMessages, sending]);

  const handleComposerKeyDown = useCallback((event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key !== "Enter" || event.shiftKey) return;
    event.preventDefault();
    void handleSend();
  }, [handleSend]);

  return (
    <aside className="ai-chat-panel" aria-label={t("aiChat.panel.aria")} data-empty-home={isAgentHome} data-embedded={embedded} data-presentation={presentation} data-status={activeStatus}>
      {!isAgentHome && (
        <>
          <header className="ai-chat-header">
            <div className="ai-chat-title">
              <Sparkles size={15} />
              <span>{activeChatSession ? aiChatSessionTitle(activeChatSession.title, t) : t("aiChat.title")}</span>
              <span className="ai-chat-status-chip" data-status={activeStatus}>{aiChatStatusLabel(activeStatus, true, t)}</span>
            </div>
            <div className="ai-chat-header-actions">
              <button
                className="ai-provider-status"
                type="button"
                data-state={providerDiagnosticRunning ? "checking" : providerDiagnostic?.ok === false ? "error" : providerDiagnostic?.ok ? "ok" : "idle"}
                aria-label={t("aiChat.provider.check")}
                title={providerStatusTitle}
                disabled={!selectedProvider || !selectedModel || providerDiagnosticRunning}
                onClick={() => void runProviderDiagnostic()}
              >
                <Wifi size={13} />
                <span>{providerStatusLabel}</span>
              </button>
              {presentation !== "agent" && (
                <button className="icon-button compact" type="button" aria-label={t("agent.newChat")} title={t("agent.newChat")} onClick={() => createAiChatSession(workspace?.root ?? null)}>
                  <Plus size={15} />
                </button>
              )}
              {showCloseButton && <button className="icon-button compact" type="button" aria-label={t("aiChat.closeChat")} title={t("aiChat.closeChat")} onClick={() => setAiChatOpen(false)}>
                <PanelRightClose size={15} />
              </button>}
            </div>
          </header>
        </>
      )}

      <div className="ai-chat-body">
        <div className="ai-chat-scroll" ref={scrollRef} onScroll={handleBodyScroll}>
          {messages.length > 0 ? (
            <section className="ai-chat-thread" aria-live="polite">
              <AiChatMessages
                messages={messages}
                parentRef={scrollRef}
                streamingMessageId={streamingMessageId}
                showResponseDuration={aiPreferences.showResponseDuration}
                onApprovalDecision={resolveToolApproval}
                t={t}
              />
              {activeSessionSending && <AiThinkingIndicator status={activeStatus} t={t} />}
              {activeSessionClosed && <AiChatClosedNotice onRestore={() => restoreAiChatSession(activeAiChatSessionId)} t={t} />}
              {sendError && <AiChatError message={sendError} canRetry={Boolean(lastUserDraft)} onRetry={retryLastRequest} t={t} />}
              {activeLastError && !sendError && <AiChatError message={activeLastError} canRetry={Boolean(lastUserDraft)} onRetry={retryLastRequest} t={t} />}
              {!activeSessionSending && messages.some((entry) => entry.role === "assistant") && (
                <div className="ai-chat-thread-actions">
                  <button type="button" onClick={regenerateLastResponse} disabled={sending || activeSessionClosed}>
                    <RotateCcw size={13} />
                    <span>{t("aiChat.regenerate")}</span>
                  </button>
                </div>
              )}
            </section>
          ) : (
            <section className="ai-chat-empty">
              <div className="ai-chat-mark"><Sparkles size={22} /></div>
              <h2>{presentation === "agent" ? (workspace ? t("agent.welcome.titleWithWorkspace", { workspaceName: workspace.name }) : t("agent.welcome.title")) : t("aiChat.empty.title")}</h2>
              {presentation === "agent" && <div className="ai-chat-empty-composer">{renderComposerContent()}</div>}
              <div className="ai-chat-suggestions" aria-label={t("aiChat.suggestions.aria")}>
                <button type="button" onClick={() => updateMessage(t("aiChat.suggestion.explainSelectedCode.prompt"))}>{t("aiChat.suggestion.explainSelectedCode.button")}</button>
                <button type="button" onClick={() => updateMessage(t("aiChat.suggestion.fixCompileErrors.prompt"))}>{t("aiChat.suggestion.fixCompileErrors.button")}</button>
                <button type="button" onClick={() => updateMessage(t("aiChat.suggestion.generateTests.prompt"))}>{t("aiChat.suggestion.generateTests.button")}</button>
              </div>
            </section>
          )}
        </div>
        {messages.length > 0 && (
          <button
            type="button"
            className="ai-chat-scroll-down"
            data-visible={showScrollDown}
            aria-hidden={!showScrollDown}
            tabIndex={showScrollDown ? 0 : -1}
            aria-label={t("aiChat.scrollToLatest")}
            title={t("aiChat.scrollToLatest")}
            onClick={() => scrollToBottom("smooth")}
          >
            <ArrowDown size={15} />
          </button>
        )}
      </div>

      {!isAgentHome && <footer className="ai-chat-composer-shell">{renderComposerContent()}</footer>}
    </aside>
  );
}

function AiChatClosedNotice({ onRestore, t }: { onRestore: () => void; t: TranslateFn }) {
  return (
    <div className="ai-chat-closed-notice" role="status">
      <span>{t("aiChat.closedNotice")}</span>
      <button type="button" onClick={onRestore}>
        <MessageSquarePlus size={13} />
        <span>{t("aiChat.restoreChat")}</span>
      </button>
    </div>
  );
}

function AiChatError({ canRetry, message, onRetry, t }: { canRetry: boolean; message: string; onRetry: () => void; t: TranslateFn }) {
  return (
    <div className="ai-chat-error" role="status">
      <span>{message}</span>
      {canRetry && (
        <button type="button" onClick={onRetry}>
          <RotateCcw size={13} />
          <span>{t("aiChat.retry")}</span>
        </button>
      )}
    </div>
  );
}

function AiThinkingIndicator({ status, t }: { status: AiChatSessionStatus; t: TranslateFn }) {
  return (
    <div className="ai-thinking-indicator" data-status={status}>
      <span />
      <span />
      <span />
      <strong>{aiChatStatusLabel(status, true, t)}</strong>
    </div>
  );
}

function statusToSessionStatus(status: "thinking" | "streaming" | "running-tools" | "waiting-approval"): AiChatSessionStatus {
  return status;
}

function replaceEmptyAssistantTail(messages: AiChatMessage[], assistantError: AiChatMessage) {
  const last = messages[messages.length - 1];
  if (last?.role === "assistant" && !last.content.trim() && (!last.toolCalls || last.toolCalls.length === 0)) {
    return [...messages.slice(0, -1), { ...last, content: assistantError.content, timestamp: assistantError.timestamp }];
  }
  return [...messages, assistantError];
}

function findLastUserMessageIndex(messages: AiChatMessage[]) {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    if (messages[index].role === "user") return index;
  }
  return -1;
}

function attachmentId(file: File) {
  return `${file.name}-${file.lastModified}-${file.size}`;
}

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function readErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}

function formatAiError(error: unknown, t: TranslateFn) {
  if (isAbortError(error)) return t("aiChat.error.cancelled");
  const message = readErrorMessage(error);
  if (/timed out|timeout/i.test(message)) return t("aiChat.error.timeout", { detail: message });
  if (/failed to fetch|connection refused|connect|ECONNREFUSED|network/i.test(message)) return t("aiChat.error.providerUnavailable", { detail: message });
  if (/non-JSON|json|expected value/i.test(message)) return t("aiChat.error.invalidJson", { detail: message });
  if (/rejected by the user/i.test(message)) return t("aiChat.error.toolRejected", { detail: message });
  if (/workspace|no workspace is open/i.test(message)) return t("aiChat.error.workspace", { detail: message });
  if (/not found|file does not exist|cannot find/i.test(message)) return t("aiChat.error.fileNotFound", { detail: message });
  if (/stream/i.test(message)) return t("aiChat.error.stream", { detail: message });
  return t("aiChat.error.generic", { detail: message });
}
