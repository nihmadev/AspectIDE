import { Component, lazy, Suspense } from "react";
import type { ComponentProps, ReactNode } from "react";
import { useTranslation } from "../../lib/i18n/useTranslation";

const AspectorChatPanelLazy = lazy(() => import("./AspectorChatPanel").then((module) => ({ default: module.AspectorChatPanel })));

type AspectorLazyChatPanelProps = ComponentProps<typeof AspectorChatPanelLazy>;

export function AspectorLazyChatPanel(props: AspectorLazyChatPanelProps) {
  return (
    <AspectorChatErrorBoundary embedded={props.embedded ?? false} presentation={props.presentation ?? "panel"}>
      <Suspense fallback={<AspectorChatPanelFallback presentation={props.presentation ?? "panel"} embedded={props.embedded ?? false} />}>
        <AspectorChatPanelLazy {...props} />
      </Suspense>
    </AspectorChatErrorBoundary>
  );
}

function AspectorChatPanelFallback({ embedded, presentation }: { embedded: boolean; presentation: "panel" | "agent" }) {
  return (
    <aside className="ai-chat-panel ai-chat-panel-loading" data-embedded={embedded} data-presentation={presentation} aria-busy="true">
      <div className="ai-chat-loading-mark" />
    </aside>
  );
}

type AspectorChatErrorBoundaryProps = {
  children: ReactNode;
  embedded: boolean;
  presentation: "panel" | "agent";
};

type AspectorChatErrorBoundaryState = {
  error: Error | null;
};

class AspectorChatErrorBoundaryBase extends Component<AspectorChatErrorBoundaryProps & { fallback: (reset: () => void) => ReactNode }, AspectorChatErrorBoundaryState> {
  state: AspectorChatErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): AspectorChatErrorBoundaryState {
    return { error };
  }

  componentDidCatch(error: Error, info: { componentStack?: string }) {
    console.error("[ai-chat] render error", error, info.componentStack);
  }

  render() {
    if (this.state.error) return this.props.fallback(() => this.setState({ error: null }));
    return this.props.children;
  }
}

function AspectorChatErrorBoundary({ children, embedded, presentation }: AspectorChatErrorBoundaryProps) {
  const { t } = useTranslation();
  return (
    <AspectorChatErrorBoundaryBase
      embedded={embedded}
      presentation={presentation}
      fallback={(reset) => (
        <aside className="ai-chat-panel ai-chat-panel-crashed" data-embedded={embedded} data-presentation={presentation} role="alert">
          <strong>{t("aiChat.crash.title")}</strong>
          <span>{t("aiChat.crash.detail")}</span>
          <button type="button" onClick={reset}>{t("aiChat.crash.reload")}</button>
        </aside>
      )}
    >
      {children}
    </AspectorChatErrorBoundaryBase>
  );
}
