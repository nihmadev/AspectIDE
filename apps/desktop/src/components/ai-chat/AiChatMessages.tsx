import { Brain, ChevronRight, Copy } from "lucide-react";
import type { CSSProperties, ReactNode, RefObject } from "react";
import { Fragment, memo, useMemo, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { lexer, type Token, type Tokens } from "marked";
import { AiToolCallsGroup } from "../AiToolCall";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import type { AiChatMessage, AiMessageSegment, AiToolApprovalDecision } from "../../lib/aiChatTypes";

type AiChatMessagesProps = {
  messages: AiChatMessage[];
  onApprovalDecision: (approvalId: string, decision: AiToolApprovalDecision) => void;
  parentRef: RefObject<HTMLDivElement | null>;
  showResponseDuration: boolean;
  streamingMessageId: string | null;
  t: TranslateFn;
};

export function AiChatMessages({ messages, onApprovalDecision, parentRef, showResponseDuration, streamingMessageId, t }: AiChatMessagesProps) {
  const virtualizer = useVirtualizer({
    count: messages.length,
    getScrollElement: () => parentRef.current,
    estimateSize: () => 180,
    overscan: 5,
  });
  const virtualItems = virtualizer.getVirtualItems();

  if (messages.length < 40) {
    return messages.map((chatMessage) => (
      <AiChatMessageView
        key={chatMessage.id}
        message={chatMessage}
        streaming={chatMessage.id === streamingMessageId}
        showResponseDuration={showResponseDuration}
        onApprovalDecision={onApprovalDecision}
        t={t}
      />
    ));
  }

  return (
    <div className="ai-chat-virtual-list" style={{ height: virtualizer.getTotalSize() }}>
      {virtualItems.map((item) => {
        const chatMessage = messages[item.index];
        return (
          <div
            key={chatMessage.id}
            className="ai-chat-virtual-row"
            data-index={item.index}
            ref={virtualizer.measureElement}
            style={{ transform: `translateY(${item.start}px)` }}
          >
            <AiChatMessageView
              message={chatMessage}
              streaming={chatMessage.id === streamingMessageId}
              showResponseDuration={showResponseDuration}
              onApprovalDecision={onApprovalDecision}
              t={t}
            />
          </div>
        );
      })}
    </div>
  );
}

const AiChatMessageView = memo(function AiChatMessageView({ message, onApprovalDecision, showResponseDuration, streaming, t }: {
  message: AiChatMessage;
  onApprovalDecision: (approvalId: string, decision: AiToolApprovalDecision) => void;
  showResponseDuration: boolean;
  streaming: boolean;
  t: TranslateFn;
}) {
  return (
    <article className="ai-chat-message" data-role={message.role}>
      <div className="ai-chat-message-meta">
        <span>{message.role === "user" ? t("aiChat.role.user") : t("aiChat.role.assistant")}</span>
        <time>{formatMessageTime(message.timestamp)}</time>
      </div>
      <AiMessageBody
        message={message}
        streaming={streaming}
        onApprovalDecision={onApprovalDecision}
        t={t}
      />
      {message.role === "assistant" && showResponseDuration && typeof message.responseDurationMs === "number" && (
        <div className="ai-chat-response-duration">{formatResponseDuration(message.responseDurationMs, t)}</div>
      )}
    </article>
  );
});

function formatResponseDuration(durationMs: number, t: TranslateFn) {
  return t("aiChat.responseDuration", {
    milliseconds: durationMs,
    seconds: (durationMs / 1000).toFixed(durationMs >= 10_000 ? 1 : 2),
  });
}

function AiMessageBody({ message, streaming, onApprovalDecision, t }: {
  message: AiChatMessage;
  streaming: boolean;
  onApprovalDecision: (approvalId: string, decision: AiToolApprovalDecision) => void;
  t: TranslateFn;
}) {
  const segments = message.segments;
  if (!segments || segments.length === 0) {
    return (
      <>
        {message.reasoning && message.reasoning.trim().length > 0 && (
          <AiReasoningBlock text={message.reasoning} streaming={streaming} hasAnswer={Boolean(message.content?.trim())} t={t} />
        )}
        {message.toolCalls && message.toolCalls.length > 0 && <AiToolCallsGroup onApprovalDecision={onApprovalDecision} t={t} toolCalls={message.toolCalls} />}
        {message.content && <MarkdownMessage content={message.content} t={t} />}
      </>
    );
  }

  const blocks: ReactNode[] = [];
  let toolBatch: AiMessageSegment[] = [];
  const flushTools = (key: string) => {
    if (toolBatch.length === 0) return;
    const toolCalls = toolBatch.map((segment) => segment.kind === "tool" ? segment.toolCall : null).filter((call): call is NonNullable<typeof call> => Boolean(call));
    blocks.push(<AiToolCallsGroup key={`tools-${key}`} onApprovalDecision={onApprovalDecision} t={t} toolCalls={toolCalls} />);
    toolBatch = [];
  };

  segments.forEach((segment, index) => {
    if (segment.kind === "tool") {
      toolBatch.push(segment);
      return;
    }
    flushTools(`${index}`);
    if (segment.kind === "reasoning") {
      if (segment.text.trim().length === 0) return;
      const isLast = index === segments.length - 1;
      const followedByAnswer = segments.slice(index + 1).some((entry) => entry.kind === "text" && entry.text.trim().length > 0);
      blocks.push(
        <AiReasoningBlock key={segment.id} text={segment.text} streaming={streaming && isLast} hasAnswer={followedByAnswer} t={t} />,
      );
      return;
    }
    if (segment.text.trim().length === 0) return;
    blocks.push(<MarkdownMessage key={segment.id} content={segment.text} t={t} />);
  });
  flushTools("tail");

  return <>{blocks}</>;
}

function AiReasoningBlock({ text, streaming, hasAnswer, t }: {
  text: string;
  streaming: boolean;
  hasAnswer: boolean;
  t: TranslateFn;
}) {
  const [userToggled, setUserToggled] = useState<boolean | null>(null);
  const autoOpen = streaming || !hasAnswer;
  const open = userToggled ?? autoOpen;
  const label = streaming ? t("aiChat.reasoning.thinking") : t("aiChat.reasoning.thought");
  return (
    <div className="ai-reasoning" data-open={open} data-streaming={streaming}>
      <button type="button" className="ai-reasoning-header" onClick={() => setUserToggled(!open)} aria-expanded={open}>
        <ChevronRight className="ai-reasoning-caret" size={13} />
        <Brain size={13} />
        <span>{label}</span>
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
  const [expanded, setExpanded] = useState(content.length <= 30_000);
  const visibleContent = expanded ? content : truncateMiddleForPreview(content, 18_000, t);
  const tokens = useMemo(() => lexer(visibleContent, { breaks: true, gfm: true }), [visibleContent]);
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
  return (
    <pre className="ai-chat-code-block" data-language={trimmedLanguage || undefined}>
      <button type="button" aria-label={t("aiChat.markdown.copyCode")} title={t("aiChat.markdown.copyCode")} onClick={() => void copyMarkdownCode(code)}>
        <Copy size={12} />
      </button>
      <code>{code}</code>
    </pre>
  );
}

function copyMarkdownCode(code: string) {
  const clipboard = navigator.clipboard;
  if (!clipboard) return Promise.resolve();
  return clipboard.writeText(code).catch(() => undefined);
}

function renderMarkdownInlines(tokens: Token[] | undefined, fallback: string, keyPrefix: string): ReactNode[] {
  if (!tokens || tokens.length === 0) return [fallback];
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
  return "text" in token ? String(token.text) : token.raw;
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

function truncateMiddleForPreview(content: string, maxChars: number, t: TranslateFn) {
  if (content.length <= maxChars) return content;
  const head = Math.floor(maxChars * 0.68);
  const tail = Math.max(1_000, maxChars - head);
  return `${content.slice(0, head)}\n\n...[${t("aiChat.message.previewTruncated", { chars: content.length - maxChars })}]...\n\n${content.slice(-tail)}`;
}

function formatMessageTime(timestamp: number) {
  return new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" }).format(timestamp);
}
