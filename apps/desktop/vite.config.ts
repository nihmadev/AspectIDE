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
  },
});
