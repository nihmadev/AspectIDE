import { luxCommands } from "./../tauri/commands";
import type { WorkspaceInfo } from "./../types/index";

export async function pickAndOpenWorkspace(): Promise<WorkspaceInfo | null> {
  const picked = await luxCommands.workspacePickFolder();
  if (!picked) return null;
  return luxCommands.workspaceOpen(picked.root);
}

export async function reloadWorkspace(workspace: WorkspaceInfo): Promise<WorkspaceInfo> {
  return luxCommands.workspaceOpen(workspace.root);
}
