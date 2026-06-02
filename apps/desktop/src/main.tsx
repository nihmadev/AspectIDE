import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@radix-ui/react-tooltip";
import { App } from "./App";
import { EditorCloseGuardProvider } from "./components/EditorCloseGuard";
import { desktopRuntimeRequiredMessage, isBrowserPreviewRuntime, isTauriRuntime } from "./lib/tauri";
import "./styles/tokens.css";
import "./styles/app.css";
import "./styles/ai-chat.css";
import "./styles/ai-tool-calls.css";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 10_000,
      retry: 1,
      refetchOnWindowFocus: false,
    },
  },
});

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

root.render(
  isTauriRuntime() || isBrowserPreviewRuntime()
    ? (
      <React.StrictMode>
        <QueryClientProvider client={queryClient}>
          <TooltipProvider delayDuration={350}>
            <EditorCloseGuardProvider>
              <App />
            </EditorCloseGuardProvider>
          </TooltipProvider>
        </QueryClientProvider>
      </React.StrictMode>
    )
    : <DesktopRuntimeRequired />,
);

function DesktopRuntimeRequired() {
  return (
    <main className="desktop-runtime-required" role="alert">
      <div className="desktop-runtime-required-panel">
        <span className="desktop-runtime-required-logo" aria-hidden="true">L</span>
        <h1>Lux desktop runtime required</h1>
        <p>{desktopRuntimeRequiredMessage("Lux IDE")}</p>
        <small>Run the app with Tauri, or set VITE_LUX_BROWSER_PREVIEW=1 for an explicit browser-only preview build.</small>
      </div>
    </main>
  );
}
