/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_LUX_BROWSER_PREVIEW?: "1";
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}

/** App version injected at build time from package.json (vite `define`). */
declare const __APP_VERSION__: string;
