import { Component, lazy, Suspense } from "react";
import type { ComponentProps, ReactNode } from "react";
import { useTranslation } from "../lib/i18n/useTranslation";

const AiChatPanel = lazy(() => import("./AiChatPanel").then((module) => ({ default: module.AiChatPanel })));

type LazyAiChatPanelProps = ComponentProps<typeof AiChatPanel>;

export function LazyAiChatPanel(props: LazyAiChatPanelProps) {
  return (
    <AiChatErrorBoundary embedded={props.embedded ?? false} presentation={props.presentation ?? "panel"}>
      <Suspense fallback={<AiChatPanelFallback presentation={props.presentation ?? "panel"} embedded={props.embedded ?? false} />}>
        <AiChatPanel {...props} />
      </Suspense>
    </AiChatErrorBoundary>
  );
}

function AiChatPanelFallback({ embedded, presentation }: { embedded: boolean; presentation: "panel" | "agent" }) {
  return (
    <aside className="ai-chat-panel ai-chat-panel-loading" data-embedded={embedded} data-presentation={presentation} aria-busy="true">
      <div className="ai-chat-loading-mark" />
    </aside>
  );
}

type AiChatErrorBoundaryProps = {
  children: ReactNode;
  embedded: boolean;
  presentation: "panel" | "agent";
};

type AiChatErrorBoundaryState = {
  error: Error | null;
};

class AiChatErrorBoundaryBase extends Component<AiChatErrorBoundaryProps & { fallback: (reset: () => void) => ReactNode }, AiChatErrorBoundaryState> {
  state: AiChatErrorBoundaryState = { error: null };

  static getDerivedStateFromError(error: Error): AiChatErrorBoundaryState {
    return { error };
  }

  render() {
    if (this.state.error) return this.props.fallback(() => this.setState({ error: null }));
    return this.props.children;
  }
}

function AiChatErrorBoundary({ children, embedded, presentation }: AiChatErrorBoundaryProps) {
  const { t } = useTranslation();
  return (
    <AiChatErrorBoundaryBase
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
    </AiChatErrorBoundaryBase>
  );
}
