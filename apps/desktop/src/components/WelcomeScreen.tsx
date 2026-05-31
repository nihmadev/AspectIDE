import { Clock3, FolderOpen, X } from "lucide-react";
import type { MouseEvent } from "react";
import { displayPath } from "../lib/fileTree";
import { useTranslation } from "../lib/i18n/useTranslation";
import type { RecentWorkspace } from "../lib/types";

type WelcomeScreenProps = {
  onOpenProject: () => void;
  onForgetRecentWorkspace: (root: string) => void;
  onOpenRecentWorkspace: (root: string) => void;
  recentWorkspaces: RecentWorkspace[];
};

export function WelcomeScreen({ onForgetRecentWorkspace, onOpenProject, onOpenRecentWorkspace, recentWorkspaces }: WelcomeScreenProps) {
  const { t } = useTranslation();
  return (
    <main className="welcome-screen">
      <section className="welcome-content" aria-label={t("welcome.aria")}>
        <div className="welcome-primary">
          <div className="welcome-brand">
            <span className="welcome-logo" aria-hidden="true">L</span>
            <div>
              <h1>{t("welcome.title")}</h1>
              <p>{t("welcome.subtitle")}</p>
            </div>
          </div>

          <div className="welcome-actions" aria-label={t("welcome.projectActions")}>
            <button className="welcome-action-card" type="button" onClick={onOpenProject}>
              <FolderOpen size={17} strokeWidth={1.8} />
              <span>{t("welcome.openProject")}</span>
              <small>{t("welcome.openProjectHint")}</small>
            </button>
          </div>
        </div>

        <div className="welcome-secondary">
          <div className="recent-projects" aria-label={t("welcome.recentProjects")}>
            <div className="recent-header">
              <span>{t("welcome.recentProjects")}</span>
              {recentWorkspaces.length > 0 && <small>{recentWorkspaces.length}</small>}
            </div>
            {recentWorkspaces.length > 0 ? (
              <div className="recent-list">
                {recentWorkspaces.map((workspace) => (
                  <RecentWorkspaceRow
                    key={workspace.root}
                    workspace={workspace}
                    onForgetRecentWorkspace={onForgetRecentWorkspace}
                    onOpenRecentWorkspace={onOpenRecentWorkspace}
                  />
                ))}
              </div>
            ) : (
              <div className="recent-empty">
                <Clock3 size={15} strokeWidth={1.8} />
                <div>
                  <span>{t("welcome.noRecentProjects")}</span>
                  <small>{t("welcome.noRecentProjectsDetail")}</small>
                </div>
              </div>
            )}
          </div>
        </div>
      </section>
    </main>
  );
}

function RecentWorkspaceRow({
  onForgetRecentWorkspace,
  onOpenRecentWorkspace,
  workspace,
}: {
  onForgetRecentWorkspace: (root: string) => void;
  onOpenRecentWorkspace: (root: string) => void;
  workspace: RecentWorkspace;
}) {
  const { t } = useTranslation();
  const forget = (event: MouseEvent<HTMLButtonElement>) => {
    event.stopPropagation();
    onForgetRecentWorkspace(workspace.root);
  };
  const rootLabel = displayPath(workspace.root);

  return (
    <div className="recent-row-wrap">
      <button className="recent-row" type="button" onClick={() => onOpenRecentWorkspace(workspace.root)} title={rootLabel}>
        <FolderOpen className="recent-row-icon" size={14} strokeWidth={1.8} />
        <span className="recent-row-text">
          <span>{workspace.name}</span>
          <small>{rootLabel}</small>
        </span>
      </button>
      <button className="recent-forget-button" type="button" aria-label={t("welcome.removeRecent", { name: workspace.name })} title={t("welcome.removeFromRecentProjects")} onClick={forget}>
        <X size={13} />
      </button>
    </div>
  );
}
