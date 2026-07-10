import { Loader2, MessageCircle, X } from "lucide-react";
import { useAspectLinkStore } from '../../lib/aspect/link-store';
import { luxCommands } from '../../lib/tauri/commands';

export function AspectLinkModal() {
  const { open, phase, code, deepLink, error, hide } = useAspectLinkStore();
  if (!open) return null;

  return (
    <div className="aspect-link-overlay" role="dialog" aria-modal="true" aria-label="Connect AspectIDE">
      <div className="aspect-link-card">
        <button className="aspect-link-close" type="button" onClick={hide} aria-label="Close">
          <X size={16} />
        </button>
        <div className="aspect-link-icon">
          <MessageCircle size={26} strokeWidth={1.8} />
        </div>
        <h2 className="aspect-link-title">Connect AspectIDE</h2>

        {phase === "starting" && <p className="aspect-link-muted">Preparing your link…</p>}

        {phase === "waiting" && (
          <>
            <p className="aspect-link-body">
              Free models are unlocked by linking your Telegram — <b>one account = your own limits</b>.
              Click below, then solve the quick example the bot sends you.
            </p>
            {deepLink ? (
              <button
                className="aspect-link-cta"
                type="button"
                onClick={() => void (luxCommands as unknown as Record<string, (url: string) => Promise<void>>).aspectOpenUrl(deepLink).catch(() => undefined)}
              >
                Open Telegram &amp; verify
              </button>
            ) : (
              <p className="aspect-link-muted">
                Open <b>@AspectIDE_bot</b> and send <code>/start {code}</code>
              </p>
            )}
            <p className="aspect-link-waiting">
              <Loader2 size={14} className="aspect-link-spin" /> Waiting for you to finish in Telegram…
            </p>
          </>
        )}

        {phase === "linked" && <p className="aspect-link-ok">✅ Linked! You're all set.</p>}

        {phase === "error" && (
          <>
            <p className="aspect-link-err">Couldn't start linking: {error}</p>
            <button className="aspect-link-cta" type="button" onClick={hide}>
              Close
            </button>
          </>
        )}
      </div>
    </div>
  );
}
