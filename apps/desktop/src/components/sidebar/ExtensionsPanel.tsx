import { Loader2, Package, RefreshCw, Search, ShieldAlert } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { useTranslation, type TranslateFn } from "../../lib/i18n/useTranslation";
import { useLuxStore } from "../../lib/store";
import { luxCommands } from "../../lib/tauri";
import type { ExtensionInfo } from "../../lib/types";
import { readErrorMessage, TreeMessage } from "./SidebarShared";

export function ExtensionsPanel() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [openError, setOpenError] = useState<string | null>(null);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);

  const extensionsMutation = useMutation({
    mutationFn: luxCommands.extensionsList,
  });

  const openExtensionManifestMutation = useMutation({
    mutationFn: (extension: ExtensionInfo) => luxCommands.editorOpenFile(extension.manifest_path),
    onSuccess: (document) => {
      setOpenError(null);
      upsertDocument(document);
    },
    onError: (error) => setOpenError(readErrorMessage(error, t)),
  });

  useEffect(() => {
    extensionsMutation.mutate();
  }, []);

  const extensions = extensionsMutation.data ?? [];
  const visibleExtensions = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase();
    if (!normalizedQuery) return extensions;
    return extensions.filter((extension) =>
      `${extension.name} ${extension.id} ${extension.version} ${extension.contributes.join(" ")} ${extension.contribution_points.map((point) => point.id).join(" ")}`
        .toLowerCase()
        .includes(normalizedQuery),
    );
  }, [extensions, query]);

  return (
    <div className="panel-content extensions-panel-content utility-panel-content">
      <form
        className="search-form extensions-search-form"
        onSubmit={(event) => {
          event.preventDefault();
          extensionsMutation.mutate();
        }}
      >
        <Search size={15} />
        <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("sidebar.extensions.search.placeholder")} />
      </form>
      <div className="panel-caption extensions-caption">
        <span>{extensionCaption(extensionsMutation.isPending, visibleExtensions.length, extensions.length, t)}</span>
        <button className="icon-button compact" type="button" aria-label={t("sidebar.extensions.actions.refresh")} title={t("sidebar.extensions.actions.refresh")} onClick={() => extensionsMutation.mutate()} disabled={extensionsMutation.isPending}>
          {extensionsMutation.isPending ? <Loader2 size={14} className="spin-icon" /> : <RefreshCw size={14} />}
        </button>
      </div>
      <div className="extensions-list" role="list" aria-label={t("sidebar.extensions.installed")}>
        {extensionsMutation.error ? <TreeMessage depth={0} tone="error" text={readErrorMessage(extensionsMutation.error, t)} /> : null}
        {openError ? <TreeMessage depth={0} tone="error" text={openError} /> : null}
        {!extensionsMutation.isPending && visibleExtensions.length === 0 ? (
          <div className="extensions-empty-state">
            <Package size={17} />
            <span>{query.trim() ? t("sidebar.extensions.empty.noSearchMatches") : t("sidebar.extensions.empty.noManifests")}</span>
          </div>
        ) : null}
        {visibleExtensions.map((extension) => (
          <ExtensionRow
            extension={extension}
            key={`${extension.root}-${extension.id}`}
            openManifest={() => openExtensionManifestMutation.mutate(extension)}
          />
        ))}
      </div>
    </div>
  );
}

function ExtensionRow({ extension, openManifest }: { extension: ExtensionInfo; openManifest: () => void }) {
  const { t } = useTranslation();
  const invalid = extension.status === "invalid";
  const active = extension.status === "active";

  return (
    <button className="extension-row" type="button" role="listitem" data-invalid={invalid} data-active={active} title={extension.error ?? extension.manifest_path} onClick={openManifest}>
      <span className="extension-row-icon">{invalid ? <ShieldAlert size={16} /> : <Package size={16} />}</span>
      <span className="extension-row-main">
        <span className="extension-row-title">
          <strong>{extension.name}</strong>
          <small>{extension.version}</small>
        </span>
        <span className="extension-row-id">{extension.id}</span>
        <span className="extension-row-contributes">{formatExtensionContributes(extension, t)}</span>
      </span>
      <span className="extension-status" data-invalid={invalid} data-active={active}>{extensionStatusLabel(extension.status, t)}</span>
    </button>
  );
}

function extensionStatusLabel(status: ExtensionInfo["status"], t: TranslateFn) {
  if (status === "active") return t("sidebar.extensions.status.active");
  if (status === "invalid") return t("sidebar.extensions.status.invalid");
  return t("sidebar.extensions.status.discovered");
}

function extensionCaption(loading: boolean, visibleCount: number, totalCount: number, t: TranslateFn) {
  if (loading) return t("sidebar.extensions.scanning");
  if (totalCount === 0) return t("sidebar.extensions.installed");
  if (visibleCount === totalCount) return t("sidebar.extensions.installedCount", { count: totalCount });
  return t("sidebar.extensions.visibleCount", { visibleCount, totalCount });
}

function formatExtensionContributes(extension: ExtensionInfo, t: TranslateFn) {
  if (extension.error) return extension.error;
  if (extension.contribution_points.length === 0) return t("sidebar.extensions.noContributionPoints");
  return extension.contribution_points.map((point) => point.id).join(", ");
}
