import { Check, Circle, Loader2 } from "lucide-react";
import { useSyncExternalStore } from "react";
import { getAiSessionTodosSnapshot, listAiSessionTodos, subscribeAiSessionTodos } from "../../lib/aiSessionTodos";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

type AiAutomaticChecklistProps = {
  sessionId: string;
  agentMode: string;
  t: TranslateFn;
};

export function AiAutomaticChecklist({ sessionId, agentMode, t }: AiAutomaticChecklistProps) {
  useSyncExternalStore(subscribeAiSessionTodos, getAiSessionTodosSnapshot, getAiSessionTodosSnapshot);
  const todos = listAiSessionTodos(sessionId);
  if (agentMode !== "automatic" || todos.length === 0) return null;

  const completed = todos.filter((todo) => todo.status === "completed").length;

  return (
    <div className="ai-automatic-checklist" aria-label={t("aiChat.automatic.checklistAria")}>
      <header>
        <strong>{t("aiChat.automatic.checklistTitle")}</strong>
        <span>{t("aiChat.automatic.checklistProgress", { done: completed, total: todos.length })}</span>
      </header>
      <ul>
        {todos.map((todo) => (
          <li key={todo.id} data-status={todo.status}>
            <span className="ai-automatic-checklist-glyph" aria-hidden="true">
              {todo.status === "completed" ? <Check size={13} /> : todo.status === "in_progress" ? <Loader2 size={13} className="spin-icon" /> : <Circle size={13} />}
            </span>
            <span>{todo.content}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}