import type { CSSProperties, ReactNode } from "react";
import { displayPath } from '../../lib/explorer/file-tree';
import type { TranslateFn } from '../../lib/i18n/useTranslation';

export type PanelAction = {
  label: string;
  icon: ReactNode;
  onClick: () => void;
  disabled?: boolean;
};

export function PanelHeader({ actions = [], title }: { actions?: PanelAction[]; title: string }) {
  return (
    <div className="panel-header">
      <span>{title}</span>
      {actions.length > 0 && (
        <div className="panel-actions">
          {actions.map((action) => (
            <button
              className="icon-button compact"
              type="button"
              aria-label={action.label}
              title={action.label}
              disabled={action.disabled}
              key={action.label}
              onClick={action.onClick}
            >
              {action.icon}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

export function TreeMessage({ depth, text, tone = "muted" }: { depth: number; text: string; tone?: "muted" | "error" }) {
  return <div className="tree-message" data-tone={tone} style={{ "--tree-depth": depth } as CSSProperties}>{text}</div>;
}

export function relativePath(root: string, path: string) {
  const normalizedRoot = displayPath(root).replace(/\/+$/, "");
  const normalizedPath = displayPath(path);
  return normalizedPath.toLowerCase().startsWith(`${normalizedRoot.toLowerCase()}/`)
    ? normalizedPath.slice(normalizedRoot.length + 1)
    : normalizedPath;
}

export function readErrorMessage(error: unknown, t: TranslateFn) {
  if (error instanceof Error) return error.message;
  if (typeof error === "string") return error;
  return t("sidebar.error.operationFailed");
}
