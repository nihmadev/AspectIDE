import { lazy, Suspense, useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import type { editor } from "monaco-editor";
import { useTranslation } from "../../lib/i18n/useTranslation";
import { aspectCommands, type GitFileDiff } from "../../lib/tauri";
import { readErrorMessage } from "./SidebarShared";

const DiffEditor = lazy(() => import("@monaco-editor/react").then((module) => ({ default: module.DiffEditor })));

const DIFF_OPTIONS: editor.IDiffEditorConstructionOptions = {
  readOnly: true,
  renderSideBySide: true,
  automaticLayout: true,
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  fontSize: 12,
  lineNumbers: "on",
  diffWordWrap: "on",
};

type GitDiffModalProps = {
  path: string;
  displayName: string;
  onClose: () => void;
};

/** Read-only side-by-side diff (HEAD vs working tree) for one changed file. */
export function GitDiffModal({ path, displayName, onClose }: GitDiffModalProps) {
  const { t } = useTranslation();
  const [diff, setDiff] = useState<GitFileDiff | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setDiff(null);
    setError(null);
    aspectCommands.gitFileDiff(path)
      .then((result) => { if (!cancelled) setDiff(result); })
      .catch((cause) => { if (!cancelled) setError(readErrorMessage(cause, t)); });
    return () => { cancelled = true; };
  }, [path, t]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => { if (event.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return createPortal(
    <div className="git-diff-overlay" role="dialog" aria-modal="true" onPointerDown={onClose}>
      <div className="git-diff-modal" onPointerDown={(event) => event.stopPropagation()}>
        <header className="git-diff-modal-header">
          <span className="git-diff-modal-title" title={path}>{displayName}</span>
          <span className="git-diff-modal-subtitle">{t("sidebar.git.diffSubtitle")}</span>
          <button className="icon-button compact" type="button" aria-label={t("sidebar.git.closeDiff")} title={t("sidebar.git.closeDiff")} onClick={onClose}>
            <X size={15} />
          </button>
        </header>
        <div className="git-diff-modal-body">
          {error ? (
            <div className="git-diff-modal-message" role="alert">{error}</div>
          ) : !diff ? (
            <div className="git-diff-modal-loading" aria-busy="true">{t("sidebar.git.diffLoading")}</div>
          ) : (
            <Suspense fallback={<div className="git-diff-modal-loading" aria-busy="true">{t("sidebar.git.diffLoading")}</div>}>
              <DiffEditor
                height="100%"
                language={languageForPath(path)}
                original={diff.headText}
                modified={diff.workingText}
                options={DIFF_OPTIONS}
              />
            </Suspense>
          )}
        </div>
      </div>
    </div>,
    document.body,
  );
}

const LANGUAGE_BY_EXTENSION: Record<string, string> = {
  ts: "typescript", tsx: "typescript", js: "javascript", jsx: "javascript", mjs: "javascript", cjs: "javascript",
  json: "json", jsonc: "json", md: "markdown", markdown: "markdown", rs: "rust", py: "python", go: "go",
  java: "java", c: "c", h: "c", cpp: "cpp", cc: "cpp", hpp: "cpp", cs: "csharp", rb: "ruby", php: "php",
  html: "html", htm: "html", css: "css", scss: "scss", less: "less", yaml: "yaml", yml: "yaml", toml: "toml",
  xml: "xml", sql: "sql", sh: "shell", bash: "shell", zsh: "shell", swift: "swift", kt: "kotlin", lua: "lua",
};

function languageForPath(path: string): string {
  const extension = path.split(".").pop()?.toLowerCase() ?? "";
  return LANGUAGE_BY_EXTENSION[extension] ?? "plaintext";
}
