import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { TooltipProvider } from "@radix-ui/react-tooltip";
import { App } from "./App";
import { EditorCloseGuardProvider } from "./components/EditorCloseGuard";
import "./styles/tokens.css";
import "./styles/app.css";
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

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <QueryClientProvider client={queryClient}>
      <TooltipProvider delayDuration={350}>
        <EditorCloseGuardProvider>
          <App />
        </EditorCloseGuardProvider>
      </TooltipProvider>
    </QueryClientProvider>
  </React.StrictMode>,
);
