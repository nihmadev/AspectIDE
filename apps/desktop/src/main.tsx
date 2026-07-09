import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@radix-ui/react-tooltip";
import { App } from "./App";
import { EditorCloseGuardProvider } from "./components/EditorCloseGuard";
import { desktopRuntimeRequiredMessage, isBrowserPreviewRuntime, isTauriRuntime } from "./lib/tauri/commands";
import "./styles/tokens.css";
import "./styles/app.css";
import "./styles/project-loading.css";
import "./styles/settings.css";
import "./styles/skills-memory.css";
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

// Suppress the native WebView right-click menu (Back / Reload / Save as / Print) —
// it is a browser chrome leak that has no place in a desktop IDE. Editable fields
// and elements that opt in via [data-native-contextmenu] keep the OS menu so text
// copy/paste still works; the app's own context menus handle everything else.
if (isTauriRuntime()) {
  window.addEventListener(
    "contextmenu",
    (event) => {
      const target = event.target as HTMLElement | null;
      const editable = target?.closest?.(
        "input, textarea, [contenteditable=''], [contenteditable='true'], [data-native-contextmenu]",
      );
      if (!editable) event.preventDefault();
    },
    { capture: true },
  );
}

// Retire the inline boot splash (index.html) once React takes over: fade, then
// remove after the transition so the splash never intercepts clicks.
function dismissBootSplash() {
  const splash = document.getElementById("boot-splash");
  if (!splash) return;
  splash.setAttribute("data-done", "true");
  window.setTimeout(() => splash.remove(), 320);
}

const root = ReactDOM.createRoot(document.getElementById("root") as HTMLElement);

requestAnimationFrame(() => requestAnimationFrame(dismissBootSplash));

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
