import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { version as appVersion } from "./package.json";

const host = process.env.TAURI_DEV_HOST ?? "127.0.0.1";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  // Single version source (package.json, kept in lockstep with the Tauri crate by
  // the release flow) — surfaced in the title bar next to the logo.
  define: {
    __APP_VERSION__: JSON.stringify(appVersion),
  },
  server: {
    host,
    port: 5173,
    strictPort: true,
    proxy: {
      "/__lux_ai_proxy": {
        target: process.env.LUX_AI_PROXY_TARGET ?? "http://127.0.0.1:8799",
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/__lux_ai_proxy/, ""),
      },
    },
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2022",
    minify: "esbuild",
    sourcemap: true,
    rollupOptions: {
      output: {
        manualChunks(id) {
          // The Vite/Rolldown preload helper (`\0vite/preload-helper.js`) is a
          // virtual module that does NOT contain "node_modules", so it used to
          // fall through to `undefined` and get folded into whichever chunk first
          // referenced it — `vendor-mermaid`. That welded the entire 2.6 MB
          // diagram stack onto the entry chunk's eager modulepreload. Pinning it
          // to its own tiny eager `runtime` chunk keeps the lazy chunks lazy.
          if (id.includes("vite/preload-helper")) return "runtime";
          if (!id.includes("node_modules")) return undefined;
          if (id.includes("@tauri-apps")) return "vendor-tauri";
          if (id.includes("react") || id.includes("zustand") || id.includes("@tanstack")) return "vendor-react";
          if (id.includes("@radix-ui") || id.includes("cmdk")) return "vendor-dialog";
          if (id.includes("react-resizable-panels")) return "vendor-layout";
          if (id.includes("lucide-react")) return "vendor-icons";
          if (id.includes("@xterm")) return "vendor-terminal";
          if (id.includes("monaco-editor") || id.includes("@monaco-editor")) return "vendor-editor";
          // Mermaid + its heavy transitive graph/layout/math deps. This is the
          // largest dependency tree and is only pulled in lazily by the diagram
          // preview, so it must be split out (and further sub-split) to keep any
          // single chunk under the bundle budget instead of landing in `vendor`.
          if (id.includes("mermaid")) return "vendor-mermaid";
          if (id.includes("cytoscape")) return "vendor-graph-cytoscape";
          if (id.includes("/dagre") || id.includes("dagre-d3") || id.includes("graphlib")) return "vendor-graph-dagre";
          if (id.includes("elkjs")) return "vendor-graph-elk";
          if (id.includes("katex")) return "vendor-katex";
          if (id.includes("/d3-") || id.includes("d3-array") || id.includes("d3-scale") || id.includes("/d3/")) return "vendor-d3";
          // `marked` is pulled in eagerly by the AI-chat + markdown panes (lazy,
          // but far more reachable than the diagram surface). Keep it — plus its
          // sanitize/date helpers — in its own small chunk so opening chat or a
          // markdown file does NOT drag the 1.9 MB mermaid chunk in for ~90 KB of
          // markdown code.
          if (id.includes("/marked") || id.includes("dompurify") || id.includes("dayjs")) return "vendor-markdown";
          // Mermaid-only transitive deps. These otherwise fall through to the
          // eager generic `vendor` chunk (~88% of its weight) even though they are
          // only reachable from the lazy diagram render. Co-locate them with
          // mermaid so they load on first diagram render, not at startup. Shared
          // runtime libs (@floating-ui, scheduler, uuid, tslib) are deliberately
          // NOT captured here.
          if (/[\\/]node_modules[\\/](?:\.pnpm[\\/][^\\/]+[\\/]node_modules[\\/])?(?:lodash-es|es-toolkit|khroma|@iconify[\\/]utils|cose-base|layout-base|@braintree[\\/]sanitize-url|stylis|roughjs|ts-dedent|@upsetjs[\\/]venn\.js)([\\/]|$)/.test(id)) return "vendor-mermaid";
          // Animation runtime (framer-motion) — sizeable and only used by motion UI.
          if (id.includes("framer-motion") || id.includes("/motion-dom") || id.includes("/motion-utils")) return "vendor-motion";
          return "vendor";
        },
      },
    },
  },
});
