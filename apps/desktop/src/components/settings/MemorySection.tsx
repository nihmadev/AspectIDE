import { Brain, Loader2, Pin, PinOff, Plus, Trash2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import {
  luxCommands,
  type MemoryRecord,
  type MemorySortOrder,
  type MemoryStats,
} from "../../lib/tauri";
import type { WorkspaceInfo } from "../../lib/types";

const COMMON_CATEGORIES = ["core", "semantic", "episodic", "procedural"];

/** Per-project durable memory browser: search, filter, pin, edit, and prune the
 *  memories the agent (and the user) accumulate for the open workspace. */
export function MemorySection({ workspace, t }: { workspace: WorkspaceInfo | null; t: TranslateFn }) {
  const [records, setRecords] = useState<MemoryRecord[]>([]);
  const [stats, setStats] = useState<MemoryStats | null>(null);
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const [sort, setSort] = useState<MemorySortOrder>("relevance");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showAdd, setShowAdd] = useState(false);

  const seqRef = useRef(0);
  const refresh = useCallback(async () => {
    if (!workspace) return;
    const seq = ++seqRef.current;
    setLoading(true);
    setError(null);
    try {
      // touch:false — browsing the manager must not inflate recall recency/usage;
      // only the agent's RecallMemory bumps access stats.
      const options = { category, limit: 200, sort, touch: false };
      const list = query.trim()
        ? await luxCommands.memorySearch(query.trim(), options)
        : await luxCommands.memoryList(options);
      const nextStats = await luxCommands.memoryStats();
      // Latest-wins: a newer refresh has superseded this one — drop its results.
      if (seq !== seqRef.current) return;
      setRecords(list);
      setStats(nextStats);
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
  }, [workspace, query, category, sort]);

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
    // (with_memory → workspace_root), not global. Name the project in the prompt so
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

  const categories = useMemo(() => stats?.byCategory ?? [], [stats]);

  if (!workspace) {
    return <p className="lux-mem-empty">{t("settings.memory.needProject")}</p>;
  }

  return (
    <div className="lux-mem">
      <header className="lux-mem-head">
        <div className="lux-mem-stat">
          <Brain size={15} />
          <span>{t("settings.memory.total", { count: stats?.total ?? 0 })}</span>
          {(stats?.pinned ?? 0) > 0 && <span className="lux-mem-pincount">{t("settings.memory.pinned", { count: stats?.pinned ?? 0 })}</span>}
        </div>
        <button type="button" className="lux-mem-add-btn" onClick={() => setShowAdd((open) => !open)}>
          <Plus size={14} /> {t("settings.memory.add")}
        </button>
      </header>

      {showAdd && (
        <MemoryComposer
          t={t}
          defaultCategory={category ?? "semantic"}
          onCreated={() => { setShowAdd(false); void refresh(); }}
          onError={setError}
        />
      )}

      <div className="lux-mem-controls">
        <input
          className="lux-mem-search"
          type="search"
          value={query}
          placeholder={t("settings.memory.searchPlaceholder")}
          onChange={(event) => setQuery(event.target.value)}
        />
        <select className="lux-mem-select" value={sort} onChange={(event) => setSort(event.target.value as MemorySortOrder)}>
          <option value="relevance">{t("settings.memory.sort.relevance")}</option>
          <option value="recent">{t("settings.memory.sort.recent")}</option>
          <option value="importance">{t("settings.memory.sort.importance")}</option>
          <option value="oldest">{t("settings.memory.sort.oldest")}</option>
        </select>
      </div>

      {categories.length > 0 && (
        <div className="lux-mem-cats">
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

      {error && <p className="lux-mem-error" role="alert">{error}</p>}
      {loading && <p className="lux-mem-loading"><Loader2 size={14} className="lux-spin" /> {t("settings.memory.loading")}</p>}

      <ul className="lux-mem-list">
        {records.map((record) => (
          <li key={record.id} className="lux-mem-row" data-pinned={record.pinned || undefined}>
            <div className="lux-mem-row-main">
              <p className="lux-mem-content">{record.content}</p>
              <div className="lux-mem-meta">
                <span className="lux-mem-badge">{record.category}</span>
                <span className="lux-mem-imp" title={t("settings.memory.importance")}>{Math.round(record.importance * 100)}%</span>
                {record.source && <span className="lux-mem-src">{record.source}</span>}
              </div>
            </div>
            <div className="lux-mem-row-actions">
              <button type="button" title={record.pinned ? t("settings.memory.unpin") : t("settings.memory.pin")} onClick={() => void togglePin(record)}>
                {record.pinned ? <PinOff size={14} /> : <Pin size={14} />}
              </button>
              <button type="button" className="lux-mem-danger" title={t("settings.memory.delete")} onClick={() => void remove(record)}>
                <Trash2 size={14} />
              </button>
            </div>
          </li>
        ))}
        {!loading && records.length === 0 && <li className="lux-mem-none">{t("settings.memory.none")}</li>}
      </ul>

      {(stats?.total ?? 0) > 0 && (
        <footer className="lux-mem-foot">
          <button type="button" className="lux-mem-wipe" onClick={() => void wipeAll()}>
            {t("settings.memory.wipeAll")}
          </button>
        </footer>
      )}
    </div>
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
    <div className="lux-mem-composer">
      <textarea
        className="lux-mem-composer-text"
        value={content}
        placeholder={t("settings.memory.contentPlaceholder")}
        onChange={(event) => setContent(event.target.value)}
        rows={3}
      />
      <div className="lux-mem-composer-row">
        <input
          className="lux-mem-composer-cat"
          list="lux-mem-categories"
          value={category}
          onChange={(event) => setCategory(event.target.value)}
          placeholder={t("settings.memory.category")}
        />
        <datalist id="lux-mem-categories">
          {COMMON_CATEGORIES.map((entry) => <option key={entry} value={entry} />)}
        </datalist>
        <label className="lux-mem-composer-imp">
          {t("settings.memory.importance")}
          <input type="range" min={0} max={1} step={0.05} value={importance} onChange={(event) => setImportance(Number(event.target.value))} />
          <span>{Math.round(importance * 100)}%</span>
        </label>
        <label className="lux-mem-composer-pin">
          <input type="checkbox" checked={pinned} onChange={(event) => setPinned(event.target.checked)} />
          {t("settings.memory.pin")}
        </label>
        <button type="button" className="lux-mem-composer-save" disabled={!content.trim() || saving} onClick={() => void save()}>
          {saving ? <><Loader2 size={14} className="lux-spin" /> {t("settings.memory.save")}</> : t("settings.memory.save")}
        </button>
      </div>
    </div>
  );
}
