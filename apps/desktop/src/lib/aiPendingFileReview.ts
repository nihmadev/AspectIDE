import { applyAcceptedHunks, buildFileDiffHunks, type FileDiffHunk } from "./aiFileDiffHunks";
import { linkAiSessionTodoToFile, listAiSessionTodos } from "./aiSessionTodos";
import { luxCommands } from "./tauri";

export type PendingFileReviewStatus = "pending" | "accepted" | "rejected";

/** Compact shape stored on chat sessions across restarts. */
export type PersistedPendingFileReview = {
  id: string;
  sessionId: string;
  path: string;
  relativePath: string;
  toolName: string;
  toolCallId: string;
  beforeText: string;
  afterText: string;
  previewOnly: boolean;
  acceptedHunkIds: string[];
  createdAt: number;
  status: PendingFileReviewStatus;
};

export type PendingFileReview = {
  id: string;
  sessionId: string;
  path: string;
  relativePath: string;
  toolName: string;
  toolCallId: string;
  beforeText: string;
  afterText: string;
  hunks: FileDiffHunk[];
  /** When true, disk was not written yet — accept must persist afterText. */
  previewOnly: boolean;
  acceptedHunkIds: string[];
  createdAt: number;
  status: PendingFileReviewStatus;
};

type PendingFileReviewListener = () => void;

let reviews: PendingFileReview[] = [];
const listeners = new Set<PendingFileReviewListener>();

export function subscribePendingFileReviews(listener: PendingFileReviewListener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function getPendingFileReviewsSnapshot() {
  return reviews;
}

export function findPendingFileReviewByToolCallId(toolCallId: string) {
  return reviews.find((review) => review.toolCallId === toolCallId && review.status === "pending") ?? null;
}

export function listPendingFileReviewsForPath(path: string) {
  const normalized = normalizePathKey(path);
  return reviews.filter((review) => review.status === "pending" && normalizePathKey(review.path) === normalized);
}

export function listPendingFileReviewsForSession(sessionId: string) {
  return reviews.filter((review) => review.status === "pending" && review.sessionId === sessionId);
}

export function hydratePendingFileReviews(entries: PersistedPendingFileReview[]) {
  reviews = entries.map((entry) => ({
    ...entry,
    hunks: buildFileDiffHunks(entry.beforeText, entry.afterText),
  }));
  emit();
}

export function exportPendingFileReviewsForPersistence(maxTextChars = 12_000): PersistedPendingFileReview[] {
  return reviews
    .filter((review) => review.status === "pending")
    .map((review) => ({
      id: review.id,
      sessionId: review.sessionId,
      path: review.path,
      relativePath: review.relativePath,
      toolName: review.toolName,
      toolCallId: review.toolCallId,
      beforeText: truncateForPersist(review.beforeText, maxTextChars),
      afterText: truncateForPersist(review.afterText, maxTextChars),
      previewOnly: review.previewOnly,
      acceptedHunkIds: review.acceptedHunkIds,
      createdAt: review.createdAt,
      status: review.status,
    }));
}

export function registerPendingFileReview(input: Omit<PendingFileReview, "id" | "createdAt" | "status" | "hunks" | "acceptedHunkIds"> & {
  hunks?: FileDiffHunk[];
  acceptedHunkIds?: string[];
}) {
  const normalized = normalizePathKey(input.path);
  reviews = reviews.filter((review) => !(
    review.status === "pending"
    && review.sessionId === input.sessionId
    && normalizePathKey(review.path) === normalized
  ));
  const hunks = input.hunks ?? buildFileDiffHunks(input.beforeText, input.afterText);
  const entry: PendingFileReview = {
    ...input,
    hunks,
    acceptedHunkIds: input.acceptedHunkIds ?? hunks.map((hunk) => hunk.id),
    id: `review-${Date.now().toString(36)}-${crypto.randomUUID().slice(0, 8)}`,
    createdAt: Date.now(),
    status: "pending",
  };
  reviews = [entry, ...reviews].slice(0, 48);
  const todos = listAiSessionTodos(input.sessionId);
  const activeTodo = todos.find((todo) => todo.status === "in_progress") ?? todos.find((todo) => todo.status === "pending");
  if (activeTodo && !activeTodo.linkedFilePath) {
    linkAiSessionTodoToFile(input.sessionId, activeTodo.id, input.path);
  }
  emit();
  return entry;
}

export async function acceptPendingFileReview(reviewId: string) {
  const review = reviews.find((entry) => entry.id === reviewId);
  if (!review || review.status !== "pending") return;
  const mergedText = applyAcceptedHunks(review.beforeText, review.afterText, new Set(review.acceptedHunkIds));
  if (review.previewOnly || mergedText !== review.afterText) {
    await luxCommands.aiFilePatch([{
      action: "rewrite",
      path: review.path,
      text: mergedText,
      overwrite: true,
    }], true, false);
  }
  reviews = reviews.filter((entry) => entry.id !== reviewId);
  emit();
}

export async function acceptPendingFileReviewHunk(reviewId: string, hunkId: string) {
  const review = reviews.find((entry) => entry.id === reviewId);
  if (!review || review.status !== "pending") return;
  if (!review.acceptedHunkIds.includes(hunkId)) {
    reviews = reviews.map((entry) => entry.id === reviewId
      ? { ...entry, acceptedHunkIds: [...entry.acceptedHunkIds, hunkId] }
      : entry);
    emit();
  }
}

export async function rejectPendingFileReviewHunk(reviewId: string, hunkId: string) {
  const review = reviews.find((entry) => entry.id === reviewId);
  if (!review || review.status !== "pending") return;
  reviews = reviews.map((entry) => entry.id === reviewId
    ? { ...entry, acceptedHunkIds: entry.acceptedHunkIds.filter((id) => id !== hunkId) }
    : entry);
  emit();
}

export async function rejectPendingFileReview(reviewId: string) {
  const review = reviews.find((entry) => entry.id === reviewId);
  if (!review || review.status !== "pending") return;
  await luxCommands.aiFilePatch([{
    action: "rewrite",
    path: review.path,
    text: review.beforeText,
    overwrite: true,
  }], true, false);
  reviews = reviews.filter((entry) => entry.id !== reviewId);
  emit();
}

export function removePendingFileReviewsForSession(sessionId: string) {
  const before = reviews.length;
  reviews = reviews.filter((review) => review.sessionId !== sessionId);
  if (reviews.length !== before) emit();
}

export async function acceptAllPendingFileReviews(sessionId?: string) {
  const pending = sessionId
    ? reviews.filter((review) => review.sessionId === sessionId && review.status === "pending")
    : reviews.filter((review) => review.status === "pending");
  for (const review of pending) {
    await acceptPendingFileReview(review.id);
  }
}

export async function rejectAllPendingFileReviews(sessionId?: string) {
  const pending = sessionId
    ? [...reviews].filter((review) => review.sessionId === sessionId && review.status === "pending")
    : [...reviews].filter((review) => review.status === "pending");
  for (const review of pending) {
    await rejectPendingFileReview(review.id);
  }
}

export async function captureFileTextSnapshot(path: string, openText?: string) {
  if (typeof openText === "string") return openText;
  try {
    const response = await luxCommands.fsReadText(path, 500_000);
    return response.text ?? "";
  } catch {
    return "";
  }
}

function emit() {
  for (const listener of listeners) listener();
}

function normalizePathKey(path: string) {
  return path.replace(/\\/g, "/").toLowerCase();
}

function truncateForPersist(text: string, maxChars: number) {
  if (text.length <= maxChars) return text;
  return `${text.slice(0, maxChars)}\n...[truncated for session persist]`;
}