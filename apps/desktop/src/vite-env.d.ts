/// <reference types="vite/client" />

interface ImportMetaEnv {
  readonly VITE_LUX_BROWSER_PREVIEW?: "1";
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
