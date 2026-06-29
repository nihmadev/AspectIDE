export type AiSessionTodoStatus = "pending" | "in_progress" | "completed" | "blocked" | "cancelled";

export type AiSessionTodoSource = "agent" | "user";

import { filterPollutedSessionTodos } from "./aiSessionOrchestrationSanitize";

export type AiSessionTodo = {
  id: string;
  content: string;
  status: AiSessionTodoStatus;
  priority: "low" | "medium" | "high";
  source: AiSessionTodoSource;
  linkedFilePath?: string;
  notes?: string;
};

const todosBySession = new Map<string, AiSessionTodo[]>();
const listeners = new Set<() => void>();

// Monotonic revision counter — incremented on every mutation so useSyncExternalStore
// subscribers see a fresh snapshot value even when the underlying Map identity (and
// size) is unchanged, e.g., a single todo status change within an existing session.
let todosRevision = 0;

function emit() {
  todosRevision += 1;
  for (const listener of listeners) listener();
}

export function subscribeAiSessionTodos(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Returns the monotonic revision so useSyncExternalStore detects every mutation. */
export function getAiSessionTodosSnapshot() {
  return todosRevision;
}

export function listAiSessionTodos(sessionId: string) {
  const todos = todosBySession.get(sessionId) ?? [];
  return filterPollutedSessionTodos(todos);
}

export function replaceAiSessionTodos(sessionId: string, todos: AiSessionTodo[]) {
  const normalized = todos.map((todo) => normalizeTodo({ ...todo, source: "agent" }));
  todosBySession.set(sessionId, filterPollutedSessionTodos(normalized));
  emit();
}

export function linkAiSessionTodoToFile(sessionId: string, todoId: string, filePath: string | undefined) {
  const todos = listAiSessionTodos(sessionId);
  const index = todos.findIndex((todo) => todo.id === todoId);
  if (index < 0) return false;
  todos[index] = { ...todos[index], linkedFilePath: filePath?.trim() || undefined };
  todosBySession.set(sessionId, todos);
  emit();
  return true;
}

export function clearAiSessionTodos(sessionId: string) {
  todosBySession.delete(sessionId);
  emit();
}

export function hydrateAiSessionTodos(sessionId: string, todos: AiSessionTodo[]) {
  todosBySession.set(sessionId, todos.map(normalizeTodo));
  emit();
}

export function hydrateAllAiSessionTodos(sessions: Array<{ id: string; sessionTodos?: AiSessionTodo[] }>) {
  todosBySession.clear();
  for (const session of sessions) {
    if (session.sessionTodos && session.sessionTodos.length > 0) {
      todosBySession.set(session.id, session.sessionTodos.map(normalizeTodo));
    }
  }
  emit();
}

function normalizeTodo(todo: AiSessionTodo): AiSessionTodo {
  return {
    ...todo,
    source: todo.source === "user" ? "user" : "agent",
  };
}