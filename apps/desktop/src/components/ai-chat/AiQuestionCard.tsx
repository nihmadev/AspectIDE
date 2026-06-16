import { Check, CornerDownLeft, MessageCircleQuestion, Monitor, X } from "lucide-react";
import { useMemo, useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import type { PendingQuestion } from "../../lib/aiPendingQuestion";

type AiQuestionCardProps = {
  question: PendingQuestion;
  /** Deliver the final answer text back into the suspended turn loop. */
  onAnswer: (answer: string) => void;
  /** Dismiss without answering (the model proceeds with its own judgment). */
  onDismiss: () => void;
  t: TranslateFn;
};

/**
 * Interactive `AskUser` card. Renders the question, 0–10 suggested options
 * (single- or multi-select), an optional sandboxed HTML5 preview, and a
 * free-form custom-answer field.
 *
 * Submit paths:
 *  - Single-select: clicking an option sends it immediately (fast path), unless
 *    custom text is being typed — then it just highlights and the Send button
 *    combines the choice with the custom text.
 *  - Multi-select: options toggle; the Send button submits the joined labels
 *    plus any custom text.
 *  - Custom-only: type and Send (or ⌘/Ctrl+Enter).
 */
export function AiQuestionCard({ question, onAnswer, onDismiss, t }: AiQuestionCardProps) {
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [custom, setCustom] = useState("");
  const [previewOpen, setPreviewOpen] = useState(true);

  const hasOptions = question.options.length > 0;
  // Guard against a broken combination (no options AND custom disabled), which would
  // otherwise render a card with no way to answer. Always offer a custom field then.
  const allowCustom = question.allowCustom || !hasOptions;
  const trimmedCustom = custom.trim();

  const composedAnswer = useMemo(() => {
    const labels = [...selected]
      .sort((a, b) => a - b)
      .map((index) => question.options[index]?.label)
      .filter((label): label is string => Boolean(label));
    const parts = [...labels];
    if (trimmedCustom) parts.push(trimmedCustom);
    return parts.join("; ");
  }, [selected, trimmedCustom, question.options]);

  const canSend = composedAnswer.length > 0;

  const toggleOption = (index: number) => {
    if (question.multiSelect) {
      setSelected((prev) => {
        const next = new Set(prev);
        if (next.has(index)) next.delete(index);
        else next.add(index);
        return next;
      });
      return;
    }
    // Single-select fast path: send straight away when no custom text is staged.
    if (!trimmedCustom) {
      onAnswer(question.options[index].label);
      return;
    }
    setSelected(new Set([index]));
  };

  const handleCustomKeyDown = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && canSend) {
      event.preventDefault();
      onAnswer(composedAnswer);
    }
  };

  return (
    <article className="ai-question-card" role="group" aria-label={t("aiChat.question.aria")}>
      <header className="ai-question-card-head">
        <span className="ai-question-card-icon" aria-hidden="true">
          <MessageCircleQuestion size={15} />
        </span>
        <div className="ai-question-card-copy">
          <strong>{question.question}</strong>
          {question.detail && <p>{question.detail}</p>}
        </div>
        <button
          type="button"
          className="ai-question-card-dismiss"
          onClick={onDismiss}
          title={t("aiChat.question.dismiss")}
          aria-label={t("aiChat.question.dismiss")}
        >
          <X size={13} />
        </button>
      </header>

      {question.htmlPreview && (
        <div className="ai-question-preview">
          <button
            type="button"
            className="ai-question-preview-toggle"
            onClick={() => setPreviewOpen((open) => !open)}
            aria-expanded={previewOpen}
          >
            <Monitor size={12} />
            <span>{t("aiChat.question.previewLabel")}</span>
            <span className="ai-question-preview-hint">
              {previewOpen ? t("aiChat.question.previewHide") : t("aiChat.question.previewShow")}
            </span>
          </button>
          {previewOpen && (
            // Sandboxed with allow-scripts but NOT allow-same-origin: scripts run in an
            // opaque origin, so the previewed HTML5 can animate/interact yet cannot reach
            // the app's DOM, storage, or cookies. srcDoc auto-escapes the document.
            <iframe
              title={t("aiChat.question.previewLabel")}
              className="ai-question-preview-frame"
              srcDoc={question.htmlPreview}
              sandbox="allow-scripts"
              loading="lazy"
            />
          )}
        </div>
      )}

      {hasOptions && (
        <ul className="ai-question-options" data-multi={question.multiSelect || undefined}>
          {question.options.map((option, index) => {
            const isSelected = selected.has(index);
            return (
              <li key={`${option.label}-${index}`}>
                <button
                  type="button"
                  className="ai-question-option"
                  data-selected={isSelected || undefined}
                  onClick={() => toggleOption(index)}
                >
                  <span className="ai-question-option-marker" data-multi={question.multiSelect || undefined} aria-hidden="true">
                    {isSelected && <Check size={11} />}
                  </span>
                  <span className="ai-question-option-body">
                    <span className="ai-question-option-label">{option.label}</span>
                    {option.description && <span className="ai-question-option-desc">{option.description}</span>}
                  </span>
                </button>
              </li>
            );
          })}
        </ul>
      )}

      {allowCustom && (
        <div className="ai-question-custom">
          <textarea
            className="ai-question-custom-input"
            placeholder={hasOptions ? t("aiChat.question.customPlaceholderWithOptions") : t("aiChat.question.customPlaceholder")}
            value={custom}
            onChange={(event) => setCustom(event.target.value)}
            onKeyDown={handleCustomKeyDown}
            rows={2}
          />
        </div>
      )}

      {(question.multiSelect || allowCustom) && (
        <footer className="ai-question-card-foot">
          <span className="ai-question-card-tip">
            <CornerDownLeft size={11} />
            {t("aiChat.question.sendHint")}
          </span>
          <button
            type="button"
            className="ai-question-send"
            disabled={!canSend}
            onClick={() => onAnswer(composedAnswer)}
          >
            {t("aiChat.question.send")}
          </button>
        </footer>
      )}
    </article>
  );
}
