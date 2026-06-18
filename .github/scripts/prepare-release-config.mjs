import { readFileSync, writeFileSync } from "node:fs";

const tauriConfigPath = "apps/desktop/src-tauri/tauri.conf.json";
const capabilitiesPath = "apps/desktop/src-tauri/capabilities/default.json";
const tauriConfig = JSON.parse(readFileSync(tauriConfigPath, "utf8"));
const capabilities = JSON.parse(readFileSync(capabilitiesPath, "utf8"));
const bundle = tauriConfig.bundle ?? {};
const windows = bundle.windows ?? {};
const macOS = bundle.macOS ?? {};

bundle.createUpdaterArtifacts = true;

// OS code-signing is OPTIONAL: a trusted Authenticode / Apple Developer ID
// certificate is paid. When the cert secrets are present we inject them so the
// installers are signed; when absent we leave the identities null and ship
// unsigned installers (users see a first-run "unknown publisher" prompt). The
// updater signature below is independent and ALWAYS required — that is what makes
// auto-update verifiable, and it is free (ed25519 via `tauri signer`).
const windowsThumbprint = optionalEnv("WINDOWS_CERTIFICATE_THUMBPRINT");
const appleSigningIdentity = optionalEnv("APPLE_SIGNING_IDENTITY");
const appleProviderShortName = optionalEnv("APPLE_PROVIDER_SHORT_NAME");
windows.certificateThumbprint = windowsThumbprint;
macOS.signingIdentity = appleSigningIdentity;
macOS.providerShortName = appleProviderShortName;
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

// Returns the trimmed value, or null when unset — used for the optional OS
// code-signing identities so the release runs without paid certificates.
function optionalEnv(name) {
  const value = String(process.env[name] ?? "").trim();
  return value.length > 0 ? value : null;
}
