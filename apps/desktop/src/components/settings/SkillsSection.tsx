import { Check, DownloadCloud, Loader2, Pencil, Plus, Trash2, Wand2 } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import { luxCommands, type ImportableSkill, type Skill, type SkillDraft, type SkillScope } from "../../lib/tauri";
import type { WorkspaceInfo } from "../../lib/types";

type ScopeFilter = "all" | SkillScope;

/** Slugify a skill name into a safe `[a-z0-9_-]` identifier. */
function slugify(name: string): string {
  return name
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 96);
}

/** Manage reusable agent skills across the global library and the open project. */
export function SkillsSection({ workspace, t }: { workspace: WorkspaceInfo | null; t: TranslateFn }) {
  const [skills, setSkills] = useState<Skill[]>([]);
  const [scopeFilter, setScopeFilter] = useState<ScopeFilter>("all");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<Skill | "new" | "import" | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setSkills(await luxCommands.skillsList());
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const visible = useMemo(
    () => skills.filter((skill) => scopeFilter === "all" || skill.scope === scopeFilter),
    [skills, scopeFilter],
  );

  const toggleEnabled = useCallback(async (skill: Skill) => {
    try {
      // In-place flag flip — preserves the file's other content (vs a full re-render).
      await luxCommands.skillsSetEnabled(skill.scope, skill.slug, !skill.enabled);
      void refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [refresh]);

  const remove = useCallback(async (skill: Skill) => {
    if (!window.confirm(t("settings.skills.confirmDelete", { name: skill.name }))) return;
    try {
      await luxCommands.skillsDelete(skill.scope, skill.slug);
      void refresh();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    }
  }, [refresh, t]);

  if (editing === "import") {
    return (
      <SkillImporter
        t={t}
        onClose={() => { setEditing(null); void refresh(); }}
      />
    );
  }
  if (editing) {
    return (
      <SkillEditor
        existing={editing === "new" ? null : editing}
        canUseProject={Boolean(workspace)}
        t={t}
        onClose={() => setEditing(null)}
        onSaved={() => { setEditing(null); void refresh(); }}
      />
    );
  }

  return (
    <div className="lux-skill">
      <header className="lux-skill-head">
        <div className="lux-skill-scopes">
          {(["all", "project", "global"] as ScopeFilter[]).map((scope) => (
            <button key={scope} type="button" data-active={scopeFilter === scope} onClick={() => setScopeFilter(scope)}>
              {t(`settings.skills.scope.${scope}` as "settings.skills.scope.all")}
            </button>
          ))}
        </div>
        <div className="lux-skill-head-actions">
          <button type="button" className="lux-skill-import" onClick={() => setEditing("import")}>
            <DownloadCloud size={14} /> {t("settings.skills.import")}
          </button>
          <button type="button" className="lux-skill-new" onClick={() => setEditing("new")}>
            <Plus size={14} /> {t("settings.skills.new")}
          </button>
        </div>
      </header>

      {error && <p className="lux-skill-error" role="alert">{error}</p>}
      {loading && <p className="lux-skill-loading"><Loader2 size={14} className="lux-spin" /> {t("settings.skills.loading")}</p>}

      <ul className="lux-skill-list">
        {visible.map((skill) => (
          <li key={`${skill.scope}:${skill.slug}`} className="lux-skill-row" data-disabled={!skill.enabled || undefined}>
            <div className="lux-skill-row-main">
              <div className="lux-skill-row-title">
                <Wand2 size={14} />
                <span className="lux-skill-name">{skill.name}</span>
                <span className="lux-skill-scope-badge" data-scope={skill.scope}>
                  {t(`settings.skills.scope.${skill.scope}` as "settings.skills.scope.project")}
                </span>
                {!skill.enabled && <span className="lux-skill-off">{t("settings.skills.disabled")}</span>}
              </div>
              <p className="lux-skill-desc">{skill.description || t("settings.skills.noDescription")}</p>
              {skill.tags.length > 0 && (
                <div className="lux-skill-tags">
                  {skill.tags.map((tag) => <span key={tag}>{tag}</span>)}
                </div>
              )}
            </div>
            <div className="lux-skill-row-actions">
              <label className="lux-skill-toggle" title={t("settings.skills.enabledToggle")}>
                <input type="checkbox" checked={skill.enabled} onChange={() => void toggleEnabled(skill)} />
                <span />
              </label>
              <button type="button" title={t("settings.skills.edit")} onClick={() => setEditing(skill)}>
                <Pencil size={14} />
              </button>
              <button type="button" className="lux-skill-danger" title={t("settings.skills.delete")} onClick={() => void remove(skill)}>
                <Trash2 size={14} />
              </button>
            </div>
          </li>
        ))}
        {!loading && visible.length === 0 && <li className="lux-skill-none">{t("settings.skills.none")}</li>}
      </ul>
    </div>
  );
}

function SkillEditor({
  existing,
  canUseProject,
  t,
  onClose,
  onSaved,
}: {
  existing: Skill | null;
  canUseProject: boolean;
  t: TranslateFn;
  onClose: () => void;
  onSaved: () => void;
}) {
  const [name, setName] = useState(existing?.name ?? "");
  const [scope, setScope] = useState<SkillScope>(existing?.scope ?? (canUseProject ? "project" : "global"));
  const [description, setDescription] = useState(existing?.description ?? "");
  const [whenToUse, setWhenToUse] = useState(existing?.whenToUse ?? "");
  const [tags, setTags] = useState((existing?.tags ?? []).join(", "));
  const [allowedTools, setAllowedTools] = useState((existing?.allowedTools ?? []).join(", "));
  const [body, setBody] = useState(existing?.body ?? "");
  const [enabled, setEnabled] = useState(existing?.enabled ?? true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const slug = existing?.slug ?? slugify(name);
  const canSave = Boolean(name.trim() && description.trim() && body.trim() && slug);

  const save = async () => {
    if (!canSave) return;
    setSaving(true);
    setError(null);
    try {
      const draft: SkillDraft = {
        name: name.trim(),
        description: description.trim(),
        whenToUse: whenToUse.trim() || undefined,
        tags: splitCsv(tags),
        allowedTools: splitCsv(allowedTools),
        enabled,
        body,
      };
      await luxCommands.skillsSave(scope, slug, draft);
      onSaved();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="lux-skill-editor">
      <div className="lux-skill-editor-head">
        <h3>{existing ? t("settings.skills.editTitle", { name: existing.name }) : t("settings.skills.newTitle")}</h3>
        <button type="button" className="lux-skill-editor-back" onClick={onClose}>{t("settings.skills.cancel")}</button>
      </div>

      <label className="lux-skill-field">
        <span>{t("settings.skills.field.name")}</span>
        <input value={name} disabled={Boolean(existing)} onChange={(event) => setName(event.target.value)} placeholder="pdf-tools" />
        {!existing && slug && <small className="lux-skill-slug">{t("settings.skills.slugPreview", { slug })}</small>}
      </label>

      <label className="lux-skill-field">
        <span>{t("settings.skills.field.scope")}</span>
        <select value={scope} disabled={Boolean(existing)} onChange={(event) => setScope(event.target.value as SkillScope)}>
          <option value="global">{t("settings.skills.scope.global")}</option>
          <option value="project" disabled={!canUseProject}>{t("settings.skills.scope.project")}</option>
        </select>
        {!canUseProject && scope === "global" && <small>{t("settings.skills.projectNeedsWorkspace")}</small>}
      </label>

      <label className="lux-skill-field">
        <span>{t("settings.skills.field.description")}</span>
        <input value={description} onChange={(event) => setDescription(event.target.value)} placeholder={t("settings.skills.descriptionHint")} />
      </label>

      <label className="lux-skill-field">
        <span>{t("settings.skills.field.whenToUse")}</span>
        <input value={whenToUse} onChange={(event) => setWhenToUse(event.target.value)} />
      </label>

      <div className="lux-skill-field-row">
        <label className="lux-skill-field">
          <span>{t("settings.skills.field.tags")}</span>
          <input value={tags} onChange={(event) => setTags(event.target.value)} placeholder="git, ci" />
        </label>
        <label className="lux-skill-field">
          <span>{t("settings.skills.field.allowedTools")}</span>
          <input value={allowedTools} onChange={(event) => setAllowedTools(event.target.value)} placeholder="Read, Shell" />
        </label>
      </div>

      <label className="lux-skill-field">
        <span>{t("settings.skills.field.body")}</span>
        <textarea value={body} onChange={(event) => setBody(event.target.value)} rows={12} placeholder={t("settings.skills.bodyHint")} />
      </label>

      <label className="lux-skill-enabled">
        <input type="checkbox" checked={enabled} onChange={(event) => setEnabled(event.target.checked)} />
        {t("settings.skills.enabledToggle")}
      </label>

      {error && <p className="lux-skill-error" role="alert">{error}</p>}

      <div className="lux-skill-editor-actions">
        <button type="button" className="lux-skill-save" disabled={!canSave || saving} onClick={() => void save()}>
          {saving ? <><Loader2 size={14} className="lux-spin" /> {t("settings.skills.save")}</> : t("settings.skills.save")}
        </button>
      </div>
    </div>
  );
}

function splitCsv(value: string): string[] {
  return value
    .split(",")
    .map((item) => item.trim())
    .filter((item) => item.length > 0);
}

/** Import skills auto-discovered in other agents' folders (Claude, Codex,
 *  OpenClaw, Hermes) — one at a time or all at once — or by pasting a raw
 *  SKILL.md. Every import lands in the global library (skills are not scoped to
 *  a single project). */
function SkillImporter({
  t,
  onClose,
}: {
  t: TranslateFn;
  onClose: () => void;
}) {
  const [candidates, setCandidates] = useState<ImportableSkill[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [importingAll, setImportingAll] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [imported, setImported] = useState<Set<string>>(new Set());
  const [manualName, setManualName] = useState("");
  const [manualContent, setManualContent] = useState("");

  useEffect(() => {
    let active = true;
    void luxCommands
      .skillsDiscoverImportable()
      .then((found) => { if (active) setCandidates(found); })
      .catch((cause) => { if (active) setError(cause instanceof Error ? cause.message : String(cause)); })
      .finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, []);

  // Skills are global-only here: a single library shared across every project.
  const TARGET_SCOPE: SkillScope = "global";

  const doImport = useCallback(async (key: string, slug: string, content: string) => {
    setBusy(key);
    setError(null);
    try {
      await luxCommands.skillsImport(TARGET_SCOPE, slug, content);
      setImported((prev) => new Set(prev).add(key));
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setBusy(null);
    }
  }, []);

  const candidateKey = (candidate: ImportableSkill) => `${candidate.source}:${candidate.slug}`;
  const pending = candidates.filter((candidate) => !imported.has(candidateKey(candidate)));

  // Import every not-yet-imported discovered skill in one click. Sequential so a
  // single failure surfaces without aborting the rest; each success ticks the row
  // over to "Imported" live so progress is visible.
  const doImportAll = useCallback(async () => {
    setImportingAll(true);
    setError(null);
    setNotice(null);
    let imports = 0;
    let failures = 0;
    for (const candidate of candidates) {
      const key = candidateKey(candidate);
      if (imported.has(key)) continue;
      try {
        await luxCommands.skillsImport(TARGET_SCOPE, candidate.slug, candidate.content);
        setImported((prev) => new Set(prev).add(key));
        imports += 1;
      } catch (cause) {
        failures += 1;
        setError(cause instanceof Error ? cause.message : String(cause));
      }
    }
    setImportingAll(false);
    if (failures === 0 && imports > 0) setNotice(t("settings.skills.importAllDone", { count: imports }));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [candidates, imported, t]);

  const manualSlug = slugify(manualName);
  const canImportManual = Boolean(manualSlug && manualContent.trim());

  return (
    <div className="lux-skill-importer">
      <div className="lux-skill-editor-head">
        <h3>{t("settings.skills.importTitle")}</h3>
        <button type="button" className="lux-skill-editor-back" onClick={onClose}>{t("settings.skills.done")}</button>
      </div>

      <p className="lux-skill-import-note">{t("settings.skills.importGlobalOnly")}</p>

      {error && <p className="lux-skill-error" role="alert">{error}</p>}
      {notice && <p className="lux-skill-import-success" role="status">{notice}</p>}

      <div className="lux-skill-import-head">
        <h4 className="lux-skill-import-h">{t("settings.skills.importDiscovered")}</h4>
        {pending.length > 0 && (
          <button
            type="button"
            className="lux-skill-import-all"
            disabled={importingAll || busy !== null}
            onClick={() => void doImportAll()}
          >
            {importingAll
              ? <><Loader2 size={13} className="lux-spin" /> {t("settings.skills.importingAll")}</>
              : <><DownloadCloud size={13} /> {t("settings.skills.importAllCount", { count: pending.length })}</>}
          </button>
        )}
      </div>
      {loading && <p className="lux-skill-loading"><Loader2 size={14} className="lux-spin" /> {t("settings.skills.importScanning")}</p>}
      {!loading && candidates.length === 0 && <p className="lux-skill-none">{t("settings.skills.importNone")}</p>}
      {candidates.length > 0 && (
      <ul className="lux-skill-import-list">
        {candidates.map((candidate) => {
          const key = candidateKey(candidate);
          const done = imported.has(key);
          return (
            <li key={key} className="lux-skill-import-row">
              <div className="lux-skill-row-main">
                <div className="lux-skill-row-title">
                  <Wand2 size={14} />
                  <span className="lux-skill-name">{candidate.name}</span>
                  <span className="lux-skill-scope-badge">{candidate.source}</span>
                </div>
                <p className="lux-skill-desc">{candidate.description || t("settings.skills.noDescription")}</p>
              </div>
              <button
                type="button"
                className="lux-skill-import-btn"
                data-done={done || undefined}
                disabled={done || busy === key || importingAll}
                onClick={() => void doImport(key, candidate.slug, candidate.content)}
              >
                {done ? <><Check size={13} /> {t("settings.skills.imported")}</>
                  : busy === key ? <><Loader2 size={13} className="lux-spin" /> {t("settings.skills.importAction")}</>
                  : t("settings.skills.importAction")}
              </button>
            </li>
          );
        })}
      </ul>
      )}

      <h4 className="lux-skill-import-h">{t("settings.skills.importManual")}</h4>
      <label className="lux-skill-field">
        <span>{t("settings.skills.field.name")}</span>
        <input value={manualName} onChange={(event) => setManualName(event.target.value)} placeholder="my-skill" />
        {manualSlug && <small className="lux-skill-slug">{t("settings.skills.slugPreview", { slug: manualSlug })}</small>}
      </label>
      <label className="lux-skill-field">
        <span>{t("settings.skills.importPaste")}</span>
        <textarea value={manualContent} onChange={(event) => setManualContent(event.target.value)} rows={8} placeholder={"---\nname: my-skill\ndescription: when to use\n---\n…"} />
      </label>
      <div className="lux-skill-editor-actions">
        <button
          type="button"
          className="lux-skill-save"
          disabled={!canImportManual || busy === "manual"}
          onClick={() => { void doImport("manual", manualSlug, manualContent).then(() => { setManualName(""); setManualContent(""); }); }}
        >
          {busy === "manual" ? <><Loader2 size={14} className="lux-spin" /> {t("settings.skills.importAction")}</> : t("settings.skills.importAction")}
        </button>
      </div>
    </div>
  );
}
