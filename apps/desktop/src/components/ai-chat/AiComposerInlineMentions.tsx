import { extractInlineMentionSpans } from "../../lib/aiChatComposerInlineMentions";

type AiComposerInlineMentionsProps = {
  message: string;
};

export function AiComposerInlineMentions({ message }: AiComposerInlineMentionsProps) {
  const spans = extractInlineMentionSpans(message);
  if (spans.length === 0) return null;
  return (
    <div className="ai-composer-inline-mentions" aria-hidden="true">
      {spans.map((span) => (
        <span key={`${span.start}-${span.label}`} className="ai-composer-inline-mention-chip">
          @{span.label}
        </span>
      ))}
    </div>
  );
}