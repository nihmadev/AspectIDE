import { Globe2, Loader2, Search } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import type { TranslateFn } from "../../lib/i18n/useTranslation";
import { luxCommands } from "../../lib/tauri";

const SEARXNG_URL_KEY = "ai.research.searxngUrl";

/**
 * Settings for the deep web-research tool (WebResearch). The only required config
 * is an optional SearxNG base URL; with none set, the tool transparently falls
 * back to a keyless DuckDuckGo search, so research works out of the box.
 */
export function WebResearchSection({ t }: { t: TranslateFn }) {
  const [url, setUrl] = useState("");
  const [loaded, setLoaded] = useState(false);
  const [saveState, setSaveState] = useState<"idle" | "saving" | "saved">("idle");
  const [test, setTest] = useState<{ status: "idle" | "running" | "ok" | "error"; detail: string }>({
    status: "idle",
    detail: "",
  });

  useEffect(() => {
    let active = true;
    void luxCommands
      .settingsGet("user", SEARXNG_URL_KEY)
      .then((value) => {
        if (!active) return;
        const stored = typeof value?.value === "string" ? value.value : "";
        setUrl(stored);
      })
      .catch(() => undefined)
      .finally(() => {
        if (active) setLoaded(true);
      });
    return () => {
      active = false;
    };
  }, []);

  const persist = useCallback(async (next: string) => {
    setSaveState("saving");
    try {
      await luxCommands.settingsSet("user", SEARXNG_URL_KEY, next.trim());
      setSaveState("saved");
      window.setTimeout(() => setSaveState("idle"), 1500);
    } catch {
      setSaveState("idle");
    }
  }, []);

  const runTest = useCallback(async () => {
    setTest({ status: "running", detail: "" });
    try {
      const result = await luxCommands.webResearch(t("settings.research.testQuery"), { maxSources: 3 });
      setTest({
        status: "ok",
        detail: t("settings.research.testOk", { count: result.sourceCount, provider: result.provider }),
      });
    } catch (cause) {
      setTest({ status: "error", detail: cause instanceof Error ? cause.message : String(cause) });
    }
  }, [t]);

  return (
    <div className="lux-research">
      <div className="lux-research-intro">
        <Globe2 size={16} />
        <p>{t("settings.research.intro")}</p>
      </div>

      <label className="lux-research-field">
        <span className="lux-research-label">{t("settings.research.searxngLabel")}</span>
        <input
          className="lux-research-input"
          type="url"
          inputMode="url"
          placeholder="https://searxng.example.com"
          value={url}
          disabled={!loaded}
          onChange={(event) => setUrl(event.target.value)}
          onBlur={() => void persist(url)}
        />
        <small className="lux-research-hint">{t("settings.research.searxngHint")}</small>
      </label>

      <div className="lux-research-status-row">
        <span className="lux-research-save" data-state={saveState}>
          {saveState === "saving" && t("settings.research.saving")}
          {saveState === "saved" && t("settings.research.saved")}
        </span>
        <button type="button" className="lux-research-test" onClick={() => void runTest()} disabled={test.status === "running"}>
          {test.status === "running" ? <Loader2 size={13} className="lux-spin" /> : <Search size={13} />}
          {t("settings.research.test")}
        </button>
      </div>

      {test.status !== "idle" && test.status !== "running" && (
        <p className="lux-research-test-result" data-status={test.status} role="status">
          {test.detail}
        </p>
      )}

      <p className="lux-research-note">{t("settings.research.fallbackNote")}</p>
    </div>
  );
}
