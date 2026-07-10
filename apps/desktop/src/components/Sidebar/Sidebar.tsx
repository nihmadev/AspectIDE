import { Bug, ChevronDown, Files, GitBranch, Package, Pin, Search } from "lucide-react";
import type { ReactNode } from "react";
import { useEffect, useRef, useState } from "react";
import { ExplorerPanel } from "./ExplorerPanel";
import { ExtensionsPanel } from "./ExtensionsPanel";
import { GitPanel } from "./GitPanel";
import { RunDebugPanel } from "./RunDebugPanel";
import { SearchPanel } from "./SearchPanel";
import { useTranslation } from '../../lib/i18n/useTranslation';
import type { MessageKey } from '../../lib/i18n';
import { useLuxStore, type Activity } from '../../lib/store/index';

const explorerActivities: Array<{ id: Activity; label: MessageKey; shortcut: string; icon: ReactNode }> = [
  { id: "explorer", label: "sidebar.explorer.title", shortcut: "Ctrl+Shift+E", icon: <Files size={18} strokeWidth={1.8} /> },
  { id: "search", label: "sidebar.search.title", shortcut: "Ctrl+Shift+F", icon: <Search size={18} strokeWidth={1.8} /> },
  { id: "git", label: "sidebar.git.title", shortcut: "Ctrl+Shift+G", icon: <GitBranch size={18} strokeWidth={1.8} /> },
  { id: "runDebug", label: "sidebar.runDebug.title", shortcut: "Ctrl+Shift+D", icon: <Bug size={18} strokeWidth={1.8} /> },
  { id: "extensions", label: "sidebar.extensions.title", shortcut: "Ctrl+Shift+X", icon: <Package size={18} strokeWidth={1.8} /> },
];

const pinnedExplorerActivityIds = new Set<Activity>(["explorer", "search"]);

export function Sidebar({ side = "left" }: { side?: "left" | "right" }) {
  const activeActivity = useLuxStore((state) => state.activeActivity);

  return (
    <aside className="sidebar" data-side={side}>
      <div className="sidebar-surface">
        <SidebarViewSwitcher />
        {activeActivity === "explorer" && <ExplorerPanel />}
        {activeActivity === "search" && <SearchPanel />}
        {activeActivity === "git" && <GitPanel />}
        {activeActivity === "runDebug" && <RunDebugPanel />}
        {activeActivity === "extensions" && <ExtensionsPanel />}
      </div>
    </aside>
  );
}

function SidebarViewSwitcher() {
  const { t } = useTranslation();
  const activeActivity = useLuxStore((state) => state.activeActivity);
  const setActiveActivity = useLuxStore((state) => state.setActiveActivity);
  const [menuOpen, setMenuOpen] = useState(false);
  const switcherRef = useRef<HTMLDivElement | null>(null);
  const pinnedActivities = explorerActivities.filter((activity) => pinnedExplorerActivityIds.has(activity.id));
  const overflowActivities = explorerActivities.filter((activity) => !pinnedExplorerActivityIds.has(activity.id));
  const overflowActive = overflowActivities.some((activity) => activity.id === activeActivity);

  useEffect(() => {
    if (!menuOpen) return;
    const closeIfOutside = (event: PointerEvent) => {
      if (switcherRef.current?.contains(event.target as Node)) return;
      setMenuOpen(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") setMenuOpen(false);
    };
    window.addEventListener("pointerdown", closeIfOutside);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.removeEventListener("pointerdown", closeIfOutside);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [menuOpen]);

  return (
    <div className="sidebar-view-switcher" ref={switcherRef}>
      <nav className="explorer-activity-strip" aria-label={t("sidebar.aria.panelViews")}>
        {pinnedActivities.map((activity) => (
          <button
            className="explorer-activity-button"
            data-active={activeActivity === activity.id}
            type="button"
            aria-label={t(activity.label)}
            title={t(activity.label)}
            key={activity.id}
            onClick={() => setActiveActivity(activity.id)}
          >
            {activity.icon}
          </button>
        ))}
        <button
          className="explorer-activity-button view-menu-toggle"
          data-active={menuOpen || overflowActive}
          type="button"
          aria-label={t("sidebar.views.more")}
          aria-expanded={menuOpen}
          title={t("sidebar.views.more")}
          onClick={() => setMenuOpen((open) => !open)}
        >
          <ChevronDown size={15} strokeWidth={1.9} />
        </button>
      </nav>
      {menuOpen && (
        <div className="view-switcher-menu">
          {overflowActivities.map((activity) => (
            <button
              className="view-switcher-item"
              data-active={activeActivity === activity.id}
              type="button"
              key={activity.id}
              onClick={() => {
                setActiveActivity(activity.id);
                setMenuOpen(false);
              }}
            >
              {activity.icon}
              <span>{t(activity.label)}</span>
              <kbd>{activity.shortcut}</kbd>
              <Pin size={13} />
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
