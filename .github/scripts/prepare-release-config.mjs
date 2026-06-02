import { readFileSync, writeFileSync } from "node:fs";

const tauriConfigPath = "apps/desktop/src-tauri/tauri.conf.json";
const capabilitiesPath = "apps/desktop/src-tauri/capabilities/default.json";
const tauriConfig = JSON.parse(readFileSync(tauriConfigPath, "utf8"));
const capabilities = JSON.parse(readFileSync(capabilitiesPath, "utf8"));
const bundle = tauriConfig.bundle ?? {};
const windows = bundle.windows ?? {};
const macOS = bundle.macOS ?? {};

bundle.createUpdaterArtifacts = true;
windows.certificateThumbprint = requiredEnv("WINDOWS_CERTIFICATE_THUMBPRINT");
macOS.signingIdentity = requiredEnv("APPLE_SIGNING_IDENTITY");
macOS.providerShortName = requiredEnv("APPLE_PROVIDER_SHORT_NAME");
tauriConfig.bundle = {
  ...bundle,
  windows,
  macOS,
};
tauriConfig.plugins = {
  ...(tauriConfig.plugins ?? {}),
  updater: {
    active: true,
    endpoints: releaseEndpoints(),
    pubkey: requiredEnv("TAURI_UPDATER_PUBLIC_KEY"),
  },
};

const permissions = new Set(Array.isArray(capabilities.permissions) ? capabilities.permissions : []);
permissions.add("updater:default");
capabilities.permissions = [...permissions].sort();

writeFileSync(tauriConfigPath, `${JSON.stringify(tauriConfig, null, 2)}\n`, "utf8");
writeFileSync(capabilitiesPath, `${JSON.stringify(capabilities, null, 2)}\n`, "utf8");
console.log("Prepared release signing and updater configuration for this CI checkout.");

function releaseEndpoints() {
  const raw = requiredEnv("TAURI_UPDATER_ENDPOINTS");
  const endpoints = raw.split(/\r?\n|,/u).map((endpoint) => endpoint.trim()).filter(Boolean);
  if (endpoints.length === 0) {
    throw new Error("TAURI_UPDATER_ENDPOINTS must define at least one production HTTPS endpoint.");
  }
  return endpoints;
}

function requiredEnv(name) {
  const value = String(process.env[name] ?? "").trim();
  if (!value) throw new Error(`${name} is required to prepare release configuration.`);
  return value;
}
