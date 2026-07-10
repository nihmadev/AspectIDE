import { extractInlineMentionSpans } from '../../lib/aspector/chat/composer-inline-mentions';

type AspectorComposerInlineMentionsProps = {
  message: string;
};

export function AspectorComposerInlineMentions({ message }: AspectorComposerInlineMentionsProps) {
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