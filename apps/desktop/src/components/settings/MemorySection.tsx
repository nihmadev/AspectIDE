import { Brain, ChevronDown, ChevronRight, Flame, Link2, Loader2, Pin, PinOff, Plus, RefreshCw, Trash2, X } from "lucide-react";
import { CompactDropdown } from "../CompactDropdown/CompactDropdown";
import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import type { TranslateFn } from '../../lib/i18n/useTranslation';
import {
  luxCommands,
  type MemoryRecord,
  type MemoryRelation,
  type MemoryRetentionReport,
  type MemorySortOrder,
  type MemoryStats,
} from '../../lib/tauri/commands';
import type { WorkspaceInfo } from '../../lib/types';

const COMMON_CATEGORIES = ["core", "semantic", "episodic", "procedural"];

/** Per-project durable memory browser: search, filter, pin, edit, and prune the
 *  memories the agent (and the user) accumulate for the open workspace. */
export function MemorySection({ workspace, t }: { workspace: WorkspaceInfo | null; t: TranslateFn }) {
  const [records, setRecords] = useState<MemoryRecord[]>([]);
  const [stats, setStats] = useState<MemoryStats | null>(null);
  const [retention, setRetention] = useState<MemoryRetentionReport | null>(null);
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const [sort, setSort] = useState<MemorySortOrder>("relevance");
  const [includeSuperseded, setIncludeSuperseded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [pruneResult, setPruneResult] = useState<string | null>(null);
  const [pruning, setPruning] = useState(false);

  const seqRef = useRef(0);
  const refresh = useCallback(async () => {
    if (!workspace) return;
    const seq = ++seqRef.current;
    setLoading(true);
    setError(null);
    try {
      // touch:false РІР‚вЂќ browsing the manager must not inflate recall recency/usage;
      // only the agent's RecallMemory bumps access stats.
      const options = { category, limit: 200, sort, touch: false, includeSuperseded };
      const list = query.trim()
        ? await luxCommands.memorySearch(query.trim(), options)
        : await luxCommands.memoryList(options);
      const [nextStats, nextRetention] = await Promise.all([
        luxCommands.memoryStats(),
        luxCommands.memoryRetention(),
      ]);
      // Latest-wins: a newer refresh has superseded this one РІР‚вЂќ drop its results.
      if (seq !== seqRef.current) return;
      setRecords(list);
      setStats(nextStats);
      setRetention(nextRetention);
      // Clear a category filter whose last entry just disappeared, so the view
      // isn't a dead-end with no active chip left to deselect.
      if (category && !nextStats.byCategory.some((entry) => entry.category === category)) {
        setCategory(null);
      }
    } catch (cause) {
      if (seq === seqRef.current) setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      if (seq === seqRef.current) setLoading(false);
    }
  }, [workspace, query, category, sort, includeSuperseded]);

  useEffect(() => {
    const handle = window.setTimeout(() => void refresh(), query ? 180 : 0);
    return () => window.clearTimeout(handle);
  }, [refresh, query]);

  const togglePin = useCallback(async (record: MemoryRecord) => {
    try {
      await luxCommands.memoryUpdate(record.id, { pinned: !record.pinned });
      void refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [refresh]);

  const remove = useCallback(async (record: MemoryRecord) => {
    if (!window.confirm(t("settings.memory.confirmDelete"))) return;
    try {
      await luxCommands.memoryDelete(record.id);
      void refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [refresh, t]);

  const wipeAll = useCallback(async () => {
    // memoryWipe(null) is scoped to the *active workspace's* store on the backend
    // (with_memory РІвЂ вЂ™ workspace_root), not global. Name the project in the prompt so
    // the user can see exactly which project's memory they are about to erase.
    const prompt = workspace
      ? `${t("settings.memory.confirmWipe")}\n\n${workspace.name}`
      : t("settings.memory.confirmWipe");
    if (!window.confirm(prompt)) return;
    try {
      await luxCommands.memoryWipe(null);
      void refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [refresh, t, workspace]);

  const pruneNow = useCallback(async () => {
    setPruning(true);
    setPruneResult(null);
    try {
      const count = await luxCommands.memoryPrune();
      setPruneResult(t("settings.memory.pruneResult", { count }));
      void refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setPruning(false);
    }
  }, [refresh, t]);

  const categories = useMemo(() => stats?.byCategory ?? [], [stats]);

  if (!workspace) {
    return <p className="aspect-mem-empty">{t("settings.memory.needProject")}</p>;
  }

  return (
    <div className="aspect-mem">
      <header className="aspect-mem-head">
        <div className="aspect-mem-stat">
          <Brain size={15} />
          <span>{t("settings.memory.total", { count: stats?.total ?? 0 })}</span>
          {(stats?.pinned ?? 0) > 0 && <span className="aspect-mem-pincount">{t("settings.memory.pinned", { count: stats?.pinned ?? 0 })}</span>}
        </div>
        <div className="aspect-mem-head-actions">
          <button type="button" className="aspect-mem-prune-btn" disabled={pruning} onClick={() => void pruneNow()} title={t("settings.memory.pruneHint")}>
            {pruning ? <Loader2 size={13} className="aspect-spin" /> : <RefreshCw size={13} />} {t("settings.memory.prune")}
          </button>
          <button type="button" className="aspect-mem-add-btn" onClick={() => setShowAdd((open) => !open)}>
            <Plus size={14} /> {t("settings.memory.add")}
          </button>
        </div>
      </header>

      {retention && (retention.hot + retention.warm + retention.cold + retention.evictable) > 0 && (
        <div className="aspect-mem-retention" role="group" aria-label={t("settings.memory.retention.label")}>
          <span className="aspect-mem-retention-chip" data-tier="hot" title={t("settings.memory.retention.hotHint")}>
            <Flame size={11} /> {t("settings.memory.retention.hot", { count: retention.hot })}
          </span>
          <span className="aspect-mem-retention-chip" data-tier="warm" title={t("settings.memory.retention.warmHint")}>
            {t("settings.memory.retention.warm", { count: retention.warm })}
          </span>
          <span className="aspect-mem-retention-chip" data-tier="cold" title={t("settings.memory.retention.coldHint")}>
            {t("settings.memory.retention.cold", { count: retention.cold })}
          </span>
          <span className="aspect-mem-retention-chip" data-tier="evictable" title={t("settings.memory.retention.evictableHint")}>
            {t("settings.memory.retention.evictable", { count: retention.evictable })}
          </span>
        </div>
      )}
      {pruneResult && <p className="aspect-mem-prune-result">{pruneResult}</p>}

      {showAdd && (
        <MemoryComposer
          t={t}
          defaultCategory={category ?? "semantic"}
          onCreated={() => { setShowAdd(false); void refresh(); }}
          onError={setError}
        />
      )}

      <div className="aspect-mem-controls">
        <input
          className="aspect-mem-search"
          type="search"
          value={query}
          placeholder={t("settings.memory.searchPlaceholder")}
          onChange={(event) => setQuery(event.target.value)}
        />
        <CompactDropdown
          className="aspect-mem-select"
          label={t("settings.memory.sort.label")}
          value={sort}
          options={[
            { label: t("settings.memory.sort.relevance"), value: "relevance" as MemorySortOrder },
            { label: t("settings.memory.sort.recent"), value: "recent" as MemorySortOrder },
            { label: t("settings.memory.sort.importance"), value: "importance" as MemorySortOrder },
            { label: t("settings.memory.sort.oldest"), value: "oldest" as MemorySortOrder },
          ]}
          onChange={(value) => setSort(value as MemorySortOrder)}
        />
        <label className="aspect-mem-superseded-toggle">
          <input type="checkbox" checked={includeSuperseded} onChange={(event) => setIncludeSuperseded(event.target.checked)} />
          {t("settings.memory.showSuperseded")}
        </label>
      </div>

      {categories.length > 0 && (
        <div className="aspect-mem-cats">
          <button type="button" data-active={category === null} onClick={() => setCategory(null)}>
            {t("settings.memory.allCategories")}
          </button>
          {categories.map((entry) => (
            <button key={entry.category} type="button" data-active={category === entry.category} onClick={() => setCategory(entry.category)}>
              {entry.category} <span>{entry.count}</span>
            </button>
          ))}
        </div>
      )}

      {error && <p className="aspect-mem-error" role="alert">{error}</p>}
      {loading && <p className="aspect-mem-loading"><Loader2 size={14} className="aspect-spin" /> {t("settings.memory.loading")}</p>}

      <ul className="aspect-mem-list">
        {records.map((record) => (
          <MemoryRow
            key={record.id}
            record={record}
            expanded={expandedId === record.id}
            onToggleExpand={() => setExpandedId((current) => (current === record.id ? null : record.id))}
            onTogglePin={() => void togglePin(record)}
            onDelete={() => void remove(record)}
            onChanged={refresh}
            t={t}
          />
        ))}
        {!loading && records.length === 0 && <li className="aspect-mem-none">{t("settings.memory.none")}</li>}
      </ul>

      {(stats?.total ?? 0) > 0 && (
        <footer className="aspect-mem-foot">
          <button type="button" className="aspect-mem-wipe" onClick={() => void wipeAll()}>
            {t("settings.memory.wipeAll")}
          </button>
        </footer>
      )}
    </div>
  );
}

/** One memory row; expands in place to show its knowledge-graph relations
 *  (fetched lazily on first expand) with an unrelate action per edge. */
function MemoryRow({
  record,
  expanded,
  onToggleExpand,
  onTogglePin,
  onDelete,
  onChanged,
  t,
}: {
  record: MemoryRecord;
  expanded: boolean;
  onToggleExpand: () => void;
  onTogglePin: () => void;
  onDelete: () => void;
  onChanged: () => void;
  t: TranslateFn;
}) {
  const [relations, setRelations] = useState<MemoryRelation[] | null>(null);
  const [relationsError, setRelationsError] = useState<string | null>(null);
  const [loadingRelations, setLoadingRelations] = useState(false);

  useEffect(() => {
    if (!expanded || relations !== null) return;
    let cancelled = false;
    setLoadingRelations(true);
    setRelationsError(null);
    luxCommands.memoryRelations(record.id)
      .then((list) => { if (!cancelled) setRelations(list); })
      .catch((cause) => { if (!cancelled) setRelationsError(cause instanceof Error ? cause.message : String(cause)); })
      .finally(() => { if (!cancelled) setLoadingRelations(false); });
    return () => { cancelled = true; };
  }, [expanded, relations, record.id]);

  const unrelate = useCallback(async (relationId: string) => {
    try {
      await luxCommands.memoryUnrelate(relationId);
      setRelations((current) => current?.filter((relation) => relation.id !== relationId) ?? current);
    } catch (cause) {
      setRelationsError(cause instanceof Error ? cause.message : String(cause));
    }
  }, []);

  return (
    <li className="aspect-mem-row" data-pinned={record.pinned || undefined} data-superseded={record.superseded || undefined}>
      <div className="aspect-mem-row-top">
        <button type="button" className="aspect-mem-expand" onClick={onToggleExpand} aria-label={t("settings.memory.relations.toggle")}>
          {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        </button>
        <div className="aspect-mem-row-main">
          <p className="aspect-mem-content">{record.content}</p>
          <div className="aspect-mem-meta">
            <span className="aspect-mem-badge">{record.category}</span>
            <span className="aspect-mem-imp" title={t("settings.memory.importance")}>{Math.round(record.importance * 100)}%</span>
            {record.source && <span className="aspect-mem-src">{record.source}</span>}
            {record.superseded && <span className="aspect-mem-superseded-pill">{t("settings.memory.superseded")}</span>}
          </div>
        </div>
        <div className="aspect-mem-row-actions">
          <button type="button" title={record.pinned ? t("settings.memory.unpin") : t("settings.memory.pin")} onClick={onTogglePin}>
            {record.pinned ? <PinOff size={14} /> : <Pin size={14} />}
          </button>
          <button type="button" className="aspect-mem-danger" title={t("settings.memory.delete")} onClick={onDelete}>
            <Trash2 size={14} />
          </button>
        </div>
      </div>
      {expanded && (
        <div className="aspect-mem-relations">
          {loadingRelations && <p className="aspect-mem-relations-loading"><Loader2 size={12} className="aspect-spin" /> {t("settings.memory.loading")}</p>}
          {relationsError && <p className="aspect-mem-error" role="alert">{relationsError}</p>}
          {!loadingRelations && relations && relations.length === 0 && (
            <p className="aspect-mem-relations-none">{t("settings.memory.relations.none")}</p>
          )}
          {!loadingRelations && relations && relations.length > 0 && (
            <ul className="aspect-mem-relations-list">
              {relations.map((relation) => {
                const otherId = relation.sourceId === record.id ? relation.targetId : relation.sourceId;
                const direction = relation.sourceId === record.id ? "РІвЂ вЂ™" : "РІвЂ С’";
                return (
                  <li key={relation.id} className="aspect-mem-relation-row">
                    <Link2 size={12} />
                    <span className="aspect-mem-relation-kind">{relation.relation}</span>
                    <span className="aspect-mem-relation-dir">{direction}</span>
                    <span className="aspect-mem-relation-target" title={otherId}>{otherId}</span>
                    <span className="aspect-mem-relation-confidence">{Math.round(relation.confidence * 100)}%</span>
                    <button
                      type="button"
                      className="aspect-mem-relation-unrelate"
                      title={t("settings.memory.relations.unrelate")}
                      onClick={() => { void unrelate(relation.id); void onChanged(); }}
                    >
                      <X size={12} />
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      )}
    </li>
  );
}

function MemoryComposer({
  t,
  defaultCategory,
  onCreated,
  onError,
}: {
  t: TranslateFn;
  defaultCategory: string;
  onCreated: () => void;
  onError: (message: string) => void;
}) {
  const [content, setContent] = useState("");
  const [category, setCategory] = useState(defaultCategory);
  const [importance, setImportance] = useState(0.5);
  const [pinned, setPinned] = useState(false);
  const [saving, setSaving] = useState(false);

  const categoryOptions = useMemo(() => {
    const options = COMMON_CATEGORIES.map((entry) => ({ label: entry, value: entry }));
    if (defaultCategory && !COMMON_CATEGORIES.includes(defaultCategory)) {
      options.unshift({ label: defaultCategory, value: defaultCategory });
    }
    return options;
  }, [defaultCategory]);

  const save = async () => {
    if (!content.trim()) return;
    setSaving(true);
    try {
      await luxCommands.memoryCreate({
        category: category.trim() || "semantic",
        content: content.trim(),
        importance,
        pinned,
        source: "user",
      });
      setContent("");
      onCreated();
    } catch (cause) {
      onError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="aspect-mem-composer">
      <textarea
        className="aspect-mem-composer-text"
        value={content}
        placeholder={t("settings.memory.contentPlaceholder")}
        onChange={(event) => setContent(event.target.value)}
        rows={3}
      />
      <div className="aspect-mem-composer-row">
        <CompactDropdown
          className="aspect-mem-composer-cat"
          label={t("settings.memory.category")}
          value={category}
          options={categoryOptions}
          onChange={(value) => setCategory(value)}
        />
        <label className="aspect-mem-composer-imp">
          {t("settings.memory.importance")}
          <input
            type="range"
            min={0}
            max={1}
            step={0.05}
            value={importance}
            // --range-progress drives the filled part of the modern slider track.
            style={{ "--range-progress": `${Math.round(importance * 100)}%` } as CSSProperties}
            onChange={(event) => setImportance(Number(event.target.value))}
          />
          <span>{Math.round(importance * 100)}%</span>
        </label>
        <label className="aspect-mem-composer-pin">
          <input type="checkbox" checked={pinned} onChange={(event) => setPinned(event.target.checked)} />
          {t("settings.memory.pin")}
        </label>
        <button type="button" className="aspect-mem-composer-save" disabled={!content.trim() || saving} onClick={() => void save()}>
          {saving ? <><Loader2 size={14} className="aspect-spin" /> {t("settings.memory.save")}</> : t("settings.memory.save")}
        </button>
      </div>
    </div>
  );
}

