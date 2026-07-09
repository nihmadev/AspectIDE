import { requestFileReviewFocus } from "./../aspector/utils/file-review/bridge";
import { useLuxStore } from "./../store/index";
import { luxCommands } from "./../tauri/commands";

export async function openWorkspaceEditorPath(path: string) {
  const normalized = normalizePathKey(path);
  const state = useLuxStore.getState();
  const existing = state.openDocuments.find((document) => document.path && normalizePathKey(document.path) === normalized);
  if (existing) {
    const group = state.editorGroups.find((entry) => entry.documentIds.includes(existing.id)) ?? state.editorGroups[0];
    if (group) {
      state.setActiveEditorGroup(group.id);
      state.setActiveDocumentInGroup(group.id, existing.id);
    } else {
      state.setActiveDocument(existing.id);
    }
    requestFileReviewFocus({ path });
    return;
  }

  const document = await luxCommands.editorOpenFile(path);
  state.upsertDocument(document);
  state.setActiveDocument(document.id);
  requestFileReviewFocus({ path });
}

function normalizePathKey(path: string) {
  return path.replace(/\\/g, "/").toLowerCase();
}