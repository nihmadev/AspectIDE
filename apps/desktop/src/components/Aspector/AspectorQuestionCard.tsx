import { Check, CornerDownLeft, MessageCircleQuestion, X } from "lucide-react";
import { useMemo, useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import type { PendingQuestion } from "../../lib/aspector/utils/pending-question";
import { HtmlArtifact } from "./HtmlArtifact";

type AspectorQuestionCardProps = {
  question: PendingQuestion;
  /** Deliver the final answer text back into the suspended turn loop. */
  onAnswer: (answer: string) => void;
  /** Dismiss without answering (the model proceeds with its own judgment). */
  onDismiss: () => void;
  t: TranslateFn;
};

/**
 * Interactive `AskUser` card. Renders the question, 0–10 suggested options, an
 * optional sandboxed HTML5 preview, and a free-form custom-answer field.
 *
 * Selection model (uniform): clicking an option only SELECTS it — it never
 * auto-submits. Options always toggle, so the user can pick one or several, and
 * can additionally type a custom answer; all picks plus the custom text are joined
 * and delivered together when the user explicitly presses Send (or ⌘/Ctrl+Enter).
 * `multiSelect` is the model's hint; the UI always allows multiple so the human
 * stays in control.
 */
export function AspectorQuestionCard({ question, onAnswer, onDismiss, t }: AspectorQuestionCardProps) {
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [custom, setCustom] = useState("");

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

  // Click only toggles selection — submission is always an explicit Send. Multiple
  // options can be selected regardless of the model's single/multi hint.
  const toggleOption = (index: number) => {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
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
          <HtmlArtifact
            html={question.htmlPreview}
            title={t("aiChat.question.previewLabel")}
            autoPreview
            t={t}
          />
        </div>
      )}

      {hasOptions && (
        <ul className="ai-question-options" data-multi>
          {question.options.map((option, index) => {
            const isSelected = selected.has(index);
            return (
              <li key={`${option.label}-${index}`}>
                <button
                  type="button"
                  className="ai-question-option"
                  data-selected={isSelected || undefined}
                  aria-pressed={isSelected}
                  onClick={() => toggleOption(index)}
                >
                  <span className="ai-question-option-marker" data-multi aria-hidden="true">
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

      {(hasOptions || allowCustom) && (
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
