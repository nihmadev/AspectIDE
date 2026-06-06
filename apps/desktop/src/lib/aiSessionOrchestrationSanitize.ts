const maxSessionGoalChars = 240;
const maxTodoItems = 20;
const maxTodoContentChars = 200;

const reviewSectionPattern =
  /^(?:what was done well|all issues(?:\s*\/\s*bugs(?:\s*\/\s*gaps)?)?(?:\s+found)?|precise fixes(?:\s+and\s+improvements)?(?:\s*\([^)]*\))?|done\.?)$/i;

const reviewNoisePattern =
  /\b(quoted exactly|issues\s*\/\s*bugs|paranoid|obsessive|maniacal|three-section)\b/i;

const reviewFindingPattern =
  /^(?:no |missing |lack of |the |plain-text|tone |not |never |without |should |must |do not )/i;

export type OrchestrationSanitizeResult<T> =
  | { ok: true; value: T }
  | { ok: false; reason: string };

export function sanitizeSessionGoal(goal: string): OrchestrationSanitizeResult<string> {
  const trimmed = goal.trim();
  if (!trimmed) return { ok: true, value: "" };
  if (trimmed.length > maxSessionGoalChars) {
    return { ok: false, reason: `Goal exceeds ${maxSessionGoalChars} characters.` };
  }
  if (looksLikeMarkdownHeading(trimmed)) {
    return { ok: false, reason: "Goal must describe session intent, not a markdown heading." };
  }
  if (reviewSectionPattern.test(trimmed) || reviewNoisePattern.test(trimmed)) {
    return { ok: false, reason: "Goal must not be a review section title or checklist label." };
  }
  if ((trimmed.match(/\|/g)?.length ?? 0) >= 3) {
    return { ok: false, reason: "Goal must not be table or checklist content." };
  }
  return { ok: true, value: trimmed };
}

export function isPollutedSessionGoal(goal: string | undefined): boolean {
  if (!goal?.trim()) return false;
  return sanitizeSessionGoal(goal).ok === false;
}

export function sanitizeSessionTodos(
  todos: Array<{ id: string; content: string; status: string; priority: string; notes?: string }>,
): OrchestrationSanitizeResult<Array<{ id: string; content: string; status: string; priority: string; notes?: string }>> {
  if (todos.length === 0) return { ok: false, reason: "TodoWrite requires at least one valid todo item." };
  const seen = new Set<string>();
  const accepted: Array<{ id: string; content: string; status: string; priority: string; notes?: string }> = [];
  let rejected = 0;

  for (const todo of todos) {
    const content = todo.content.trim();
    if (!content) {
      rejected += 1;
      continue;
    }
    if (content.length > maxTodoContentChars) {
      rejected += 1;
      continue;
    }
    if (looksLikeMarkdownHeading(content) || reviewSectionPattern.test(content) || reviewFindingPattern.test(content)) {
      rejected += 1;
      continue;
    }
    const key = content.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    accepted.push({ ...todo, content });
    if (accepted.length >= maxTodoItems) break;
  }

  if (accepted.length === 0) {
    return { ok: false, reason: "No valid todos after filtering review headings and duplicates." };
  }
  if (rejected > 0 && accepted.length < Math.min(3, todos.length)) {
    return { ok: false, reason: "Todo list looked like review output, not actionable tasks." };
  }
  return { ok: true, value: accepted };
}

function looksLikeMarkdownHeading(text: string) {
  if (/^#{1,6}\s/.test(text)) return true;
  if (/^\d+[.)]\s/.test(text)) return true;
  if (/^[-*+]\s/.test(text)) return true;
  return false;
}

export function filterPollutedSessionTodos<T extends { content: string }>(todos: T[]): T[] {
  if (todos.length === 0) return todos;
  const sanitized = sanitizeSessionTodos(
    todos.map((todo, index) => ({
      id: `todo-${index + 1}`,
      content: todo.content,
      status: "pending",
      priority: "medium",
    })),
  );
  if (!sanitized.ok) return [];
  const allowed = new Set(sanitized.value.map((todo) => todo.content));
  return todos.filter((todo) => allowed.has(todo.content.trim())).slice(0, maxTodoItems);
}