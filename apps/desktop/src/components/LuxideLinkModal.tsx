import { Loader2, MessageCircle, X } from "lucide-react";
import { useLuxideLinkStore } from "../lib/luxideLinkStore";
import { luxCommands } from "../lib/tauri";

/**
 * Modal that walks the user through linking their Telegram account to unlock the
 * free bundled LuxIDE models. Driven by luxideEnroll.ts via useLuxideLinkStore:
 * shows a deep link to @LuxIDE_bot, the user solves a captcha there, and the flow
 * polls until the gateway returns a token (then auto-closes).
 */
export function LuxideLinkModal() {
  const { open, phase, code, deepLink, error, hide } = useLuxideLinkStore();
  if (!open) return null;

  return (
    <div className="luxide-link-overlay" role="dialog" aria-modal="true" aria-label="Connect LuxIDE">
      <div className="luxide-link-card">
        <button className="luxide-link-close" type="button" onClick={hide} aria-label="Close">
          <X size={16} />
        </button>
        <div className="luxide-link-icon">
          <MessageCircle size={26} strokeWidth={1.8} />
        </div>
        <h2 className="luxide-link-title">Connect LuxIDE</h2>

        {phase === "starting" && <p className="luxide-link-muted">Preparing your link…</p>}

        {phase === "waiting" && (
          <>
            <p className="luxide-link-body">
              Free models are unlocked by linking your Telegram — <b>one account = your own limits</b>.
              Click below, then solve the quick example the bot sends you.
            </p>
            {deepLink ? (
              <button
                className="luxide-link-cta"
                type="button"
                onClick={() => void luxCommands.luxideOpenUrl(deepLink).catch(() => undefined)}
              >
                Open Telegram &amp; verify
              </button>
            ) : (
              <p className="luxide-link-muted">
                Open <b>@LuxIDE_bot</b> and send <code>/start {code}</code>
              </p>
            )}
            <p className="luxide-link-waiting">
              <Loader2 size={14} className="luxide-link-spin" /> Waiting for you to finish in Telegram…
            </p>
          </>
        )}

        {phase === "linked" && <p className="luxide-link-ok">✅ Linked! You're all set.</p>}

        {phase === "error" && (
          <>
            <p className="luxide-link-err">Couldn’t start linking: {error}</p>
            <button className="luxide-link-cta" type="button" onClick={hide}>
              Close
            </button>
          </>
        )}
      </div>
    </div>
  );
}
