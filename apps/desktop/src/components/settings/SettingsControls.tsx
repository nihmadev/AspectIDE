import { Check, ChevronDown, Minus, Plus } from "lucide-react";
import type { ReactNode } from "react";
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

export function NumberSetting({ detail, label, max, min, onChange, step = 1, value }: { detail?: string; label: string; max: number; min: number; onChange: (value: number) => void; step?: number; value: number }) {
  return (
    <SettingField detail={detail ?? `${min}-${max}`} label={label}>
      <div className="settings-stepper">
        <button type="button" aria-label={`Decrease ${label}`} disabled={value <= min} onClick={() => onChange(value - step)}><Minus size={13} /></button>
        <input aria-label={label} type="number" min={min} max={max} value={value} onChange={(event) => onChange(Number(event.target.value))} />
        <button type="button" aria-label={`Increase ${label}`} disabled={value >= max} onClick={() => onChange(value + step)}><Plus size={13} /></button>
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

export function TextSetting({ detail, label, onChange, password = false, placeholder, readOnly = false, value, wide = false }: { detail?: string; label: string; onChange: (value: string) => void; password?: boolean; placeholder?: string; readOnly?: boolean; value: string; wide?: boolean }) {
  return (
    <SettingField detail={detail} label={label} wide={wide}>
      <input className="settings-input-control" aria-label={label} type={password ? "password" : "text"} value={value} placeholder={placeholder} readOnly={readOnly} spellCheck={false} onChange={(event) => onChange(event.currentTarget.value)} />
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
    <label className="settings-field" data-wide={wide}>
      <span className="settings-field-copy">
        <strong>{label}</strong>
        {detail && <small>{detail}</small>}
      </span>
      {children}
    </label>
  );
}

export function SaveIndicator({ state, t }: { state: SaveState; t: TranslateFn }) {
  if (state === "idle") return <span className="settings-save-state">{t("settings.save.userSettings")}</span>;
  if (state === "saving") return <span className="settings-save-state">{t("settings.save.saving")}</span>;
  if (state === "error") return <span className="settings-save-state" data-tone="error">{t("settings.save.failed")}</span>;
  return <span className="settings-save-state" data-tone="saved"><Check size={12} /> {t("settings.save.saved")}</span>;
}
