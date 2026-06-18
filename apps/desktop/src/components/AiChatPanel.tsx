import { ArrowDown, ArrowUpRight, Brain, Bug, Code2, FlaskConical, Globe, MessageSquarePlus, PanelRightClose, Plus, RotateCcw, Sparkles, X } from "lucide-react";
import type { ChangeEvent, ClipboardEvent, DragEvent, KeyboardEvent } from "react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { mapComposerAttachments } from "./ai-chat/AiComposerAttachments";
import { AiChatComposer } from "./ai-chat/AiChatComposer";
import { AiChatHistoryPopover } from "./ai-chat/AiChatHistoryPopover";
import { AiChatMessages } from "./ai-chat/AiChatMessages";

import { AiAgentOrchestrationRail } from "./ai-chat/AiAgentOrchestrationRail";
import { AiQuestionCard } from "./ai-chat/AiQuestionCard";
import { AiPlanCard } from "./ai-chat/AiPlanCard";
import { AiSubagentPanel } from "./ai-chat/AiSubagentPanel";
import { AiAutomaticChecklist } from "./ai-chat/AiAutomaticChecklist";
import { buildContextDropSummary } from "../lib/aiChatContextReport";
import { aiChatErrorFromMessage, classifyAiChatError, type AiChatErrorPresentation } from "../lib/aiChatErrors";
import { shouldAutoRefreshIndexForAutomatic } from "../lib/aiProjectIndexPolicy";

import { AiChatSlashMenu } from "./ai-chat/AiChatSlashMenu";
import { AiChatMentionMenu } from "./ai-chat/AiChatMentionMenu";
import {
  compactChatHistory as runContextCompaction,
  pruneReducedTokenEstimate,
  pruneStaleToolOutputs,
  shouldAutoCompactContext,
} from "../lib/aiChatContextCompaction";
import { loadProjectSlashCommands, type ProjectSlashCommand } from "../lib/aiChatProjectCommands";
import { scheduleChatSessionTitleRefresh } from "../lib/aiChatSessionTitle";
import {
  composerTextAfterSlashPick,
  filterSlashCommands,
  type SlashCommandMatch,
  isExactSlashCommand,
  parseGoalSlashCommand,
  parseSlashQuery,
} from "../lib/aiChatSlashCommands";
import { openAgentBrowserPreviewTab } from "../lib/agentBrowserPreviewDocument";
import { buildAiChatContextUsageSummary, formatCompactTokens } from "../lib/aiChatContextUsage";
import { aiChatSessionTitle, aiChatStatusLabel } from "../lib/aiChatPresentation";
import { documentDisplayPath, documentTitle } from "../lib/documents";
import { useTranslation, type TranslateFn } from "../lib/i18n/useTranslation";
import { sanitizeSessionGoal } from "../lib/aiSessionOrchestrationSanitize";
import { getAiSessionGoal, setAiSessionGoal } from "../lib/aiSessionGoal";
import {
  createInternalGoalOrchestrationMessage,
  filterVisibleChatMessages,
} from "../lib/aiChatGoalOrchestration";
import { evaluateGoalRunContinuationAfterTurn } from "../lib/aiGoalEvaluator";
import {
  buildGoalContinuationDirective,
  buildGoalKickoffDirective,
  formatGoalRunDuration,
  formatGoalRunElapsedMs,
  formatGoalRunStatusText,
  formatGoalRunTokenTotal,
  getActiveGoalRun,
  getGoalRunSnapshot,
  lastAssistantMessage,
  pauseGoalRun,
  recordGoalRunTurnUsage,
  resolveGoalContinuationDelayMs,
  resumeGoalRun,
  startGoalRun,
  stopGoalRun,
  syncGoalRunFromAssistantMessage,
} from "../lib/aiSessionGoalRun";
import { replaceAiSessionTodos } from "../lib/aiSessionTodos";
import {
  AI_AGENT_MODE_ORDER,
  AI_PREFERENCES_KEY,
  getAiAgentProfile,
  getAiModel,
  getAiProjectInstructions,
  getAiProvider,
  isDefaultAiAgentProfile,
  isFullExecutionAgentMode,
  mergeAiPreferences,
  type AiPreferences,
} from "../lib/aiPreferences";
import { resolveVisionImageFormat } from "../lib/aiVisionFormat";
import { loadChatCheckpointStore, saveChatCheckpointStore } from "../lib/aiChatCheckpointStore";
import { buildCheckpointSendInput } from "../lib/aiChatCheckpointInput";
import {
  createTurnCheckpointBeforeSend,
  hasUserTurnCheckpoint,
  repairMessageTurnCheckpointIds,
} from "../lib/aiChatTurnCheckpoints";
import { restoreChatBeforeUserMessage, undoLastAgentTurn } from "../lib/aiChatTurnRestore";
import {
  buildMessageDisplayAttachments,
  collectClipboardFiles,
  createComposerFileAttachment,
  createComposerMentionAttachment,
  createComposerSelectionAttachment,
  revokeComposerAttachmentPreview,
  revokeComposerAttachmentPreviews,
  type ComposerAttachment,
} from "../lib/aiChatComposerAttachments";
import { buildMentionRuntimeAttachments, collectMentionHints } from "../lib/aiChatMentionAttachments";
import { applyMentionSelection, mentionMenuVisible, parseMentionQuery, searchMentionCandidates, type AiMentionCandidate } from "../lib/aiChatMentions";
import { buildPlanHandoffUserMessage, extractPlanHandoffPayload } from "../lib/aiChatPlanHandoff";
import { clearPendingPlan, getPendingPlanForSession, getPendingPlansSnapshot, subscribePendingPlans } from "../lib/aiPendingPlan";
import { getPendingQuestionForSession, getPendingQuestionsSnapshot, settlePendingQuestion, subscribePendingQuestions } from "../lib/aiPendingQuestion";
import { readEditorDocumentAttachment, readSelectionAttachment } from "../lib/aiChatDocumentAttachment";
import { formatSelectionLabel, getEditorSelectionSnapshot } from "../lib/editorSelectionBridge";
import { readChatAttachment, sendAiChatMessage } from "../lib/aiChatRuntime";
import { runNativeChatTurn } from "../lib/aiNativeTurn";
import { appendAiUsageLogEntry } from "../lib/aiUsageLog";
import { dragEventHasEditorTab, readEditorTabDrop } from "../lib/editorChatBridge";
import {
  abortAiChatTurn,
  finishAiChatTurn,
  getAiChatTurnRuntimeSnapshot,
  getTurnGeneration,
  isActiveChatTurn,
  requestAiToolApproval,
  requestStopAfterToolRound,
  resolveAiToolApproval,
  startAiChatTurn,
  subscribeAiChatTurnRuntime,
} from "../lib/aiChatTurnRuntime";
import type { AiChatAttachmentInput, AiChatMessage, AiToolApprovalDecision, AiToolApprovalRequest } from "../lib/aiChatTypes";
import { isAiChatSessionBusyStatus, selectActiveAiChatSession, useLuxStore, type AiChatSessionStatus } from "../lib/store";
import { isTauriRuntime, luxCommands } from "../lib/tauri";
import { useVoiceInput } from "../lib/useVoiceInput";
import {
  getComposerAttachments,
  getComposerDraft,
  setComposerAttachments,
  setComposerDraft,
} from "../lib/aiChatComposerSession";
import { findAnyPendingToolApproval } from "../lib/aiChatPendingApproval";
import { openWorkspaceEditorPath } from "../lib/openWorkspaceEditorPath";
import { AiChatGlobalApprovalBanner } from "./ai-chat/AiChatGlobalApprovalBanner";
import { AiThinkingIndicator, isPendingAssistantShell } from "./ai-chat/AiThinkingIndicator";

type AiChatPanelProps = {
  embedded?: boolean;
  presentation?: "panel" | "agent";
  showCloseButton?: boolean;
};

/**
 * Append a completed assistant turn to the persisted usage log (model, project,
 * speed, tokens, cost). Reads provider/model/workspace fresh from the store so it
 * is closure-safe inside the turn `finally` block. Best-effort: never throws into
 * the turn lifecycle.
 */
function recordAiUsageLogEntry(assistant: AiChatMessage | null | undefined) {
  const usage = assistant?.turnUsage;
  if (!usage) return;
  const state = useLuxStore.getState();
  const prefs = state.aiPreferences;
  const provider = getAiProvider(prefs.providers, prefs.selectedProviderId) ?? prefs.providers[0] ?? null;
  const model = getAiModel(provider, prefs.selectedModelId) ?? provider?.models[0] ?? null;
  void appendAiUsageLogEntry({
    workspaceRoot: state.workspace?.root,
    workspaceName: state.workspace?.name,
    model: model?.alias || model?.id || prefs.selectedModelId,
    provider: provider?.name ?? "",
    agentMode: prefs.agentMode,
    promptTokens: usage.promptTokens,
    completionTokens: usage.completionTokens,
    totalTokens: usage.totalTokens,
    cachedPromptTokens: usage.cachedPromptTokens,
    estimatedCostUsd: usage.estimatedCostUsd,
    durationMs: assistant?.responseTiming?.totalMs ?? assistant?.responseDurationMs ?? 0,
  });
}

export function AiChatPanel({ embedded = false, presentation = "panel", showCloseButton = true }: AiChatPanelProps) {
  const activeDocumentId = useLuxStore((state) => state.activeDocumentId);
  const aiIndex = useLuxStore((state) => state.aiIndex);
  const aiPreferences = useLuxStore((state) => state.aiPreferences);
  const aiChatSessions = useLuxStore((state) => state.aiChatSessions);
  const activeChatSession = useLuxStore(selectActiveAiChatSession);
  const activeAiChatSessionId = useLuxStore((state) => state.activeAiChatSessionId);
  const appendAiChatMessage = useLuxStore((state) => state.appendAiChatMessage);
  const renameAiChatSession = useLuxStore((state) => state.renameAiChatSession);
  const createAiChatSession = useLuxStore((state) => state.createAiChatSession);
  const ensureAiChatSession = useLuxStore((state) => state.ensureAiChatSession);
  const replaceAiChatMessages = useLuxStore((state) => state.replaceAiChatMessages);
  const restoreAiChatSession = useLuxStore((state) => state.restoreAiChatSession);
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
  const fileEntries = useLuxStore((state) => state.fileEntries);
  const openSettingsSection = useLuxStore((state) => state.openSettingsSection);
  const requestAiIndexRefresh = useLuxStore((state) => state.requestAiIndexRefresh);
  const setAiChatSessionContextBudgetReport = useLuxStore((state) => state.setAiChatSessionContextBudgetReport);
  const setActiveAiChatSession = useLuxStore((state) => state.setActiveAiChatSession);
  const { locale, t } = useTranslation();
  const [message, setMessage] = useState("");
  const [projectSlashCommands, setProjectSlashCommands] = useState<ProjectSlashCommand[]>([]);
  const [attachments, setAttachments] = useState<ComposerAttachment[]>([]);
  const [contextOpen, setContextOpen] = useState(false);
  const [draggingFiles, setDraggingFiles] = useState(false);
  const [sendError, setSendError] = useState<AiChatErrorPresentation | null>(null);
  const [lastUserDraft, setLastUserDraft] = useState<string | null>(null);
  const [showScrollDown, setShowScrollDown] = useState(false);
  const [slashMenuOpen, setSlashMenuOpen] = useState(false);
  const [slashActiveIndex, setSlashActiveIndex] = useState(0);
  const [mentionMenuOpen, setMentionMenuOpen] = useState(false);
  const [mentionActiveIndex, setMentionActiveIndex] = useState(0);
  // True once the user arrow-navigates the mention menu. Until then, Enter sends
  // the message instead of picking a candidate (so "@foo" + Enter isn't swallowed).
  const [mentionNavigated, setMentionNavigated] = useState(false);
  const [mentionCandidates, setMentionCandidates] = useState<AiMentionCandidate[]>([]);
  const [compacting, setCompacting] = useState(false);
  const [restoreNotice, setRestoreNotice] = useState<string | null>(null);
  const slashMenuRef = useRef<HTMLDivElement | null>(null);
  const mentionMenuRef = useRef<HTMLDivElement | null>(null);

  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const pinnedToBottomRef = useRef(true);
  const goalContinuationTimersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const runtimeSnapshot = useSyncExternalStore(subscribeAiChatTurnRuntime, getAiChatTurnRuntimeSnapshot, getAiChatTurnRuntimeSnapshot);
  const messages = activeChatSession?.messages ?? [];
  const visibleMessages = useMemo(() => filterVisibleChatMessages(messages), [messages]);
  const activeStatus = activeChatSession?.status ?? "idle";
  const activeLastError = activeChatSession?.lastError ?? null;
  const activeSessionClosed = Boolean(activeChatSession?.closedAt);
  const sendingSessionId = runtimeSnapshot.sendingSessionId;
  const activeSessionBusy = sendingSessionId === activeAiChatSessionId || isAiChatSessionBusyStatus(activeStatus);
  const showStopGeneration = Boolean(sendingSessionId) || isAiChatSessionBusyStatus(activeStatus);
  const pendingCrossSessionApproval = useMemo(() => {
    const pending = findAnyPendingToolApproval(aiChatSessions);
    if (!pending || pending.sessionId === activeAiChatSessionId) return null;
    return pending;
  }, [activeAiChatSessionId, aiChatSessions, messages]);
  // The last assistant message is "live" while this session is generating, so its
  // reasoning block auto-expands and collapses once the turn settles.
  const streamingMessageId = activeSessionBusy
    ? [...messages].reverse().find((entry) => entry.role === "assistant")?.id ?? null
    : null;
  const showStandaloneThinking = useMemo(() => {
    if (!activeSessionBusy) return false;
    const tailMessage = messages[messages.length - 1];
    if (!tailMessage) return true;
    return !isPendingAssistantShell(tailMessage, tailMessage.id === streamingMessageId);
  }, [activeSessionBusy, messages, streamingMessageId]);
  const isAgentHome = presentation === "agent" && visibleMessages.length === 0;
  const showOrchestrationRail = isFullExecutionAgentMode(aiPreferences.agentMode);
  const showSessionChrome = presentation === "panel" || presentation === "agent";

  // Collapsible floating agent island so it doesn't always eat chat width in workspace/editor layout.
  // User can collapse to a tiny 36px status strip when they want maximum chat space.
  // Default: expanded only in full dedicated Agent workspace (presentation="agent"), collapsed in side chat.
  const [isAgentIslandCollapsed, setIsAgentIslandCollapsed] = useState(presentation !== "agent");

  const startNewChat = useCallback(() => {
    const result = ensureAiChatSession(workspace?.root ?? null);
    if (result.reused) window.alert(t("agent.chat.emptyExists"));
  }, [ensureAiChatSession, t, workspace?.root]);

  const openBrowserPreviewTab = useCallback(() => {
    if (!activeAiChatSessionId || !aiPreferences.agentBrowserEnabled) return;
    const title = activeChatSession
      ? `${t("aiChat.browserPreview.title")} · ${aiChatSessionTitle(activeChatSession.title, t)}`
      : t("aiChat.browserPreview.title");
    // Only open the preview pane. It does NOT launch Chromium — the live stream
    // attaches automatically when the agent actually uses the browser (or when the
    // user starts a session from the dashboard). Opening a browser on click was wrong.
    openAgentBrowserPreviewTab(activeAiChatSessionId, title);
  }, [activeAiChatSessionId, activeChatSession, aiPreferences.agentBrowserEnabled, t]);

  const activeDocument = useMemo(
    () => openDocuments.find((document) => document.id === activeDocumentId) ?? null,
    [activeDocumentId, openDocuments],
  );

  const selectedProvider = getAiProvider(aiPreferences.providers, aiPreferences.selectedProviderId) ?? aiPreferences.providers[0] ?? null;
  const selectedModel = getAiModel(selectedProvider, aiPreferences.selectedModelId) ?? selectedProvider?.models[0] ?? null;
  const selectedAgent = aiPreferences.agentProfiles.find((profile) => profile.id === aiPreferences.selectedAgentId) ?? aiPreferences.agentProfiles[0] ?? null;
  const projectInstructions = getAiProjectInstructions(aiPreferences, workspace?.root);
  const runtimeInstructionText = [selectedAgent?.instructions ?? "", aiPreferences.globalInstructions, projectInstructions].filter((entry) => entry.trim()).join("\n");
  const modelSupportsEffort = Boolean(selectedModel?.effortLevels.length);
  const agentOptions = aiPreferences.agentProfiles.map((profile) => ({ label: profile.name, value: profile.id }));
  const providerOptions = aiPreferences.providers.map((provider) => ({ label: provider.name, value: provider.id }));
  const modelOptions = selectedProvider?.models.map((model) => ({ label: model.name, value: model.id })) ?? [];
  const effortOptions = selectedModel?.effortLevels.map((effort) => ({ label: effort.label, value: effort.id })) ?? [];
  const pinnedEditorPaths = useMemo(
    () => attachments.flatMap((attachment) => {
      if (attachment.kind === "editor") {
        const document = openDocuments.find((candidate) => candidate.id === attachment.documentId);
        return document ? [documentDisplayPath(document)] : [attachment.name];
      }
      if (attachment.kind === "mention" && attachment.path) return [attachment.path];
      if (attachment.kind === "selection") return [attachment.path];
      return [];
    }),
    [attachments, openDocuments],
  );

  const slashCommands = useMemo(
    () => filterSlashCommands(message, t, projectSlashCommands),
    [message, projectSlashCommands, t],
  );
  const contextUsage = useMemo(() => buildAiChatContextUsageSummary({
    pinnedEditorPaths,
    aiIndexStatus: aiIndex.status,
    agentInstruction: runtimeInstructionText,
    agentName: selectedAgent?.name ?? "",
    attachments,
    conversation: messages,
    message,
    preferences: aiPreferences,
    selectedModel: selectedModel ?? null,
    selectedModelAlias: selectedModel?.alias ?? selectedModel?.id ?? "",
    t,
    hasGlobalInstructions: aiPreferences.globalInstructions.trim().length > 0,
    hasProjectInstructions: projectInstructions.trim().length > 0,
  }), [aiIndex.status, aiPreferences, attachments, message, messages, pinnedEditorPaths, runtimeInstructionText, selectedAgent, selectedModel, t, projectInstructions]);

  useEffect(() => {
    loadChatCheckpointStore();
  }, []);

  useEffect(() => {
    let cancelled = false;
    void loadProjectSlashCommands(workspace?.root).then((commands) => {
      if (!cancelled) setProjectSlashCommands(commands);
    });
    return () => {
      cancelled = true;
    };
  }, [workspace?.root]);

  useEffect(() => {
    if (!activeChatSession) return;
    const repaired = repairMessageTurnCheckpointIds(activeChatSession.id, activeChatSession.messages);
    if (repaired !== activeChatSession.messages) {
      replaceAiChatMessages(activeChatSession.id, repaired);
    }
  }, [activeChatSession, replaceAiChatMessages]);

  useEffect(() => {
    const parsed = parseSlashQuery(message);
    setSlashMenuOpen(Boolean(parsed && !message.includes("\n") && !mentionMenuVisible(message)));
    setSlashActiveIndex(0);
    setMentionMenuOpen(mentionMenuVisible(message));
    setMentionActiveIndex(0);
    setMentionNavigated(false);
  }, [message]);

  useEffect(() => {
    if (!mentionMenuOpen) {
      setMentionCandidates([]);
      return;
    }
    const parsed = parseMentionQuery(message);
    if (!parsed) return;
    let cancelled = false;
    const handle = window.setTimeout(() => {
      void searchMentionCandidates({
        query: parsed.query,
        kindFilter: parsed.kindFilter,
        workspaceRoot: workspace?.root ?? null,
        openDocuments,
        fileEntries,
      }).then((candidates) => {
        if (!cancelled) setMentionCandidates(candidates);
      });
    }, 120);
    return () => {
      cancelled = true;
      window.clearTimeout(handle);
    };
  }, [fileEntries, mentionMenuOpen, message, openDocuments, workspace?.root]);

  const planHandoff = useMemo(() => {
    if (selectedAgent?.mode !== "plan") return null;
    return extractPlanHandoffPayload(messages);
  }, [messages, selectedAgent?.mode]);

  // Interactive AskUser / PresentPlan prompts, driven by the native turn loop.
  useSyncExternalStore(subscribePendingQuestions, getPendingQuestionsSnapshot, getPendingQuestionsSnapshot);
  useSyncExternalStore(subscribePendingPlans, getPendingPlansSnapshot, getPendingPlansSnapshot);
  const pendingQuestion = activeAiChatSessionId ? getPendingQuestionForSession(activeAiChatSessionId) : null;
  const pendingPlan = activeAiChatSessionId ? getPendingPlanForSession(activeAiChatSessionId) : null;
  // A structured PresentPlan card supersedes the heuristic prose-checklist handoff.
  const showLegacyPlanHandoff = planHandoff && !pendingPlan;
  const contextTitle = t("aiChat.context.tooltip", {
    percent: contextUsage.percent,
    totalTokens: formatCompactTokens(contextUsage.totalTokens),
    tokenBudget: formatCompactTokens(contextUsage.tokenBudget),
  });
  const contextDrops = useMemo(
    () => buildContextDropSummary(
      activeChatSession?.contextBudgetReport,
      activeChatSession?.contextCompaction ?? null,
    ),
    [activeChatSession?.contextBudgetReport, activeChatSession?.contextCompaction],
  );
  const activeLastErrorPresentation = useMemo(
    () => (activeChatSession?.lastError ? classifyAiChatError(new Error(activeChatSession.lastError), t) : null),
    [activeChatSession?.lastError, t],
  );

  useEffect(() => {
    if (!workspace) return;
    if (!shouldAutoRefreshIndexForAutomatic(aiPreferences.projectIndexingEnabled, aiIndex)) return;
    if (aiIndex.status === "indexing") return;
    requestAiIndexRefresh();
  }, [
    aiIndex.indexedFiles,
    aiIndex.quality,
    aiIndex.status,
    aiPreferences.projectIndexingEnabled,
    requestAiIndexRefresh,
    workspace,
  ]);
  const composerAttachments = useMemo(() => mapComposerAttachments(attachments), [attachments]);
  const canSend =
    Boolean(selectedProvider && selectedModel && (message.trim() || attachments.length > 0))
    && !activeSessionBusy
    && !activeSessionClosed;

  const updateAiPreference = useCallback((patch: Partial<AiPreferences>) => {
    const nextPreferences = mergeAiPreferences(aiPreferences, patch);
    setAiPreferences(nextPreferences);
    void luxCommands.settingsSet("user", AI_PREFERENCES_KEY, nextPreferences).catch(() => undefined);
  }, [aiPreferences, setAiPreferences]);

  const updateModel = useCallback((selectedModelId: string) => {
    updateAiPreference({ selectedModelId });
  }, [updateAiPreference]);

  const updateProvider = useCallback((selectedProviderId: string) => {
    const provider = getAiProvider(aiPreferences.providers, selectedProviderId) ?? null;
    const nextModelId = provider?.models.some((model) => model.id === aiPreferences.selectedModelId)
      ? aiPreferences.selectedModelId
      : provider?.models[0]?.id;
    updateAiPreference(nextModelId ? { selectedProviderId, selectedModelId: nextModelId } : { selectedProviderId });
  }, [aiPreferences.providers, aiPreferences.selectedModelId, updateAiPreference]);

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

  const hydratedComposerSessionRef = useRef<string | null>(null);
  useEffect(() => {
    if (hydratedComposerSessionRef.current === activeAiChatSessionId) return;
    hydratedComposerSessionRef.current = activeAiChatSessionId;
    const nextMessage = getComposerDraft(activeAiChatSessionId);
    const nextAttachments = getComposerAttachments(activeAiChatSessionId);
    setMessage((current) => (current.trim() && !nextMessage.trim() ? current : nextMessage));
    setAttachments(nextAttachments);
    setContextOpen(false);
    setDraggingFiles(false);
    requestAnimationFrame(() => resizeComposerTextarea());
  }, [activeAiChatSessionId, resizeComposerTextarea]);

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
    setRestoreNotice(null);
  }, [activeAiChatSessionId]);

  useEffect(() => () => {
    const timers = goalContinuationTimersRef.current;
    for (const timer of timers.values()) clearTimeout(timer);
    timers.clear();
  }, []);

  const resolveComposerSessionId = useCallback(() => {
    return useLuxStore.getState().activeAiChatSessionId ?? activeChatSession?.id ?? null;
  }, [activeChatSession?.id]);

  const updateMessage = useCallback((nextMessage: string) => {
    const sessionId = resolveComposerSessionId();
    setComposerDraft(sessionId, nextMessage);
    setMessage(nextMessage);
    requestAnimationFrame(() => resizeComposerTextarea());
  }, [resolveComposerSessionId, resizeComposerTextarea]);

  const handleMessageChange = useCallback((event: ChangeEvent<HTMLTextAreaElement>) => {
    resizeComposerTextarea(event.currentTarget);
    updateMessage(event.currentTarget.value);
  }, [resizeComposerTextarea, updateMessage]);

  const runCompaction = useCallback(async (force = false) => {
    if (!activeChatSession || !selectedProvider || !selectedModel || compacting || activeSessionBusy) return false;
    setCompacting(true);
    setSendError(null);
    try {
      const result = await runContextCompaction({
        chatSessionId: activeChatSession.id,
        messages: activeChatSession.messages,
        compactionState: activeChatSession.contextCompaction ?? null,
        model: selectedModel,
        provider: selectedProvider,
        selectedEffortId: aiPreferences.selectedEffortId,
        threshold: aiPreferences.contextAutoCompactThreshold,
        autoCompactEnabled: aiPreferences.contextAutoCompactEnabled,
        force,
      });
      if (result.compacted) {
        replaceAiChatMessages(activeChatSession.id, result.messages, { contextCompaction: result.compactionState });
      } else {
        // Persist the cooldown/throttle state even when nothing was compacted: the
        // "no-reduction" path returns a fresh state carrying lastCompactedAt so the
        // expensive summarization isn't re-run on every subsequent over-threshold send.
        // Skip the write when the state is unchanged (other skip reasons return it as-is).
        if (result.compactionState && result.compactionState !== (activeChatSession.contextCompaction ?? null)) {
          replaceAiChatMessages(activeChatSession.id, result.messages, { contextCompaction: result.compactionState });
        }
        if (force) {
          const reasonKey = result.reason === "too-few-messages"
            ? "aiChat.compact.skippedFewMessages"
            : result.reason === "no-reduction" || result.reason === "same-fingerprint"
              ? "aiChat.compact.skippedNoGain"
              : "aiChat.compact.skippedBelowThreshold";
          setSendError(aiChatErrorFromMessage(t(reasonKey as Parameters<typeof t>[0]), t));
        }
      }
      return result.compacted;
    } catch (error) {
      setSendError(aiChatErrorFromMessage(t("aiChat.compact.failed", { detail: readErrorMessage(error) }), t));
      return false;
    } finally {
      setCompacting(false);
    }
  }, [activeChatSession, activeSessionBusy, aiPreferences.contextAutoCompactEnabled, aiPreferences.contextAutoCompactThreshold, aiPreferences.selectedEffortId, compacting, replaceAiChatMessages, selectedModel, selectedProvider, t]);

  const handleSlashSelect = useCallback((command: SlashCommandMatch) => {
    setSlashMenuOpen(false);
    if (command.kind === "project") {
      updateMessage(command.template);
      requestAnimationFrame(() => {
        textareaRef.current?.focus();
        const end = command.template.length;
        textareaRef.current?.setSelectionRange(end, end);
      });
      return;
    }
    if (command.id === "compact") {
      updateMessage("");
      void runCompaction(true);
      return;
    }
    if (command.id === "clear") {
      if (!activeChatSession || activeSessionBusy) return;
      replaceAiChatMessages(activeChatSession.id, [], { contextCompaction: null });
      void luxCommands.aiBlackboardClear(activeChatSession.id).catch(() => undefined);
      updateMessage("");
      setSendError(null);
      return;
    }
    if (command.id === "help") {
      const picked = composerTextAfterSlashPick("help", message);
      if (picked) updateMessage(picked);
      return;
    }
    if (command.id === "goal") {
      const picked = composerTextAfterSlashPick("goal", message);
      if (picked) updateMessage(picked);
      requestAnimationFrame(() => {
        textareaRef.current?.focus();
        const end = (picked ?? "/goal ").length;
        textareaRef.current?.setSelectionRange(end, end);
      });
      return;
    }
    if (command.id === "model" && selectedProvider) {
      const models = selectedProvider.models;
      const currentIndex = models.findIndex((model) => model.id === aiPreferences.selectedModelId);
      const nextModel = models[(currentIndex + 1) % models.length];
      if (nextModel) updateAiPreference({ selectedModelId: nextModel.id });
      updateMessage("");
      return;
    }
    if (command.id === "agent") {
      const current = getAiAgentProfile(aiPreferences.agentProfiles, aiPreferences.selectedAgentId)
        ?? aiPreferences.agentProfiles[0];
      if (current) {
        const modeIndex = AI_AGENT_MODE_ORDER.indexOf(current.mode);
        const nextMode = AI_AGENT_MODE_ORDER[(modeIndex + 1) % AI_AGENT_MODE_ORDER.length];
        const nextProfile = aiPreferences.agentProfiles.find((profile) => profile.mode === nextMode && isDefaultAiAgentProfile(profile.id))
          ?? aiPreferences.agentProfiles.find((profile) => profile.mode === nextMode);
        if (nextProfile) updateAiPreference({ selectedAgentId: nextProfile.id });
      }
      updateMessage("");
      return;
    }
    if (command.id === "settings") {
      openSettingsSection("ai-runtime");
      updateMessage("");
      return;
    }
    if (command.id === "index") {
      setRestoreNotice(t("aiChat.slash.indexStatus", {
        status: aiIndex.status,
        files: aiIndex.indexedFiles,
        quality: aiIndex.quality,
      }));
      updateMessage("");
      return;
    }
  }, [activeChatSession, activeSessionBusy, aiIndex.indexedFiles, aiIndex.quality, aiIndex.status, aiPreferences, message, openSettingsSection, replaceAiChatMessages, runCompaction, selectedProvider, t, updateAiPreference, updateMessage]);

  const voiceInput = useVoiceInput({ message, preferences: aiPreferences, updateMessage });

  const attachFiles = (files: FileList | File[] | null) => {
    if (!files || files.length === 0) return;
    // Mint each blob preview URL exactly once in the event handler, never inside
    // the updater — React may invoke an updater more than once (StrictMode/dev or
    // a discarded concurrent render), which would leak the discarded URL.
    const incoming = Array.from(files).map((file) => createComposerFileAttachment(file));
    setAttachments((current) => {
      const byId = new Map(current.map((attachment) => [attachment.id, attachment]));
      for (const next of incoming) {
        const existing = byId.get(next.id);
        if (existing?.kind === "file") revokeComposerAttachmentPreview(existing);
        byId.set(next.id, next);
      }
      const nextAttachments = [...byId.values()];
      setComposerAttachments(activeAiChatSessionId, nextAttachments);
      return nextAttachments;
    });
  };

  const attachMention = useCallback((candidate: AiMentionCandidate) => {
    const next = createComposerMentionAttachment({
      mentionType: candidate.kind,
      name: candidate.label,
      path: candidate.path,
      symbolName: candidate.symbolName,
      line: candidate.line,
      column: candidate.column,
    });
    setAttachments((current) => {
      const byId = new Map(current.map((attachment) => [attachment.id, attachment]));
      byId.set(next.id, next);
      const nextAttachments = [...byId.values()];
      setComposerAttachments(activeAiChatSessionId, nextAttachments);
      return nextAttachments;
    });
  }, [activeAiChatSessionId]);

  const attachSelection = useCallback((selection = getEditorSelectionSnapshot()) => {
    if (!selection) return false;
    const next = createComposerSelectionAttachment({
      documentId: selection.documentId,
      name: formatSelectionLabel(selection),
      path: selection.path,
      text: selection.text,
      startLine: selection.startLine,
      endLine: selection.endLine,
      startColumn: selection.startColumn,
      endColumn: selection.endColumn,
      languageId: selection.languageId,
    });
    setAttachments((current) => {
      const byId = new Map(current.map((attachment) => [attachment.id, attachment]));
      byId.set(next.id, next);
      const nextAttachments = [...byId.values()];
      setComposerAttachments(activeAiChatSessionId, nextAttachments);
      return nextAttachments;
    });
    return true;
  }, [activeAiChatSessionId]);

  const attachEditorDocument = useCallback((documentId: string) => {
    const document = openDocuments.find((candidate) => candidate.id === documentId);
    if (!document) return;
    const id = `editor:${documentId}`;
    const name = documentTitle(document);
    setAttachments((current) => {
      const byId = new Map(current.map((attachment) => [attachment.id, attachment]));
      byId.set(id, { kind: "editor", documentId, id, name, size: document.text.length });
      const nextAttachments = [...byId.values()];
      setComposerAttachments(activeAiChatSessionId, nextAttachments);
      return nextAttachments;
    });
  }, [activeAiChatSessionId, openDocuments]);

  const removeAttachment = (id: string) => {
    setAttachments((current) => {
      const removed = current.find((attachment) => attachment.id === id);
      if (removed) revokeComposerAttachmentPreview(removed);
      const nextAttachments = current.filter((attachment) => attachment.id !== id);
      setComposerAttachments(activeAiChatSessionId, nextAttachments);
      return nextAttachments;
    });
  };

  const handleComposerDragOver = (event: DragEvent<HTMLDivElement>) => {
    const hasFiles = event.dataTransfer.types.includes("Files");
    const hasEditorTab = dragEventHasEditorTab(event.dataTransfer);
    if (!hasFiles && !hasEditorTab) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setDraggingFiles(true);
  };

  const handleComposerDrop = (event: DragEvent<HTMLDivElement>) => {
    const editorTabId = readEditorTabDrop(event.dataTransfer);
    const hasFiles = event.dataTransfer.types.includes("Files");
    if (!editorTabId && !hasFiles) return;
    event.preventDefault();
    setDraggingFiles(false);
    if (editorTabId) attachEditorDocument(editorTabId);
    if (hasFiles) attachFiles(event.dataTransfer.files);
  };

  const handleComposerPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = collectClipboardFiles(event.clipboardData);
    if (files.length === 0) return;
    event.preventDefault();
    attachFiles(files);
  };

  const handleCancelSend = useCallback(() => {
    const sessionId = sendingSessionId
      ?? (isAiChatSessionBusyStatus(activeStatus) ? activeAiChatSessionId : null);
    if (!sessionId) return;
    const pendingContinuation = goalContinuationTimersRef.current.get(sessionId);
    if (pendingContinuation !== undefined) {
      clearTimeout(pendingContinuation);
      goalContinuationTimersRef.current.delete(sessionId);
    }
    abortAiChatTurn(sessionId);
    stopGoalRun(sessionId);
    setAiChatSessionStatus(sessionId, "idle");
    setSendError(null);
    trimCancelledAssistantShell(sessionId, replaceAiChatMessages);
  }, [activeAiChatSessionId, activeStatus, replaceAiChatMessages, sendingSessionId, setAiChatSessionStatus]);

  const requestToolApproval = useCallback((request: AiToolApprovalRequest) => {
    return requestAiToolApproval(request.id);
  }, []);

  const resolveToolApproval = useCallback((approvalId: string, decision: AiToolApprovalDecision) => {
    resolveAiToolApproval(approvalId, decision);
  }, []);

  const handleSend = useCallback(async (
    overrideMessage?: string,
    overrideHistory?: AiChatMessage[],
    options?: {
      force?: boolean;
      skipGoalSlash?: boolean;
      goalContinuation?: boolean;
      /** Silent /goal orchestration — internal history only, never rendered in chat. */
      goalOrchestration?: "kickoff" | "continuation";
      useComposerAttachments?: boolean;
      sessionId?: string;
      /** Goal kickoff / continuation — skip slash re-parse and busy gate on active tab. */
      internalSend?: boolean;
      /** Review-request turn: render a badge instead of the raw prompt, and leave the
       *  composer draft/attachments untouched (the prompt is system-authored, not typed). */
      reviewRequest?: boolean;
    },
  ) => {
    const isInternalSend = options?.internalSend === true;
    const nextMessage = (overrideMessage ?? message).trim();
    let sessionId = options?.sessionId ?? activeChatSession?.id ?? useLuxStore.getState().activeAiChatSessionId;
    if (!sessionId) {
      sessionId = createAiChatSession(workspace?.root ?? null);
    }
    const targetSession = useLuxStore.getState().aiChatSessions.find((entry) => entry.id === sessionId);
    const targetSessionClosed = Boolean(targetSession?.closedAt);
    const targetSessionBusy = getAiChatTurnRuntimeSnapshot().sendingSessionId === sessionId
      || isAiChatSessionBusyStatus(targetSession?.status ?? "idle");
    const sendBlocked = options?.sessionId
      ? targetSessionClosed || targetSessionBusy
      : activeSessionClosed || activeSessionBusy;

    if (!isInternalSend) {
      if (!nextMessage && attachments.length === 0) return;
      if (sendBlocked && !options?.force) return;

      if (!overrideMessage && isExactSlashCommand(nextMessage, "compact")) {
        updateMessage("");
        await runCompaction(true);
        return;
      }
      if (!overrideMessage && isExactSlashCommand(nextMessage, "clear")) {
        if (activeChatSession) {
          replaceAiChatMessages(activeChatSession.id, [], { contextCompaction: null });
          void luxCommands.aiBlackboardClear(activeChatSession.id).catch(() => undefined);
        }
        updateMessage("");
        return;
      }
      if (!overrideMessage && isExactSlashCommand(nextMessage, "undo")) {
        updateMessage("");
        if (!workspace) {
          setRestoreNotice(t("aiChat.slash.undoNeedProject"));
          return;
        }
        if (!activeChatSession || activeSessionClosed) return;
        const input = buildRestoreInput();
        if (!input) return;
        abortAiChatTurn(activeChatSession.id);
        try {
          const result = await undoLastAgentTurn({
            currentMessages: activeChatSession.messages,
            input,
            sessionId: activeChatSession.id,
          });
          replaceAiChatMessages(activeChatSession.id, result.messages, { contextCompaction: null });
          showRestoreSuccess(result);
          saveChatCheckpointStore();
        } catch (error) {
          const detail = error instanceof Error ? error.message : classifyAiChatError(error, t).message;
          setRestoreNotice(/nothing to undo/i.test(detail) ? t("aiChat.slash.undoEmpty") : detail);
        }
        return;
      }
      if (!overrideMessage && isExactSlashCommand(nextMessage, "help")) {
        updateMessage("/");
        return;
      }
      const goalSlash = !options?.skipGoalSlash ? parseGoalSlashCommand(nextMessage) : null;
      if (goalSlash) {
        if (goalSlash.kind === "incomplete") {
          setRestoreNotice(t("aiChat.slash.goalNeedText"));
          updateMessage("/goal ");
          return;
        }
        if (goalSlash.kind === "flagError") {
          setRestoreNotice(t("aiChat.slash.goalFlagError", { detail: goalSlash.errors.join("; ") }));
          return;
        }
        if (goalSlash.kind === "status") {
          const run = getGoalRunSnapshot(sessionId);
          const pinned = getAiSessionGoal(sessionId);
          setRestoreNotice(
            run
              ? formatGoalRunStatusText(run)
              : pinned
                ? `${t("aiChat.slash.goalStatusPinned")}\n${pinned}`
                : t("aiChat.slash.goalStatusEmpty"),
          );
          updateMessage("");
          return;
        }
        if (goalSlash.kind === "history") {
          const run = getGoalRunSnapshot(sessionId);
          if (!run?.history.length) {
            setRestoreNotice(t("aiChat.slash.goalHistoryEmpty"));
          } else {
            setRestoreNotice(
              run.history.map((entry) => `- ${entry.type}: ${entry.detail}`).join("\n"),
            );
          }
          updateMessage("");
          return;
        }
        if (goalSlash.kind === "pause") {
          const paused = pauseGoalRun(sessionId);
          setRestoreNotice(paused ? t("aiChat.slash.goalPaused") : t("aiChat.slash.goalPauseEmpty"));
          updateMessage("");
          return;
        }
        if (goalSlash.kind === "resume") {
          const resumed = resumeGoalRun(sessionId);
          if (!resumed) {
            setRestoreNotice(t("aiChat.slash.goalResumeEmpty"));
            updateMessage("");
            return;
          }
          setRestoreNotice(t("aiChat.slash.goalResumed"));
          updateMessage("");
          if (sendBlocked) return;
          try {
            await handleSend(buildGoalContinuationDirective(sessionId), undefined, {
              sessionId,
              skipGoalSlash: true,
              goalOrchestration: "continuation",
              internalSend: true,
              force: true,
            });
          } catch (error) {
            setSendError(classifyAiChatError(error, t));
          }
          return;
        }
        if (goalSlash.kind === "clear") {
          stopGoalRun(sessionId);
          setAiSessionGoal(sessionId, "");
          setRestoreNotice(t("aiChat.slash.goalCleared"));
          updateMessage("");
          return;
        }
        const sanitized = sanitizeSessionGoal(goalSlash.goal);
        if (!sanitized.ok) {
          setRestoreNotice(t("aiChat.slash.goalRejected", { reason: sanitized.reason }));
          return;
        }
        setAiSessionGoal(sessionId, sanitized.value);
        let runPreferences = useLuxStore.getState().aiPreferences;
        if (!isFullExecutionAgentMode(runPreferences.agentMode)) {
          const agentProfile = runPreferences.agentProfiles.find((profile) => profile.mode === "agent")
            ?? runPreferences.agentProfiles.find((profile) => profile.mode === "automatic");
          if (agentProfile) {
            updateAiPreference({ selectedAgentId: agentProfile.id, agentMode: agentProfile.mode });
            runPreferences = useLuxStore.getState().aiPreferences;
          }
        }
        if (!isFullExecutionAgentMode(runPreferences.agentMode)) {
          setRestoreNotice(t("aiChat.slash.goalSet"));
          return;
        }
        if (sendBlocked) return;
        startGoalRun(sessionId, sanitized.value, {
          agentMode: runPreferences.agentMode,
          toolRoundLimit: runPreferences.toolRoundLimit,
          limits: goalSlash.limits,
        });
        updateMessage("");
        setRestoreNotice(t("aiChat.slash.goalRunStarted"));
        try {
          await handleSend(buildGoalKickoffDirective(sanitized.value, goalSlash.extraMessage), undefined, {
            sessionId,
            skipGoalSlash: true,
            goalOrchestration: "kickoff",
            internalSend: true,
            force: true,
          });
        } catch (error) {
          stopGoalRun(sessionId);
          setSendError(classifyAiChatError(error, t));
          setRestoreNotice(null);
        }
        return;
      }
    }

    if (sendBlocked && !options?.force && !options?.goalContinuation) return;

    const abortController = new AbortController();
    let workingHistory = overrideHistory ?? useLuxStore.getState().aiChatSessions.find((session) => session.id === sessionId)?.messages ?? [];
    const sessionSnapshot = useLuxStore.getState().aiChatSessions.find((session) => session.id === sessionId);
    if (!overrideMessage && workingHistory.length >= 6) {
      const prunedHistory = pruneStaleToolOutputs(workingHistory);
      const savedTokens = pruneReducedTokenEstimate(workingHistory, prunedHistory);
      if (prunedHistory !== workingHistory && savedTokens >= 400) {
        replaceAiChatMessages(sessionId, prunedHistory, { contextCompaction: sessionSnapshot?.contextCompaction ?? null });
        workingHistory = prunedHistory;
      }
    }
    if (!overrideMessage && selectedModel && selectedProvider && shouldAutoCompactContext({
      messages: workingHistory,
      model: selectedModel,
      threshold: aiPreferences.contextAutoCompactThreshold,
      autoCompactEnabled: aiPreferences.contextAutoCompactEnabled,
      compactionState: sessionSnapshot?.contextCompaction ?? null,
    })) {
      setCompacting(true);
      try {
        const compacted = await runContextCompaction({
          chatSessionId: sessionId,
          messages: workingHistory,
          compactionState: sessionSnapshot?.contextCompaction ?? null,
          model: selectedModel,
          provider: selectedProvider,
          selectedEffortId: aiPreferences.selectedEffortId,
          threshold: aiPreferences.contextAutoCompactThreshold,
          autoCompactEnabled: true,
          force: false,
          abortSignal: abortController.signal,
        });
        if (compacted.compacted) {
          replaceAiChatMessages(sessionId, compacted.messages, { contextCompaction: compacted.compactionState });
          workingHistory = compacted.messages;
        }
      } finally {
        setCompacting(false);
      }
    }
    const orchestrationKind = options?.goalOrchestration
      ?? (options?.goalContinuation ? "continuation" : undefined);
    const isGoalOrchestration = orchestrationKind != null;
    const displayMessage = isGoalOrchestration ? "" : nextMessage;
    const modelMessage = nextMessage;
    const runtimePreferences = useLuxStore.getState().aiPreferences;
    const currentAttachments = overrideMessage && !options?.useComposerAttachments ? [] : attachments;
    const messageAttachments = isGoalOrchestration ? [] : await buildMessageDisplayAttachments(currentAttachments);
    const userMessageId = crypto.randomUUID();
    let turnCheckpointId: string | undefined;
    let turnFileCheckpointId: string | undefined;
    if (workspace && selectedProvider && selectedModel && !overrideMessage && !isGoalOrchestration) {
      try {
        const turn = await createTurnCheckpointBeforeSend({
          input: buildCheckpointSendInput({
            activeDocument,
            aiPreferences,
            locale,
            openDocuments,
            selectedModel,
            selectedProvider,
            terminal,
            terminalOutputBuffers,
            terminalSessions,
            workspace,
          }),
          label: displayMessage.slice(0, 80) || t("aiChat.turnCheckpoint.defaultLabel"),
          messages: workingHistory,
          sessionId,
          userMessageId,
        });
        turnCheckpointId = turn.id;
        turnFileCheckpointId = turn.fileCheckpointId;
      } catch (error) {
        console.warn("Turn checkpoint failed:", error);
      }
    }
    const history = workingHistory;
    startAiChatTurn(sessionId, abortController);
    const turnGeneration = getTurnGeneration(sessionId);
    const isActiveTurn = () => isActiveChatTurn(sessionId, turnGeneration, abortController);
    if (isGoalOrchestration && orchestrationKind) {
      appendAiChatMessage(sessionId, createInternalGoalOrchestrationMessage(modelMessage, orchestrationKind));
    } else {
      const userMessage: AiChatMessage = {
        id: userMessageId,
        role: "user",
        kind: options?.reviewRequest ? "review-request" : undefined,
        content: displayMessage,
        attachments: messageAttachments.length > 0 ? messageAttachments : undefined,
        turnCheckpointId,
        timestamp: Date.now(),
      };
      appendAiChatMessage(sessionId, userMessage);
      // A review request is a system-authored prompt, not user-typed text: skip the
      // title refresh (the long instruction must not become the chat title) and the
      // draft bookkeeping below.
      if (selectedProvider && selectedModel && !options?.reviewRequest) {
        scheduleChatSessionTitleRefresh({
          sessionId,
          firstUserMessage: displayMessage,
          rename: renameAiChatSession,
          readSession: (id) => {
            const session = useLuxStore.getState().aiChatSessions.find((entry) => entry.id === id);
            return session ? { title: session.title, messages: session.messages } : null;
          },
        });
      }
      pinnedToBottomRef.current = true;
      setShowScrollDown(false);
      // Review requests don't originate from the composer, so leave the user's draft and
      // pending attachments intact instead of clearing them.
      if (!options?.reviewRequest) {
        setLastUserDraft(displayMessage);
        setComposerDraft(sessionId, "");
        setComposerAttachments(sessionId, []);
        setMessage("");
        setAttachments((current) => {
          revokeComposerAttachmentPreviews(current);
          return [];
        });
      }
    }
    setSendError(null);
    setAiChatSessionStatus(sessionId, "thinking");

    let completedAssistantMessage: AiChatMessage | undefined;
    try {
      const attachmentOptions = {
        includeVisionImage: aiPreferences.includeImages,
        visionImageFormat: resolveVisionImageFormat(selectedProvider, selectedModel, aiPreferences.visionImageFormat),
        includeMediaContext: true,
        localSttCommand: aiPreferences.localSttCommand,
        localSttModelPath: aiPreferences.localSttModelPath,
        voiceInputLanguage: aiPreferences.voiceInputLanguage,
      };
      const runtimeAttachments: AiChatAttachmentInput[] = await Promise.all(currentAttachments.map(async (attachment) => {
        if (attachment.kind === "file") return readChatAttachment(attachment.file, attachmentOptions);
        if (attachment.kind === "selection") {
          return readSelectionAttachment(attachment, openDocuments);
        }
        if (attachment.kind === "mention") {
          return (await buildMentionRuntimeAttachments([attachment], openDocuments, attachmentOptions))[0] ?? {
            name: attachment.name,
            size: 0,
            text: attachment.name,
          };
        }
        const document = openDocuments.find((candidate) => candidate.id === attachment.documentId);
        if (!document) {
          return { name: attachment.name, size: attachment.size, text: `Pinned editor tab is no longer open: ${attachment.name}` };
        }
        return readEditorDocumentAttachment(document, attachmentOptions);
      }));
      // The native Rust turn-loop is the only orchestration path in the desktop
      // runtime; the TS turn-loop runs solely in dev-only browser preview where no
      // Rust/Tauri backend exists.
      const runTurn = isTauriRuntime() ? runNativeChatTurn : sendAiChatMessage;
      completedAssistantMessage = await runTurn({
        abortSignal: abortController.signal,
        activeDocument,
        attachments: runtimeAttachments,
        mentionHints: collectMentionHints(currentAttachments),
        chatSessionId: sessionId,
        turnCheckpoint: turnCheckpointId && turnFileCheckpointId
          ? { turnCheckpointId, fileCheckpointId: turnFileCheckpointId }
          : undefined,
        history,
        locale,
        message: modelMessage,
        openDocuments,
        preferences: runtimePreferences,
        provider: selectedProvider,
        globalInstructions: aiPreferences.globalInstructions,
        projectInstructions,
        selectedAgentInstructions: selectedAgent?.instructions ?? "",
        selectedAgentName: selectedAgent?.name ?? "",
        selectedModel,
        terminal,
        terminalContext: { activeTerminalId, outputBuffers: terminalOutputBuffers, sessions: terminalSessions },
        workspace,
        onAssistantMessage: (assistantMessage) => {
          if (!isActiveTurn()) return;
          const session = useLuxStore.getState().aiChatSessions.find((candidate) => candidate.id === sessionId);
          if (session?.messages.some((candidate) => candidate.id === assistantMessage.id)) return;
          appendAiChatMessage(sessionId, assistantMessage);
        },
        onAssistantMessageUpdate: (messageId, patch) => {
          if (!isActiveTurn()) return;
          updateAiChatMessage(sessionId, messageId, patch);
        },
        onStatusChange: (status) => {
          if (!isActiveTurn()) return;
          setAiChatSessionStatus(sessionId, statusToSessionStatus(status));
        },
        onToolApproval: requestToolApproval,
        onContextBudgetReport: (report) => {
          if (!isActiveTurn()) return;
          setAiChatSessionContextBudgetReport(sessionId, report);
        },
        onFilePathsEdited: (paths) => {
          const first = paths.find((path) => path.trim());
          if (first) void openWorkspaceEditorPath(first);
        },
      });
      if (isActiveTurn()) setAiChatSessionStatus(sessionId, "idle");
    } catch (error) {
      if (isAbortError(error)) {
        setAiChatSessionStatus(sessionId, "idle");
        trimCancelledAssistantShell(sessionId, replaceAiChatMessages);
        return;
      }
      if (!isActiveTurn()) return;
      const errorPresentation = classifyAiChatError(error, t);
      const errorMessage = errorPresentation.message;
      const assistantError: AiChatMessage = {
        id: crypto.randomUUID(),
        role: "assistant",
        content: errorMessage,
        timestamp: Date.now(),
      };
      replaceAiChatMessages(sessionId, replaceEmptyAssistantTail(useLuxStore.getState().aiChatSessions.find((session) => session.id === sessionId)?.messages ?? [], assistantError));
      setAiChatSessionStatus(sessionId, isAbortError(error) ? "idle" : "error", errorMessage);
      if (useLuxStore.getState().activeAiChatSessionId === sessionId) setSendError(errorPresentation);
    } finally {
      finishAiChatTurn(sessionId, abortController);
      requestAnimationFrame(() => resizeComposerTextarea());
      const pendingContinuation = goalContinuationTimersRef.current.get(sessionId);
      if (pendingContinuation !== undefined) {
        clearTimeout(pendingContinuation);
        goalContinuationTimersRef.current.delete(sessionId);
      }
      if (abortController.signal.aborted) {
        stopGoalRun(sessionId);
      } else {
        const sessionAfterTurn = useLuxStore.getState().aiChatSessions.find((entry) => entry.id === sessionId);
        const turnFailed = sessionAfterTurn?.status === "error";
        if (turnFailed) {
          stopGoalRun(sessionId);
        } else {
          const historyAfterTurn = sessionAfterTurn?.messages ?? [];
          const assistantTail = completedAssistantMessage ?? lastAssistantMessage(historyAfterTurn);
          recordGoalRunTurnUsage(sessionId, assistantTail?.turnUsage, assistantTail);
          recordAiUsageLogEntry(assistantTail);
          syncGoalRunFromAssistantMessage(sessionId, assistantTail);
          const continuation = await evaluateGoalRunContinuationAfterTurn({
            sessionId,
            messages: historyAfterTurn,
            agentMode: useLuxStore.getState().aiPreferences.agentMode,
            provider: selectedProvider,
            selectedModel,
            selectedEffortId: useLuxStore.getState().aiPreferences.selectedEffortId,
            abortSignal: abortController.signal,
          });
          if (continuation.continue) {
            const continuationSessionId = sessionId;
            const handle = window.setTimeout(() => {
              if (goalContinuationTimersRef.current.get(continuationSessionId) !== handle) return;
              goalContinuationTimersRef.current.delete(continuationSessionId);
              const live = useLuxStore.getState().aiChatSessions.find((entry) => entry.id === continuationSessionId);
              if (!live || isAiChatSessionBusyStatus(live.status) || live.status === "error") return;
              if (getAiChatTurnRuntimeSnapshot().sendingSessionId === continuationSessionId) return;
              if (!getActiveGoalRun(continuationSessionId)) return;
              void handleSendRef.current(
                buildGoalContinuationDirective(continuationSessionId, { budgetWrapup: continuation.budgetWrapup }),
                undefined,
                {
                  skipGoalSlash: true,
                  goalOrchestration: "continuation",
                  sessionId: continuationSessionId,
                  internalSend: true,
                  force: true,
                },
              );
            }, resolveGoalContinuationDelayMs(continuationSessionId));
            goalContinuationTimersRef.current.set(continuationSessionId, handle);
          } else if (
            continuation.reason === "completed"
            || continuation.reason === "max_rounds"
            || continuation.reason === "blocked"
            || continuation.reason === "stall"
            || continuation.reason === "paused"
          ) {
            const run = getGoalRunSnapshot(sessionId);
            if (run && useLuxStore.getState().activeAiChatSessionId === sessionId) {
              const elapsed = formatGoalRunDuration(formatGoalRunElapsedMs(run));
              const tokens = formatCompactTokens(formatGoalRunTokenTotal(run));
              setRestoreNotice(
                continuation.reason === "completed"
                  ? t("aiChat.goalRun.completed", { elapsed, tokens, rounds: run.round, progress: run.progress })
                  : continuation.reason === "blocked"
                    ? t("aiChat.goalRun.blocked", { elapsed, rounds: run.round, progress: run.progress })
                    : continuation.reason === "stall"
                      ? t("aiChat.goalRun.stalled", { elapsed, rounds: run.round, progress: run.progress })
                      : continuation.reason === "paused"
                        ? t("aiChat.goalRun.paused", { elapsed, rounds: run.round, progress: run.progress })
                        : t("aiChat.goalRun.maxRounds", {
                            elapsed,
                            tokens,
                            rounds: run.round,
                            maxRounds: run.limits.maxRounds,
                            progress: run.progress,
                          }),
              );
            }
          }
        }
      }
    }
  }, [activeChatSession?.id, activeDocument, activeSessionBusy, activeSessionClosed, activeTerminalId, aiPreferences, appendAiChatMessage, attachments, createAiChatSession, locale, message, openDocuments, projectInstructions, replaceAiChatMessages, requestToolApproval, resizeComposerTextarea, runCompaction, selectedAgent, selectedModel, selectedProvider, setAiChatSessionContextBudgetReport, setAiChatSessionStatus, t, terminal, terminalOutputBuffers, terminalSessions, updateAiChatMessage, updateMessage, workspace]);

  // Keep the freshest handleSend instance reachable from the deferred goal-continuation
  // timer so a delayed continuation turn picks up current provider/model/openDocuments/
  // terminal context instead of the closure frozen when the timer was scheduled.
  const handleSendRef = useRef(handleSend);
  handleSendRef.current = handleSend;

  const buildRestoreInput = useCallback(() => {
    if (!workspace || !selectedProvider || !selectedModel) return null;
    return buildCheckpointSendInput({
      activeDocument,
      aiPreferences,
      locale,
      openDocuments,
      selectedModel,
      selectedProvider,
      terminal,
      terminalOutputBuffers,
      terminalSessions,
      workspace,
    });
  }, [activeDocument, aiPreferences, locale, openDocuments, selectedModel, selectedProvider, terminal, terminalOutputBuffers, terminalSessions, workspace]);

  const showRestoreSuccess = useCallback((result: { restoredFileCount: number; removedTurnCheckpoints: number }) => {
    setRestoreNotice(t("aiChat.turnCheckpoint.restored", {
      files: result.restoredFileCount,
      turns: result.removedTurnCheckpoints,
    }));
    setSendError(null);
  }, [t]);

  const handleEditUserMessage = useCallback(async (userMessageId: string, nextContent: string, options?: { skipConfirm?: boolean }) => {
    if (!activeChatSession || activeSessionClosed || !nextContent.trim()) return;
    const input = buildRestoreInput();
    if (!input) return;
    if (!options?.skipConfirm) {
      const confirmed = window.confirm(t("aiChat.turnCheckpoint.editConfirm"));
      if (!confirmed) return;
    }
    abortAiChatTurn(activeChatSession.id);
    try {
      const result = await restoreChatBeforeUserMessage({
        currentMessages: activeChatSession.messages,
        input,
        sessionId: activeChatSession.id,
        userMessageId,
      });
      replaceAiChatMessages(activeChatSession.id, result.messages, { contextCompaction: null });
      showRestoreSuccess(result);
      saveChatCheckpointStore();
      await handleSend(nextContent.trim(), result.messages, { force: true });
    } catch (error) {
      setSendError(classifyAiChatError(error, t));
      setRestoreNotice(null);
    }
  }, [activeChatSession, activeSessionClosed, buildRestoreInput, handleSend, replaceAiChatMessages, showRestoreSuccess, t]);

  const canRestoreUserMessage = useCallback((userMessageId: string) => {
    if (!activeChatSession || !workspace) return false;
    return Boolean(
      activeChatSession.messages.find((message) => message.id === userMessageId)?.turnCheckpointId
      || hasUserTurnCheckpoint(activeChatSession.id, userMessageId),
    );
  }, [activeChatSession, workspace]);

  const retryLastRequest = useCallback(() => {
    const draft = lastUserDraft ?? [...messages].reverse().find((entry) => entry.role === "user")?.content ?? "";
    if (!draft.trim() || activeSessionBusy || activeSessionClosed) return;
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
  }, [activeChatSession, activeSessionBusy, activeSessionClosed, handleSend, lastUserDraft, messages, replaceAiChatMessages]);

  const handleReviewAction = useCallback((messageId: string) => {
    if (!activeChatSession || activeSessionBusy || activeSessionClosed) return;
    // Send the review instruction straight to the agent as a review-request turn. The
    // full prompt reaches the model as the message content, but the chat renders a badge
    // (not the raw text) and the composer draft is left untouched. messageId scopes the
    // review to "the turn whose Review button was clicked" for the model's benefit.
    const prompt = t("aiChat.review.prompt");
    void handleSend(prompt, undefined, { reviewRequest: true, force: true });
  }, [activeChatSession, activeSessionBusy, activeSessionClosed, handleSend, t]);

  const handleMentionSelect = useCallback((candidate: AiMentionCandidate) => {
    const parsed = parseMentionQuery(message);
    if (!parsed) return;
    attachMention(candidate);
    updateMessage(applyMentionSelection(message, parsed));
    setMentionMenuOpen(false);
  }, [attachMention, message, updateMessage]);

  const handlePlanHandoff = useCallback(() => {
    if (!planHandoff || !selectedProvider || !selectedModel) return;
    const sessionId = activeChatSession?.id ?? createAiChatSession(workspace?.root ?? null);
    const goalText = planHandoff.steps.join(" → ").slice(0, 2_000);
    setAiSessionGoal(sessionId, goalText);
    replaceAiSessionTodos(sessionId, planHandoff.steps.map((step, index) => ({
      id: `plan-${index + 1}`,
      content: step,
      status: index === 0 ? "in_progress" : "pending",
      priority: "medium",
      source: "agent",
    })));
    const handoffMessage = buildPlanHandoffUserMessage(planHandoff.steps);
    const agentProfile = aiPreferences.agentProfiles.find((profile) => profile.mode === "agent")
      ?? aiPreferences.agentProfiles[0];
    if (agentProfile) updateAiPreference({ selectedAgentId: agentProfile.id, agentMode: "agent" });
    updateMessage(handoffMessage);
    void handleSend(handoffMessage, messages, { force: true });
  }, [activeChatSession?.id, aiPreferences.agentProfiles, createAiChatSession, handleSend, messages, planHandoff, selectedModel, selectedProvider, updateAiPreference, updateMessage, workspace?.root]);

  // Deliver a human answer to a pending AskUser question. settlePendingQuestion
  // resolves the browser/dev waiter (no-op on native); the command unblocks the
  // native Rust loop (no-op/ignored on the dev path). Calling both keeps one
  // handler correct for either turn-loop.
  const handleQuestionAnswer = useCallback((answer: string) => {
    if (!pendingQuestion) return;
    settlePendingQuestion(pendingQuestion.requestId, { answer, cancelled: false });
    if (isTauriRuntime()) {
      void luxCommands
        .aiResolveTurnQuestion(pendingQuestion.turnId, pendingQuestion.requestId, { answer, cancelled: false })
        .catch(() => undefined);
    }
  }, [pendingQuestion]);

  const handleQuestionDismiss = useCallback(() => {
    if (!pendingQuestion) return;
    settlePendingQuestion(pendingQuestion.requestId, { answer: "", cancelled: true });
    if (isTauriRuntime()) {
      void luxCommands
        .aiResolveTurnQuestion(pendingQuestion.turnId, pendingQuestion.requestId, { answer: "", cancelled: true })
        .catch(() => undefined);
    }
  }, [pendingQuestion]);

  // Start a proposed plan: switch to Agent mode, clear the card, and hand the
  // plan to execution. Goal + task list were already pinned by the PresentPlan
  // tool, so the rail already reflects it.
  const handlePlanStart = useCallback(() => {
    if (!pendingPlan || activeSessionBusy || activeSessionClosed) return;
    const sessionId = pendingPlan.sessionId;
    clearPendingPlan(pendingPlan.planId);
    const agentProfile = aiPreferences.agentProfiles.find((profile) => profile.mode === "agent")
      ?? aiPreferences.agentProfiles[0];
    if (agentProfile) updateAiPreference({ selectedAgentId: agentProfile.id, agentMode: "agent" });
    const handoffMessage = buildPlanHandoffUserMessage(pendingPlan.steps.map((step) => step.title));
    void handleSend(handoffMessage, undefined, { force: true });
    void sessionId;
  }, [pendingPlan, activeSessionBusy, activeSessionClosed, aiPreferences.agentProfiles, updateAiPreference, handleSend]);

  const handleComposerKeyDown = useCallback((event: KeyboardEvent<HTMLTextAreaElement>) => {
    // While an IME is composing (CJK candidate selection), let the IME consume
    // every key — Enter/Tab/arrows confirm the candidate, they must not submit
    // the message or pick a mention/slash command.
    if (event.nativeEvent.isComposing || event.keyCode === 229) return;
    if (mentionMenuOpen && mentionCandidates.length > 0) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setMentionNavigated(true);
        setMentionActiveIndex((index) => (index + 1) % mentionCandidates.length);
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setMentionNavigated(true);
        setMentionActiveIndex((index) => (index - 1 + mentionCandidates.length) % mentionCandidates.length);
        return;
      }
      // Tab always picks the highlighted candidate. Enter only picks it when the
      // user has actually navigated the menu (arrow keys); a bare "@word" + Enter
      // must SEND the message, not silently swallow it into a mention pick. This
      // also avoids acting on stale candidates from the search debounce.
      if (event.key === "Tab" || (event.key === "Enter" && !event.shiftKey && mentionNavigated)) {
        event.preventDefault();
        const selected = mentionCandidates[mentionActiveIndex];
        if (selected) handleMentionSelect(selected);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setMentionMenuOpen(false);
        return;
      }
    }
    if (slashMenuOpen && slashCommands.length > 0) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setSlashActiveIndex((index) => (index + 1) % slashCommands.length);
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setSlashActiveIndex((index) => (index - 1 + slashCommands.length) % slashCommands.length);
        return;
      }
      if (event.key === "Tab" || (event.key === "Enter" && !event.shiftKey)) {
        event.preventDefault();
        const selected = slashCommands[slashActiveIndex];
        if (selected) handleSlashSelect(selected);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        setSlashMenuOpen(false);
        return;
      }
    }
    if (event.key !== "Enter" || event.shiftKey) return;
    event.preventDefault();
    void handleSend();
  }, [handleMentionSelect, handleSend, handleSlashSelect, mentionActiveIndex, mentionCandidates, mentionMenuOpen, mentionNavigated, slashActiveIndex, slashCommands, slashMenuOpen]);

  // In the dedicated Agent workspace the left rail already owns New chat,
  // History and the browser-preview button — so the in-chat header would only
  // duplicate them. Keep header actions for the side panel ("panel") only.
  const showHeaderSessionChrome = showSessionChrome && presentation !== "agent";

  const renderHeaderActions = () => (
    <div className="ai-chat-header-actions">
      {showHeaderSessionChrome && (
        <>
          <button
            className="icon-button compact"
            type="button"
            aria-label={t("agent.newChat")}
            title={t("agent.newChat")}
            onClick={startNewChat}
          >
            <Plus size={15} />
          </button>
          <AiChatHistoryPopover workspaceRoot={workspace?.root ?? null} />
          {aiPreferences.agentBrowserEnabled && activeAiChatSessionId && (
            <button
              className="icon-button compact"
              type="button"
              aria-label={t("aiChat.browserPreview.openTab")}
              title={t("aiChat.browserPreview.openTab")}
              onClick={openBrowserPreviewTab}
            >
              <Globe size={15} />
            </button>
          )}
        </>
      )}
      {showCloseButton && (
        <button className="icon-button compact" type="button" aria-label={t("aiChat.closeChat")} title={t("aiChat.closeChat")} onClick={() => setAiChatOpen(false)}>
          <PanelRightClose size={15} />
        </button>
      )}
    </div>
  );

  const renderComposerContent = () => (
    <AiChatComposer
      activeSessionSending={showStopGeneration}
      disabled={activeSessionClosed}
      agentOptions={agentOptions}
      attachments={composerAttachments}
      attachFiles={attachFiles}
      canSend={canSend}
      contextOpen={contextOpen}
      contextTitle={contextTitle}
      contextUsage={contextUsage}
      contextDrops={contextDrops}
      draggingFiles={draggingFiles}
      effortOptions={effortOptions}
      fileInputRef={fileInputRef}
      handleCancelSend={handleCancelSend}
      handleComposerDragOver={handleComposerDragOver}
      handleComposerDrop={handleComposerDrop}
      compacting={compacting}
      handleComposerKeyDown={handleComposerKeyDown}
      handleComposerPaste={handleComposerPaste}
      handleMessageChange={handleMessageChange}
      handleSend={() => void handleSend()}
      onSlashHighlight={setSlashActiveIndex}
      onSlashSelect={handleSlashSelect}
      slashActiveIndex={slashActiveIndex}
      slashCommands={slashCommands}
      slashMenuOpen={slashMenuOpen}
      slashMenuRef={slashMenuRef}
      isAgentHome={isAgentHome}
      message={message}
      modelOptions={modelOptions}
      modelSupportsEffort={modelSupportsEffort}
      providerOptions={providerOptions}
      selectedProviderId={selectedProvider?.id ?? aiPreferences.selectedProviderId}
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
      updateProvider={updateProvider}
      voiceInput={voiceInput}
      mentionMenuOpen={mentionMenuOpen}
      mentionCandidates={mentionCandidates}
      mentionActiveIndex={mentionActiveIndex}
      mentionMenuRef={mentionMenuRef}
      onMentionHighlight={setMentionActiveIndex}
      onMentionSelect={handleMentionSelect}
      onContextCompact={() => void runCompaction(true)}
      onOpenSettings={() => openSettingsSection("ai-runtime")}
    />
  );

  return (
    <aside className="ai-chat-panel" aria-label={t("aiChat.panel.aria")} data-empty-home={isAgentHome} data-embedded={embedded} data-presentation={presentation} data-status={activeStatus}>
      {isAgentHome ? (
        <header className="ai-chat-header ai-chat-header-minimal">
          <div className="ai-chat-title" aria-hidden="true" />
          {renderHeaderActions()}
        </header>
      ) : (
        <header className="ai-chat-header">
          <div className="ai-chat-title">
            <Sparkles size={15} />
            <span>{activeChatSession ? aiChatSessionTitle(activeChatSession.title, t) : t("aiChat.title")}</span>
            <span className="ai-chat-status-chip" data-status={activeStatus}>{aiChatStatusLabel(activeStatus, true, t)}</span>
          </div>
          {renderHeaderActions()}
        </header>
      )}

      {pendingCrossSessionApproval && (
        <AiChatGlobalApprovalBanner
          pending={pendingCrossSessionApproval}
          hiddenOnActiveSession={false}
          onFocusSession={() => setActiveAiChatSession(pendingCrossSessionApproval.sessionId)}
          onDecision={resolveToolApproval}
          t={t}
        />
      )}

      <div
        className="ai-chat-body"
        data-orchestration-rail={showOrchestrationRail || undefined}
        data-agent-island-collapsed={showOrchestrationRail && isAgentIslandCollapsed ? "" : undefined}
      >
        <div className="ai-chat-main">
        {showOrchestrationRail && activeChatSession && (
          <AiAgentOrchestrationRail
            sessionId={activeChatSession.id}
            agentMode={aiPreferences.agentMode}
            sessionStatus={activeStatus}
            preferences={aiPreferences}
            t={t}
            collapsed={isAgentIslandCollapsed}
            onToggleCollapsed={() => setIsAgentIslandCollapsed((v) => !v)}
          />
        )}
        <div className="ai-chat-scroll" ref={scrollRef} onScroll={handleBodyScroll}>
          {!showOrchestrationRail && activeChatSession && (
            <AiSubagentPanel sessionId={activeChatSession.id} t={t} />
          )}
          {!showOrchestrationRail && (
            <AiAutomaticChecklist
              sessionId={activeChatSession?.id ?? ""}
              agentMode={aiPreferences.agentMode}
              t={t}
            />
          )}
          {messages.length > 0 ? (
            <section className="ai-chat-thread" aria-live="polite">
              <AiChatMessages
                canMutateHistory={!activeSessionBusy && !activeSessionClosed && Boolean(workspace)}
                canRestoreUserMessage={canRestoreUserMessage}
                messages={visibleMessages}
                parentRef={scrollRef}
                streamingMessageId={streamingMessageId}
                sessionStatus={activeStatus}
                showResponseDuration={aiPreferences.showResponseDuration}
                contextCompaction={activeChatSession?.contextCompaction}
                workspaceRoot={workspace?.root ?? null}
                onApprovalDecision={resolveToolApproval}
                onEditUserMessage={handleEditUserMessage}
                onStopAfterTool={requestStopAfterToolRound}
                canStopAfterTool={activeSessionBusy}
                t={t}
                onReviewAction={handleReviewAction}
              />
              {pendingPlan && (
                <AiPlanCard
                  plan={pendingPlan}
                  onStart={handlePlanStart}
                  busy={activeSessionBusy || activeSessionClosed}
                  t={t}
                />
              )}
              {pendingQuestion && (
                <AiQuestionCard
                  question={pendingQuestion}
                  onAnswer={handleQuestionAnswer}
                  onDismiss={handleQuestionDismiss}
                  t={t}
                />
              )}
              {showLegacyPlanHandoff && !activeSessionBusy && (
                <div className="ai-plan-handoff" role="region" aria-label={t("aiChat.planHandoff.aria")}>
                  <div>
                    <strong>{t("aiChat.planHandoff.title")}</strong>
                    <p>{t("aiChat.planHandoff.description", { count: planHandoff.steps.length })}</p>
                  </div>
                  <button type="button" className="primary" onClick={() => handlePlanHandoff()}>
                    {t("aiChat.planHandoff.run")}
                  </button>
                </div>
              )}
              {restoreNotice && <div className="ai-chat-restore-notice" role="status">{restoreNotice}</div>}
              {showStandaloneThinking && <AiThinkingIndicator status={activeStatus} t={t} />}
              {activeSessionClosed && <AiChatClosedNotice onRestore={() => restoreAiChatSession(activeAiChatSessionId)} t={t} />}
              {sendError && (
                <AiChatError
                  presentation={sendError}
                  canRetry={Boolean(lastUserDraft)}
                  onRetry={retryLastRequest}
                  onOpenSettings={() => openSettingsSection("ai-runtime")}
                  t={t}
                />
              )}
              {activeLastErrorPresentation && !sendError && (
                <AiChatError
                  presentation={activeLastErrorPresentation}
                  canRetry={Boolean(lastUserDraft)}
                  onRetry={retryLastRequest}
                  onOpenSettings={() => openSettingsSection("ai-runtime")}
                  t={t}
                />
              )}
            </section>
          ) : (
            <section className="ai-chat-empty">
              <div className="ai-chat-mark"><Sparkles size={22} /></div>
              <h2>{presentation === "agent" ? (workspace ? t("agent.welcome.titleWithWorkspace", { workspaceName: workspace.name }) : t("agent.welcome.title")) : t("aiChat.empty.title")}</h2>
              {presentation === "agent" && (
                <p className="ai-chat-empty-subtitle">{workspace ? t("agent.welcome.subtitle") : t("agent.welcome.subtitleNoWorkspace")}</p>
              )}
              {presentation === "agent" && (
                <div className="ai-chat-empty-composer">
                  {renderComposerContent()}
                </div>
              )}
              <div className="ai-chat-suggestions" aria-label={t("aiChat.suggestions.aria")}>
                <button
                  type="button"
                  onClick={() => {
                    attachSelection();
                    updateMessage(t("aiChat.suggestion.explainSelectedCode.prompt"));
                  }}
                >
                  <Code2 size={15} className="ai-chat-suggestion-icon" />
                  <span>{t("aiChat.suggestion.explainSelectedCode.button")}</span>
                  <ArrowUpRight size={14} className="ai-chat-suggestion-arrow" />
                </button>
                <button type="button" onClick={() => updateMessage(t("aiChat.suggestion.fixCompileErrors.prompt"))}>
                  <Bug size={15} className="ai-chat-suggestion-icon" />
                  <span>{t("aiChat.suggestion.fixCompileErrors.button")}</span>
                  <ArrowUpRight size={14} className="ai-chat-suggestion-arrow" />
                </button>
                <button type="button" onClick={() => updateMessage(t("aiChat.suggestion.generateTests.prompt"))}>
                  <FlaskConical size={15} className="ai-chat-suggestion-icon" />
                  <span>{t("aiChat.suggestion.generateTests.button")}</span>
                  <ArrowUpRight size={14} className="ai-chat-suggestion-arrow" />
                </button>
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
      </div>

      {!isAgentHome && (
        <footer className="ai-chat-composer-shell">
          {presentation === "agent" && !workspace && (
            <p className="ai-chat-checkpoint-agent-hint" role="note">{t("aiChat.turnCheckpoint.agentNeedProject")}</p>
          )}
          {renderComposerContent()}
        </footer>
      )}
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

function AiChatError({
  canRetry,
  presentation,
  onRetry,
  onOpenSettings,
  t,
}: {
  canRetry: boolean;
  presentation: AiChatErrorPresentation;
  onRetry: () => void;
  onOpenSettings?: () => void;
  t: TranslateFn;
}) {
  const retryLabel = presentation.kind === "approval"
    ? t("aiChat.error.action.retryApproval")
    : presentation.canRetryTools
      ? t("aiChat.error.action.retryTools")
      : t("aiChat.error.action.retry");
  const showRetry = canRetry && (presentation.canRetry || presentation.canRetryTools);

  return (
    <div className="ai-chat-error" role="status" data-kind={presentation.kind}>
      <span>{presentation.message}</span>
      <div className="ai-chat-error-actions">
        {showRetry && (
          <button type="button" onClick={onRetry}>
            <RotateCcw size={13} />
            <span>{retryLabel}</span>
          </button>
        )}
        {presentation.canOpenSettings && onOpenSettings && (
          <button type="button" onClick={onOpenSettings}>
            <span>{t("aiChat.error.action.openSettings")}</span>
          </button>
        )}
      </div>
    </div>
  );
}

function statusToSessionStatus(status: "thinking" | "streaming" | "running-tools" | "waiting-approval"): AiChatSessionStatus {
  return status;
}

function trimCancelledAssistantShell(
  sessionId: string,
  replaceMessages: (sessionId: string, messages: AiChatMessage[]) => void,
) {
  const session = useLuxStore.getState().aiChatSessions.find((entry) => entry.id === sessionId);
  if (!session) return;
  const last = session.messages[session.messages.length - 1];
  if (last?.role !== "assistant") return;
  const hasContent = Boolean(
    last.content.trim()
    || last.reasoning?.trim()
    || (last.toolCalls?.length ?? 0) > 0
    || (last.segments?.length ?? 0) > 0,
  );
  if (!hasContent) replaceMessages(sessionId, session.messages.slice(0, -1));
}

function replaceEmptyAssistantTail(messages: AiChatMessage[], assistantError: AiChatMessage) {
  const last = messages[messages.length - 1];
  if (
    last?.role === "assistant"
    && !last.content.trim()
    && !last.reasoning?.trim()
    && (last.toolCalls?.length ?? 0) === 0
    && (last.segments?.length ?? 0) === 0
  ) {
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

function isAbortError(error: unknown) {
  return error instanceof DOMException && error.name === "AbortError";
}

function readErrorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}


