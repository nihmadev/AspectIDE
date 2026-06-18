import { execFileSync } from "node:child_process";
import { cpSync, mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";
import { tmpdir } from "node:os";

const root = process.cwd();
const primaryEndpoints = [
  "https://updates.lux-ide.dev/stable/latest.json",
  "https://updates.lux-ide.dev/beta/latest.json",
].join(",");
const mismatchedEndpoints = [
  "https://updates.lux-ide.dev/stable/latest.json",
  "https://updates.lux-ide.dev/canary/latest.json",
].join(",");
// The free path ships a single GitHub "latest" manifest with no channel segment.
const singleEndpoint = "https://github.com/GofMan5/lux-ide/releases/latest/download/latest.json";

runNode(root, [".github/scripts/verify-release-policy.mjs", "--source"]);
expectFailure(root, [".github/scripts/verify-release-policy.mjs", "--allow-unsigned-draft"], {}, "Unknown release policy option");

// Fully signed release (all OS + updater secrets) passes.
withPolicyCheckout((checkout) => {
  runNode(checkout, [".github/scripts/prepare-release-config.mjs"], releaseEnv(primaryEndpoints));
  runNode(checkout, [".github/scripts/verify-release-policy.mjs", "--prepared"], releaseEnv(primaryEndpoints));
});

// The prepared endpoints must match what was injected from TAURI_UPDATER_ENDPOINTS.
withPolicyCheckout((checkout) => {
  runNode(checkout, [".github/scripts/prepare-release-config.mjs"], releaseEnv(primaryEndpoints));
  expectFailure(
    checkout,
    [".github/scripts/verify-release-policy.mjs", "--prepared"],
    releaseEnv(mismatchedEndpoints),
    "plugins.updater.endpoints must be injected from TAURI_UPDATER_ENDPOINTS",
  );
});

// Free path: updater secrets only, no OS code-signing certs → unsigned installers,
// single GitHub "latest" endpoint, still a valid release config.
withPolicyCheckout((checkout) => {
  runNode(checkout, [".github/scripts/prepare-release-config.mjs"], updaterOnlyEnv(singleEndpoint));
  runNode(checkout, [".github/scripts/verify-release-policy.mjs", "--prepared"], updaterOnlyEnv(singleEndpoint));
});

// Updater signing stays mandatory even on the free path.
withPolicyCheckout((checkout) => {
  runNode(checkout, [".github/scripts/prepare-release-config.mjs"], updaterOnlyEnv(singleEndpoint));
  const withoutKey = updaterOnlyEnv(singleEndpoint);
  delete withoutKey.TAURI_SIGNING_PRIVATE_KEY;
  expectFailure(
    checkout,
    [".github/scripts/verify-release-policy.mjs", "--prepared"],
    withoutKey,
    "Tauri updater signing private key is required",
  );
});

// A half-set Windows cert (thumbprint without PFX) is rejected.
withPolicyCheckout((checkout) => {
  const partial = { ...releaseEnv(primaryEndpoints) };
  delete partial.WINDOWS_CERTIFICATE_PFX_BASE64;
  runNode(checkout, [".github/scripts/prepare-release-config.mjs"], partial);
  expectFailure(
    checkout,
    [".github/scripts/verify-release-policy.mjs", "--prepared"],
    partial,
    "Windows signing certificate PFX is required when a thumbprint is set",
  );
});

console.log("Release policy tests passed.");

function withPolicyCheckout(callback) {
  const checkout = mkdtempSync(join(tmpdir(), "lux-release-policy-test-"));
  try {
    copyPolicyFile(".github/scripts/prepare-release-config.mjs", checkout);
    copyPolicyFile(".github/scripts/verify-release-policy.mjs", checkout);
    copyPolicyFile("apps/desktop/src-tauri/tauri.conf.json", checkout);
    copyPolicyFile("apps/desktop/src-tauri/capabilities/default.json", checkout);
    callback(checkout);
  } finally {
    rmSync(checkout, { force: true, recursive: true });
  }
}

function copyPolicyFile(relativePath, checkout) {
  const destination = join(checkout, relativePath);
  mkdirSync(dirname(destination), { recursive: true });
  cpSync(join(root, relativePath), destination);
}

function runNode(cwd, args, env = {}) {
  execFileSync(process.execPath, args, {
    cwd,
    env: { ...process.env, ...env },
    stdio: "pipe",
  });
}

function expectFailure(cwd, args, env, expectedText) {
  try {
    runNode(cwd, args, env);
  } catch (error) {
    const stderr = error.stderr?.toString("utf8") ?? "";
    const stdout = error.stdout?.toString("utf8") ?? "";
    const output = `${stdout}\n${stderr}`;
    if (output.includes(expectedText)) return;
    throw new Error(`Expected failure containing "${expectedText}". Actual output:\n${output}`);
  }

  throw new Error(`Expected ${args.join(" ")} to fail.`);
}

// The free-release secret set: updater signing only, no paid OS certificates.
function updaterOnlyEnv(updaterEndpoints) {
  return {
    TAURI_SIGNING_PRIVATE_KEY: "LuxUpdaterSigningPrivateKeyMaterial-2026-Prod-9A8B7C6D5E",
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "TauriUpdaterKeyPassword-2026-Prod!",
    TAURI_UPDATER_ENDPOINTS: updaterEndpoints,
    TAURI_UPDATER_PUBLIC_KEY: "LuxUpdaterPublicKeyMaterial-2026-Prod-9A8B7C6D5E",
  };
}

function releaseEnv(updaterEndpoints) {
  return {
    APPLE_CERTIFICATE_P12_BASE64: "UVJTVFVWV1hZWjEyMzQ1Njc4OTAxMjM0NTY=",
    APPLE_CERTIFICATE_PASSWORD: "AppleCertPassword-2026-Prod!",
    APPLE_ID: "release@lux-ide.dev",
    APPLE_KEYCHAIN_PASSWORD: "AppleKeychainPassword-2026-Prod!",
    APPLE_PASSWORD: "AppleAppPassword-2026-Prod!",
    APPLE_PROVIDER_SHORT_NAME: "LUXIDE",
    APPLE_SIGNING_IDENTITY: "Developer ID Application: Lux IDE Team (9A8B7C6D5E)",
    TAURI_SIGNING_PRIVATE_KEY: "LuxUpdaterSigningPrivateKeyMaterial-2026-Prod-9A8B7C6D5E",
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: "TauriUpdaterKeyPassword-2026-Prod!",
    TAURI_UPDATER_ENDPOINTS: updaterEndpoints,
    TAURI_UPDATER_PUBLIC_KEY: "LuxUpdaterPublicKeyMaterial-2026-Prod-9A8B7C6D5E",
    WINDOWS_CERTIFICATE_PASSWORD: "WinCertPassword-2026-Prod!",
    WINDOWS_CERTIFICATE_PFX_BASE64: "QUJDREVGR0hJSktMTU5PUFFSU1RVVldYWVo123456",
    WINDOWS_CERTIFICATE_THUMBPRINT: "A1B2C3D4E5F678901234567890ABCDEF12345678",
  };
}
