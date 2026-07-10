import { Clock3, FolderOpen, MessageCircle, X } from "lucide-react";
import type { MouseEvent } from "react";
import { displayPath } from '../../lib/explorer/file-tree';
import { useTranslation } from '../../lib/i18n/useTranslation';
import type { RecentWorkspace } from '../../lib/types/index';

type WelcomeScreenProps = {
  loading?: boolean;
  onOpenProject: () => void;
  onForgetRecentWorkspace: (root: string) => void;
  onOpenRecentWorkspace: (root: string) => void;
  recentWorkspaces: RecentWorkspace[];
};

export function WelcomeScreen({ loading = false, onForgetRecentWorkspace, onOpenProject, onOpenRecentWorkspace, recentWorkspaces }: WelcomeScreenProps) {
  const { t } = useTranslation();

  return (
    <main className="welcome-screen">
      <section className="welcome-content" aria-label={t("welcome.aria")}>
        <header className="welcome-hero">
          <span className="welcome-logo" aria-hidden="true">
            <img src="/aspect-mark.svg" alt="" draggable={false} />
          </span>
          <h1>{t("welcome.title")}</h1>
          <p>{t("welcome.subtitle")}</p>
        </header>

        {/* Community banner is permanent by design вЂ” no dismiss affordance. */}
        <div className="welcome-telegram-banner">
          <MessageCircle size={18} strokeWidth={1.8} />
          <div className="welcome-telegram-text">
            <span>{t("welcome.telegramBanner.text")}</span>
            <a
              href="https://t.me/aspect_ide"
              target="_blank"
              rel="noopener noreferrer"
              className="welcome-telegram-link"
            >
              {t("welcome.telegramBanner.link")}
            </a>
          </div>
        </div>

        <div className="welcome-actions" aria-label={t("welcome.projectActions")}>
          <button className="welcome-action-card" type="button" disabled={loading} onClick={onOpenProject}>
            <span className="welcome-action-icon" aria-hidden="true">
              <FolderOpen size={18} strokeWidth={1.8} />
            </span>
            <span className="welcome-action-text">
              <span>{t("welcome.openProject")}</span>
              <small>{t("welcome.openProjectHint")}</small>
            </span>
          </button>
        </div>

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
                  loading={loading}
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
      </section>
    </main>
  );
}

function RecentWorkspaceRow({
  onForgetRecentWorkspace,
  onOpenRecentWorkspace,
  loading,
  workspace,
}: {
  onForgetRecentWorkspace: (root: string) => void;
  onOpenRecentWorkspace: (root: string) => void;
  loading: boolean;
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
      <button className="recent-row" type="button" disabled={loading} onClick={() => onOpenRecentWorkspace(workspace.root)} title={rootLabel}>
        <FolderOpen className="recent-row-icon" size={14} strokeWidth={1.8} />
        <span className="recent-row-text">
          <span>{workspace.name}</span>
          <small>{rootLabel}</small>
        </span>
      </button>
      <button className="recent-forget-button" type="button" disabled={loading} aria-label={t("welcome.removeRecent", { name: workspace.name })} title={t("welcome.removeFromRecentProjects")} onClick={forget}>
        <X size={13} />
      </button>
    </div>
  );
}
