import { Check, ChevronDown, Minus, Plus } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { CompactDropdown } from "../CompactDropdown";
import { withFontFallback } from "../../lib/editorPreferences";
import type { TranslateFn } from "../../lib/i18n/useTranslation";

export type SaveState = "idle" | "saving" | "saved" | "error";

export function SettingsPanel({ children, description, title }: { children: ReactNode; description?: string; title?: string }) {
  return (
    <section className="settings-panel">
      {(title || description) && (
        <div className="settings-panel-head">
          {title && <h3>{title}</h3>}
          {description && <p>{description}</p>}
        </div>
      )}
      {children}
    </section>
  );
}

export function SettingsGrid({ children }: { children: ReactNode }) {
  return <div className="settings-control-grid">{children}</div>;
}

/** Clamp `value` into [min, max], rounding to the nearest valid `step` offset from `min`. */
function clampToStep(value: number, min: number, max: number, step: number) {
  const bounded = Math.min(max, Math.max(min, value));
  if (step <= 0) return bounded;
  // Snap to the step grid anchored at `min`, then re-bound (rounding can push past max).
  const snapped = min + Math.round((bounded - min) / step) * step;
  return Math.min(max, Math.max(min, Number(snapped.toFixed(6))));
}

/**
 * Numeric stepper with a *string draft*: the user can freely clear the field or type
 * a multi-digit / decimal value without each intermediate keystroke being committed.
 * `onCommit` fires only for a finite, in-range (clamped) value, and only on blur,
 * Enter, or a +/- step — never on every change. This prevents transient invalid
 * states (empty → 0, half-typed numbers) from being persisted to runtime settings.
 */
function NumberStepper({ ariaLabel, max, min, onCommit, step, value }: { ariaLabel: string; max: number; min: number; onCommit: (value: number) => void; step: number; value: number }) {
  const [draft, setDraft] = useState(() => String(value));
  const focusedRef = useRef(false);

  // Keep the draft in sync with the committed value while the user isn't editing,
  // so external updates (reset, refresh, stepper) are reflected without clobbering
  // an in-progress edit.
  useEffect(() => {
    if (!focusedRef.current) setDraft(String(value));
  }, [value]);

  const commitDraft = () => {
    const parsed = Number(draft);
    if (draft.trim() === "" || !Number.isFinite(parsed)) {
      setDraft(String(value)); // revert invalid/empty input to the last good value
      return;
    }
    const clamped = clampToStep(parsed, min, max, step);
    setDraft(String(clamped));
    if (clamped !== value) onCommit(clamped);
  };

  const step_ = (delta: number) => {
    const clamped = clampToStep(value + delta, min, max, step);
    setDraft(String(clamped));
    if (clamped !== value) onCommit(clamped);
  };

  return (
    <div className="settings-stepper">
      <button type="button" aria-label={`Decrease ${ariaLabel}`} disabled={value <= min} onClick={() => step_(-step)}><Minus size={13} /></button>
      <input
        aria-label={ariaLabel}
        type="number"
        min={min}
        max={max}
        step={step}
        value={draft}
        onFocus={() => { focusedRef.current = true; }}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={() => { focusedRef.current = false; commitDraft(); }}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            commitDraft();
          } else if (event.key === "Escape") {
            setDraft(String(value)); // discard the edit
          }
        }}
      />
      <button type="button" aria-label={`Increase ${ariaLabel}`} disabled={value >= max} onClick={() => step_(step)}><Plus size={13} /></button>
    </div>
  );
}

export function NumberSetting({ detail, label, max, min, onChange, step = 1, value }: { detail?: string; label: string; max: number; min: number; onChange: (value: number) => void; step?: number; value: number }) {
  return (
    <SettingField detail={detail ?? `${min}-${max}`} label={label}>
      <NumberStepper ariaLabel={label} min={min} max={max} step={step} value={value} onCommit={onChange} />
    </SettingField>
  );
}

export function ToolRoundLimitSetting({ detail, fallbackLimitedValue, label, limitedLabel, max, min, onChange, step = 1, unlimitedLabel, value }: { detail?: string; fallbackLimitedValue: number; label: string; limitedLabel: string; max: number; min: number; onChange: (value: number | null) => void; step?: number; unlimitedLabel: string; value: number | null }) {
  const limitedValue = value ?? fallbackLimitedValue;
  const boundedLimitedValue = clampToStep(limitedValue, min, max, step);
  return (
    <SettingField detail={detail} label={label}>
      <div className="settings-compound-control">
        <div className="settings-segmented" role="radiogroup" aria-label={label}>
          <button type="button" role="radio" aria-checked={value === null} data-active={value === null} onClick={() => onChange(null)}>{unlimitedLabel}</button>
          <button type="button" role="radio" aria-checked={value !== null} data-active={value !== null} onClick={() => onChange(boundedLimitedValue)}>{limitedLabel}</button>
        </div>
        {value !== null && (
          // Reuse the draft/commit stepper so the limited value gets the same
          // clear-to-edit / clamp-on-commit semantics as every other number field.
          <NumberStepper ariaLabel={label} min={min} max={max} step={step} value={boundedLimitedValue} onCommit={onChange} />
        )}
      </div>
    </SettingField>
  );
}

export function SelectSetting<T extends string>({ detail, label, onChange, options, value }: { detail?: string; label: string; onChange: (value: T) => void; options: Array<{ label: string; value: T }>; value: T }) {
  return (
    <SettingField detail={detail} label={label}>
      <label className="settings-select-control">
        <select aria-label={label} value={value} onChange={(event) => onChange(event.currentTarget.value as T)}>
          {options.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
        </select>
        <ChevronDown size={14} />
      </label>
    </SettingField>
  );
}

/**
 * Text input. By default it is controlled and reports every keystroke via `onChange`.
 * Pass `commitOnBlur` for fields wired to live runtime state (provider URLs/keys,
 * aliases): the value is then held in a local draft and only reported on blur or
 * Enter, so a half-typed value can't reach an in-flight AI request mid-typing.
 */
export function TextSetting({ commitOnBlur = false, detail, label, onChange, password = false, placeholder, readOnly = false, value, wide = false }: { commitOnBlur?: boolean; detail?: string; label: string; onChange: (value: string) => void; password?: boolean; placeholder?: string; readOnly?: boolean; value: string; wide?: boolean }) {
  const [draft, setDraft] = useState(value);
  const focusedRef = useRef(false);

  useEffect(() => {
    if (commitOnBlur && !focusedRef.current) setDraft(value);
  }, [commitOnBlur, value]);

  const controlledValue = commitOnBlur ? draft : value;
  const commit = () => {
    focusedRef.current = false;
    if (draft !== value) onChange(draft);
  };

  return (
    <SettingField detail={detail} label={label} wide={wide}>
      <input
        className="settings-input-control"
        aria-label={label}
        type={password ? "password" : "text"}
        value={controlledValue}
        placeholder={placeholder}
        readOnly={readOnly}
        spellCheck={false}
        onFocus={commitOnBlur ? () => { focusedRef.current = true; } : undefined}
        onChange={(event) => (commitOnBlur ? setDraft(event.currentTarget.value) : onChange(event.currentTarget.value))}
        onBlur={commitOnBlur ? commit : undefined}
        onKeyDown={commitOnBlur ? (event) => {
          if (event.key === "Enter") commit();
          else if (event.key === "Escape") setDraft(value);
        } : undefined}
      />
    </SettingField>
  );
}

/**
 * Multi-line text. Like {@link TextSetting}, pass `commitOnBlur` for fields wired to
 * live runtime state (e.g. tool permission rules): the textarea then keeps the exact
 * typed text in a local draft and only reports it on blur, so blank/partial lines
 * aren't persisted and re-normalized away while the user is still composing.
 */
export function TextareaSetting({ commitOnBlur = false, detail, label, onChange, placeholder, rows = 8, value, wide = false }: { commitOnBlur?: boolean; detail?: string; label: string; onChange: (value: string) => void; placeholder?: string; rows?: number; value: string; wide?: boolean }) {
  const [draft, setDraft] = useState(value);
  const focusedRef = useRef(false);

  useEffect(() => {
    if (commitOnBlur && !focusedRef.current) setDraft(value);
  }, [commitOnBlur, value]);

  const controlledValue = commitOnBlur ? draft : value;
  const commit = () => {
    focusedRef.current = false;
    if (draft !== value) onChange(draft);
  };

  return (
    <SettingField detail={detail} label={label} wide={wide}>
      <textarea
        className="settings-textarea-control"
        aria-label={label}
        value={controlledValue}
        placeholder={placeholder}
        rows={rows}
        spellCheck={false}
        onFocus={commitOnBlur ? () => { focusedRef.current = true; } : undefined}
        onChange={(event) => (commitOnBlur ? setDraft(event.currentTarget.value) : onChange(event.currentTarget.value))}
        onBlur={commitOnBlur ? commit : undefined}
      />
    </SettingField>
  );
}

/**
 * Font-family picker: a searchable dropdown over the system font list where every
 * option previews itself in its own typeface. The empty value is the "default"
 * entry (built-in stack); `fonts` may still be loading — the current value and the
 * default entry are always selectable so the control never blocks on the scan.
 */
export function FontSelectSetting({ defaultLabel, detail, fonts, label, onChange, searchEmptyLabel, searchPlaceholder, value }: {
  defaultLabel: string;
  detail?: string;
  fonts: string[];
  label: string;
  onChange: (value: string) => void;
  searchEmptyLabel?: string;
  searchPlaceholder?: string;
  value: string;
}) {
  const options = [
    { label: defaultLabel, value: "" },
    // A persisted font that vanished from the system (or is still loading) stays
    // visible so the trigger reflects the real setting instead of lying "default".
    ...(value && !fonts.includes(value) ? [{ label: value, value }] : []),
    ...fonts.map((family) => ({ label: family, value: family })),
  ];
  return (
    <SettingField detail={detail} label={label}>
      <CompactDropdown
        className="settings-font-dropdown"
        label={label}
        value={value}
        options={options}
        onChange={onChange}
        searchable
        searchPlaceholder={searchPlaceholder}
        searchEmptyLabel={searchEmptyLabel}
        getOptionStyle={(family) => (family ? { fontFamily: withFontFallback(family, "sans-serif") } : undefined)}
      />
    </SettingField>
  );
}

export function SegmentedSetting<T extends string>({ detail, label, onChange, options, value }: { detail?: string; label: string; onChange: (value: T) => void; options: Array<{ label: string; value: T }>; value: T }) {
  return (
    <SettingField detail={detail} label={label}>
      <div className="settings-segmented" role="radiogroup" aria-label={label}>
        {options.map((option) => (
          <button key={option.value} type="button" role="radio" aria-checked={option.value === value} data-active={option.value === value} onClick={() => onChange(option.value)}>{option.label}</button>
        ))}
      </div>
    </SettingField>
  );
}

export function ToggleSetting({ checked, detail, label, onChange }: { checked: boolean; detail?: string; label: string; onChange: (checked: boolean) => void }) {
  return (
    <label className="settings-field settings-toggle-field">
      <span className="settings-field-copy">
        <strong>{label}</strong>
        {detail && <small>{detail}</small>}
      </span>
      <input type="checkbox" checked={checked} onChange={(event) => onChange(event.target.checked)} />
      <span className="settings-switch" aria-hidden="true" />
    </label>
  );
}

function SettingField({ children, detail, label, wide = false }: { children: ReactNode; detail?: string; label: string; wide?: boolean }) {
  return (
    <div className="settings-field" data-wide={wide}>
      <span className="settings-field-copy">
        <strong>{label}</strong>
        {detail && <small>{detail}</small>}
      </span>
      {children}
    </div>
  );
}

export function SaveIndicator({ state, t }: { state: SaveState; t: TranslateFn }) {
  if (state === "idle") return <span className="settings-save-state">{t("settings.save.userSettings")}</span>;
  if (state === "saving") return <span className="settings-save-state">{t("settings.save.saving")}</span>;
  if (state === "error") return <span className="settings-save-state" data-tone="error">{t("settings.save.failed")}</span>;
  return <span className="settings-save-state" data-tone="saved"><Check size={12} /> {t("settings.save.saved")}</span>;
}
