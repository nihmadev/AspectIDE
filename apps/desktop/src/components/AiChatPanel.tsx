import { ArrowDown, ArrowUpRight, Brain, Bug, Code2, FlaskConical, Globe, PanelRightClose, Plus, Sparkles, X } from "lucide-react";
import type { ChangeEvent, ClipboardEvent, CSSProperties, DragEvent, KeyboardEvent } from "react";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { mapComposerAttachments } from "./ai-chat/AiComposerAttachments";
import { AiChatComposer } from "./ai-chat/AiChatComposer";
import { AiChatHistoryPopover } from "./ai-chat/AiChatHistoryPopover";
import { AiChatMessages, MarkdownSmoothStreamContext } from "./ai-chat/AiChatMessages";

import { AiAgentOrchestrationRail } from "./ai-chat/AiAgentOrchestrationRail";
import { AiQuestionCard } from "./ai-chat/AiQuestionCard";
import { AiPlanCard } from "./ai-chat/AiPlanCard";
import { AiPlanRunCard, type ActivePlanRun } from "./ai-chat/AiPlanRunCard";
import { AiSessionReviewBar } from "./ai-chat/AiSessionReviewBar";
import { AiSubagentPanel } from "./ai-chat/AiSubagentPanel";
import { AiAutomaticChecklist } from "./ai-chat/AiAutomaticChecklist";
import { buildContextDropSummary } from "../lib/aiChatContextReport";
import { aiChatErrorFromMessage, classifyAiChatError, type AiChatErrorPresentation } from "../lib/aiChatErrors";
import { clearAiRetryNotice, getAiRetryNotice, setAiRetryNotice } from "../lib/aiRetryNotice";
import { automaticRetryReason, getAutomaticRetryAttempts, isTransientRetryKind, nextAutomaticRetry, resetAutomaticRetry } from "../lib/aiAutomaticRetry";
import { isAutomaticSocialOnlyMessage } from "../lib/aiAutomaticSocialMessage";

import { AiChatSlashMenu } from "./ai-chat/AiChatSlashMenu";
import { AiChatMentionMenu } from "./ai-chat/AiChatMentionMenu";
import {
  compactChatHistory as runContextCompaction,
  pruneReducedTokenEstimate,
  pruneStaleToolOutputs,
  reconcileCompactedMessages,
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
  resolveEffectiveAutoCompactThreshold,
  type AiPreferences,
} from "../lib/aiPreferences";
import { luxideAvailability, luxideWeeklyBadge, useLuxideModelSync, useLuxideUsagePoller } from "../lib/luxideModelSync";
import { useLuxideUsageStore } from "../lib/luxideUsageStore";
import { isLuxideProvider, relinkLuxide } from "../lib/luxideEnroll";
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
  readClipboardImageFile,
  revokeComposerAttachmentPreviews,
} from "../lib/aiChatComposerAttachments";
import { buildMentionRuntimeAttachments, collectMentionHints } from "../lib/aiChatMentionAttachments";
import { applyMentionSelection, mentionMenuVisible, parseMentionQuery, searchMentionCandidates, type AiMentionCandidate } from "../lib/aiChatMentions";
import { buildPlanHandoffUserMessage, extractPlanHandoffPayload } from "../lib/aiChatPlanHandoff";
import { clearPendingPlan, getPendingPlanForSession, getPendingPlansSnapshot, subscribePendingPlans } from "../lib/aiPendingPlan";
import { getPendingQuestionForSession, getPendingQuestionsSnapshot, settlePendingQuestion, subscribePendingQuestions } from "../lib/aiPendingQuestion";
import { buildQueuedMessagePayload, dequeueFirstForSession, enqueueChatMessage, getQueuedMessagesForSession, removeQueuedMessage, setQueuedMessageInjectedTurn, updateQueuedMessage, useAllQueuedMessages, type QueuedMessage } from "../lib/aiChatQueue";
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
  restoreUnansweredUserMessage,
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
  TURN_SUPERSEDED,
} from "../lib/aiChatTurnRuntime";
import type { AiChatAttachmentInput, AiChatMessage, AiToolApprovalDecision } from "../lib/aiChatTypes";
import { normalizeVisibleReasoning } from "../lib/aiChatReasoning";
import { DEFAULT_UI_FONT_STACK, withFontFallback } from "../lib/editorPreferences";
import { isAiChatSessionBusyStatus, selectActiveAiChatSession, useLuxStore, type AiChatSessionStatus } from "../lib/store";
import { isTauriRuntime, luxCommands } from "../lib/tauri";
import { getActiveTurnId } from "../lib/aiActiveTurns";
import { useVoiceInput } from "../lib/useVoiceInput";
import { useLiveTokenSpeed } from "../lib/useLiveTokenSpeed";
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
import { AiAgentNowPlaque } from "./ai-chat/AiAgentNowPlaque";
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

/** Longest single-line tail shown in the live work ticker. */
const LIVE_WORK_TAIL_CHARS = 160;

/** The freshest snippet of the agent's raw output for the live status ticker:
 *  the tail of the CURRENT turn's assistant message (streamed text, or reasoning
 *  while still thinking). Collapsed to one line and clamped. "" when there is
 *  nothing live to show — a tool-only round, or the gap after the user message is
 *  sent but before this turn's assistant shell exists (stopping at the newest
 *  user message prevents showing the PREVIOUS turn's answer as if it were live). */
function extractLiveWorkTail(messages: AiChatMessage[]): string {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const message = messages[index];
    // Reached the current turn's user message before any assistant output: the
    // response hasn't started, so there is nothing live to show yet.
    if (message.role === "user") return "";
    if (message.role !== "assistant") continue;
    const source = message.content.trim() || normalizeVisibleReasoning(message.reasoning)?.trim() || "";
    if (!source) return "";
    const oneLine = source.replace(/\s+/g, " ").trim();
    return oneLine.length > LIVE_WORK_TAIL_CHARS ? `…${oneLine.slice(-(LIVE_WORK_TAIL_CHARS - 1))}` : oneLine;
  }
  return "";
}

export function AiChatPanel({ embedded = false, presentation = "panel", showCloseButton = true }: AiChatPanelProps) {
  const activeDocumentId = useLuxStore((state) => state.activeDocumentId);
  const aiIndex = useLuxStore((state) => state.aiIndex);
  const aiPreferences = useLuxStore((state) => state.aiPreferences);
  const chatFontFamily = useLuxStore((state) => state.editorPreferences.chatFontFamily);
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
  const clearAiChatErrorHistory = useLuxStore((state) => state.clearAiChatErrorHistory);
  const appendAiChatSessionError = useLuxStore((state) => state.appendAiChatSessionError);
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
  // Keep the bundled LuxIDE provider's model list in sync with the gateway so admin
  // enable/disable toggles (via @LuxIDE_bot) appear/disappear here live.
  useLuxideModelSync();
  // Single poll of this user's per-model usage → shared store (composer plaque + the
  // model-picker weekly badge/status dot both read it).
  useLuxideUsagePoller();
  const luxideUsageMap = useLuxideUsageStore((state) => state.map);
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
  // Optimistic-send stage notice: which pre-dispatch step the current send is in
  // (attachment encoding → pre-turn checkpoint → waiting for the model). Cleared
  // the moment the turn starts reporting its own status.
  const [sendPhase, setSendPhase] = useState<{ sessionId: string; stage: "attachments" | "checkpoint" | "dispatch" } | null>(null);
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
  // Live tok/s of the running turn, shown left of the context stats. null hides it.
  const liveTokenSpeed = useLiveTokenSpeed(
    activeAiChatSessionId,
    aiPreferences.showTokenSpeed,
  );
  // Live "plan run" card shown after the user presses Start on a plan (#35). Set on
  // handoff, cleared on dismiss or session switch. Its steps/progress are driven
  // live by the session's todo store (see AiPlanRunCard), not by this state.
  const [activePlanRun, setActivePlanRun] = useState<ActivePlanRun | null>(null);
  const [restoreNotice, setRestoreNotice] = useState<string | null>(null);
  // Seconds left before the transient restore/status notice auto-dismisses.
  // null when no notice is showing. Reset to RESTORE_NOTICE_SECONDS whenever
  // a new notice appears (the effect keyed on `restoreNotice` drives it).
  const [restoreNoticeSeconds, setRestoreNoticeSeconds] = useState<number | null>(null);
  const slashMenuRef = useRef<HTMLDivElement | null>(null);
  const mentionMenuRef = useRef<HTMLDivElement | null>(null);

  const fileInputRef = useRef<HTMLInputElement | null>(null);
  const goalContinuationTimersRef = useRef<Map<string, ReturnType<typeof setTimeout>>>(new Map());
  // Sessions with a send in its pre-turn async window (checkpoint + auto-compaction
  // run BEFORE `startAiChatTurn` marks the session busy). A second Enter/click during
  // that window would otherwise slip past the busy gate and double-send the message,
  // so we reserve the session synchronously here and fold it into the busy check.
  const pendingSendLockRef = useRef<Set<string>>(new Set());
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const runtimeSnapshot = useSyncExternalStore(subscribeAiChatTurnRuntime, getAiChatTurnRuntimeSnapshot, getAiChatTurnRuntimeSnapshot);
  const messages = activeChatSession?.messages ?? [];
  const visibleMessages = useMemo(() => filterVisibleChatMessages(messages), [messages]);
  const activeStatus = activeChatSession?.status ?? "idle";
  const activeLastError = activeChatSession?.lastError ?? null;
  const activeSessionClosed = Boolean(activeChatSession?.closedAt);
  const sendingSessionId = runtimeSnapshot.sendingSessionId;
  const activeSessionBusy = sendingSessionId === activeAiChatSessionId || isAiChatSessionBusyStatus(activeStatus);
  // Live raw-work tail for the status plaque's single-line ticker (setting-gated,
  // default on). Recomputed on each streaming update — cheap tail slice.
  const liveWorkTail = useMemo(
    () => (aiPreferences.liveWorkTicker && activeSessionBusy ? extractLiveWorkTail(messages) : ""),
    [aiPreferences.liveWorkTicker, activeSessionBusy, messages],
  );
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
    const options: { label: string; value: string; group?: string; badge?: string; status?: "ok" | "blocked" }[] = [];
    const multiProvider = aiPreferences.providers.length > 1;
    for (const provider of aiPreferences.providers) {
      const luxide = isLuxideProvider(provider);
      for (const model of provider.models) {
        const value = `${provider.id}${MODEL_VALUE_SEP}${model.id}`;
        if (hiddenModelSet.has(value) && value !== selectedModelValue) continue;
        // LuxIDE models carry a weekly used/cap badge and a green/red availability dot.
        const usage = luxide ? luxideUsageMap[model.alias] ?? null : null;
        options.push({
          label: model.name,
          value,
          group: multiProvider ? provider.name : undefined,
          badge: luxide ? luxideWeeklyBadge(usage) ?? undefined : undefined,
          status: luxide ? luxideAvailability(usage) ?? undefined : undefined,
        });
      }
    }
    return options;
  }, [aiPreferences.providers, hiddenModelSet, selectedModelValue, luxideUsageMap]);
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
    selectedProvider: selectedProvider ?? null,
    selectedModel: selectedModel ?? null,
    selectedModelAlias: selectedModel?.alias ?? selectedModel?.id ?? "",
    t,
    hasGlobalInstructions: aiPreferences.globalInstructions.trim().length > 0,
    hasProjectInstructions: projectInstructions.trim().length > 0,
  // Depend on `message` (not `message.length`) so replacing the composer text with
  // different content of the same length still recomputes the context budget.
}), [aiIndex.status, aiPreferences, attachments, contextUsageKey, message, pinnedEditorPaths, runtimeInstructionText, selectedAgent, selectedModel, selectedProvider, t, projectInstructions]);

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
    setActivePlanRun(null);
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
    const compactionSessionId = activeChatSession.id;
    const snapshotMessages = activeChatSession.messages;
    setCompacting(true);
    setSendError(null);
    try {
      const result = await runContextCompaction({
        chatSessionId: compactionSessionId,
        messages: snapshotMessages,
        compactionState: activeChatSession.contextCompaction ?? null,
        model: selectedModel,
        provider: selectedProvider,
        selectedEffortId: aiPreferences.selectedEffortId,
        threshold: resolveEffectiveAutoCompactThreshold(aiPreferences.contextAutoCompactThreshold, selectedProvider, selectedModel),
        autoCompactEnabled: aiPreferences.contextAutoCompactEnabled,
        force,
      });
      if (result.compacted) {
        // The summarization ran for seconds; a send may have committed messages
        // in that window. Replacing with the stale snapshot would wipe the new
        // user message AND the in-flight assistant turn (its stream updates
        // then no-op on the missing id). Reconcile instead — and when a turn is
        // live, skip the replace entirely: it was built from the uncompacted
        // history, so the checkpoint buys nothing and only risks divergence.
        const liveMessages = useLuxStore.getState().aiChatSessions
          .find((session) => session.id === compactionSessionId)?.messages ?? [];
        const reconciled = reconcileCompactedMessages(snapshotMessages, result.messages, liveMessages);
        const turnLive = getAiChatTurnRuntimeSnapshot().sendingSessionId === compactionSessionId
          || pendingSendLockRef.current.has(compactionSessionId);
        if (reconciled.divergedDuringCompaction && turnLive) {
          return false;
        }
        replaceAiChatMessages(compactionSessionId, reconciled.messages, { contextCompaction: result.compactionState });
      } else {
        // Persist the cooldown/throttle state even when nothing was compacted: the
        // "no-reduction" path returns a fresh state carrying lastCompactedAt so the
        // expensive summarization isn't re-run on every subsequent over-threshold send.
        // Skip the write when the state is unchanged (other skip reasons return it as-is).
        // Write the LIVE messages, not the pre-await snapshot — a send committed
        // during the summarization must not be wiped by a state-only persist.
        if (result.compactionState && result.compactionState !== (activeChatSession.contextCompaction ?? null)) {
          const liveMessages = useLuxStore.getState().aiChatSessions
            .find((session) => session.id === compactionSessionId)?.messages ?? result.messages;
          replaceAiChatMessages(compactionSessionId, liveMessages, { contextCompaction: result.compactionState });
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
    if (files.length > 0) {
      event.preventDefault();
      attachFiles(files);
      return;
    }
    // Fallback for Linux WebKitGTK, where the paste event carries no image: read
    // the OS clipboard natively. No-op when it holds no image, so pasting text
    // into the composer still works normally.
    void readClipboardImageFile().then((file) => {
      if (file) attachFiles([file]);
    });
  };

  const handleCancelSend = useCallback(() => {
    // Cancel only the active session. A background session that is currently
    // running should be cancelled via the cross-session banner, not from
    // this panel's composer stop button (which belongs to the active session).
    const sessionId = (sendingSessionId === activeAiChatSessionId ? sendingSessionId : null)
      ?? (isAiChatSessionBusyStatus(activeStatus) ? activeAiChatSessionId : null);
    if (!sessionId) return;
    // Snapshot the retry state BEFORE the resets below wipe it: it decides
    // whether the tail user message is a doomed retry orphan worth recovering.
    const retryCycleActive = getAiRetryNotice(sessionId) !== null
      || getAutomaticRetryAttempts(sessionId) > 0;
    const turnInFlight = getAiChatTurnRuntimeSnapshot().sendingSessionId === sessionId;
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
    // Stop pressed during the retry backoff (no attempt in flight): the tail
    // user message will never be answered — hand its text back to the composer
    // instead of leaving it hanging in the transcript forever. Mid-attempt
    // cancels keep today's behavior (the aborted turn's own cleanup still runs
    // asynchronously and would race a transcript rewrite). A non-empty composer
    // draft is never clobbered — in that case the message stays in the chat.
    if (retryCycleActive && !turnInFlight && sessionId === activeAiChatSessionId && !message.trim()) {
      const live = useLuxStore.getState().aiChatSessions.find((entry) => entry.id === sessionId);
      const restored = live ? restoreUnansweredUserMessage(live.messages, live.lastError ?? null) : null;
      if (restored) {
        replaceAiChatMessages(sessionId, restored.messages);
        updateMessage(restored.draft);
        requestAnimationFrame(() => textareaRef.current?.focus());
      }
    }
  }, [activeAiChatSessionId, activeStatus, message, replaceAiChatMessages, sendingSessionId, setAiChatSessionStatus, updateMessage]);

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
      /** Retry of the FAILED turn (manual Retry button): keep the session's
       *  error history accumulating across attempts. Fresh sends clear it so a
       *  new logical turn never inherits a previous failure's ladder. */
      preserveErrorHistory?: boolean;
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
      || isAiChatSessionBusyStatus(targetSession?.status ?? "idle")
      // A send already committed for this session but hasn't reached startAiChatTurn
      // yet (checkpoint/compaction awaits): treat it as busy so a rapid second send
      // can't double-fire during that window.
      || pendingSendLockRef.current.has(sessionId);
    // targetSessionBusy is the ONLY check reading the fresh runtime snapshot and
    // the pending-send lock (render-time activeSessionBusy stays false through the
    // pre-turn awaits) — it must gate the composer path too, or a rapid second
    // Enter double-sends and supersedes the first turn.
    const sendBlocked = options?.sessionId
      ? targetSessionClosed || targetSessionBusy
      : activeSessionClosed || activeSessionBusy || targetSessionBusy;

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
          // A user-initiated /goal resume is a fresh logical action: drop any prior
          // failure's error-history ladder so it can't leak under a later error card.
          // The recursive dispatch below is internalSend, which skips the clear guard.
          clearAiChatErrorHistory(sessionId);
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
        // A fresh /goal kickoff is a new logical task: clear any stale error-history
        // from an earlier unrelated failure (the internalSend dispatch below skips
        // the fresh-send clear guard, so do it explicitly here).
        clearAiChatErrorHistory(sessionId);
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

    // We are committed to sending: reserve the session synchronously (before the
    // checkpoint/compaction awaits below) so a second send bails at the busy gate.
    // Released in the finally after the turn settles.
    pendingSendLockRef.current.add(sessionId);

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
      threshold: resolveEffectiveAutoCompactThreshold(aiPreferences.contextAutoCompactThreshold, selectedProvider, selectedModel),
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
          threshold: resolveEffectiveAutoCompactThreshold(aiPreferences.contextAutoCompactThreshold, selectedProvider, selectedModel),
          autoCompactEnabled: true,
          force: false,
          abortSignal: abortController.signal,
        });
        if (compacted.compacted) {
          // A force-send (queued "Send now", goal continuation) may have started
          // a turn during the summarization await — reconcile with the live
          // messages so its user message and streaming assistant shell survive,
          // and skip the store replace entirely when such a turn is running.
          const liveMessages = useLuxStore.getState().aiChatSessions
            .find((session) => session.id === sessionId)?.messages ?? [];
          const reconciled = reconcileCompactedMessages(workingHistory, compacted.messages, liveMessages);
          if (!(reconciled.divergedDuringCompaction
            && getAiChatTurnRuntimeSnapshot().sendingSessionId === sessionId)) {
            replaceAiChatMessages(sessionId, reconciled.messages, { contextCompaction: compacted.compactionState });
          }
          workingHistory = compacted.messages;
        }
      } catch (compactionError) {
        // A failed auto-compaction must NOT drop the user's message: proceed with the
        // uncompacted history. A user abort during compaction bails cleanly (releasing
        // the send reservation) instead of sending.
        if (isAbortError(compactionError)) {
          pendingSendLockRef.current.delete(sessionId);
          return;
        }
        console.warn("Auto-compaction failed; sending uncompacted:", compactionError);
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
    const userMessageId = crypto.randomUUID();
    const checkpointWillRun = Boolean(
      workspace && selectedProvider && selectedModel && (!overrideMessage || options?.checkpoint) && !isGoalOrchestration,
    );
    // ── Optimistic send: the user's bubble lands in the transcript IMMEDIATELY.
    // Attachment encoding and the pre-turn checkpoint (both potentially slow)
    // run AFTER and patch the message in place, while the session flips to
    // "preparing" at once — no dead silence between Enter and visible feedback.
    // The send-progress notice narrates which stage is running.
    if (!isGoalOrchestration) {
      appendAiChatMessage(sessionId, {
        id: userMessageId,
        role: "user",
        kind: options?.reviewRequest ? "review-request" : undefined,
        content: displayMessage,
        timestamp: Date.now(),
      });
      setAiChatSessionStatus(sessionId, "preparing");
      setSendPhase({
        sessionId,
        stage: currentAttachments.length > 0 ? "attachments" : checkpointWillRun ? "checkpoint" : "dispatch",
      });
      pinToBottom();
    }
    let messageAttachments: Awaited<ReturnType<typeof buildMessageDisplayAttachments>>;
    try {
      messageAttachments = isGoalOrchestration ? [] : await buildMessageDisplayAttachments(currentAttachments);
    } catch (error) {
      // Defense in depth: this await runs before the turn's try/finally, so a
      // throw here must release the pending-send reservation and surface the
      // error — otherwise the send silently vanishes and the session is locked
      // out of every future non-forced send. The optimistic bubble is removed:
      // this message never became a turn (the composer draft is still intact).
      if (!isGoalOrchestration) {
        const live = useLuxStore.getState().aiChatSessions.find((session) => session.id === sessionId);
        if (live) replaceAiChatMessages(sessionId, live.messages.filter((entry) => entry.id !== userMessageId));
        setAiChatSessionStatus(sessionId, "idle");
      }
      setSendPhase((current) => (current?.sessionId === sessionId ? null : current));
      pendingSendLockRef.current.delete(sessionId);
      setSendError(classifyAiChatError(error, t));
      return;
    }
    if (!isGoalOrchestration && messageAttachments.length > 0) {
      updateAiChatMessage(sessionId, userMessageId, { attachments: messageAttachments });
    }
    let turnCheckpointId: string | undefined;
    let turnFileCheckpointId: string | undefined;
    if (checkpointWillRun && workspace && selectedProvider && selectedModel) {
      setSendPhase((current) => (current?.sessionId === sessionId ? { sessionId, stage: "checkpoint" } : current));
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
        // The bubble is already in the transcript (optimistic send) — attach the
        // checkpoint id in place so Edit/Roll back light up as soon as it exists.
        updateAiChatMessage(sessionId, userMessageId, { turnCheckpointId });
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
      // The user bubble was appended optimistically before the attachment and
      // checkpoint awaits — only the title refresh and composer bookkeeping run here.
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
    // A fresh (non-retry) send starts a new logical turn: drop the previous
    // failure's error-history ladder so an unrelated later error never shows a
    // stale "previous attempts" list. Retries (manual Retry button, the
    // auto-retry ladder, goal continuations) keep accumulating.
    if (!options?.preserveErrorHistory && !isInternalSend) {
      clearAiChatErrorHistory(sessionId);
    }
    setSendPhase((current) => (current?.sessionId === sessionId ? { sessionId, stage: "dispatch" } : current));
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
          // First backend status = the request reached the model; the optimistic
          // send-progress notice hands off to the regular thinking/streaming UI.
          setSendPhase((current) => (current?.sessionId === sessionId ? null : current));
          setAiChatSessionStatus(sessionId, statusToSessionStatus(status));
        },
        onUserMessageInjected: (text) => {
          if (!isActiveTurn()) return;
          // The user's mid-work message was folded into the running turn (Rust
          // appended it between rounds). It was usually already rendered
          // optimistically the moment it was staged (see the recommendation drain
          // below) — only append if that optimistic bubble isn't present, so Rust's
          // confirmation never double-renders it.
          const already = useLuxStore.getState().aiChatSessions
            .find((entry) => entry.id === sessionId)?.messages
            .some((entry) => entry.role === "user" && entry.recommendation && entry.content === text);
          if (!already) {
            appendAiChatMessage(sessionId, {
              id: crypto.randomUUID(),
              role: "user",
              content: text,
              recommendation: true,
              timestamp: Date.now(),
            });
          }
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
          if (notice) {
            setAiRetryNotice(sessionId, notice);
            // Feed the backend's in-turn retry into the session's error history so
            // the "Retrying" banner's expandable log fills up live DURING the ladder
            // (the turn-level catch only records the FINAL failure). Record once per
            // attempt number so a re-emit can't inflate the count; the detail carries
            // the real provider error, falling back to the reason label.
            if (notice.attempt !== lastRecordedRetryAttemptRef.current.get(sessionId)) {
              lastRecordedRetryAttemptRef.current.set(sessionId, notice.attempt);
              const reasonLabel = t(`aiChat.retryNotice.reason.${notice.reason}` as "aiChat.retryNotice.reason.generic");
              const detail = notice.detail?.trim();
              appendAiChatSessionError(sessionId, detail ? `${reasonLabel} — ${detail}` : reasonLabel);
            }
          } else {
            clearAiRetryNotice(sessionId);
            lastRecordedRetryAttemptRef.current.delete(sessionId);
          }
        },
        // Tag each approval with THIS turn's session + generation: defaulting to
        // the global sendingSessionId would hand ownership to whichever session
        // sent last, so finishing that session would silently auto-reject this
        // session's pending approval.
        onToolApproval: (request) => requestAiToolApproval(request.id, sessionId, turnGeneration),
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
      // The error card (AiChatError) is the single error surface — it carries the
      // retry/settings actions and the previous-attempts history. Don't duplicate
      // its text as an assistant bubble; just drop an empty streaming shell so no
      // blank assistant message dangles above the card.
      trimCancelledAssistantShell(sessionId, replaceAiChatMessages);
      setAiChatSessionStatus(sessionId, "error", errorPresentation.message);
      if (useLuxStore.getState().activeAiChatSessionId === sessionId) setSendError(errorPresentation);
      // A LuxIDE auth failure means the device token is stale/rejected (e.g. after the
      // gateway moved to Telegram-linked identity) — drop it and open the link modal.
      if (errorPresentation.kind === "auth") {
        const prov = getAiProvider(
          useLuxStore.getState().aiPreferences.providers,
          useLuxStore.getState().aiPreferences.selectedProviderId,
        );
        if (prov && isLuxideProvider(prov)) void relinkLuxide(prov.baseUrl);
      }
    } finally {
      pendingSendLockRef.current.delete(sessionId);
      setSendPhase((current) => (current?.sessionId === sessionId ? null : current));
      finishAiChatTurn(sessionId, abortController);
      clearAiRetryNotice(sessionId);
      lastRecordedRetryAttemptRef.current.delete(sessionId);
      requestAnimationFrame(() => resizeComposerTextarea());
      const pendingContinuation = goalContinuationTimersRef.current.get(sessionId);
      if (pendingContinuation !== undefined) {
        clearTimeout(pendingContinuation);
        goalContinuationTimersRef.current.delete(sessionId);
      }
      if (abortController.signal.aborted && abortController.signal.reason !== TURN_SUPERSEDED) {
        // User pressed Stop (or /undo, edit-resend, dispose) — the only things
        // that end Automatic's retry loop. A supersede (force-send / queued
        // "Send now" replacing this turn) is NOT a Stop: the replacement turn is
        // live and the goal run must survive for its continuation evaluation.
        resetAutomaticRetry(sessionId);
        stopGoalRun(sessionId);
      } else if (!abortController.signal.aborted) {
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
        // Context overflow is recoverable in EVERY mode: the retry force-compacts the
        // transcript first (below), so it must be allowed to retry like a transient.
        const contextOverflow = lastTurnError?.kind === "context-overflow";
        const retryPlan = (turnFailed && lastTurnError && (automatic || transientRetryable || contextOverflow))
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
          // Offline-aware backoff: while the OS reports no network, firing the
          // retry is guaranteed to fail — it would just burn the finite ladder
          // (manual/plan give up after 10). Hold THIS attempt instead: keep
          // re-checking every few seconds, show a "waiting for network" notice,
          // and fire ~immediately once connectivity returns. The attempt number
          // is not re-consumed while holding.
          const OFFLINE_RECHECK_MS = 3_000;
          const scheduleRetryFire = (delay: number) => {
            const handle = window.setTimeout(() => {
              if (goalContinuationTimersRef.current.get(retrySessionId) !== handle) return;
              if (typeof navigator !== "undefined" && navigator.onLine === false) {
                setAiRetryNotice(retrySessionId, {
                  attempt,
                  maxAttempts,
                  reason: "offline",
                  detail: "",
                  delayMs: OFFLINE_RECHECK_MS,
                });
                scheduleRetryFire(OFFLINE_RECHECK_MS);
                return;
              }
              goalContinuationTimersRef.current.delete(retrySessionId);
              void fireRetry();
            }, delay);
            goalContinuationTimersRef.current.set(retrySessionId, handle);
          };
          const fireRetry = async () => {
            // Skip if another turn is already running or the session went away. A mode
            // switch mid-backoff no longer cancels the retry — the ladder is bounded in
            // manual/plan and the task should still reach a valid result.
            if (getAiChatTurnRuntimeSnapshot().sendingSessionId === retrySessionId) return;
            // Context overflow: shrink the transcript BEFORE retrying, otherwise the
            // retry re-sends the same oversized history and fails identically. Force
            // compaction summarizes the older bulk; it escalates on each attempt.
            if (lastTurnError?.kind === "context-overflow") {
              try {
                await runCompaction(true);
              } catch {
                // Compaction failed (e.g. summarizer offline) — retry anyway; the
                // transient ladder still applies and may recover.
              }
              if (getAiChatTurnRuntimeSnapshot().sendingSessionId === retrySessionId) return;
            }
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
            // NO checkpoint here: this is the SILENT auto-retry ladder (fires every
            // ~30s, unbounded in Automatic), so a per-attempt file snapshot + full
            // checkpoint-store rewrite would repeat forever on a persistent outage,
            // snapshotting identical state. Editability is a MANUAL-retry concern
            // (retryLastRequest passes checkpoint:true); the user didn't click here.
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
          };
          // Already offline right now? Skip the pointless ladder wait — go
          // straight into the cheap online-recheck loop.
          const offlineNow = typeof navigator !== "undefined" && navigator.onLine === false;
          if (offlineNow) {
            setAiRetryNotice(sessionId, { attempt, maxAttempts, reason: "offline", detail: "", delayMs: OFFLINE_RECHECK_MS });
          }
          scheduleRetryFire(offlineNow ? OFFLINE_RECHECK_MS : delayMs);
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
  }, [activeChatSession?.id, activeDocument, activeSessionBusy, activeSessionClosed, aiPreferences, appendAiChatMessage, attachments, clearAiChatErrorHistory, createAiChatSession, locale, message, openDocuments, projectInstructions, replaceAiChatMessages, resizeComposerTextarea, runCompaction, selectedAgent, selectedModel, selectedProvider, setAiChatSessionContextBudgetReport, setAiChatSessionStatus, t, updateAiChatMessage, updateMessage, workspace]);

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

  const showRestoreSuccess = useCallback((result: { restoredFileCount: number; removedTurnCheckpoints: number; snapshotFileCount: number }) => {
    // Honest toast (#31c): "Restored 0 files" used to read as success even when no
    // file snapshot existed at all for the turn. Distinguish the three real outcomes:
    // files actually changed on disk; files matched already (checkpoint existed,
    // nothing to do); no snapshot existed at all (files were never touched by design).
    if (result.restoredFileCount > 0) {
      setRestoreNotice(t("aiChat.turnCheckpoint.restored", {
        files: result.restoredFileCount,
        turns: result.removedTurnCheckpoints,
      }));
    } else if (result.snapshotFileCount === 0) {
      setRestoreNotice(t("aiChat.turnCheckpoint.restoredNoSnapshot"));
    } else {
      setRestoreNotice(t("aiChat.turnCheckpoint.restoredClean"));
    }
    setSendError(null);
  }, [t]);

  const editUserMessageNow = useCallback(async (userMessageId: string, nextContent: string, options?: { skipConfirm?: boolean }) => {
    if (!activeChatSession || activeSessionClosed || !nextContent.trim()) return;
    const input = buildRestoreInput();
    if (!input) {
      // Was a silent no-op ("откаты не работают"): name the actual blocker.
      setSendError(classifyAiChatError(new Error(t("aiChat.turnCheckpoint.agentNeedProject")), t));
      return;
    }
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
      // disappears after the first edit. Send through the ref so the resend uses the
      // CURRENT provider/model/effort/preferences, not the ones frozen in this closure.
      await handleSendRef.current(nextContent.trim(), result.messages, { force: true, checkpoint: true });
    } catch (error) {
      setSendError(classifyAiChatError(error, t));
      setRestoreNotice(null);
    }
  }, [activeChatSession, activeSessionClosed, buildRestoreInput, replaceAiChatMessages, showRestoreSuccess, t]);

  // AiChatMessages rows are memoized with a comparator that deliberately ignores
  // handler identity, so a row can hold an onEditUserMessage captured many renders
  // ago. Editing a message then resent with a stale model/thinking-effort/format.
  // These ref-stable wrappers make the captured identity irrelevant: whichever
  // version a row holds, the call lands on the latest implementation.
  const editUserMessageNowRef = useRef(editUserMessageNow);
  editUserMessageNowRef.current = editUserMessageNow;
  const handleEditUserMessage = useCallback(
    (userMessageId: string, nextContent: string, options?: { skipConfirm?: boolean }) =>
      editUserMessageNowRef.current(userMessageId, nextContent, options),
    [],
  );

  // Roll back to before a user message WITHOUT resending: files, tasks and goal
  // return to the snapshot; the message text lands back in the composer so the
  // user can tweak or discard it. This is the "откатиться" button next to Edit.
  const restoreUserMessageNow = useCallback(async (userMessageId: string) => {
    if (!activeChatSession || activeSessionClosed) return;
    const input = buildRestoreInput();
    if (!input) {
      setSendError(classifyAiChatError(new Error(t("aiChat.turnCheckpoint.agentNeedProject")), t));
      return;
    }
    if (!window.confirm(t("aiChat.turnCheckpoint.restoreConfirm"))) return;
    const original = activeChatSession.messages.find((message) => message.id === userMessageId)?.content ?? "";
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
      if (original.trim()) updateMessage(original);
    } catch (error) {
      setSendError(classifyAiChatError(error, t));
      setRestoreNotice(null);
    }
  }, [activeChatSession, activeSessionClosed, buildRestoreInput, replaceAiChatMessages, showRestoreSuccess, t, updateMessage]);

  // Same stale-closure hazard as handleEditUserMessage (memoized rows ignore
  // handler identity): route Restore through a ref-stable wrapper too.
  const restoreUserMessageNowRef = useRef(restoreUserMessageNow);
  restoreUserMessageNowRef.current = restoreUserMessageNow;
  const handleRestoreUserMessage = useCallback(
    (userMessageId: string) => restoreUserMessageNowRef.current(userMessageId),
    [],
  );

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
        // checkpoint: an override send skips turn-checkpoint creation by default,
        // which left every retried message without Edit/Roll back affordances —
        // request one explicitly, the same way edit-resend does.
        void handleSend(t("aiChat.retry.continue"), trimmed, { force: true, preserveErrorHistory: true, checkpoint: true });
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
        void handleSend(draft, nextHistory, { preserveErrorHistory: true, checkpoint: true });
        return;
      }
    }
    const draft = lastUserDraft ?? [...messages].reverse().find((entry) => entry.role === "user")?.content ?? "";
    if (draft.trim()) void handleSend(draft, undefined, { preserveErrorHistory: true, checkpoint: true });
  }, [activeChatSession, activeSessionBusy, activeSessionClosed, handleSend, lastUserDraft, messages, replaceAiChatMessages, t]);

  const reviewActionNow = useCallback((messageId: string) => {
    if (!activeChatSession || activeSessionBusy || activeSessionClosed) return;
    // Build a model-side prompt that scopes the review to the exact turn the user
    // clicked. The displayed badge comes from reviewRequest:true, so the raw prompt
    // text is never rendered in the chat — only the scoped model message is sent.
    const basePrompt = t("aiChat.review.prompt");
    const scopedPrompt = `${basePrompt}\n\n<!-- review-target-message-id: ${messageId} -->`;
    void handleSendRef.current(basePrompt, undefined, { reviewRequest: true, force: true, modelMessageOverride: scopedPrompt });
  }, [activeChatSession, activeSessionBusy, activeSessionClosed, t]);

  // Ref-stable for the same reason as handleEditUserMessage: memoized message rows
  // ignore handler identity, so Review must resolve to the latest closure at call time.
  const reviewActionNowRef = useRef(reviewActionNow);
  reviewActionNowRef.current = reviewActionNow;
  const handleReviewAction = useCallback((messageId: string) => reviewActionNowRef.current(messageId), []);

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
  // tool, so the rail already reflects it. Also opens the live plan-run card
  // (#35) so the handoff is followed by a visible checklist, not silence.
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
    setActivePlanRun({ planId: pendingPlan.planId, sessionId, title: pendingPlan.title, stepCount: pendingPlan.steps.length });
    const handoffMessage = buildPlanHandoffUserMessage(pendingPlan.steps.map((step) => step.title));
    void handleSend(handoffMessage, undefined, { force: true });
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
    // Clear the composer after enqueue: the queued text is already captured, so the
    // field resets for the next message just like a normal send.
    if (activeSessionBusy && activeAiChatSessionId && message.trim()) {
      enqueueChatMessage(activeAiChatSessionId, message, "queued");
      updateMessage("");
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
  // Last retry-attempt number recorded into a session's error history, so the
  // same attempt's re-emitted notice never double-counts (see onRetryNotice).
  const lastRecordedRetryAttemptRef = useRef<Map<string, number>>(new Map());

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
        // Clear BOTH the transient ref AND the persisted injectedTurnId: the chip
        // locks to a control-less "Folding in…" while injectedTurnId is set, so a
        // turn that dies before Rust confirms would otherwise strand the chip with no
        // delete/edit/send affordance for the rest of the session.
        injectedTextBySessionRef.current.delete(session.id);
        for (const entry of getQueuedMessagesForSession(session.id)) {
          if (entry.mode === "recommendation" && (injectingRef.current.has(entry.id) || entry.injectedTurnId)) {
            injectingRef.current.delete(entry.id);
            setQueuedMessageInjectedTurn(entry.id, null);
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
      const session = aiChatSessions.find((candidate) => candidate.id === entry.sessionId);
      if (!session || session.closedAt) continue;
      const running = sendingSessionId === entry.sessionId || isAiChatSessionBusyStatus(session.status);
      if (!running) continue;
      // ai_inject_message is scoped by session+turn, so we need the live turn_id.
      // If it isn't published yet (the native turn hasn't launched), leave the chip
      // as a "recommendation" and retry on the next tick instead of mis-routing it.
      const turnId = getActiveTurnId(entry.sessionId);
      if (!turnId) continue;
      // In-flight is tracked on the queue entry itself (module state), not just a
      // component ref: a panel remount mid-turn would reset the ref and re-inject
      // the same text into the live Rust loop. Keyed by turn id so an entry left
      // unconfirmed by a dead turn becomes injectable again for the next turn.
      if (entry.injectedTurnId === turnId || injectingRef.current.has(entry.id)) continue;
      // Mark in-flight BEFORE the await so a re-render can't double-inject. The chip
      // is NOT removed here — it is removed only when Rust confirms the fold-in
      // (onUserMessageInjected), so a turn that ends before the drain never loses it.
      injectingRef.current.add(entry.id);
      setQueuedMessageInjectedTurn(entry.id, turnId);
      const pending = injectedTextBySessionRef.current.get(entry.sessionId) ?? [];
      pending.push(entry.text);
      injectedTextBySessionRef.current.set(entry.sessionId, pending);
      // Optimistic in-dialog render: show the recommendation as a user bubble (with
      // its "sent as recommendation" caption) the instant it's dispatched, instead
      // of waiting for Rust to echo the fold-in. Keyed by a STABLE per-entry id (not
      // text) so two recommendations with identical text still render as two bubbles.
      const optimisticId = `rec-${entry.id}`;
      const alreadyShown = useLuxStore.getState().aiChatSessions
        .find((session) => session.id === entry.sessionId)?.messages
        .some((message) => message.id === optimisticId);
      if (!alreadyShown) {
        appendAiChatMessage(entry.sessionId, {
          id: optimisticId,
          role: "user",
          content: entry.text,
          recommendation: true,
          timestamp: Date.now(),
        });
      }
      void luxCommands.aiInjectMessage(entry.sessionId, turnId, entry.text)
        .catch(() => {
          // Inject call itself failed — un-track so the end-of-turn drain re-sends it
          // as a follow-up turn (flip to "queued") instead of stranding it.
          injectingRef.current.delete(entry.id);
          setQueuedMessageInjectedTurn(entry.id, null);
          const list = injectedTextBySessionRef.current.get(entry.sessionId);
          if (list) {
            const at = list.indexOf(entry.text);
            if (at >= 0) list.splice(at, 1);
          }
          // Roll back the optimistic bubble by its stable id: the fold-in never
          // happened, and the entry will re-send as its own "queued" follow-up turn
          // (rendering a normal bubble then), so leaving this one would duplicate it.
          if (!alreadyShown) {
            const live = useLuxStore.getState().aiChatSessions.find((session) => session.id === entry.sessionId);
            if (live?.messages.some((message) => message.id === optimisticId)) {
              replaceAiChatMessages(entry.sessionId, live.messages.filter((message) => message.id !== optimisticId));
            }
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
    // Guard the race: a recommendation already folded into the running turn
    // (injectedTurnId set, or in-flight) must not ALSO be force-sent as a fresh
    // turn — that would double-send to the model and double-render the bubble.
    if (entry.injectedTurnId || injectingRef.current.has(entry.id)) return;
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
      tokenSpeed={liveTokenSpeed}
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

  // Custom chat font: re-point --font-ui for the panel subtree only, so every
  // chat surface (messages, composer, menus) follows while code spans keep mono.
  const chatFontStyle = useMemo(
    () => (chatFontFamily ? { "--font-ui": withFontFallback(chatFontFamily, DEFAULT_UI_FONT_STACK) } as CSSProperties : undefined),
    [chatFontFamily],
  );

  return (
    <aside className="ai-chat-panel" style={chatFontStyle} aria-label={t("aiChat.panel.aria")} data-empty-home={isAgentHome} data-embedded={embedded} data-presentation={presentation} data-status={activeStatus}>
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
              <MarkdownSmoothStreamContext.Provider value={aiPreferences.chatSmoothStream}>
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
                onRestoreUserMessage={handleRestoreUserMessage}
                onStopAfterTool={requestStopAfterToolRound}
                canStopAfterTool={activeSessionBusy}
                t={t}
                onReviewAction={handleReviewAction}
                reviewDisabled={activeSessionBusy || activeSessionClosed}
              />
              </MarkdownSmoothStreamContext.Provider>
              {pendingPlan && (
                <AiPlanCard
                  plan={pendingPlan}
                  onStart={handlePlanStart}
                  busy={activeSessionBusy || activeSessionClosed}
                  agentMode={aiPreferences.agentMode}
                  t={t}
                />
              )}
              {activePlanRun && activePlanRun.sessionId === activeAiChatSessionId && (
                <AiPlanRunCard
                  run={activePlanRun}
                  turnSettled={!activeSessionBusy}
                  onDismiss={() => setActivePlanRun(null)}
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
              {sendPhase && sendPhase.sessionId === activeChatSession?.id && (
                <div className="ai-chat-send-progress" role="status">
                  <div className="ai-chat-send-progress-text">
                    <strong>{t("aiChat.sendProgress.title")}</strong>
                    <span>{t(`aiChat.sendProgress.${sendPhase.stage}`)}</span>
                  </div>
                  <div className="ai-chat-send-progress-bar" aria-hidden="true"><i /></div>
                </div>
              )}
              {activeChatSession && <AiRetryBanner sessionId={activeChatSession.id} history={activeChatSession.errorHistory} t={t} />}
              {activeSessionBusy && activeChatSession && (
                <AiAgentNowPlaque sessionId={activeChatSession.id} status={activeStatus} workTail={liveWorkTail} t={t} />
              )}
              {activeSessionClosed && <AiChatClosedNotice onRestore={() => restoreAiChatSession(activeAiChatSessionId)} t={t} />}
              {sendError && (
                <AiChatError
                  presentation={sendError}
                  // sendError is shared by several error domains (turn failure,
                  // compaction, edit/restore guards); the session's retry history
                  // belongs only under the turn-failure card it was built for.
                  history={sendError.message === activeChatSession?.lastError ? activeChatSession?.errorHistory : undefined}
                  canRetry={Boolean(lastUserDraft)}
                  onRetry={retryLastRequest}
                  onOpenSettings={() => openSettingsSection("ai-runtime")}
                  t={t}
                />
              )}
              {activeLastErrorPresentation && !sendError && (
                <AiChatError
                  presentation={activeLastErrorPresentation}
                  history={activeChatSession?.errorHistory}
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
          <AiSessionReviewBar sessionId={activeAiChatSessionId} t={t} />
          {presentation === "agent" && !workspace && (
            <p className="ai-chat-checkpoint-agent-hint" role="note">{t("aiChat.turnCheckpoint.agentNeedProject")}</p>
          )}
          {renderComposerContent()}
        </footer>
      )}
    </aside>
  );
}




