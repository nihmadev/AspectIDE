import { normalizePathSlashes } from "./aiRuntimeShared";

export type PersistedFileCheckpointFile = {
  path: string;
  relativePath: string;
  existed: boolean;
  text: string;
  size: number;
  truncated: boolean;
  source: "editor" | "disk" | "missing";
  error?: string;
};

export type PersistedFileCheckpoint = {
  id: string;
  label: string;
  workspaceRoot: string;
  createdAt: string;
  maxBytesPerFile: number;
  files: PersistedFileCheckpointFile[];
};

export type PersistedTurnCheckpoint = {
  id: string;
  sessionId: string;
  userMessageId: string;
  label: string;
  createdAt: number;
  messageCount: number;
  fileCheckpointId: string;
  changedPaths: string[];
  messages: unknown[];
};

type CheckpointStoreDocument = {
  version: 1;
  fileByWorkspace: Record<string, PersistedFileCheckpoint[]>;
  turnBySession: Record<string, PersistedTurnCheckpoint[]>;
};

const storageKey = "ai.chat.checkpoints.v1";
const maxFileCheckpointsPerWorkspace = 32;
const maxTurnCheckpointsPerSession = 48;

let memory: CheckpointStoreDocument = { version: 1, fileByWorkspace: {}, turnBySession: {} };
let loaded = false;

export function loadChatCheckpointStore() {
  if (loaded) return memory;
  loaded = true;
  try {
    const raw = window.localStorage.getItem(storageKey);
    if (!raw) return memory;
    const parsed = JSON.parse(raw) as CheckpointStoreDocument;
    if (parsed?.version === 1 && parsed.fileByWorkspace && parsed.turnBySession) {
      memory = parsed;
    }
  } catch {
    memory = { version: 1, fileByWorkspace: {}, turnBySession: {} };
  }
  return memory;
}

export function saveChatCheckpointStore() {
  loadChatCheckpointStore();
  try {
    window.localStorage.setItem(storageKey, JSON.stringify(memory));
  } catch {
    // Storage quota — drop oldest workspace file checkpoints and retry once.
    trimOldestFileCheckpoints(8);
    try {
      window.localStorage.setItem(storageKey, JSON.stringify(memory));
    } catch {
      // Best effort only.
    }
  }
}

export function workspaceCheckpointKey(workspaceRoot: string) {
  return normalizePathSlashes(workspaceRoot).replace(/\/+$/, "").toLowerCase();
}

export function listFileCheckpoints(workspaceRoot: string) {
  loadChatCheckpointStore();
  return [...(memory.fileByWorkspace[workspaceCheckpointKey(workspaceRoot)] ?? [])];
}

export function getFileCheckpoint(workspaceRoot: string, id: string) {
  return listFileCheckpoints(workspaceRoot).find((entry) => entry.id === id) ?? null;
}

export function upsertFileCheckpoint(workspaceRoot: string, checkpoint: PersistedFileCheckpoint) {
  loadChatCheckpointStore();
  const key = workspaceCheckpointKey(workspaceRoot);
  const store = memory.fileByWorkspace[key] ?? [];
  const next = [checkpoint, ...store.filter((entry) => entry.id !== checkpoint.id)].slice(0, maxFileCheckpointsPerWorkspace);
  memory.fileByWorkspace[key] = next;
  saveChatCheckpointStore();
  return checkpoint;
}

/** Persist the removal of a file checkpoint so a delete survives reload (the
 *  in-memory runtime store alone would let deleted checkpoints reappear). Returns
 *  true when a checkpoint was actually removed. */
export function removeFileCheckpoint(workspaceRoot: string, id: string) {
  loadChatCheckpointStore();
  const key = workspaceCheckpointKey(workspaceRoot);
  const store = memory.fileByWorkspace[key];
  if (!store) return false;
  const next = store.filter((entry) => entry.id !== id);
  if (next.length === store.length) return false;
  memory.fileByWorkspace[key] = next;
  saveChatCheckpointStore();
  return true;
}

export function listTurnCheckpoints(sessionId: string) {
  loadChatCheckpointStore();
  return [...(memory.turnBySession[sessionId] ?? [])];
}

export function getTurnCheckpoint(sessionId: string, id: string) {
  return listTurnCheckpoints(sessionId).find((entry) => entry.id === id) ?? null;
}

export function upsertTurnCheckpoint(checkpoint: PersistedTurnCheckpoint) {
  loadChatCheckpointStore();
  const store = memory.turnBySession[checkpoint.sessionId] ?? [];
  const next = [checkpoint, ...store.filter((entry) => entry.id !== checkpoint.id)].slice(0, maxTurnCheckpointsPerSession);
  memory.turnBySession[checkpoint.sessionId] = next;
  saveChatCheckpointStore();
  return checkpoint;
}

/** Store is newest-first; removes the checkpoint at `fromIndex` and all newer turns. */
export function removeTurnCheckpointsAtAndAfter(sessionId: string, fromIndex: number) {
  loadChatCheckpointStore();
  const store = memory.turnBySession[sessionId] ?? [];
  if (fromIndex < 0) return;
  memory.turnBySession[sessionId] = store.slice(fromIndex + 1);
  saveChatCheckpointStore();
}

export function appendTurnCheckpointChangedPaths(sessionId: string, turnCheckpointId: string, paths: string[]) {
  if (paths.length === 0) return;
  loadChatCheckpointStore();
  const store = memory.turnBySession[sessionId] ?? [];
  const index = store.findIndex((entry) => entry.id === turnCheckpointId);
  if (index < 0) return;
  const turn = store[index];
  const merged = [...new Set([...turn.changedPaths, ...paths.map((path) => normalizePathSlashes(path))])];
  store[index] = { ...turn, changedPaths: merged };
  memory.turnBySession[sessionId] = store;
  saveChatCheckpointStore();
}

function trimOldestFileCheckpoints(keepPerWorkspace: number) {
  for (const key of Object.keys(memory.fileByWorkspace)) {
    const store = memory.fileByWorkspace[key];
    if (!store || store.length <= keepPerWorkspace) continue;
    memory.fileByWorkspace[key] = store.slice(0, keepPerWorkspace);
  }
}