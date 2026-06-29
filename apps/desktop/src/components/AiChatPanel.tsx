import { ArrowDown, ArrowUpRight, Brain, Bug, Code2, FlaskConical, Globe, PanelRightClose, Plus, Sparkles, X } from "lucide-react";
import type { ChangeEvent, ClipboardEvent, CSSProperties, DragEvent, KeyboardEvent } from "react";
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
import { clearAiRetryNotice, setAiRetryNotice } from "../lib/aiRetryNotice";
import { automaticRetryReason, isTransientRetryKind, nextAutomaticRetry, resetAutomaticRetry } from "../lib/aiAutomaticRetry";
import { isAutomaticSocialOnlyMessage } from "../lib/aiAutomaticSocialMessage";

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
  revokeComposerAttachmentPreviews,
} from "../lib/aiChatComposerAttachments";
import { buildMentionRuntimeAttachments, collectMentionHints } from "../lib/aiChatMentionAttachments";
import { applyMentionSelection, mentionMenuVisible, parseMentionQuery, searchMentionCandidates, type AiMentionCandidate } from "../lib/aiChatMentions";
import { buildPlanHandoffUserMessage, extractPlanHandoffPayload } from "../lib/aiChatPlanHandoff";
import { clearPendingPlan, getPendingPlanForSession, getPendingPlansSnapshot, subscribePendingPlans } from "../lib/aiPendingPlan";
import { getPendingQuestionForSession, getPendingQuestionsSnapshot, settlePendingQuestion, subscribePendingQuestions } from "../lib/aiPendingQuestion";
import { buildQueuedMessagePayload, dequeueFirstForSession, enqueueChatMessage, getQueuedMessagesForSession, removeQueuedMessage, updateQueuedMessage, useAllQueuedMessages, type QueuedMessage } from "../lib/aiChatQueue";
import { AiChatQueuedMessages } from "./ai-chat/AiChatQueuedMessages";
import { readEditorDocumentAttachment, readSelectionAttachment } from "../lib/aiChatDocumentAttachment";
import { readChatAttachment, sendAiChatMessage } from "../lib/aiChatRuntime";
import { runNativeChatTurn } from "../lib/aiNativeTurn";
import {
  findLastUserMessageIndex,
  isAbortError,
  messageHasAssistantWork,
  readErrorMessage,
  recordAiUsageLogEntry,
  replaceEmptyAssistantTail,
  statusToSessionStatus,
  stripTrailingErrorBubble,
  trimCancelledAssistantShell,
} from "../lib/aiChatPanelTurnHelpers";
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
import { getActiveTurnId } from "../lib/aiActiveTurns";
import { useVoiceInput } from "../lib/useVoiceInput";
import { useAiChatScroll } from "../lib/useAiChatScroll";
import { useAiChatComposerAttachments } from "../lib/useAiChatComposerAttachments";
import { useComposerSessionDraft } from "../lib/useComposerSessionDraft";
import {
  setComposerAttachments,
  setComposerDraft,
} from "../lib/aiChatComposerSession";
import { findAnyPendingToolApproval } from "../lib/aiChatPendingApproval";
import { openWorkspaceEditorPath } from "../lib/openWorkspaceEditorPath";
import { AiChatGlobalApprovalBanner } from "./ai-chat/AiChatGlobalApprovalBanner";
import { AiThinkingIndicator, isPendingAssistantShell } from "./ai-chat/AiThinkingIndicator";
import { AiChatClosedNotice } from "./ai-chat/AiChatClosedNotice";
import { AiChatError } from "./ai-chat/AiChatErrorNotice";
import { AiRetryBanner } from "./ai-chat/AiRetryBanner";

type AiChatPanelProps = {
  embedded?: boolean;
  presentation?: "panel" | "agent";
  showCloseButton?: boolean;
};

// How long a transient restore/status notice stays before auto-dismissing,
// shown as a live countdown ring so the banner never lingers forever.
const RESTORE_NOTICE_SECONDS = 10;

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
  // Terminal state is NOT subscribed here: `terminalOutputBuffers` changes on
  // every PTY chunk, which would re-render this ~1800-line panel on every line of
  // build/install output even while the chat is idle. It is only ever read when
  // assembling a turn, so the send/restore callbacks read it lazily from
  // `useLuxStore.getState()` at call time instead.
  const workspace = useLuxStore((state) => state.workspace);
  const fileEntries = useLuxStore((state) => state.fileEntries);
  const openSettingsSection = useLuxStore((state) => state.openSettingsSection);
  const setAiChatSessionContextBudgetReport = useLuxStore((state) => state.setAiChatSessionContextBudgetReport);
  const setActiveAiChatSession = useLuxStore((state) => state.setActiveAiChatSession);
  const { locale, t } = useTranslation();
  const [message, setMessage] = useState("");
  const [projectSlashCommands, setProjectSlashCommands] = useState<ProjectSlashCommand[]>([]);
  const {
    attachments,
    setAttachments,
    pinnedEditorPaths,
    attachFiles,
    attachWorkspacePath,
    attachMention,
    attachSelection,
    attachEditorDocument,
    removeAttachment,
  } = useAiChatComposerAttachments({ sessionId: activeAiChatSessionId, openDocuments });
  const [contextOpen, setContextOpen] = useState(false);
  const [draggingFiles, setDraggingFiles] = useState(false);
  const [sendError, setSendError] = useState<AiChatErrorPresentation | null>(null);
  const [lastUserDraft, setLastUserDraft] = useState<string | null>(null);
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
  // Seconds left before the transient restore/status notice auto-dismisses.
  // null when no notice is showing. Reset to RESTORE_NOTICE_SECONDS whenever
  // a new notice appears (the effect keyed on `restoreNotice` drives it).
  const [restoreNoticeSeconds, setRestoreNoticeSeconds] = useState<number | null>(null);
  const slashMenuRef = useRef<HTMLDivElement | null>(null);
  const mentionMenuRef = useRef<HTMLDivElement | null>(null);

  const fileInputRef = useRef<HTMLInputElement | null>(null);
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
  // Show the Stop button only when the active session is busy, not when any
  // background session is running. Background activity is surfaced via the
  // cross-session banner so Stop doesn't silently cancel the wrong session.
  const showStopGeneration = sendingSessionId === activeAiChatSessionId || isAiChatSessionBusyStatus(activeStatus);
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
  // Composite picker value separator. Newline never appears in a provider/model id,
  // so `providerId\nmodelId` round-trips unambiguously (split on the first newline).
  const MODEL_VALUE_SEP = "\n";
  // Unified model picker: every provider's models in one grouped, searchable list.
  // The value is a composite providerId + modelId so selecting a model from another
  // provider switches both at once. Junk models the user hid are filtered out (except
  // the active one, which stays so the picker always reflects the real selection).
  const selectedModelValue = `${aiPreferences.selectedProviderId}${MODEL_VALUE_SEP}${aiPreferences.selectedModelId}`;
  const hiddenModelSet = useMemo(() => new Set(aiPreferences.hiddenModelIds), [aiPreferences.hiddenModelIds]);
  const modelOptions = useMemo(() => {
    const options: { label: string; value: string; group?: string }[] = [];
    const multiProvider = aiPreferences.providers.length > 1;
    for (const provider of aiPreferences.providers) {
      for (const model of provider.models) {
        const value = `${provider.id}${MODEL_VALUE_SEP}${model.id}`;
        if (hiddenModelSet.has(value) && value !== selectedModelValue) continue;
        options.push({ label: model.name, value, group: multiProvider ? provider.name : undefined });
      }
    }
    return options;
  }, [aiPreferences.providers, hiddenModelSet, selectedModelValue]);
  const effortOptions = selectedModel?.effortLevels.map((effort) => ({ label: effort.label, value: effort.id })) ?? [];
  const slashCommands = useMemo(
    () => filterSlashCommands(message, t, projectSlashCommands),
    [message, projectSlashCommands, t],
  );
  // The context-usage memo is recomputed on every streaming token with R1's
  // coalescing (~30-60/sec), so we derive a cheap signature to skip it when only
  // the streaming message's content grew (which doesn't change token-line
  // proportions materially). Two writes to the same split produce the identical
  // signature — and the full pass still runs on more significant history shape
  // changes (like a fresh message or tool call).
  const contextUsageKey = messages.length
    + ":" + (messages[messages.length - 1]?.id ?? "")
    + ":" + (messages[messages.length - 1]?.segments?.length ?? 0)
    + ":" + (messages[messages.length - 1]?.toolCalls?.length ?? 0);
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
  // Depend on `message` (not `message.length`) so replacing the composer text with
  // different content of the same length still recomputes the context budget.
}), [aiIndex.status, aiPreferences, attachments, contextUsageKey, message, pinnedEditorPaths, runtimeInstructionText, selectedAgent, selectedModel, t, projectInstructions]);

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
  // Blocking interaction cards live in separate stores, so the scroll hook can't
  // see them via `messages`; this key force-reveals a card the moment it appears.
  const { scrollRef, showScrollDown, scrollToBottom, handleBodyScroll, pinToBottom } = useAiChatScroll({
    messages,
    activeSessionId: activeAiChatSessionId,
    revealKey: pendingQuestion?.requestId ?? (pendingPlan ? `plan:${pendingPlan.planId}` : null),
  });
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

  const updateProvider = useCallback((selectedProviderId: string) => {
    const provider = getAiProvider(aiPreferences.providers, selectedProviderId) ?? null;
    const nextModelId = provider?.models.some((model) => model.id === aiPreferences.selectedModelId)
      ? aiPreferences.selectedModelId
      : provider?.models[0]?.id;
    updateAiPreference(nextModelId ? { selectedProviderId, selectedModelId: nextModelId } : { selectedProviderId });
  }, [aiPreferences.providers, aiPreferences.selectedModelId, updateAiPreference]);

  // Composite model selection: "providerId\nmodelId" → switch provider + model atomically.
  const selectComposedModel = useCallback((composite: string) => {
    const sep = composite.indexOf("\n");
    if (sep < 0) { updateAiPreference({ selectedModelId: composite }); return; }
    const selectedProviderId = composite.slice(0, sep);
    const selectedModelId = composite.slice(sep + 1);
    updateAiPreference({ selectedProviderId, selectedModelId });
  }, [updateAiPreference]);

  const hideComposedModel = useCallback((composite: string) => {
    const current = useLuxStore.getState().aiPreferences;
    const activeValue = `${current.selectedProviderId}\n${current.selectedModelId}`;
    if (composite === activeValue || current.hiddenModelIds.includes(composite)) return;
    updateAiPreference({ hiddenModelIds: [...current.hiddenModelIds, composite] });
  }, [updateAiPreference]);

  const showHiddenModels = useCallback(() => {
    updateAiPreference({ hiddenModelIds: [] });
  }, [updateAiPreference]);

  const resizeComposerTextarea = useCallback((target?: HTMLTextAreaElement | null) => {
    const textarea = target ?? textareaRef.current;
    if (!textarea) return;
    const maxHeight = 160;
    textarea.style.height = "auto";
    const nextHeight = Math.min(maxHeight, Math.max(34, textarea.scrollHeight));
    textarea.style.height = `${nextHeight}px`;
    textarea.style.overflowY = textarea.scrollHeight > maxHeight ? "auto" : "hidden";
  }, []);

  useLayoutEffect(() => {
    resizeComposerTextarea();
  }, [message]);

  const resetComposerUi = useCallback(() => {
    setContextOpen(false);
    setDraggingFiles(false);
  }, []);
  // Deterministically hydrate the composer from the active session's own persisted
  // draft/attachments on every session switch (see useComposerSessionDraft) so a
  // previous session's unsaved prompt never leaks into a different AI session.
  useComposerSessionDraft({
    sessionId: activeAiChatSessionId,
    setMessage,
    setAttachments,
    resetComposerUi,
    resizeComposerTextarea,
  });

  useEffect(() => {
    setSendError(null);
    setRestoreNotice(null);
  }, [activeAiChatSessionId]);

  // Auto-dismiss the transient notice with a visible per-second countdown.
  // Re-runs whenever a new notice appears (resetting to RESTORE_NOTICE_SECONDS);
  // tears down its timers on change/unmount so no leak or stale tick remains.
  useEffect(() => {
    if (!restoreNotice) {
      setRestoreNoticeSeconds(null);
      return;
    }
    setRestoreNoticeSeconds(RESTORE_NOTICE_SECONDS);
    const interval = setInterval(() => {
      setRestoreNoticeSeconds((prev) => (prev === null ? null : Math.max(0, prev - 1)));
    }, 1000);
    const timeout = setTimeout(() => setRestoreNotice(null), RESTORE_NOTICE_SECONDS * 1000);
    return () => {
      clearInterval(interval);
      clearTimeout(timeout);
    };
  }, [restoreNotice]);

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

  const handleComposerDragOver = (event: DragEvent<HTMLDivElement>) => {
    const hasFiles = event.dataTransfer.types.includes("Files");
    const hasEditorTab = dragEventHasEditorTab(event.dataTransfer);
    const hasWorkspacePath = event.dataTransfer.types.includes("application/x-lux-path");
    if (!hasFiles && !hasEditorTab && !hasWorkspacePath) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    setDraggingFiles(true);
  };

  const handleComposerDrop = (event: DragEvent<HTMLDivElement>) => {
    const editorTabId = readEditorTabDrop(event.dataTransfer);
    const hasFiles = event.dataTransfer.types.includes("Files");
    const workspacePath = event.dataTransfer.types.includes("application/x-lux-path")
      ? event.dataTransfer.getData("application/x-lux-path")
      : "";
    const workspaceKind = event.dataTransfer.types.includes("application/x-lux-kind")
      ? event.dataTransfer.getData("application/x-lux-kind")
      : "";
    if (!editorTabId && !hasFiles && !workspacePath) return;
    event.preventDefault();
    setDraggingFiles(false);
    if (editorTabId) attachEditorDocument(editorTabId);
    if (hasFiles) attachFiles(event.dataTransfer.files);
    if (workspacePath && !hasFiles) void attachWorkspacePath(workspacePath, workspaceKind === "directory" ? "folder" : "file");
  };

  const handleComposerPaste = (event: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = collectClipboardFiles(event.clipboardData);
    if (files.length === 0) return;
    event.preventDefault();
    attachFiles(files);
  };

  const handleCancelSend = useCallback(() => {
    // Cancel only the active session. A background session that is currently
    // running should be cancelled via the cross-session banner, not from
    // this panel's composer stop button (which belongs to the active session).
    const sessionId = (sendingSessionId === activeAiChatSessionId ? sendingSessionId : null)
      ?? (isAiChatSessionBusyStatus(activeStatus) ? activeAiChatSessionId : null);
    if (!sessionId) return;
    const pendingContinuation = goalContinuationTimersRef.current.get(sessionId);
    if (pendingContinuation !== undefined) {
      clearTimeout(pendingContinuation);
      goalContinuationTimersRef.current.delete(sessionId);
    }
    abortAiChatTurn(sessionId);
    resetAutomaticRetry(sessionId);
    clearAiRetryNotice(sessionId);
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
      /** Force a turn checkpoint even on an override send (edit-resend): without it the
       *  re-sent user message has no checkpoint and stops being editable. */
      checkpoint?: boolean;
      /** Send this text to the MODEL instead of the displayed message — lets a staged
       *  "recommendation" carry its fold-in framing to the model while the chat bubble
       *  shows only the clean text the user typed. */
      modelMessageOverride?: string;
      /** Queue flush / send-now: this turn's text comes from the queue, not the live
       *  composer, so leave the user's in-progress draft + attachments untouched. */
      keepComposerDraft?: boolean;
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
          preferences: {
            goalRunMaxTokens: runPreferences.goalRunMaxTokens,
            goalRunMaxRounds: runPreferences.goalRunMaxRounds,
            automaticModeHardStopMinutes: runPreferences.automaticModeHardStopMinutes,
          },
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

    // Automatic mode = full autonomy: a real task should keep working across turns
    // until the completion check passes, not stop after a single turn. Start a goal
    // run keyed on the user's message so the existing continuation loop drives it.
    // Skip greetings/small-talk and internal continuation/retry sends (which carry
    // their own orchestration), and require provider/model/workspace to be ready.
    if (
      !isInternalSend
      && !options?.goalOrchestration
      && !options?.goalContinuation
      && nextMessage
      && !getActiveGoalRun(sessionId)
      && !isAutomaticSocialOnlyMessage(nextMessage)
      && selectedProvider
      && selectedModel
      && workspace
      && useLuxStore.getState().aiPreferences.agentMode === "automatic"
    ) {
      const runPrefs = useLuxStore.getState().aiPreferences;
      startGoalRun(sessionId, nextMessage, {
        agentMode: runPrefs.agentMode,
        toolRoundLimit: runPrefs.toolRoundLimit,
        preferences: {
          goalRunMaxTokens: runPrefs.goalRunMaxTokens,
          goalRunMaxRounds: runPrefs.goalRunMaxRounds,
          automaticModeHardStopMinutes: runPrefs.automaticModeHardStopMinutes,
        },
      });
    }

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
    const modelMessage = options?.modelMessageOverride ?? nextMessage;
    const runtimePreferences = useLuxStore.getState().aiPreferences;
    // Read terminal context live at send time (it is intentionally not subscribed
    // at the top of the component — see the note by the other store selectors).
    const { terminal, terminalOutputBuffers, terminalSessions, activeTerminalId } = useLuxStore.getState();
    const currentAttachments = overrideMessage && !options?.useComposerAttachments ? [] : attachments;
    const messageAttachments = isGoalOrchestration ? [] : await buildMessageDisplayAttachments(currentAttachments);
    const userMessageId = crypto.randomUUID();
    let turnCheckpointId: string | undefined;
    let turnFileCheckpointId: string | undefined;
    if (workspace && selectedProvider && selectedModel && (!overrideMessage || options?.checkpoint) && !isGoalOrchestration) {
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
      pinToBottom();
      // Review requests and queue flushes don't originate from the live composer,
      // so leave the user's in-progress draft and pending attachments intact
      // instead of clearing them (a queued message auto-sending must not wipe text
      // the user is typing right now).
      if (!options?.reviewRequest && !options?.keepComposerDraft) {
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
    // Captured in catch so the finally block can decide whether Automatic mode
    // should retry (and with what reason) instead of stopping the run.
    let lastTurnError: AiChatErrorPresentation | null = null;
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
        onUserMessageInjected: (text) => {
          if (!isActiveTurn()) return;
          // The user's mid-work message was folded into the running turn (Rust
          // appended it between rounds). Render it as a user bubble in order so the
          // transcript shows the steer the next answer responds to.
          appendAiChatMessage(sessionId, {
            id: crypto.randomUUID(),
            role: "user",
            content: text,
            timestamp: Date.now(),
          });
          // Now that Rust confirmed the fold-in, retire the matching staged chip and
          // clear its in-flight tracking (the recommendation has landed in-thread).
          const pending = injectedTextBySessionRef.current.get(sessionId);
          if (pending) {
            const at = pending.indexOf(text);
            if (at >= 0) pending.splice(at, 1);
          }
          const match = getQueuedMessagesForSession(sessionId).find(
            (entry) => entry.mode === "recommendation" && entry.text === text,
          );
          if (match) {
            removeQueuedMessage(match.id);
            injectingRef.current.delete(match.id);
          }
        },
        onRetryNotice: (notice) => {
          if (!isActiveTurn()) return;
          if (notice) setAiRetryNotice(sessionId, notice);
          else clearAiRetryNotice(sessionId);
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
        // Guard with isActiveTurn() so a stale aborted promise cannot overwrite
        // a newer running turn's "idle" status or trim its assistant shell.
        if (isActiveTurn()) {
          setAiChatSessionStatus(sessionId, "idle");
          trimCancelledAssistantShell(sessionId, replaceAiChatMessages);
        }
        return;
      }
      if (!isActiveTurn()) return;
      const errorPresentation = classifyAiChatError(error, t);
      lastTurnError = errorPresentation;
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
      clearAiRetryNotice(sessionId);
      requestAnimationFrame(() => resizeComposerTextarea());
      const pendingContinuation = goalContinuationTimersRef.current.get(sessionId);
      if (pendingContinuation !== undefined) {
        clearTimeout(pendingContinuation);
        goalContinuationTimersRef.current.delete(sessionId);
      }
      if (abortController.signal.aborted) {
        // User pressed Stop — the only thing that ends Automatic's retry loop.
        resetAutomaticRetry(sessionId);
        stopGoalRun(sessionId);
      } else {
        const sessionAfterTurn = useLuxStore.getState().aiChatSessions.find((entry) => entry.id === sessionId);
        const turnFailed = sessionAfterTurn?.status === "error";
        const agentMode = useLuxStore.getState().aiPreferences.agentMode;
        const automatic = agentMode === "automatic";
        // Retry the failed turn with an escalating 3/6/9… ladder. Automatic mode never
        // gives up. Manual/plan modes auto-recover transient transport failures (provider
        // down, stream interrupted, timeout, rate-limit) for up to the retry budget, then
        // surface the error so the user can intervene — this is the "works in all modes"
        // ladder, not an automatic-only behavior.
        const transientRetryable = Boolean(lastTurnError && isTransientRetryKind(lastTurnError.kind));
        const retryPlan = (turnFailed && lastTurnError && (automatic || transientRetryable))
          ? nextAutomaticRetry(sessionId)
          : null;
        const willRetry = retryPlan !== null && (automatic || !retryPlan.exhausted);
        if (willRetry && retryPlan && lastTurnError) {
          // Keep retrying the failed turn with escalating backoff. The goal run is NOT
          // stopped. Keep the session busy during the backoff so Stop stays available,
          // and drop the scary error bubble while a recovery is in flight.
          const { attempt, maxAttempts, delayMs } = retryPlan;
          const errored = useLuxStore.getState().aiChatSessions.find((entry) => entry.id === sessionId);
          if (errored) {
            const trimmed = stripTrailingErrorBubble(errored.messages, errored.lastError);
            if (trimmed !== errored.messages) replaceAiChatMessages(sessionId, trimmed);
          }
          setAiChatSessionStatus(sessionId, "thinking");
          if (useLuxStore.getState().activeAiChatSessionId === sessionId) setSendError(null);
          setAiRetryNotice(sessionId, {
            attempt,
            maxAttempts,
            reason: automaticRetryReason(lastTurnError.kind),
            detail: lastTurnError.detail,
            delayMs,
          });
          const retrySessionId = sessionId;
          const handle = window.setTimeout(() => {
            if (goalContinuationTimersRef.current.get(retrySessionId) !== handle) return;
            goalContinuationTimersRef.current.delete(retrySessionId);
            // Skip if another turn is already running or the session went away. A mode
            // switch mid-backoff no longer cancels the retry — the ladder is bounded in
            // manual/plan and the task should still reach a valid result.
            if (getAiChatTurnRuntimeSnapshot().sendingSessionId === retrySessionId) return;
            const live = useLuxStore.getState().aiChatSessions.find((entry) => entry.id === retrySessionId);
            if (!live || live.closedAt) return;
            if (getActiveGoalRun(retrySessionId)) {
              void handleSendRef.current(
                buildGoalContinuationDirective(retrySessionId),
                undefined,
                { skipGoalSlash: true, goalOrchestration: "continuation", sessionId: retrySessionId, internalSend: true, force: true },
              );
              return;
            }
            // No goal run (e.g. a social turn): resume the failed turn directly,
            // mirroring the manual retry — continue from preserved work, else replay.
            const tail = live.messages[live.messages.length - 1];
            if (tail?.role === "assistant" && messageHasAssistantWork(tail)) {
              void handleSendRef.current(t("aiChat.retry.continue"), live.messages, { force: true, internalSend: true, sessionId: retrySessionId });
              return;
            }
            const lastUserIndex = findLastUserMessageIndex(live.messages);
            if (lastUserIndex >= 0) {
              const draft = live.messages[lastUserIndex].content ?? "";
              if (!draft.trim()) return;
              const nextHistory = live.messages.slice(0, lastUserIndex);
              replaceAiChatMessages(retrySessionId, nextHistory);
              void handleSendRef.current(draft, nextHistory, { force: true, internalSend: true, sessionId: retrySessionId });
            }
          }, delayMs);
          goalContinuationTimersRef.current.set(retrySessionId, handle);
        } else if (turnFailed) {
          // Non-retryable error, or the manual/plan retry budget is exhausted: the error
          // bubble is already shown; clear the backoff streak and stop any goal run.
          resetAutomaticRetry(sessionId);
          stopGoalRun(sessionId);
        } else {
          resetAutomaticRetry(sessionId);
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
              // Seamless mid-work injection: if the user staged a message while the
              // agent was running, send it at THIS inter-turn gap instead of the
              // silent goal-continuation directive — so a recommendation lands right
              // after the current request, not after the whole run. The goal run is
              // not stopped: when this injected turn finishes, the finally block
              // re-evaluates and schedules the next continuation as usual.
              const pendingStaged = dequeueFirstForSession(continuationSessionId);
              if (pendingStaged) {
                void handleSendRef.current(
                  pendingStaged.text,
                  undefined,
                  {
                    force: true,
                    sessionId: continuationSessionId,
                    modelMessageOverride: buildQueuedMessagePayload(pendingStaged),
                  },
                );
                return;
              }
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
  }, [activeChatSession?.id, activeDocument, activeSessionBusy, activeSessionClosed, aiPreferences, appendAiChatMessage, attachments, createAiChatSession, locale, message, openDocuments, projectInstructions, replaceAiChatMessages, requestToolApproval, resizeComposerTextarea, runCompaction, selectedAgent, selectedModel, selectedProvider, setAiChatSessionContextBudgetReport, setAiChatSessionStatus, t, updateAiChatMessage, updateMessage, workspace]);

  // Keep the freshest handleSend instance reachable from the deferred goal-continuation
  // timer so a delayed continuation turn picks up current provider/model/openDocuments/
  // terminal context instead of the closure frozen when the timer was scheduled.
  const handleSendRef = useRef(handleSend);
  handleSendRef.current = handleSend;

  const buildRestoreInput = useCallback(() => {
    if (!workspace || !selectedProvider || !selectedModel) return null;
    const { terminal, terminalOutputBuffers, terminalSessions } = useLuxStore.getState();
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
  }, [activeDocument, aiPreferences, locale, openDocuments, selectedModel, selectedProvider, workspace]);

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
      // Re-checkpoint the edited turn so the new user message stays editable; without
      // this the resend (an override send) skips the checkpoint and the Edit affordance
      // disappears after the first edit.
      await handleSend(nextContent.trim(), result.messages, { force: true, checkpoint: true });
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
    if (activeSessionBusy || activeSessionClosed) return;
    if (activeChatSession) {
      // Strip only the failed turn's error bubble; the AI's reasoning and tool
      // calls stay in the transcript.
      const trimmed = stripTrailingErrorBubble(activeChatSession.messages, activeChatSession.lastError);
      const tail = trimmed[trimmed.length - 1];
      if (tail?.role === "assistant" && messageHasAssistantWork(tail)) {
        // The model produced real work before failing: keep it and resume with a
        // "continue" turn instead of wiping the thinking/actions and replaying.
        if (trimmed !== activeChatSession.messages) {
          replaceAiChatMessages(activeChatSession.id, trimmed);
        }
        void handleSend(t("aiChat.retry.continue"), trimmed, { force: true });
        return;
      }
      // Nothing was produced (e.g. the request failed before the first token):
      // replay the original prompt fresh, the same as before.
      const lastUserIndex = findLastUserMessageIndex(trimmed);
      if (lastUserIndex >= 0) {
        const draft = lastUserDraft ?? trimmed[lastUserIndex].content ?? "";
        if (!draft.trim()) return;
        const nextHistory = trimmed.slice(0, lastUserIndex);
        replaceAiChatMessages(activeChatSession.id, nextHistory);
        void handleSend(draft, nextHistory);
        return;
      }
    }
    const draft = lastUserDraft ?? [...messages].reverse().find((entry) => entry.role === "user")?.content ?? "";
    if (draft.trim()) void handleSend(draft);
  }, [activeChatSession, activeSessionBusy, activeSessionClosed, handleSend, lastUserDraft, messages, replaceAiChatMessages, t]);

  const handleReviewAction = useCallback((messageId: string) => {
    if (!activeChatSession || activeSessionBusy || activeSessionClosed) return;
    // Build a model-side prompt that scopes the review to the exact turn the user
    // clicked. The displayed badge comes from reviewRequest:true, so the raw prompt
    // text is never rendered in the chat — only the scoped model message is sent.
    const basePrompt = t("aiChat.review.prompt");
    const scopedPrompt = `${basePrompt}\n\n<!-- review-target-message-id: ${messageId} -->`;
    void handleSend(basePrompt, undefined, { reviewRequest: true, force: true, modelMessageOverride: scopedPrompt });
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
    // Automatic owns the whole run — keep it in Automatic on hand-off. Only a
    // manual Start from a read-only/plan mode switches into Agent for execution.
    if (aiPreferences.agentMode !== "automatic") {
      const agentProfile = aiPreferences.agentProfiles.find((profile) => profile.mode === "agent")
        ?? aiPreferences.agentProfiles[0];
      if (agentProfile) updateAiPreference({ selectedAgentId: agentProfile.id, agentMode: "agent" });
    }
    const handoffMessage = buildPlanHandoffUserMessage(pendingPlan.steps.map((step) => step.title));
    void handleSend(handoffMessage, undefined, { force: true });
    void sessionId;
  }, [pendingPlan, activeSessionBusy, activeSessionClosed, aiPreferences.agentMode, aiPreferences.agentProfiles, updateAiPreference, handleSend]);

  // Automatic safety net: a plan must never strand execution in Automatic mode. If
  // the model presented a plan but the turn ended without proceeding (idle, plan
  // still pending), auto-hand it to execution. The busy guard prevents double-firing
  // while the model is already continuing in-turn; clearing the plan stops re-entry.
  useEffect(() => {
    if (aiPreferences.agentMode !== "automatic") return;
    if (!pendingPlan || activeSessionBusy || activeSessionClosed) return;
    handlePlanStart();
  }, [aiPreferences.agentMode, pendingPlan, activeSessionBusy, activeSessionClosed, handlePlanStart]);

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
    // While the agent is busy, Enter stages the message into the per-session queue;
    // it drains automatically when the current turn finishes (see the drain effect).
    // Ctrl/Cmd+Enter also queues — both run verbatim as the next turn.
    // Don't clear the composer input after enqueue — the user may want to type
    // a second message immediately while the first is still queued (input independence).
    if (activeSessionBusy && activeAiChatSessionId && message.trim()) {
      enqueueChatMessage(activeAiChatSessionId, message, "queued");
      return;
    }
    void handleSend();
  }, [activeSessionBusy, activeAiChatSessionId, message, updateMessage, handleMentionSelect, handleSend, handleSlashSelect, mentionActiveIndex, mentionCandidates, mentionMenuOpen, mentionNavigated, slashActiveIndex, slashCommands, slashMenuOpen]);

  // Mid-work injection bookkeeping (shared by the drain effect below and the inject
  // effect further down). `injectingRef` = recommendation ids handed to the Rust turn
  // loop but not yet confirmed back via the userMessageInjected event; the chip stays
  // visible until confirmation, so a recommendation can never vanish on an optimistic
  // delete nor be double-delivered by the end-of-turn drain. `injectedTextBySessionRef`
  // tracks the in-flight texts per session for matching the confirmation event.
  const injectingRef = useRef<Set<string>>(new Set());
  const injectedTextBySessionRef = useRef<Map<string, string[]>>(new Map());

  // Drain each session's queue when THAT session's turn finishes — tracked per
  // session id, not just the active one, so a queued message never strands when the
  // user switches away from a busy chat (and switching tabs never falsely drains).
  // One message per turn-completion edge: each queued item runs as its own follow-up
  // turn (verbatim for "queued", wrapped for "recommendation"); the next drains when
  // that turn ends, so a backlog processes sequentially in order.
  const prevSessionBusyRef = useRef<Record<string, boolean>>({});
  useEffect(() => {
    const prev = prevSessionBusyRef.current;
    const next: Record<string, boolean> = {};
    for (const session of aiChatSessions) {
      const busy = sendingSessionId === session.id || isAiChatSessionBusyStatus(session.status);
      next[session.id] = busy;
      if (prev[session.id] && !busy && !session.closedAt) {
        // The turn ended: any recommendation still marked in-flight never got
        // confirmed (its inject raced the turn close), so release it back to a plain
        // queued follow-up instead of losing it — then drain one entry as the next turn.
        injectedTextBySessionRef.current.delete(session.id);
        for (const entry of getQueuedMessagesForSession(session.id)) {
          if (entry.mode === "recommendation" && injectingRef.current.has(entry.id)) {
            injectingRef.current.delete(entry.id);
          }
        }
        const queued = dequeueFirstForSession(session.id);
        if (queued) {
          void handleSend(queued.text, undefined, {
            force: true,
            sessionId: session.id,
            modelMessageOverride: buildQueuedMessagePayload(queued),
            keepComposerDraft: true,
          });
        }
      }
    }
    prevSessionBusyRef.current = next;
  }, [aiChatSessions, sendingSessionId, handleSend]);

  // Seamless mid-work injection: a "recommendation" staged while a turn is RUNNING is
  // pushed straight into the live Rust turn loop (ai_inject_message), which folds it in
  // as a user message at the next round boundary — so it lands during the work, in the
  // gap between the agent's requests, not after the whole turn ends. "Queued" entries
  // are left for the end-of-turn drain (they run verbatim as their own next turn).
  const allQueuedMessages = useAllQueuedMessages();
  useEffect(() => {
    if (!isTauriRuntime()) return;
    for (const entry of allQueuedMessages) {
      if (entry.mode !== "recommendation") continue;
      if (injectingRef.current.has(entry.id)) continue;
      const session = aiChatSessions.find((candidate) => candidate.id === entry.sessionId);
      if (!session || session.closedAt) continue;
      const running = sendingSessionId === entry.sessionId || isAiChatSessionBusyStatus(session.status);
      if (!running) continue;
      // ai_inject_message is scoped by session+turn, so we need the live turn_id.
      // If it isn't published yet (the native turn hasn't launched), leave the chip
      // as a "recommendation" and retry on the next tick instead of mis-routing it.
      const turnId = getActiveTurnId(entry.sessionId);
      if (!turnId) continue;
      // Mark in-flight BEFORE the await so a re-render can't double-inject. The chip
      // is NOT removed here — it is removed only when Rust confirms the fold-in
      // (onUserMessageInjected), so a turn that ends before the drain never loses it.
      injectingRef.current.add(entry.id);
      const pending = injectedTextBySessionRef.current.get(entry.sessionId) ?? [];
      pending.push(entry.text);
      injectedTextBySessionRef.current.set(entry.sessionId, pending);
      void luxCommands.aiInjectMessage(entry.sessionId, turnId, entry.text)
        .catch(() => {
          // Inject call itself failed — un-track so the end-of-turn drain re-sends it
          // as a follow-up turn (flip to "queued") instead of stranding it.
          injectingRef.current.delete(entry.id);
          const list = injectedTextBySessionRef.current.get(entry.sessionId);
          if (list) {
            const at = list.indexOf(entry.text);
            if (at >= 0) list.splice(at, 1);
          }
          updateQueuedMessage(entry.id, { mode: "queued" });
        });
    }
  }, [allQueuedMessages, aiChatSessions, sendingSessionId]);

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

  const handleQueuedSendNow = useCallback((entry: QueuedMessage) => {
    removeQueuedMessage(entry.id);
    void handleSend(entry.text, undefined, {
      force: true,
      modelMessageOverride: buildQueuedMessagePayload(entry),
      keepComposerDraft: true,
    });
  }, [handleSend]);

  const renderComposerContent = () => (
    <>
      <AiChatQueuedMessages sessionId={activeAiChatSessionId} onSendNow={handleQueuedSendNow} t={t} />
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
      modelSearchPlaceholder={t("aiChat.model.searchPlaceholder")}
      modelSearchEmptyHint={t("aiChat.model.searchEmpty")}
      onHideModel={hideComposedModel}
      hideModelLabel={t("aiChat.model.hide")}
      modelFooter={aiPreferences.hiddenModelIds.length > 0 ? (
        <button type="button" className="compact-dropdown-footer-action" onClick={showHiddenModels}>
          {t("aiChat.model.showHidden", { count: aiPreferences.hiddenModelIds.length })}
        </button>
      ) : undefined}
      providerOptions={providerOptions}
      selectedProviderId={selectedProvider?.id ?? aiPreferences.selectedProviderId}
      preferences={aiPreferences}
      removeAttachment={removeAttachment}
      selectedModelId={selectedModelValue}
      selectedProviderReady={Boolean(selectedProvider && selectedModel)}
      setContextOpen={setContextOpen}
      setDraggingFiles={setDraggingFiles}
      t={t}
      textareaRef={textareaRef}
      updateAiPreference={updateAiPreference}
      updateModel={selectComposedModel}
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
    </>
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
                  agentMode={aiPreferences.agentMode}
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
              {restoreNotice && (
                <div className="ai-chat-restore-notice" role="status">
                  <span className="ai-chat-restore-notice__text">{restoreNotice}</span>
                  <button
                    type="button"
                    className="ai-chat-restore-notice__dismiss"
                    onClick={() => setRestoreNotice(null)}
                    aria-label={t("aiChat.turnCheckpoint.dismiss")}
                    title={t("aiChat.turnCheckpoint.dismiss")}
                    style={{
                      "--notice-pct": `${((restoreNoticeSeconds ?? RESTORE_NOTICE_SECONDS) / RESTORE_NOTICE_SECONDS) * 100}%`,
                    } as CSSProperties}
                  >
                    <span className="ai-chat-restore-notice__count" aria-hidden="true">
                      {restoreNoticeSeconds ?? RESTORE_NOTICE_SECONDS}
                    </span>
                    <span className="ai-chat-restore-notice__x" aria-hidden="true">×</span>
                  </button>
                </div>
              )}
              {activeChatSession && <AiRetryBanner sessionId={activeChatSession.id} t={t} />}
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




