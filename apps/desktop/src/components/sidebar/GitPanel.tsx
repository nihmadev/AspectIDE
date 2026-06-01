import { GitBranch } from "lucide-react";
import { useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { displayPath, joinPath } from "../../lib/fileTree";
import { useTranslation } from "../../lib/i18n/useTranslation";
import { useLuxStore } from "../../lib/store";
import { luxCommands } from "../../lib/tauri";
import type { GitFileStatus } from "../../lib/types";
import { readErrorMessage, TreeMessage } from "./SidebarShared";

export function GitPanel() {
  const { t } = useTranslation();
  const gitStatus = useLuxStore((state) => state.gitStatus);
  const workspace = useLuxStore((state) => state.workspace);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);
  const [openError, setOpenError] = useState<string | null>(null);

  const openGitFileMutation = useMutation({
    mutationFn: (file: GitFileStatus) => luxCommands.editorOpenFile(gitFileAbsolutePath(file, workspace?.root)),
    onSuccess: (document) => {
      setOpenError(null);
      upsertDocument(document);
    },
    onError: (error) => setOpenError(readErrorMessage(error, t)),
  });

  return (
    <div className="panel-content utility-panel-content">
      <div className="branch-summary">
        <GitBranch size={16} />
        <span>{gitStatus?.branch ?? t("sidebar.git.noRepository")}</span>
      </div>
      {openError && <TreeMessage depth={0} tone="error" text={openError} />}
      <div className="file-tree">
        {gitStatus?.files.map((file) => (
          <button className="file-row" type="button" key={file.path} onClick={() => openGitFileMutation.mutate(file)}>
            <span className="status-pill">{file.index_status.trim() || file.worktree_status.trim() || "M"}</span>
            <span>{displayPath(file.path)}</span>
          </button>
        ))}
      </div>
    </div>
  );
}

function gitFileAbsolutePath(file: GitFileStatus, workspaceRoot?: string) {
  const targetPath = gitStatusTargetPath(file.path);
  return workspaceRoot ? joinPath(workspaceRoot, targetPath) : targetPath;
}

function gitStatusTargetPath(path: string) {
  const renameSeparator = " -> ";
  const separatorIndex = path.lastIndexOf(renameSeparator);
  return separatorIndex === -1 ? path : path.slice(separatorIndex + renameSeparator.length);
}
