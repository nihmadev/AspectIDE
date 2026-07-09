import { AlertTriangle, ArrowDown, ArrowUp, CheckCircle2, ChevronsRight, GitBranch } from "lucide-react";
import { displayPath } from "../lib/fileTree";
import { useTranslation } from "../lib/i18n/useTranslation";
import { useLuxStore } from "../lib/store";

export function StatusBar() {
  const workspace = useLuxStore((state) => state.workspace);
  const diagnosticsByPath = useLuxStore((state) => state.diagnosticsByPath);
  const gitStatus = useLuxStore((state) => state.gitStatus);
  const setActiveActivity = useLuxStore((state) => state.setActiveActivity);
  const toggleBottomPanel = useLuxStore((state) => state.toggleBottomPanel);
  const { t } = useTranslation();

  const branchChip = workspace && gitStatus?.branch ? (
    <button
      className="status-item status-branch"
      type="button"
      title={t("sidebar.git.title")}
      onClick={() => setActiveActivity("git")}
    >
      <GitBranch size={13} />
      <span>{gitStatus.branch}</span>
      {gitStatus.behind > 0 && <span className="status-branch-count"><ArrowDown size={11} />{gitStatus.behind}</span>}
      {gitStatus.ahead > 0 && <span className="status-branch-count"><ArrowUp size={11} />{gitStatus.ahead}</span>}
    </button>
  ) : null;

  let errorCount = 0;
  let warningCount = 0;
  for (const diagnostics of Object.values(diagnosticsByPath)) {
    for (const diagnostic of diagnostics) {
      if (diagnostic.severity === "error") errorCount += 1;
      if (diagnostic.severity === "warning") warningCount += 1;
    }
  }

  if (!workspace) {
    return (
      <footer className="status-bar no-project-status">
        <div className="status-group">
          <span className="status-item status-remote"><ChevronsRight size={14} /></span>
          <div className="status-problems-group" aria-label={t("status.noProblems")}>
            <button className="status-item status-count" type="button" aria-label={t("status.noErrors")} onClick={() => toggleBottomPanel("problems")}>
              <CheckCircle2 size={14} />{errorCount}
            </button>
            <button className="status-item status-count" type="button" aria-label={t("status.noWarnings")} onClick={() => toggleBottomPanel("problems")}>
              <AlertTriangle size={14} />{warningCount}
            </button>
          </div>
        </div>
        <div className="status-group">
          <span className="status-item">{t("status.noWorkspace")}</span>
        </div>
      </footer>
    );
  }

  return (
    <footer className="status-bar">
      <div className="status-group">
        <button className="status-item status-count" type="button" aria-label={t("status.problemsSummary", { errorCount, warningCount })} onClick={() => toggleBottomPanel("problems")}>
          <CheckCircle2 size={14} /> {errorCount}
          <AlertTriangle size={14} /> {warningCount}
        </button>
        {branchChip}
      </div>
      <div className="status-group">
        <span className="status-item">{workspace ? displayPath(workspace.root) : t("status.noWorkspace")}</span>
      </div>
    </footer>
  );
}
