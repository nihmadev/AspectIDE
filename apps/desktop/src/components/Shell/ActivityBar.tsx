import { Bug, Files, GitBranch, Package, Search } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import type { MessageKey } from "../lib/i18n";
import { useTranslation } from "../lib/i18n/useTranslation";
import { useAspectStore, type Activity } from "../lib/store/index";

const activities: Array<{ id: Activity; labelKey: MessageKey; icon: LucideIcon }> = [
  { id: "explorer", labelKey: "activity.explorer", icon: Files },
  { id: "search", labelKey: "activity.search", icon: Search },
  { id: "git", labelKey: "activity.sourceControl", icon: GitBranch },
  { id: "runDebug", labelKey: "activity.runDebug", icon: Bug },
  { id: "extensions", labelKey: "activity.extensions", icon: Package },
];

export function ActivityBar() {
  const activeActivity = useAspectStore((state) => state.activeActivity);
  const setActiveActivity = useAspectStore((state) => state.setActiveActivity);
  const { t } = useTranslation();

  return (
    <nav className="activity-bar" aria-label={t("activity.primary")}>
      <div className="brand-mark" aria-label="Aspect IDE">
        L
      </div>
      <div className="activity-buttons">
        {activities.map(({ id, labelKey, icon: Icon }) => {
          const label = t(labelKey);
          return (
            <button
              key={id}
              className="icon-button activity-button"
              data-active={activeActivity === id}
              type="button"
              aria-label={label}
              title={label}
              onClick={() => setActiveActivity(id)}
            >
              <Icon size={20} strokeWidth={1.85} />
            </button>
          );
        })}
      </div>
    </nav>
  );
}
