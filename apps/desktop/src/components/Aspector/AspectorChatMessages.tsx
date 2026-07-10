import { Brain, ChevronRight, Copy, FoldVertical, Lightbulb, MoveRight, SearchCheck } from "lucide-react";
import type { CSSProperties, ReactNode, RefObject } from "react";
import { createContext, Fragment, memo, useContext, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { lexer, type Token, type Tokens } from "marked";
import { AspectorChatMessageActions } from "./AspectorChatMessageActions";
import { AspectorAssistantMessageActions } from "./AspectorAssistantMessageActions";
import { AspectorToolCallsGroup } from "./AspectorToolCall";
import type { TranslateFn } from '../../lib/i18n/useTranslation';
import { isCompactionCheckpointMessage, type ContextCompactionState } from '../../lib/aspector/chat/context-compaction';
import { formatCompactTokens } from '../../lib/aspector/chat/context-usage';
import { AspectorPathEvidenceNotice } from "./AspectorPathEvidenceNotice";
import { AspectorTurnSummaryCard } from "./AspectorTurnSummaryCard";
import { isPendingAssistantShell } from "./AspectorThinkingIndicator";
import type { AiChatSessionStatus } from '../../lib/store/index';
import * as chatDisplayText from '../../lib/aspector/chat/display-text';
import { useElapsedSeconds, formatThinkingElapsed } from '../../lib/hooks/use-elapsed-seconds';
import { HtmlArtifact } from "./HtmlArtifact";
import { isReviewRequestMessage, type AiChatMessage, type AiChatMessageAttachment, type AiChatResponseTiming, type AiInlineNotice, type AiMessageSegment, type AiToolApprovalDecision } from '../../lib/aspector/chat/types';

const coerceChatMessageText =
  chatDisplayText.coerceChatMessageText
  ?? ((value: unknown) => (typeof value === "string" ? value : value == null ? "" : String(value)));

const decodeChatDisplayText =
  chatDisplayText.decodeChatDisplayText
  ?? ((text: string | null | undefined) => (typeof text === "string" ? text : ""));

const trimChatMessageEnd =
  chatDisplayText.trimChatMessageEnd
  ?? ((text: string) => text.replace(/\s+$/, ""));

// Threads at/above this length switch from "render everything" to windowed
// virtualization. Below it the transcript is small enough that virtual layout
// overhead outweighs the savings.
const VIRTUALIZE_THRESHOLD = 40;
const ESTIMATED_ROW_HEIGHT = 180;
const VIRTUAL_OVERSCAN = 5;

type AspectorChatMessagesProps = {
  canMutateHistory: boolean;
  canRestoreUserMessage: (userMessageId: string) => boolean;
  messages: AiChatMessage[];
  onApprovalDecision: (approvalId: string, decision: AiToolApprovalDecision) => void;
  onEditUserMessage: (messageId: string, nextContent: string) => void;
  onRestoreUserMessage?: (messageId: string) => void;
  onStopAfterTool?: () => void;
  canStopAfterTool?: boolean;
  parentRef: RefObject<HTMLDivElement | null>;
  showResponseDuration: boolean;
  contextCompaction?: ContextCompactionState | null;
  workspaceRoot: string | null;
  streamingMessageId: string | null;
  sessionStatus: AiChatSessionStatus;
  t: TranslateFn;
  onReviewAction?: (messageId: string) => void;
  /** Disable (not hide) Review buttons while a turn is running or the session is closed. */
  reviewDisabled?: boolean;
};

export function AspectorChatMessages({
  canMutateHistory,
  canRestoreUserMessage,
  messages,
  onApprovalDecision,
  onEditUserMessage,
  onRestoreUserMessage,
  onStopAfterTool,
  canStopAfterTool = false,
  parentRef,
  showResponseDuration,
  contextCompaction,
  workspaceRoot,
  streamingMessageId,
  sessionStatus,
  t,
  onReviewAction,
  reviewDisabled = false,
}: AspectorChatMessagesProps) {
  // Streaming only ever lands on the last message. We render that single row in
  // normal document flow (below the virtual window) instead of as an absolutely
  // positioned virtual row: its token-by-token growth then feeds the scroll
  // container's real height immediately, keeping sticky-bottom pixel-accurate with
  // no ResizeObserver measurement lag — while every other row stays virtualized.
  const streamingIndex = useMemo(
    () => (streamingMessageId ? messages.findIndex((message) => message.id === streamingMessageId) : -1),
    [messages, streamingMessageId],
  );
  const streamingTail =
    streamingIndex >= 0 && streamingIndex === messages.length - 1 ? messages[streamingIndex] : null;
  const virtualCount = streamingTail ? messages.length - 1 : messages.length;

  const virtualizer = useVirtualizer({
    count: virtualCount,
    getScrollElement: () => parentRef.current,
    getItemKey: (index) => messages[index]?.id ?? index,
    estimateSize: () => ESTIMATED_ROW_HEIGHT,
    overscan: VIRTUAL_OVERSCAN,
  });
  const virtualItems = virtualizer.getVirtualItems();

  // Every assistant turn keeps its Review affordance — the review prompt is
  // scoped to the clicked message id, so reviewing an older turn is valid. The
  // old "last assistant only" rule made the button vanish forever the moment a
  // review (or any new turn) was sent, which read as a broken button.
  const getOnReviewFor = (id: string) =>
    onReviewAction ? () => onReviewAction(id) : undefined;

  // Session-level compaction stats describe the LATEST compaction only, so they
  // attach to the newest checkpoint card — an older surviving checkpoint must not
  // wear stats from a compaction it didn't produce.
  const latestCheckpointId = useMemo(() => {
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const candidate = messages[index];
      if (candidate && isCompactionCheckpointMessage(candidate)) return candidate.id;
    }
    return null;
  }, [messages]);

  const renderRow = (chatMessage: AiChatMessage) => (
    <AspectorChatMessageView
      key={chatMessage.id}
      message={chatMessage}
      streaming={chatMessage.id === streamingMessageId}
      showResponseDuration={showResponseDuration}
      contextCompaction={chatMessage.id === latestCheckpointId ? contextCompaction : null}
      workspaceRoot={workspaceRoot}
      canMutateHistory={canMutateHistory}
      canRestoreUserMessage={canRestoreUserMessage}
      onApprovalDecision={onApprovalDecision}
      onEditUserMessage={onEditUserMessage}
      onRestoreUserMessage={onRestoreUserMessage}
      onStopAfterTool={onStopAfterTool}
      canStopAfterTool={canStopAfterTool}
      sessionStatus={sessionStatus}
      t={t}
      onReview={getOnReviewFor(chatMessage.id)}
      reviewDisabled={reviewDisabled}
    />
  );

  // Small threads: render the whole transcript (no virtual layout overhead).
  if (messages.length < VIRTUALIZE_THRESHOLD) {
    return <>{messages.map(renderRow)}</>;
  }

  // Long threads stay virtualized even mid-stream: a token update reconciles only
  // the windowed rows (and, via the row memo, only the streaming row) instead of
  // mounting the entire transcript.
  return (
    <>
      <div className="ai-chat-virtual-list" style={{ height: virtualizer.getTotalSize() }}>
        {virtualItems.map((item) => {
          const chatMessage = messages[item.index];
          if (!chatMessage) return null;
          return (
            <div
              key={chatMessage.id}
              className="ai-chat-virtual-row"
              data-index={item.index}
              ref={virtualizer.measureElement}
              style={{ transform: `translateY(${item.start}px)` }}
            >
              {renderRow(chatMessage)}
            </div>
          );
        })}
      </div>
      {streamingTail && renderRow(streamingTail)}
    </>
  );
}

const AspectorChatMessageView = memo(function AspectorChatMessageView({
  canMutateHistory,
  canRestoreUserMessage,
  message,
  onApprovalDecision,
  onEditUserMessage,
  onRestoreUserMessage,
  onStopAfterTool,
  canStopAfterTool,
  showResponseDuration,
  contextCompaction,
  workspaceRoot,
  streaming,
  sessionStatus,
  t,
  onReview,
  reviewDisabled = false,
}: {
  canMutateHistory: boolean;
  canRestoreUserMessage: (userMessageId: string) => boolean;
  message: AiChatMessage;
  onApprovalDecision: (approvalId: string, decision: AiToolApprovalDecision) => void;
  onEditUserMessage: (messageId: string, nextContent: string) => void;
  onRestoreUserMessage?: (messageId: string) => void;
  onStopAfterTool?: () => void;
  canStopAfterTool?: boolean;
  showResponseDuration: boolean;
  contextCompaction?: ContextCompactionState | null;
  workspaceRoot: string | null;
  streaming: boolean;
  sessionStatus: AiChatSessionStatus;
  t: TranslateFn;
  onReview?: () => void;
  reviewDisabled?: boolean;
}) {
  // In-place edit of a checkpointed user message: the bubble text itself becomes
  // the editor (no separate framed textarea below the message).
  const [editingUser, setEditingUser] = useState(false);
  const pendingShell = isPendingAssistantShell(message, streaming);
  // An empty streaming shell would render a second "Thinking…" indicator that
  // duplicates the live status plaque below the thread — skip the shell entirely
  // and let the plaque be the single busy indicator. Once real tokens/tools
  // arrive, pendingShell is false and the answer renders here as usual.
  if (pendingShell) return null;
  if (isReviewRequestMessage(message)) {
    return (
      <article className="ai-chat-message ai-chat-review-request" data-role="user">
        <div className="ai-chat-review-badge" role="note">
          <span className="ai-chat-review-badge-icon" aria-hidden="true"><SearchCheck size={15} /></span>
          <span className="ai-chat-review-badge-copy">
            <strong>{t("aiChat.review.requestTitle")}</strong>
            <small>{t("aiChat.review.requestSubtitle")}</small>
          </span>
          <time>{formatMessageTime(message.timestamp)}</time>
        </div>
      </article>
    );
  }
  if (isCompactionCheckpointMessage(message)) {
    // Stats arrive only on the latest checkpoint (see latestCheckpointId in the
    // parent); older surviving checkpoints render without a stats row.
    const stats = contextCompaction ?? null;
    const reductionPercent = stats && stats.tokensBefore > 0
      ? Math.max(0, Math.round((1 - stats.tokensAfter / stats.tokensBefore) * 100))
      : 0;
    return (
      <article className="ai-chat-message ai-chat-compaction-checkpoint" data-role="system">
        <header className="ai-compaction-head">
          <span className="ai-compaction-title">
            <FoldVertical size={13} aria-hidden="true" />
            {t("aiChat.compact.checkpointLabel")}
          </span>
          {stats && (
            <span
              className="ai-compaction-tokens"
              title={t("aiChat.compact.tokensBeforeAfter", {
                before: formatCompactTokens(stats.tokensBefore),
                after: formatCompactTokens(stats.tokensAfter),
              })}
            >
              {formatCompactTokens(stats.tokensBefore)}
              <MoveRight size={11} aria-hidden="true" />
              {formatCompactTokens(stats.tokensAfter)}
              {reductionPercent > 0 && (
                <em className="ai-compaction-reduction">−{reductionPercent}%</em>
              )}
            </span>
          )}
          <time>{formatMessageTime(message.timestamp)}</time>
        </header>
        <div className="ai-chat-compaction-body">
          <p>{t("aiChat.compact.checkpointHint")}</p>
          {stats?.droppedItems && stats.droppedItems.length > 0 && (
            <details className="ai-chat-compaction-details">
              <summary>
                {t("aiChat.compact.droppedSummary", {
                  count: stats.droppedItems.length,
                  tokens: formatCompactTokens(stats.droppedTokens ?? 0),
                })}
              </summary>
              <ul className="ai-compaction-dropped">
                {stats.droppedItems.map((item) => (
                  <li key={`${item.kind}-${item.label}-${item.tokens}`}>
                    <span>{item.label}</span>
                    <span>{formatCompactTokens(item.tokens)}</span>
                  </li>
                ))}
              </ul>
            </details>
          )}
          {/* Collapsed by default: the full checkpoint is reference material,
              not conversation — one line in the transcript, expandable on demand. */}
          <details className="ai-chat-compaction-details">
            <summary>{t("aiChat.compact.checkpointExpand")}</summary>
            <pre>{formatCompactionPreview(message.content)}</pre>
          </details>
        </div>
      </article>
    );
  }
  return (
    <article className="ai-chat-message" data-role={message.role}>
      <div className="ai-chat-message-meta">
        <span>{message.role === "user" ? t("aiChat.role.user") : t("aiChat.role.assistant")}</span>
        <time>{formatMessageTime(message.timestamp)}</time>
      </div>
      {message.role === "user" && editingUser ? (
        <AspectorUserInlineEdit
          initial={message.content}
          onCancel={() => setEditingUser(false)}
          onSubmit={(text) => {
            setEditingUser(false);
            onEditUserMessage(message.id, text);
          }}
          t={t}
        />
      ) : (
        <AspectorMessageBody
          message={message}
          streaming={streaming}
          sessionStatus={sessionStatus}
          onApprovalDecision={onApprovalDecision}
          t={t}
        />
      )}
      {message.role === "user" && message.recommendation && !editingUser && (
        <p className="ai-chat-message-recommendation-caption">
          <Lightbulb size={11} aria-hidden="true" />
          {t("aiChat.queue.sentAsRecommendation")}
        </p>
      )}
      {message.role === "user" ? (
        <AspectorChatMessageActions
          canMutate={canMutateHistory}
          canRestoreUser={canRestoreUserMessage(message.id)}
          editing={editingUser}
          message={message}
          onStartEdit={() => setEditingUser(true)}
          onRestore={() => onRestoreUserMessage?.(message.id)}
          t={t}
        />
      ) : (
        <AspectorAssistantMessageActions
          canMutate={canMutateHistory}
          canStopAfterTool={canStopAfterTool ?? false}
          onStopAfterTool={() => onStopAfterTool?.()}
          t={t}
        />
      )}
      {message.role === "assistant" && !streaming && (
        <AspectorPathEvidenceNotice message={message} streaming={streaming} t={t} />
      )}
      {message.role === "assistant" && !streaming && (
        <AspectorTurnSummaryCard message={message} workspaceRoot={workspaceRoot} t={t} onReview={onReview} reviewDisabled={reviewDisabled} />
      )}
      {message.role === "assistant" && showResponseDuration && typeof message.responseDurationMs === "number" && !message.responseTiming && !message.turnUsage && (
        <div className="ai-chat-response-duration" title={formatResponseTimingTitle(message, t)}>{formatResponseDuration(message.responseDurationMs, t)}</div>
      )}
    </article>
  );
}, areMessageViewPropsEqual);

// Custom memo comparator. The parent re-renders on every streamed token and passes
// fresh handler closures + a new `messages` array each time; a shallow compare would
// then re-render EVERY row per token. Message objects are immutable (new ref on any
// change), so identity-comparing `message` plus the few primitives that actually alter
// this row's output lets unchanged rows bail out — only the streaming message re-renders
// while tokens arrive. Handler props are intentionally excluded: they are behavior-stable
// (same effect regardless of closure identity).
function areMessageViewPropsEqual(
  prev: Readonly<{ message: AiChatMessage; streaming: boolean; showResponseDuration: boolean; canMutateHistory: boolean; canRestoreUserMessage: (id: string) => boolean; canStopAfterTool?: boolean; sessionStatus: AiChatSessionStatus; contextCompaction?: ContextCompactionState | null; workspaceRoot: string | null; t: TranslateFn; onReview?: () => void; reviewDisabled?: boolean }>,
  next: Readonly<{ message: AiChatMessage; streaming: boolean; showResponseDuration: boolean; canMutateHistory: boolean; canRestoreUserMessage: (id: string) => boolean; canStopAfterTool?: boolean; sessionStatus: AiChatSessionStatus; contextCompaction?: ContextCompactionState | null; workspaceRoot: string | null; t: TranslateFn; onReview?: () => void; reviewDisabled?: boolean }>,
) {
  return (
    prev.message === next.message
    && prev.streaming === next.streaming
    && prev.showResponseDuration === next.showResponseDuration
    && prev.canMutateHistory === next.canMutateHistory
    // Restore-eligibility is read during render via canRestoreUserMessage(message.id);
    // compare its identity so a turn-checkpoint change still refreshes the row. The
    // panel memoizes it (useCallback), so this stays a cheap reference check.
    && prev.canRestoreUserMessage === next.canRestoreUserMessage
    && prev.canStopAfterTool === next.canStopAfterTool
    && prev.sessionStatus === next.sessionStatus
    && prev.contextCompaction === next.contextCompaction
    && prev.workspaceRoot === next.workspaceRoot
    && prev.t === next.t
    // Presence (not identity) is what matters for onReview; disabled state must
    // refresh rows so buttons re-enable when the running turn finishes.
    && Boolean(prev.onReview) === Boolean(next.onReview)
    && prev.reviewDisabled === next.reviewDisabled
  );
}

/**
 * In-place editor for a checkpointed user message: replaces the bubble body
 * with a textarea styled like the bubble itself. Enter resends (rolling back
 * the turn first), Shift+Enter inserts a newline, Escape cancels.
 */
function AspectorUserInlineEdit({ initial, onCancel, onSubmit, t }: {
  initial: string;
  onCancel: () => void;
  onSubmit: (text: string) => void;
  t: TranslateFn;
}) {
  const [draft, setDraft] = useState(initial);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);

  // Grow with content; focus at the end like an editor, not a form field.
  useEffect(() => {
    const node = textareaRef.current;
    if (!node) return;
    node.focus();
    node.setSelectionRange(node.value.length, node.value.length);
    node.style.height = "auto";
    node.style.height = `${node.scrollHeight}px`;
  }, []);

  const submit = () => {
    if (draft.trim()) onSubmit(draft.trim());
  };

  return (
    <div className="ai-chat-user-inline-edit">
      <textarea
        ref={textareaRef}
        value={draft}
        rows={1}
        spellCheck={false}
        onChange={(event) => {
          setDraft(event.target.value);
          event.currentTarget.style.height = "auto";
          event.currentTarget.style.height = `${event.currentTarget.scrollHeight}px`;
        }}
        onKeyDown={(event) => {
          if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
            return;
          }
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
            event.preventDefault();
            submit();
          }
        }}
      />
      <div className="ai-chat-user-inline-edit-foot">
        <span>{t("aiChat.turnCheckpoint.editInlineHint")}</span>
        <div className="ai-chat-user-inline-edit-actions">
          <button type="button" onClick={onCancel}>{t("common.cancel")}</button>
          <button type="button" className="primary" disabled={!draft.trim()} onClick={submit}>
            {t("aiChat.turnCheckpoint.editResend")}
          </button>
        </div>
      </div>
    </div>
  );
}

function formatResponseDuration(durationMs: number, t: TranslateFn) {
  return t("aiChat.responseDuration", {
    milliseconds: durationMs,
    seconds: (durationMs / 1000).toFixed(durationMs >= 10_000 ? 1 : 2),
  });
}

function formatResponseTimingTitle(message: AiChatMessage, t: TranslateFn) {
  const timing = message.responseTiming;
  if (!timing) return formatResponseDuration(message.responseDurationMs ?? 0, t);
  return t("aiChat.responseTiming.detail", {
    total: formatTimingMs(timing.totalMs, t),
    model: formatTimingMs(timing.modelMs, t),
    firstToken: formatNullableTimingMs(timing.firstTokenMs, t),
    stream: formatNullableTimingMs(timing.streamMs, t),
    tools: formatTimingMs(timing.toolMs, t),
    overhead: formatTimingMs(timing.overheadMs, t),
    modelCalls: timing.modelCalls,
    toolCalls: timing.toolCalls,
    rounds: timing.rounds,
    mode: timing.streamed ? t("aiChat.responseTiming.streamed") : t("aiChat.responseTiming.request"),
  });
}

function formatNullableTimingMs(value: number | null, t: TranslateFn) {
  return value === null ? "-" : formatTimingMs(value, t);
}

function formatTimingMs(value: number, t: TranslateFn) {
  if (value < 1_000) return t("aiChat.responseTiming.milliseconds", { milliseconds: value });
  const seconds = (value / 1_000).toFixed(value >= 10_000 ? 1 : 2);
  return t("aiChat.responseDuration", { milliseconds: value, seconds });
}

// Whether the markdown block currently being rendered is part of a still-streaming
// message. Consumed by MarkdownCodeBlock so a fenced ```html artifact stays plain
// code until the turn settles (a half-written <script> would thrash the iframe).
const MarkdownStreamingContext = createContext(false);

// Smooth-streaming preference (Settings → AI Runtime). Provided by AiChatPanel
// around the transcript; consumed inside MarkdownMessage. Context (not a prop)
// so the memoized message rows don't need the toggle in their comparator.
export const MarkdownSmoothStreamContext = createContext(true);

function AspectorMessageBody({ message, streaming, sessionStatus, onApprovalDecision, t }: {
  message: AiChatMessage;
  streaming: boolean;
  sessionStatus: AiChatSessionStatus;
  onApprovalDecision: (approvalId: string, decision: AiToolApprovalDecision) => void;
  t: TranslateFn;
}) {
  const attachmentGallery = message.role === "user" ? (
    <AspectorMessageAttachmentGallery attachments={message.attachments} t={t} />
  ) : null;
  const segments = message.segments;
  // Defensive floor under the reasoning shimmer: the model can finish its last
  // token and move on to building a tool call (status leaves thinking/streaming)
  // a beat before toolCallStarted actually appends the tool segment that would
  // otherwise flip `isLast` to false. Without this the header kept shimmering
  // "Thinking…" into a phase that is no longer thinking.
  const reasoningLive = streaming && (sessionStatus === "thinking" || sessionStatus === "streaming");

  const flowNodes: ReactNode[] = [];
  if (!segments || segments.length === 0) {
    if (message.reasoning && message.reasoning.trim().length > 0) {
      flowNodes.push(
        <AspectorReasoningBlock key="reasoning" text={coerceChatMessageText(message.reasoning)} streaming={reasoningLive} t={t} />,
      );
    }
    if (message.toolCalls && message.toolCalls.length > 0) {
      flowNodes.push(<AspectorToolCallsGroup key="tools" onApprovalDecision={onApprovalDecision} t={t} toolCalls={message.toolCalls} />);
    }
    if (message.content) {
      flowNodes.push(<MarkdownMessage key="answer" content={message.content} t={t} />);
    }
  } else {
    let toolBatch: AiMessageSegment[] = [];
    const flushTools = (key: string) => {
      if (toolBatch.length === 0) return;
      const toolCalls = toolBatch.map((segment) => segment.kind === "tool" ? segment.toolCall : null).filter((call): call is NonNullable<typeof call> => Boolean(call));
      flowNodes.push(<AspectorToolCallsGroup key={`tools-${key}`} onApprovalDecision={onApprovalDecision} t={t} toolCalls={toolCalls} />);
      toolBatch = [];
    };

    segments.forEach((segment, index) => {
      if (segment.kind === "tool") {
        toolBatch.push(segment);
        return;
      }
      // An inline event plaque (e.g. reasoning-effort fallback) IS a real visual
      // break: it happened between rounds, so close the tool group before it.
      if (segment.kind === "notice") {
        flushTools(`${index}`);
        flowNodes.push(<AspectorInlineNoticePlaque key={segment.id} notice={segment.notice} t={t} />);
        return;
      }
      // An empty reasoning/text segment is not a real visual break, so it must NOT
      // split an in-progress tool group — otherwise consecutive tool rounds with
      // only blank scaffolding between them render as several collapsed groups
      // instead of one. Only flush the accumulated tools right before a segment
      // that actually renders.
      if (segment.text.trim().length === 0) return;
      flushTools(`${index}`);
      if (segment.kind === "reasoning") {
        const isLast = index === segments.length - 1;
        flowNodes.push(
          <AspectorReasoningBlock key={segment.id} text={coerceChatMessageText(segment.text)} streaming={reasoningLive && isLast} t={t} />,
        );
        return;
      }
      flowNodes.push(<MarkdownMessage key={segment.id} content={coerceChatMessageText(segment.text)} t={t} />);
    });
    flushTools("tail");
  }

  // Assistant turns render as a single connected timeline (reasoning → tools →
  // answer), tied together by one left rail. User messages stay a plain bubble.
  if (message.role === "assistant") {
    return (
      <MarkdownStreamingContext.Provider value={streaming}>
        <div className="ai-turn-flow" data-streaming={streaming || undefined}>{flowNodes}</div>
      </MarkdownStreamingContext.Provider>
    );
  }
  return (
    <>
      {attachmentGallery}
      {flowNodes}
    </>
  );
}

/** Inline event plaque inside the assistant timeline — rendered at the exact
 *  position the event happened (e.g. the provider rejected the configured
 *  reasoning effort mid-turn and the strongest accepted one was applied). */
function AspectorInlineNoticePlaque({ notice, t }: { notice: AiInlineNotice; t: TranslateFn }) {
  return (
    <div className="ai-turn-inline-notice" role="note" data-notice={notice.type}>
      <Brain size={13} aria-hidden="true" />
      <span className="ai-turn-inline-notice-text">
        <strong>
          {t("aiChat.reasoningFallback.change", { requested: notice.requested, applied: notice.applied })}
        </strong>
        <span>{t("aiChat.reasoningFallback.body", { requested: notice.requested, applied: notice.applied })}</span>
      </span>
    </div>
  );
}

function AspectorMessageAttachmentGallery({ attachments, t }: {
  attachments?: AiChatMessageAttachment[];
  t: TranslateFn;
}) {
  if (!attachments || attachments.length === 0) return null;
  const images = attachments.filter((attachment) => attachment.kind === "image" && attachment.previewUrl);
  const files = attachments.filter((attachment) => attachment.kind !== "image" || !attachment.previewUrl);
  if (images.length === 0 && files.length === 0) return null;
  return (
    <div className="ai-chat-message-attachments" aria-label={t("aiChat.attachments.aria")}>
      {images.length > 0 && (
        <div className="ai-chat-message-image-grid">
          {images.map((attachment) => (
            <figure className="ai-chat-message-image" key={attachment.id}>
              <a href={attachment.previewUrl} target="_blank" rel="noreferrer noopener" title={attachment.name}>
                <img src={attachment.previewUrl} alt={attachment.name} loading="lazy" decoding="async" draggable={false} />
              </a>
              <figcaption title={attachment.name}>{attachment.name}</figcaption>
            </figure>
          ))}
        </div>
      )}
      {files.length > 0 && (
        <ul className="ai-chat-message-file-list">
          {files.map((attachment) => (
            <li key={attachment.id} title={attachment.name}>
              <span>{attachment.name}</span>
              <small>{formatAttachmentSize(attachment.size, t)}</small>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function formatAttachmentSize(bytes: number, t: TranslateFn) {
  if (bytes < 1024) return t("common.fileSize.bytes", { bytes });
  const kilobytes = bytes / 1024;
  if (kilobytes < 1024) return t("common.fileSize.kilobytes", { kilobytes: kilobytes >= 10 ? kilobytes.toFixed(0) : kilobytes.toFixed(1) });
  const megabytes = kilobytes / 1024;
  return t("common.fileSize.megabytes", { megabytes: megabytes >= 10 ? megabytes.toFixed(0) : megabytes.toFixed(1) });
}

function AspectorReasoningBlock({ text, streaming, t }: {
  text: string;
  streaming: boolean;
  t: TranslateFn;
}) {
  const [userToggled, setUserToggled] = useState<boolean | null>(null);
  // Open only while this block is actively thinking. Once thinking finishes it
  // auto-collapses to a single quiet "Thought for Ns" line (a finished thought
  // shouldn't stay expanded just because the final answer hasn't arrived yet —
  // e.g. while tools are still running).
  const autoOpen = streaming;
  const open = userToggled ?? autoOpen;
  // Live thinking timer: counts up while this block streams, freezes the total when
  // thinking ends ("thought for Ns"). Self-contained, so the row's memo bail-out
  // doesn't stop the tick. A block loaded from history (never streamed in this
  // session) has no timing data — it falls back to the generic "Thought process".
  const elapsedMs = useElapsedSeconds(streaming);
  const hasElapsed = streaming || elapsedMs > 0;
  const label = streaming
    ? t("aiChat.reasoning.thinking")
    : hasElapsed
      ? t("aiChat.reasoning.thoughtFor", { seconds: formatThinkingElapsed(elapsedMs) })
      : t("aiChat.reasoning.thought");
  useEffect(() => {
    if (streaming) setUserToggled(null);
  }, [streaming]);
  return (
    <div className="ai-reasoning" data-open={open} data-streaming={streaming}>
      <button type="button" className="ai-reasoning-header" onClick={() => setUserToggled(!open)} aria-expanded={open}>
        <ChevronRight className="ai-reasoning-caret" size={13} />
        <span className="ai-reasoning-label">{label}</span>
        {streaming && (
          <span className="ai-reasoning-elapsed" data-streaming="true">
            {t("aiChat.reasoning.elapsed", { seconds: formatThinkingElapsed(elapsedMs) })}
          </span>
        )}
      </button>
      {open && (
        <div className="ai-reasoning-body">
          <MarkdownMessage content={text} t={t} />
        </div>
      )}
    </div>
  );
}

function MarkdownMessage({ content, t }: { content: string; t: TranslateFn }) {
  const safeContent = coerceChatMessageText(content);
  const [expanded, setExpanded] = useState(safeContent.length <= 30_000);
  const normalizedContent = useMemo(() => trimChatMessageEnd(decodeChatDisplayText(safeContent)), [safeContent]);
  const visibleContent = expanded ? normalizedContent : truncateMiddleForPreview(normalizedContent, 18_000, t);
  const streaming = useContext(MarkdownStreamingContext);
  const smoothStream = useContext(MarkdownSmoothStreamContext);
  // Smooth mode: animate the visible cursor toward the latest text (adaptive
  // typewriter) — its per-frame paints replace the chunk throttle entirely.
  const revealedContent = useSmoothRevealText(visibleContent, streaming && smoothStream);
  // While tokens stream in at ~30-60/sec, re-lexing the growing markdown every
  // time is the dominant per-token CPU cost on longer answers (O(n²) over the
  // reply). With smoothing off, throttle the lexer input during streaming: the
  // preview lags by at most ~1 frame of text, which is imperceptible for
  // reading-speed text, and once the response settles the full text is lexed
  // immediately.
  const throttledVisible = useThrottledWhileStreaming(revealedContent, streaming && !smoothStream, 160);
  const tokens = useMemo(() => tokenizeChatMarkdown(throttledVisible), [throttledVisible]);
  return (
    <div className="ai-chat-message-content ai-chat-markdown" data-collapsed={!expanded || undefined}>
      {renderMarkdownBlocks(tokens, t)}
      {!expanded && (
        <button type="button" className="ai-chat-expand-message" onClick={() => setExpanded(true)}>
          {t("aiChat.message.showFull")}
        </button>
      )}
    </div>
  );
}

function renderMarkdownBlocks(tokens: Token[], t: TranslateFn, keyPrefix = "md"): ReactNode[] {
  return tokens.map((token, index) => renderMarkdownBlock(token, `${keyPrefix}-${index}`, t)).filter(Boolean);
}

function renderMarkdownBlock(token: Token, key: string, t: TranslateFn): ReactNode {
  switch (token.type) {
    case "space":
    case "def":
      return null;
    case "hr":
      return <hr key={key} />;
    case "heading":
      return renderMarkdownHeading(token as Tokens.Heading, key);
    case "paragraph":
      return <p key={key}>{renderMarkdownInlines((token as Tokens.Paragraph).tokens, (token as Tokens.Paragraph).text, key)}</p>;
    case "text":
      return <p key={key}>{renderMarkdownInlines(isTokenWithChildren(token) ? token.tokens : undefined, textFromToken(token), key)}</p>;
    case "code":
      return <MarkdownCodeBlock key={key} language={(token as Tokens.Code).lang} code={(token as Tokens.Code).text} t={t} />;
    case "blockquote":
      return <blockquote key={key}>{renderMarkdownBlocks((token as Tokens.Blockquote).tokens, t, key)}</blockquote>;
    case "list":
      return renderMarkdownList(token as Tokens.List, key, t);
    case "table":
      return renderMarkdownTable(token as Tokens.Table, key);
    case "html":
      return <p key={key}>{textFromToken(token)}</p>;
    default:
      return isTokenWithChildren(token)
        ? <Fragment key={key}>{renderMarkdownBlocks(token.tokens, t, key)}</Fragment>
        : <p key={key}>{textFromToken(token)}</p>;
  }
}

function renderMarkdownHeading(token: Tokens.Heading, key: string) {
  const children = renderMarkdownInlines(token.tokens, token.text, key);
  const depth = Math.min(Math.max(token.depth, 1), 6);
  if (depth === 1) return <h1 key={key}>{children}</h1>;
  if (depth === 2) return <h2 key={key}>{children}</h2>;
  if (depth === 3) return <h3 key={key}>{children}</h3>;
  if (depth === 4) return <h4 key={key}>{children}</h4>;
  if (depth === 5) return <h5 key={key}>{children}</h5>;
  return <h6 key={key}>{children}</h6>;
}

function renderMarkdownList(token: Tokens.List, key: string, t: TranslateFn) {
  const children = token.items.map((item, index) => (
    <li key={`${key}-${index}`} data-task={item.task || undefined}>
      {item.task && <input aria-label={t("aiChat.markdown.taskItem")} type="checkbox" checked={Boolean(item.checked)} disabled readOnly />}
      <div className="ai-chat-list-item-content">{renderMarkdownBlocks(item.tokens, t, `${key}-${index}`)}</div>
    </li>
  ));
  return token.ordered
    ? <ol key={key} start={typeof token.start === "number" ? token.start : undefined}>{children}</ol>
    : <ul key={key}>{children}</ul>;
}

function renderMarkdownTable(token: Tokens.Table, key: string) {
  return (
    <div className="ai-chat-table-wrap" key={key}>
      <table>
        <thead>
          <tr>
            {token.header.map((cell, index) => <th key={`${key}-h-${index}`} style={markdownCellStyle(cell)}>{renderMarkdownInlines(cell.tokens, cell.text, `${key}-h-${index}`)}</th>)}
          </tr>
        </thead>
        <tbody>
          {token.rows.map((row, rowIndex) => (
            <tr key={`${key}-r-${rowIndex}`}>
              {row.map((cell, cellIndex) => <td key={`${key}-r-${rowIndex}-${cellIndex}`} style={markdownCellStyle(cell)}>{renderMarkdownInlines(cell.tokens, cell.text, `${key}-r-${rowIndex}-${cellIndex}`)}</td>)}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function MarkdownCodeBlock({ code, language, t }: { code: string; language?: string; t: TranslateFn }) {
  const trimmedLanguage = language?.trim();
  const streaming = useContext(MarkdownStreamingContext);
  const artifact = parseHtmlArtifactLang(trimmedLanguage);
  if (artifact) {
    // Live HTML/3D artifact (sandboxed). `html preview`/`html live` auto-render the
    // preview; a bare `html` fence opens the code with a one-click Preview tab.
    return (
      <HtmlArtifact
        html={code}
        autoPreview={artifact.autoPreview}
        settled={!streaming}
        t={t}
      />
    );
  }
  return (
    <pre className="ai-chat-code-block" data-language={trimmedLanguage || undefined}>
      <button type="button" aria-label={t("aiChat.markdown.copyCode")} title={t("aiChat.markdown.copyCode")} onClick={() => void copyMarkdownCode(code)}>
        <Copy size={12} />
      </button>
      <code>{code}</code>
    </pre>
  );
}

/**
 * Detect a fenced HTML artifact from a code block's info-string. `html` / `htm`
 * qualify; an explicit `preview`/`live`/`run` modifier (e.g. ```html preview)
 * auto-opens the live sandbox, while a bare ```html opens code-first with a
 * one-click Preview tab. Returns null for any non-HTML language.
 */
export function parseHtmlArtifactLang(language: string | undefined): { autoPreview: boolean } | null {
  if (!language) return null;
  const parts = language.toLowerCase().split(/[\s,]+/).filter(Boolean);
  if (parts.length === 0) return null;
  if (parts[0] !== "html" && parts[0] !== "htm") return null;
  const autoPreview = parts.slice(1).some((p) => p === "preview" || p === "live" || p === "run");
  return { autoPreview };
}

function copyMarkdownCode(code: string) {
  const clipboard = navigator.clipboard;
  if (!clipboard) return Promise.resolve();
  return clipboard.writeText(code).catch(() => undefined);
}

function tokenizeChatMarkdown(content: string): Token[] {
  try {
    return lexer(content, { breaks: true, gfm: true });
  } catch {
    return [{ type: "paragraph", raw: content, text: content, tokens: [{ type: "text", raw: content, text: content }] } as Tokens.Paragraph];
  }
}

function renderMarkdownInlines(tokens: Token[] | undefined, fallback: string, keyPrefix: string): ReactNode[] {
  if (!tokens || tokens.length === 0) return [decodeChatDisplayText(coerceChatMessageText(fallback))];
  return tokens.map((token, index) => renderMarkdownInline(token, `${keyPrefix}-i-${index}`));
}

function renderMarkdownInline(token: Token, key: string): ReactNode {
  switch (token.type) {
    case "text":
    case "escape":
      return isTokenWithChildren(token)
        ? <Fragment key={key}>{renderMarkdownInlines(token.tokens, textFromToken(token), key)}</Fragment>
        : <Fragment key={key}>{textFromToken(token)}</Fragment>;
    case "strong":
      return <strong key={key}>{renderMarkdownInlines((token as Tokens.Strong).tokens, (token as Tokens.Strong).text, key)}</strong>;
    case "em":
      return <em key={key}>{renderMarkdownInlines((token as Tokens.Em).tokens, (token as Tokens.Em).text, key)}</em>;
    case "codespan":
      return <code className="ai-chat-inline-code" key={key}>{(token as Tokens.Codespan).text}</code>;
    case "br":
      return <br key={key} />;
    case "del":
      return <del key={key}>{renderMarkdownInlines((token as Tokens.Del).tokens, (token as Tokens.Del).text, key)}</del>;
    case "link":
      return renderMarkdownLink(token as Tokens.Link, key);
    case "image":
      return renderMarkdownImage(token as Tokens.Image, key);
    case "html":
      return <Fragment key={key}>{textFromToken(token)}</Fragment>;
    default:
      return isTokenWithChildren(token)
        ? <Fragment key={key}>{renderMarkdownInlines(token.tokens, textFromToken(token), key)}</Fragment>
        : <Fragment key={key}>{textFromToken(token)}</Fragment>;
  }
}

function isTokenWithChildren(token: Token): token is Token & { tokens: Token[] } {
  return "tokens" in token && Array.isArray(token.tokens);
}

function textFromToken(token: Token) {
  const raw = "text" in token && typeof token.text === "string"
    ? token.text
    : typeof token.raw === "string"
      ? token.raw
      : "";
  return decodeChatDisplayText(raw);
}

function renderMarkdownLink(token: Tokens.Link, key: string) {
  const href = safeMarkdownHref(token.href);
  const children = renderMarkdownInlines(token.tokens, token.text, key);
  if (!href) return <span className="ai-chat-link-disabled" key={key}>{children}</span>;
  return <a key={key} href={href} title={token.title ?? undefined} target={href.startsWith("#") ? undefined : "_blank"} rel={href.startsWith("#") ? undefined : "noreferrer noopener"}>{children}</a>;
}

function renderMarkdownImage(token: Tokens.Image, key: string) {
  const href = safeMarkdownHref(token.href);
  const label = token.text.trim() || token.href;
  if (!href) return <span className="ai-chat-link-disabled" key={key}>{label}</span>;
  return <a className="ai-chat-image-link" key={key} href={href} title={token.title ?? undefined} target="_blank" rel="noreferrer noopener">{label}</a>;
}

function safeMarkdownHref(href: string) {
  const trimmed = href.trim();
  if (trimmed.startsWith("#")) return trimmed;
  try {
    const url = new URL(trimmed);
    return url.protocol === "http:" || url.protocol === "https:" || url.protocol === "mailto:" ? trimmed : null;
  } catch {
    return null;
  }
}

function markdownCellStyle(cell: Tokens.TableCell): CSSProperties | undefined {
  return cell.align ? { textAlign: cell.align } : undefined;
}

function formatCompactionPreview(content: string) {
  const lines = decodeChatDisplayText(coerceChatMessageText(content)).split("\n");
  const bodyStart = lines.findIndex((line) => line.trim() && !line.startsWith("[Aspect") && !line.startsWith("covered_messages=") && !line.startsWith("Continue "));
  const body = bodyStart >= 0 ? lines.slice(bodyStart).join("\n").trim() : content.trim();
  return body.length > 4_000 ? `${body.slice(0, 4_000)}\n…` : body;
}

function truncateMiddleForPreview(content: string, maxChars: number, t: TranslateFn) {
  if (content.length <= maxChars) return content;
  const head = Math.floor(maxChars * 0.68);
  const tail = Math.max(1_000, maxChars - head);
  return `${content.slice(0, head)}\n\n...[${t("aiChat.message.previewTruncated", { chars: content.length - maxChars })}]...\n\n${content.slice(-tail)}`;
}

function formatMessageTime(timestamp: number) {
  return new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" }).format(timestamp);
}

/**
 * Returns `value` passthrough when `active` is false (the settled state). While
 * `active`, it syncs to the latest `value` but skips updates more frequent than
 * `intervalMs` — the interval-gated copy becomes the derived-state input for
 * expensive downstream computation (e.g. markdown lexing), while the true `value`
 * is still accumulated elsewhere for correctness. On deactivation the latest
 * `value` is emitted immediately, so the settled view is always complete.
 */
/**
 * Smooth streaming reveal. Instead of repainting the whole latest chunk (which
 * reads as jerky block-jumps when providers deliver large deltas), this animates
 * a visible-character cursor toward the real text length every animation frame:
 * a fast adaptive typewriter. The rate has a reading-speed floor and drains any
 * backlog proportionally (~1/4s to catch up), so it never lags a fast model.
 *
 * Mount shows the current text in full — only growth AFTER mount animates. That
 * way completed earlier segments (or rows remounted by virtualization) never
 * replay from empty; only the live tail types.
 *
 * Paint frequency is quantized by content size: short answers update every
 * frame, longer ones step down toward the plain 160ms throttle cadence, because
 * each paint re-lexes the whole markdown (the known O(n²) streaming cost).
 */
/** Reading-speed floor for the smooth reveal, characters per second. */
const SMOOTH_REVEAL_MIN_CPS = 360;
/** Proportional catch-up: fraction of the backlog revealed per second (4 ≈ drain in 250ms). */
const SMOOTH_REVEAL_CATCHUP = 4;

function useSmoothRevealText(text: string, active: boolean): string {
  const [visible, setVisible] = useState(text.length);
  const visibleRef = useRef(visible);
  const targetRef = useRef(text.length);
  targetRef.current = text.length;

  useEffect(() => {
    if (!active) {
      // Stream settled (or smoothing toggled off): show everything at once so
      // the final markdown is always complete.
      visibleRef.current = targetRef.current;
      setVisible(targetRef.current);
      return;
    }
    let raf = 0;
    let lastPaint = 0;
    const step = (now: number) => {
      raf = requestAnimationFrame(step);
      const target = targetRef.current;
      // Content replaced/normalized shorter mid-stream: snap down, never slice
      // beyond the string.
      if (visibleRef.current > target) {
        visibleRef.current = target;
        lastPaint = now;
        setVisible(target);
        return;
      }
      const backlog = target - visibleRef.current;
      if (backlog <= 0) {
        // Idle (waiting for the next delta): keep the paint clock fresh so a
        // burst after a pause animates from one frame's dt, not the whole wait.
        lastPaint = now;
        return;
      }
      // Larger documents re-lex more expensively per paint — widen the paint
      // interval as the answer grows (60fps → ~30fps → the old 160ms cadence).
      const paintInterval = target <= 4_000 ? 0 : target <= 12_000 ? 66 : 160;
      if (now - lastPaint < paintInterval) return;
      // dt spans everything since the LAST PAINT so gated frames still count —
      // the reveal rate is cadence-independent.
      const dt = lastPaint === 0 ? 1 / 60 : Math.min((now - lastPaint) / 1000, 0.25);
      lastPaint = now;
      const rate = Math.max(SMOOTH_REVEAL_MIN_CPS, backlog * SMOOTH_REVEAL_CATCHUP);
      const advance = Math.max(1, Math.round(rate * dt));
      visibleRef.current = Math.min(target, visibleRef.current + advance);
      setVisible(visibleRef.current);
    };
    raf = requestAnimationFrame(step);
    return () => cancelAnimationFrame(raf);
  }, [active]);

  if (!active) return text;
  let cut = Math.min(visible, text.length);
  // Never split a surrogate pair (emoji/CJK-ext): a lone high surrogate renders
  // as a broken glyph for a frame.
  if (cut > 0 && cut < text.length) {
    const code = text.charCodeAt(cut - 1);
    if (code >= 0xd800 && code <= 0xdbff) cut -= 1;
  }
  return text.slice(0, cut);
}

function useThrottledWhileStreaming(value: string, active: boolean, intervalMs: number): string {
  const [throttled, setThrottled] = useState(value);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const latestRef = useRef(value);
  latestRef.current = value;

  useEffect(() => {
    if (!active) {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      setThrottled(value);
      return;
    }
    if (timerRef.current !== null) return; // already waiting
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      setThrottled(latestRef.current);
    }, intervalMs);
    return () => {
      if (timerRef.current !== null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [active, intervalMs, value]);

  return active ? throttled : value;
}
