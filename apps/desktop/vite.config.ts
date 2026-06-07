import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const host = process.env.TAURI_DEV_HOST ?? "127.0.0.1";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
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
          // Markdown/diagram sanitization + date helpers are pulled in by the same
          // lazy preview surfaces as Mermaid; keep them with it so the generic
          // `vendor` chunk stays small and eager.
          if (id.includes("dompurify") || id.includes("/marked") || id.includes("dayjs")) return "vendor-mermaid";
          // Animation runtime (framer-motion) — sizeable and only used by motion UI.
          if (id.includes("framer-motion") || id.includes("/motion-dom") || id.includes("/motion-utils")) return "vendor-motion";
          return "vendor";
        },
      },
    },
  },
});
