import { CaseSensitive, Loader2, Regex, RefreshCw, Replace, ReplaceAll, Search, SearchX, WholeWord } from "lucide-react";
import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useMutation } from "@tanstack/react-query";
import { fileIconForName } from "../../lib/fileIcons";
import { displayPath } from "../../lib/fileTree";
import { useTranslation } from "../../lib/i18n/useTranslation";
import { useLuxStore } from "../../lib/store";
import { luxCommands } from "../../lib/tauri";
import type { LspWorkspaceEdit, SearchHit, SearchOptions } from "../../lib/types";
import { PanelHeader, readErrorMessage, relativePath, TreeMessage } from "./SidebarShared";

type SearchResultGroup = {
  path: string;
  label: string;
  hits: SearchHit[];
};

export function SearchPanel() {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [replaceValue, setReplaceValue] = useState("");
  const [includePattern, setIncludePattern] = useState("");
  const [excludePattern, setExcludePattern] = useState("");
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [wholeWord, setWholeWord] = useState(false);
  const [useRegex, setUseRegex] = useState(false);
  const [includeHidden, setIncludeHidden] = useState(false);
  const lastScheduledSearchKey = useRef("");
  const [openError, setOpenError] = useState<string | null>(null);
  const [searchError, setSearchError] = useState<string | null>(null);
  const searchResponse = useLuxStore((state) => state.searchResponse);
  const setSearchResponse = useLuxStore((state) => state.setSearchResponse);
  const upsertDocument = useLuxStore((state) => state.upsertDocument);
  const updateOpenDocuments = useLuxStore((state) => state.updateOpenDocuments);
  const setPendingEditorReveal = useLuxStore((state) => state.setPendingEditorReveal);
  const workspace = useLuxStore((state) => state.workspace);

  const searchOptions = useMemo<SearchOptions>(() => ({
    case_sensitive: caseSensitive,
    whole_word: wholeWord,
    use_regex: useRegex,
    include_hidden: includeHidden,
    include_globs: parseGlobList(includePattern),
    exclude_globs: parseGlobList(excludePattern),
    max_results: 500,
  }), [caseSensitive, excludePattern, includeHidden, includePattern, useRegex, wholeWord]);

  const searchMutation = useMutation({
    mutationFn: ({ options, value }: { value: string; options: SearchOptions }) => luxCommands.searchQuery(value, options),
    onSuccess: (response) => {
      setOpenError(null);
      setSearchError(null);
      setSearchResponse(response);
    },
    onError: (error) => setSearchError(readErrorMessage(error, t)),
  });

  const openSearchHitMutation = useMutation({
    mutationFn: async (hit: SearchHit) => ({ hit, document: await luxCommands.editorOpenFile(hit.path) }),
    onSuccess: ({ document, hit }) => {
      setOpenError(null);
      upsertDocument(document);
      setPendingEditorReveal({ documentId: document.id, line: hit.line, column: hit.column });
    },
    onError: (error) => setOpenError(readErrorMessage(error, t)),
  });

  const replaceMutation = useMutation({
    mutationFn: async (hits: SearchHit[]) => luxCommands.editorApplyWorkspaceEdit(buildSearchReplaceEdit(hits, query, replaceValue, { caseSensitive, useRegex })),
    onSuccess: (result) => {
      setOpenError(null);
      updateOpenDocuments(result.edited_documents);
      runSearchRef.current();
    },
    onError: (error) => setOpenError(readErrorMessage(error, t)),
  });

  const resultLabel = useMemo(() => {
    if (!searchResponse) return t("sidebar.search.noSearchExecuted");
    const truncatedIndicator = searchResponse.truncated ? "+" : "";
    return t("sidebar.search.resultCount", { count: searchResponse.hits.length, truncatedIndicator, elapsedMs: searchResponse.elapsed_ms });
  }, [searchResponse, t]);

  const groupedHits = useMemo(() => groupSearchHits(searchResponse?.hits ?? [], workspace?.root ?? null), [searchResponse?.hits, workspace?.root]);
  const canReplace = Boolean(query.trim() && searchResponse?.hits.length && !searchMutation.isPending && !replaceMutation.isPending);

  const searchKey = useMemo(() => JSON.stringify({ query, searchOptions }), [query, searchOptions]);
  const runSearchRef = useRef<() => void>(() => undefined);

  const runSearch = useCallback(() => {
    lastScheduledSearchKey.current = searchKey;
    searchMutation.mutate({ value: query, options: searchOptions });
  }, [query, searchKey, searchMutation, searchOptions]);

  useEffect(() => {
    runSearchRef.current = runSearch;
  }, [runSearch]);

  useEffect(() => {
    if (!query.trim()) return;
    if (lastScheduledSearchKey.current === searchKey) return;
    const timer = window.setTimeout(() => runSearchRef.current(), 260);
    return () => window.clearTimeout(timer);
  }, [query, searchKey]);

  return (
    <div className="panel-content utility-panel-content search-panel-content">
      <PanelHeader
        title={t("sidebar.search.title")}
        actions={[
          { label: t("sidebar.search.actions.refresh"), icon: searchMutation.isPending ? <Loader2 size={14} className="spin-icon" /> : <RefreshCw size={14} />, onClick: runSearch, disabled: searchMutation.isPending || !query.trim() },
          { label: t("sidebar.search.actions.clearResults"), icon: <SearchX size={14} />, onClick: () => setSearchResponse(null), disabled: !searchResponse },
        ]}
      />
      <form
        className="search-panel-form"
        onSubmit={(event) => {
          event.preventDefault();
          runSearch();
        }}
      >
        <div className="search-input-row">
          <Search size={14} />
          <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder={t("sidebar.search.title")} spellCheck={false} />
          <SearchToggle active={caseSensitive} label={t("sidebar.search.toggle.matchCase")} onClick={() => setCaseSensitive((active) => !active)}><CaseSensitive size={14} /></SearchToggle>
          <SearchToggle active={wholeWord} label={t("sidebar.search.toggle.matchWholeWord")} onClick={() => setWholeWord((active) => !active)}><WholeWord size={14} /></SearchToggle>
          <SearchToggle active={useRegex} label={t("sidebar.search.toggle.useRegularExpression")} onClick={() => setUseRegex((active) => !active)}><Regex size={14} /></SearchToggle>
        </div>
        <div className="search-input-row">
          <Replace size={14} />
          <input value={replaceValue} onChange={(event) => setReplaceValue(event.target.value)} placeholder={t("sidebar.search.replace")} spellCheck={false} />
          <button className="search-inline-button" type="button" aria-label={t("sidebar.search.replace")} title={t("sidebar.search.replace")} disabled={!canReplace} onClick={() => searchResponse?.hits[0] && replaceMutation.mutate([searchResponse.hits[0]])}><Replace size={13} /></button>
          <button className="search-inline-button" type="button" aria-label={t("sidebar.search.replaceAll")} title={t("sidebar.search.replaceAll")} disabled={!canReplace} onClick={() => searchResponse && replaceMutation.mutate(searchResponse.hits)}><ReplaceAll size={13} /></button>
        </div>
        <label className="search-filter-field">
          <span>{t("sidebar.search.filesToInclude")}</span>
          <input value={includePattern} onChange={(event) => setIncludePattern(event.target.value)} placeholder={t("sidebar.search.includePlaceholder")} spellCheck={false} />
        </label>
        <label className="search-filter-field">
          <span>{t("sidebar.search.filesToExclude")}</span>
          <input value={excludePattern} onChange={(event) => setExcludePattern(event.target.value)} placeholder={t("sidebar.search.excludePlaceholder")} spellCheck={false} />
        </label>
        <label className="search-hidden-toggle">
          <input type="checkbox" checked={includeHidden} onChange={(event) => setIncludeHidden(event.target.checked)} />
          <span>{t("sidebar.search.includeHiddenFiles")}</span>
        </label>
      </form>
      <div className="panel-caption">{replaceMutation.isPending ? t("sidebar.search.replacing") : searchMutation.isPending ? t("sidebar.search.searching") : resultLabel}</div>
      {searchError && <TreeMessage depth={0} tone="error" text={searchError} />}
      {openError && <TreeMessage depth={0} tone="error" text={openError} />}
      <div className="search-results">
        {groupedHits.map((group) => (
          <section className="search-result-group" key={group.path}>
            <div className="search-result-file">
              {(() => {
                const iconMeta = fileIconForName(group.path);
                const Icon = iconMeta.Icon;
                return <Icon size={15} className={iconMeta.className} />;
              })()}
              <span>{group.label}</span>
              <small>{group.hits.length}</small>
            </div>
            {group.hits.map((hit, index) => (
              <button
                className="search-hit"
                type="button"
                key={`${hit.path}-${hit.line}-${hit.column}-${index}`}
                onClick={() => openSearchHitMutation.mutate(hit)}
              >
                <span>{highlightPreview(hit)}</span>
                <small>{hit.line}:{hit.column}</small>
              </button>
            ))}
          </section>
        ))}
      </div>
    </div>
  );
}

function SearchToggle({ active, children, label, onClick }: { active: boolean; children: ReactNode; label: string; onClick: () => void }) {
  return (
    <button className="search-inline-button" data-active={active} type="button" aria-label={label} title={label} onClick={onClick}>
      {children}
    </button>
  );
}

function parseGlobList(value: string) {
  return value
    .split(",")
    .map((pattern) => pattern.trim())
    .filter(Boolean);
}

function groupSearchHits(hits: SearchHit[], workspaceRoot: string | null): SearchResultGroup[] {
  const groups = new Map<string, SearchResultGroup>();
  for (const hit of hits) {
    const path = displayPath(hit.path);
    const group = groups.get(path);
    if (group) {
      group.hits.push(hit);
      continue;
    }
    groups.set(path, { path, label: workspaceRoot ? relativePath(workspaceRoot, path) : path, hits: [hit] });
  }
  return [...groups.values()];
}

function buildSearchReplaceEdit(
  hits: SearchHit[],
  query: string,
  replacement: string,
  options: { caseSensitive: boolean; useRegex: boolean },
): LspWorkspaceEdit {
  const files = new Map<string, LspWorkspaceEdit["files"][number]>();
  for (const hit of hits) {
    const range = replacementRangeForHit(hit);
    const path = hit.path;
    const fileEdit = files.get(path) ?? { path, edits: [] };
    fileEdit.edits.push({ range, text: replacementTextForHit(hit, query, replacement, options) });
    files.set(path, fileEdit);
  }
  return { files: [...files.values()] };
}

function replacementRangeForHit(hit: SearchHit) {
  const startColumn = Math.max(1, hit.column);
  return {
    start_line: hit.line,
    start_column: startColumn,
    end_line: hit.line,
    end_column: startColumn + Math.max(0, hit.match_length),
  };
}

function replacementTextForHit(hit: SearchHit, query: string, replacement: string, options: { caseSensitive: boolean; useRegex: boolean }) {
  if (!options.useRegex) return replacement;
  try {
    return hit.match_text.replace(new RegExp(query, options.caseSensitive ? "" : "i"), replacement);
  } catch {
    return replacement;
  }
}

function highlightPreview(hit: SearchHit): ReactNode {
  const preview = hit.preview;
  const matchIndex = Math.max(0, Math.min(preview.length, hit.preview_match_start));
  const matchEnd = Math.max(matchIndex, Math.min(preview.length, hit.preview_match_start + Math.max(0, hit.preview_match_length)));
  if (matchEnd <= matchIndex) return preview;
  return (
    <>
      {preview.slice(0, matchIndex)}
      <mark>{preview.slice(matchIndex, matchEnd)}</mark>
      {preview.slice(matchEnd)}
    </>
  );
}
