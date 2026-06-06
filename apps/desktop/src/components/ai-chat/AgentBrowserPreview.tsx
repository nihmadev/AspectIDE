import { ExternalLink, Globe, LayoutDashboard, Loader2, MousePointer2, Radio, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { AiPreferences } from "../../lib/aiPreferences";
import { getAiChatTurnRuntimeSnapshot, subscribeAiChatTurnRuntime } from "../../lib/aiChatTurnRuntime";
import { browserSessionName, ensureBrowserStream, queryBrowserStream } from "../../lib/agentBrowser";
import { AgentBrowserStreamClient, mapPreviewCoordinates } from "../../lib/agentBrowserStream";
import { useTranslation } from "../../lib/i18n/useTranslation";
import { luxCommands } from "../../lib/tauri";

type PreviewView = "live" | "dashboard";
type BrowserPreviewConnection = "connecting" | "live" | "waiting" | "disconnected" | "error";

type AgentBrowserPreviewProps = {
  chatSessionId: string;
  preferences: AiPreferences;
  onOpenDashboard?: () => void;
  /** Editor tab uses a compact chrome; title comes from the tab bar. */
  variant?: "editor" | "panel";
};

export function AgentBrowserPreview({ chatSessionId, onOpenDashboard, preferences, variant = "editor" }: AgentBrowserPreviewProps) {
  const { t } = useTranslation();
  const [view, setView] = useState<PreviewView>("live");
  const [frameSrc, setFrameSrc] = useState<string | null>(null);
  const [streamUrl, setStreamUrl] = useState<string | null>(null);
  const [connection, setConnection] = useState<BrowserPreviewConnection>("waiting");
  const [error, setError] = useState<string | null>(null);
  const [dashboardUrl, setDashboardUrl] = useState<string | null>(null);
  const [dashboardError, setDashboardError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const viewportRef = useRef<HTMLDivElement | null>(null);
  const clientRef = useRef<AgentBrowserStreamClient | null>(null);
  const hasFrameRef = useRef(false);
  const metadataRef = useRef<{ deviceWidth?: number; deviceHeight?: number }>({});
  const streamRefreshToken = useSyncExternalStore(
    subscribeAiChatTurnRuntime,
    () => getAiChatTurnRuntimeSnapshot().browserStreamRefreshToken,
    () => getAiChatTurnRuntimeSnapshot().browserStreamRefreshToken,
  );

  const connectionLabel = connection === "live"
    ? t("aiChat.browserPreview.live")
    : connection === "waiting"
      ? t("aiChat.browserPreview.waitingSession")
      : connection === "disconnected"
        ? t("aiChat.browserPreview.disconnected")
        : connection === "error"
          ? (error ?? t("aiChat.browserPreview.disconnected"))
          : t("aiChat.browserPreview.connecting");

  const commandPath = preferences.agentBrowserCommand.trim() || undefined;
  const browserEnabled = preferences.agentBrowserEnabled;
  const autoStreamPreview = preferences.agentBrowserAutoStreamPreview;
  const sessionName = browserSessionName(chatSessionId);

  const attachStreamClient = useCallback((url: string) => {
    if (streamUrl === url && clientRef.current) return;
    setStreamUrl(url);
    clientRef.current?.disconnect();
    const client = new AgentBrowserStreamClient({
      url,
      onOpen: () => setConnection(hasFrameRef.current ? "live" : "connecting"),
      onClose: () => {
        if (hasFrameRef.current) setConnection("disconnected");
        else setConnection("waiting");
      },
      onError: (message) => {
        setError(message);
        setConnection("error");
      },
      onFrame: (frame) => {
        metadataRef.current = frame.metadata ?? {};
        hasFrameRef.current = true;
        setConnection("live");
        setFrameSrc(`data:image/jpeg;base64,${frame.data}`);
      },
    });
    clientRef.current = client;
    client.connect();
  }, [streamUrl]);

  const syncLiveStream = useCallback(async (activate: boolean) => {
    if (!browserEnabled || view !== "live") return;
    setRefreshing(true);
    setError(null);
    if (!hasFrameRef.current) setConnection(activate ? "connecting" : "waiting");
    try {
      const status = activate
        ? await ensureBrowserStream(sessionName, commandPath)
        : await queryBrowserStream(sessionName, commandPath);
      const url = status.websocketUrl;
      if (!url) {
        if (!hasFrameRef.current) {
          setStreamUrl(null);
          setFrameSrc(null);
          setConnection("waiting");
        }
        return;
      }
      attachStreamClient(url);
    } catch (streamError) {
      const message = streamError instanceof Error ? streamError.message : String(streamError);
      setError(message);
      setConnection("error");
    } finally {
      setRefreshing(false);
    }
  }, [attachStreamClient, browserEnabled, commandPath, sessionName, view]);

  const ensureDashboard = useCallback(async () => {
    const port = preferences.agentBrowserDashboardPort;
    const url = `http://127.0.0.1:${port}`;
    setDashboardError(null);
    try {
      const response = await luxCommands.agentBrowserDashboard({
        action: "start",
        port,
        commandPath: preferences.agentBrowserCommand.trim() || null,
      });
      if (!response.success) {
        setDashboardError(response.detail);
      }
      setDashboardUrl(response.url ?? url);
    } catch (dashboardStartError) {
      setDashboardError(dashboardStartError instanceof Error ? dashboardStartError.message : String(dashboardStartError));
      setDashboardUrl(url);
    }
  }, [preferences.agentBrowserCommand, preferences.agentBrowserDashboardPort]);

  useEffect(() => {
    if (view !== "live") {
      clientRef.current?.disconnect();
      return;
    }
    void syncLiveStream(false);
    return () => {
      clientRef.current?.disconnect();
    };
  }, [syncLiveStream, view]);

  useEffect(() => {
    if (view !== "live" || streamRefreshToken <= 0) return;
    void syncLiveStream(autoStreamPreview);
  }, [autoStreamPreview, streamRefreshToken, syncLiveStream, view]);

  useEffect(() => {
    if (view === "dashboard") {
      void ensureDashboard();
    }
  }, [view, ensureDashboard]);

  useEffect(() => {
    clientRef.current?.disconnect();
    hasFrameRef.current = false;
    setFrameSrc(null);
    setStreamUrl(null);
    setConnection("waiting");
    setError(null);
    if (view === "live") void syncLiveStream(false);
  }, [chatSessionId, syncLiveStream, view]);

  const sendPointer = (event: React.MouseEvent<HTMLImageElement>, eventType: "mousePressed" | "mouseReleased") => {
    const rect = event.currentTarget.getBoundingClientRect();
    const mapped = mapPreviewCoordinates(event.clientX, event.clientY, rect, metadataRef.current);
    clientRef.current?.sendMouse(eventType, mapped.x, mapped.y);
  };

  const sendKeyboard = (event: React.KeyboardEvent<HTMLImageElement>) => {
    if (event.key === "Tab" || event.key.startsWith("Arrow") || event.key === " ") {
      event.preventDefault();
    }
    clientRef.current?.sendKey("keyDown", event.key, event.code);
    if (event.key.length === 1 || event.key === "Enter" || event.key === "Backspace") {
      clientRef.current?.sendKey("keyUp", event.key, event.code);
    }
  };

  const showLivePlaceholder = view === "live" && !frameSrc && !error;
  const showLiveFrame = view === "live" && frameSrc && !error;

  return (
    <section
      className="agent-browser-preview"
      aria-label={t("aiChat.browserPreview.aria")}
      data-view={view}
      data-variant={variant}
      data-connection={connection}
      data-refreshing={refreshing || undefined}
    >
      <header className="agent-browser-preview-head">
        {variant === "panel" && (
          <span className="agent-browser-preview-brand">
            <Globe size={14} />
            {t("aiChat.browserPreview.title")}
          </span>
        )}
        <div className="agent-browser-preview-tabs" role="tablist">
          <button
            type="button"
            role="tab"
            aria-selected={view === "live"}
            data-active={view === "live"}
            onClick={() => setView("live")}
          >
            <Radio size={12} />
            {t("aiChat.browserPreview.tabLive")}
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={view === "dashboard"}
            data-active={view === "dashboard"}
            onClick={() => setView("dashboard")}
          >
            <LayoutDashboard size={12} />
            {t("aiChat.browserPreview.tabDashboard")}
          </button>
        </div>
        <div className="agent-browser-preview-actions">
          <span className="agent-browser-preview-status" title={connectionLabel}>
            <span className="agent-browser-preview-status-dot" aria-hidden="true" />
            <span className="agent-browser-preview-status-text">
              {view === "live" ? connectionLabel : t("aiChat.browserPreview.dashboardEmbedded")}
            </span>
          </span>
          {view === "live" && (
            <button
              type="button"
              className="icon-button compact"
              title={t("aiChat.browserPreview.refresh")}
              disabled={refreshing}
              onClick={() => void syncLiveStream(true)}
            >
              <RefreshCw size={14} className={refreshing ? "spin-icon" : undefined} />
            </button>
          )}
          {onOpenDashboard && (
            <button type="button" className="icon-button compact" title={t("aiChat.browserPreview.dashboard")} onClick={onOpenDashboard}>
              <ExternalLink size={14} />
            </button>
          )}
        </div>
      </header>

      <div className="agent-browser-preview-body" ref={viewportRef}>
        <div className="agent-browser-preview-stage">
          {view === "live" && (
            <>
              {error && (
                <div className="agent-browser-preview-message agent-browser-preview-error" role="alert">
                  <p>{error}</p>
                  <button type="button" className="secondary-button" onClick={() => void syncLiveStream(true)}>
                    {t("aiChat.browserPreview.refresh")}
                  </button>
                </div>
              )}
              {showLivePlaceholder && (
                <div className="agent-browser-preview-message agent-browser-preview-empty" role="status">
                  <Loader2 size={22} className="spin-icon" aria-hidden="true" />
                  <strong>{connectionLabel}</strong>
                  <p>{connection === "waiting" ? t("aiChat.browserPreview.openHint") : t("aiChat.browserPreview.loadingHint")}</p>
                </div>
              )}
              {showLiveFrame && (
                <div className="agent-browser-preview-frame-shell">
                  <img
                    className="agent-browser-preview-frame"
                    src={frameSrc}
                    alt={t("aiChat.browserPreview.frameAlt")}
                    draggable={false}
                    onMouseDown={(event) => sendPointer(event, "mousePressed")}
                    onMouseUp={(event) => sendPointer(event, "mouseReleased")}
                    onWheel={(event) => {
                      event.preventDefault();
                      clientRef.current?.sendWheel(event.deltaY, event.deltaX);
                    }}
                    onKeyDown={sendKeyboard}
                    tabIndex={0}
                  />
                  {refreshing && (
                    <div className="agent-browser-preview-frame-overlay" aria-hidden="true">
                      <Loader2 size={18} className="spin-icon" />
                    </div>
                  )}
                </div>
              )}
            </>
          )}
          {view === "dashboard" && (
            <>
              {dashboardError && (
                <div className="agent-browser-preview-message agent-browser-preview-error" role="alert">
                  <p>{dashboardError}</p>
                  <button type="button" className="secondary-button" onClick={() => void ensureDashboard()}>
                    {t("aiChat.browserPreview.refresh")}
                  </button>
                </div>
              )}
              {dashboardUrl ? (
                <iframe
                  className="agent-browser-preview-dashboard"
                  src={dashboardUrl}
                  title={t("aiChat.browserPreview.dashboardFrameTitle")}
                  sandbox="allow-scripts allow-same-origin allow-forms allow-popups"
                />
              ) : (
                <div className="agent-browser-preview-message agent-browser-preview-empty" role="status">
                  <Loader2 size={22} className="spin-icon" aria-hidden="true" />
                  <strong>{t("aiChat.browserPreview.dashboardLoading")}</strong>
                </div>
              )}
            </>
          )}
        </div>
      </div>

      <footer className="agent-browser-preview-foot">
        <MousePointer2 size={12} />
        <span>
          {view === "dashboard"
            ? t("aiChat.browserPreview.dashboardHint")
            : streamUrl
              ? t("aiChat.browserPreview.pairHint")
              : t("aiChat.browserPreview.openHint")}
        </span>
      </footer>
    </section>
  );
}