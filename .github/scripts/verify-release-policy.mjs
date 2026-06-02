import { readFileSync } from "node:fs";

const options = parseOptions(process.argv.slice(2));
const tauriConfig = JSON.parse(readFileSync(options.configPath, "utf8"));
const capabilities = JSON.parse(readFileSync(options.capabilitiesPath, "utf8"));
const bundle = tauriConfig.bundle ?? {};
const updater = tauriConfig.plugins?.updater ?? null;
const permissions = Array.isArray(capabilities.permissions) ? capabilities.permissions : [];
const errors = [];
const prepared = options.mode === "prepared";

if (!bundle.active) {
  errors.push("Tauri bundle.active must be true for release installers.");
}

requireTargets(["nsis", "app", "dmg", "appimage", "deb", "rpm"]);

if (String(bundle.windows?.digestAlgorithm ?? "").toLowerCase() !== "sha256") {
  errors.push("Windows signing digestAlgorithm must be sha256.");
}

if (prepared) verifyPreparedReleasePolicy();
else verifySourceReleasePolicy();

if (errors.length > 0) {
  throw new Error(`Release policy verification failed:\n- ${errors.join("\n- ")}`);
}

console.log("Release policy verification passed.");

function parseOptions(args) {
  const options = {
    capabilitiesPath: "apps/desktop/src-tauri/capabilities/default.json",
    configPath: "apps/desktop/src-tauri/tauri.conf.json",
    mode: "source",
  };

  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index];
    if (arg === "--strict" || arg === "--prepared") {
      options.mode = "prepared";
      continue;
    }
    if (arg === "--source") {
      options.mode = "source";
      continue;
    }
    if (arg === "--config") {
      options.configPath = requiredValue(args, index, arg);
      index += 1;
      continue;
    }
    if (arg === "--capabilities") {
      options.capabilitiesPath = requiredValue(args, index, arg);
      index += 1;
      continue;
    }
    throw new Error(`Unknown release policy option: ${arg}`);
  }

  return options;
}

function requiredValue(args, index, name) {
  const value = args[index + 1];
  if (!value) throw new Error(`${name} requires a path.`);
  return value;
}

function verifySourceReleasePolicy() {
  if (bundle.windows?.certificateThumbprint !== null) {
    errors.push("Do not commit a Windows certificate thumbprint into tauri.conf.json; inject it in CI.");
  }

  if (bundle.macOS?.signingIdentity !== null) {
    errors.push("Do not commit a macOS signing identity into tauri.conf.json; inject it in CI.");
  }

  if (bundle.macOS?.providerShortName !== null) {
    errors.push("Do not commit an Apple provider short name into tauri.conf.json; inject it in CI.");
  }

  if (bundle.createUpdaterArtifacts !== false) {
    errors.push("Source tauri.conf.json must keep bundle.createUpdaterArtifacts disabled until CI prepares a signed release config.");
  }

  if (updater) {
    errors.push("Source tauri.conf.json must not commit production updater endpoints or pubkeys; inject them in CI.");
  }

  if (permissions.some((permission) => String(permission).startsWith("updater:"))) {
    errors.push("Source capabilities must not expose updater permissions before a signed release workflow prepares them.");
  }
}

function verifyPreparedReleasePolicy() {
  const windowsThumbprint = requireEnv("WINDOWS_CERTIFICATE_THUMBPRINT", "Windows Authenticode certificate thumbprint is required.", isCertificateThumbprint);
  const appleSigningIdentity = requireEnv("APPLE_SIGNING_IDENTITY", "Apple Developer ID signing identity is required.", isDeveloperIdApplicationIdentity);
  const appleProviderShortName = requireEnv("APPLE_PROVIDER_SHORT_NAME", "Apple notarization provider short name is required.", isProductionIdentifier);
  const updaterPublicKey = requireEnv("TAURI_UPDATER_PUBLIC_KEY", "Tauri updater public key is required.", isProductionKeyMaterial);
  const updaterEndpoints = parseUpdaterEndpoints(requireEnv("TAURI_UPDATER_ENDPOINTS", "Tauri updater HTTPS endpoints are required.", isProductionValue));
  requireEnv("WINDOWS_CERTIFICATE_PFX_BASE64", "Windows signing certificate PFX is required on GitHub-hosted runners.", isProductionKeyMaterial);
  requireEnv("WINDOWS_CERTIFICATE_PASSWORD", "Windows signing certificate password is required.", isProductionSecret);
  requireEnv("APPLE_CERTIFICATE_P12_BASE64", "Apple signing certificate P12 is required on GitHub-hosted runners.", isProductionKeyMaterial);
  requireEnv("APPLE_CERTIFICATE_PASSWORD", "Apple signing certificate password is required.", isProductionSecret);
  requireEnv("APPLE_KEYCHAIN_PASSWORD", "Temporary macOS signing keychain password is required.", isProductionSecret);
  requireEnv("APPLE_ID", "Apple notarization account id is required.", isProductionAppleId);
  requireEnv("APPLE_PASSWORD", "Apple notarization app-specific password is required.", isProductionSecret);
  requireEnv("TAURI_SIGNING_PRIVATE_KEY", "Tauri updater signing private key is required.", isProductionKeyMaterial);
  requireEnv("TAURI_SIGNING_PRIVATE_KEY_PASSWORD", "Tauri updater signing key password is required.", isProductionSecret);

  if (bundle.windows?.certificateThumbprint !== windowsThumbprint) {
    errors.push("Prepared config must set bundle.windows.certificateThumbprint from WINDOWS_CERTIFICATE_THUMBPRINT.");
  }

  if (bundle.macOS?.signingIdentity !== appleSigningIdentity) {
    errors.push("Prepared config must set bundle.macOS.signingIdentity from APPLE_SIGNING_IDENTITY.");
  }

  if (bundle.macOS?.providerShortName !== appleProviderShortName) {
    errors.push("Prepared config must set bundle.macOS.providerShortName from APPLE_PROVIDER_SHORT_NAME.");
  }

  if (bundle.createUpdaterArtifacts !== true) {
    errors.push("bundle.createUpdaterArtifacts must be true for release channels.");
  }

  if (!updater) {
    errors.push("plugins.updater configuration is required for release channels.");
  } else {
    const pubkey = String(updater.pubkey ?? "").trim();
    if (pubkey !== updaterPublicKey) {
      errors.push("plugins.updater.pubkey must be injected from TAURI_UPDATER_PUBLIC_KEY.");
    }

    const endpoints = Array.isArray(updater.endpoints) ? updater.endpoints : [];
    if (endpoints.length === 0) {
      errors.push("plugins.updater.endpoints must include production HTTPS update manifests.");
    }

    if (!sameStringSet(endpoints, updaterEndpoints)) {
      errors.push("plugins.updater.endpoints must be injected from TAURI_UPDATER_ENDPOINTS.");
    }

    for (const endpoint of endpoints) {
      validateUpdaterEndpoint(String(endpoint));
    }

    requireReleaseChannels(endpoints);
  }

  if (!permissions.some((permission) => String(permission).startsWith("updater:"))) {
    errors.push("Tauri capabilities must include explicit updater permissions when release updater artifacts are enabled.");
  }
}

function requireTargets(expectedTargets) {
  const targets = new Set(Array.isArray(bundle.targets) ? bundle.targets : []);
  for (const target of expectedTargets) {
    if (!targets.has(target)) {
      errors.push(`Missing Tauri bundle target: ${target}.`);
    }
  }
}

function requireEnv(name, message, isValid) {
  const value = String(process.env[name] ?? "").trim();
  if (!value) {
    errors.push(`${message} Set ${name} in GitHub release secrets.`);
  } else if (!isValid(value)) {
    errors.push(`${name} must contain a production value, not a placeholder, test, example, or malformed value.`);
  }
  return value;
}

function parseUpdaterEndpoints(raw) {
  const endpoints = raw.split(/\r?\n|,/u).map((endpoint) => endpoint.trim()).filter(Boolean);
  if (endpoints.length === 0) {
    errors.push("TAURI_UPDATER_ENDPOINTS must define at least one production HTTPS endpoint.");
  }
  if (new Set(endpoints).size !== endpoints.length) {
    errors.push("TAURI_UPDATER_ENDPOINTS must not contain duplicate endpoints.");
  }
  return endpoints;
}

function sameStringSet(actual, expected) {
  const actualValues = actual.map((value) => String(value).trim()).filter(Boolean);
  const expectedValues = expected.map((value) => String(value).trim()).filter(Boolean);
  if (actualValues.length !== expectedValues.length) return false;

  const expectedSet = new Set(expectedValues);
  return actualValues.every((value) => expectedSet.has(value));
}

function isProductionValue(value) {
  const trimmed = value.trim();
  const normalized = trimmed.toLowerCase();
  const compact = normalized.replace(/[^a-z0-9]/gu, "");
  const obviousPlaceholders = new Set([
    "changeme",
    "dummy",
    "dummykey",
    "example",
    "examplekey",
    "fake",
    "placeholder",
    "placeholderkey",
    "test",
    "testkey",
    "todo",
  ]);

  return trimmed.length > 0
    && !obviousPlaceholders.has(compact)
    && !/(^|[^a-z0-9])(placeholder|example|changeme|todo|dummy|fake|test)([^a-z0-9]|$)/u.test(normalized);
}

function isProductionSecret(value) {
  return value.trim().length >= 8 && isProductionValue(value);
}

function isProductionKeyMaterial(value) {
  return value.trim().length >= 32 && isProductionValue(value);
}

function isProductionIdentifier(value) {
  return /^[A-Z0-9][A-Z0-9._-]{1,31}$/u.test(value.trim()) && isProductionValue(value);
}

function isProductionAppleId(value) {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/u.test(value.trim()) && isProductionValue(value);
}

function isCertificateThumbprint(value) {
  return /^[A-Fa-f0-9]{40}$/u.test(value.trim()) && isProductionValue(value);
}

function isDeveloperIdApplicationIdentity(value) {
  return /^Developer ID Application: .+ \([A-Z0-9]{10}\)$/u.test(value.trim()) && isProductionValue(value);
}

function validateUpdaterEndpoint(endpoint) {
  let url;
  try {
    url = new URL(endpoint);
  } catch {
    errors.push(`Invalid updater endpoint URL: ${endpoint}`);
    return;
  }

  if (url.protocol !== "https:") {
    errors.push(`Updater endpoint must use HTTPS: ${endpoint}`);
  }

  const hostname = url.hostname.toLowerCase();
  if (["localhost", "127.0.0.1", "0.0.0.0", "example.com", "example.org"].includes(hostname) || hostname.endsWith(".local")) {
    errors.push(`Updater endpoint must not use a local or placeholder host: ${endpoint}`);
  }

  if (endpointChannels(url).length === 0) {
    errors.push(`Updater endpoint must include an explicit release channel path: ${endpoint}`);
  }
}

function requireReleaseChannels(endpoints) {
  const channels = new Set();
  for (const endpoint of endpoints) {
    try {
      for (const channel of endpointChannels(new URL(String(endpoint)))) {
        channels.add(channel);
      }
    } catch {
      // validateUpdaterEndpoint already reports invalid URLs.
    }
  }

  for (const channel of ["stable", "beta", "nightly"]) {
    if (!channels.has(channel)) {
      errors.push(`Updater endpoints must include a ${channel} release channel.`);
    }
  }
}

function endpointChannels(url) {
  const pathname = url.pathname.toLowerCase();
  return ["stable", "beta", "nightly"].filter((channel) => {
    const pattern = new RegExp(`(^|/)${channel}($|[/.])`, "u");
    return pattern.test(pathname);
  });
}
