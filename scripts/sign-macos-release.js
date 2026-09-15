#!/usr/bin/env bun
// Apple credentials are confined to this entry point and a disposable keychain.
import { spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import { existsSync, lstatSync, mkdirSync, readdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { join, resolve, sep } from "node:path";
import { releaseChannel } from "./linux-update-envelope.js";

export function signingConfiguration(env) {
  if (releaseChannel(env.RELEASE_VERSION) !== "stable" || env.REHEARSAL === "true") {
    throw new Error("Production Apple signing requires a stable, non-rehearsal release");
  }
  for (const name of ["APPLE_CERTIFICATE_BASE64", "APPLE_CERTIFICATE_PASSWORD", "APPLE_API_KEY_BASE64", "APPLE_API_KEY_ID", "APPLE_API_ISSUER", "APPLE_TEAM_ID", "APPLE_SIGNING_IDENTITY", "RUNNER_TEMP"]) {
    if (!env[name]?.trim()) throw new Error(`Missing required ${name}`);
  }
  if (!/^[A-Z0-9]{10}$/.test(env.APPLE_TEAM_ID) || !/^[A-Z0-9]{10}$/.test(env.APPLE_API_KEY_ID) ||
      !/^[0-9a-f-]{36}$/i.test(env.APPLE_API_ISSUER) ||
      !env.APPLE_SIGNING_IDENTITY.startsWith("Developer ID Application: ") ||
      !env.APPLE_SIGNING_IDENTITY.endsWith(`(${env.APPLE_TEAM_ID})`)) throw new Error("Invalid public Apple identity configuration");
  for (const key of ["APPLE_CERTIFICATE_BASE64", "APPLE_API_KEY_BASE64"]) {
    if (Buffer.from(env[key], "base64").toString("base64") !== env[key]) throw new Error(`Invalid base64 in ${key}`);
  }
  return env;
}

export function verifySignatureDetails(details, team, identity, runtime = true) {
  const lines = details.split(/\r?\n/);
  if (!lines.includes(`TeamIdentifier=${team}`) || !lines.includes(`Authority=${identity}`) ||
      !/^Timestamp=.+$/m.test(details) || /Signature=adhoc|linker-signed/.test(details) ||
      (runtime && !/flags=.*\bruntime\b/.test(details))) throw new Error("Missing Developer ID, expected Team ID, secure timestamp, or hardened runtime");
}

// Never include subprocess arguments/output in failures: security and notarytool
// may echo credentials or authentication responses. No shell and no tracing.
function executeCommand(tool, args) {
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("APPLE_") && key !== "GH_TOKEN" && key !== "GITHUB_TOKEN"));
  const result = spawnSync(tool, args, { encoding: "utf8", env });
  if (result.status !== 0 || result.error) throw new Error(`${tool} failed; inspect the operation with credentials redacted`);
  return `${result.stdout ?? ""}${result.stderr ?? ""}`;
}

function workspace() {
  if (!process.env.RUNNER_TEMP) throw new Error("RUNNER_TEMP is required");
  return join(resolve(process.env.RUNNER_TEMP), "unfocus-apple-signing");
}
export function cleanup(execute = executeCommand) {
  const work = workspace();
  const failures = [];
  const keychain = join(work, "release.keychain-db");
  const mount = join(work, "mount");
  let detached = true;
  if (existsSync(mount) && readdirSync(mount).length) {
    try { execute("/usr/bin/hdiutil", ["detach", mount]); }
    catch { detached = false; failures.push("detach temporary signing volume"); }
  }
  if (existsSync(keychain)) {
    try { execute("/usr/bin/security", ["delete-keychain", keychain]); }
    catch { failures.push("delete temporary signing keychain"); }
  }
  // Remove secret files even when a tool fails, but never recursively remove a
  // mounted volume. The always-run workflow step retries remaining cleanup.
  for (const file of ["certificate.p12", "AuthKey.p8", "release.keychain-db"]) {
    rmSync(join(work, file), { force: true });
  }
  if (detached) rmSync(work, { recursive: true, force: true });
  if (failures.length) throw new Error(`Apple cleanup failed: ${failures.join(", ")}`);
}

export function signMacOSRelease(input, output, target, command = executeCommand) {
  const env = signingConfiguration(process.env);
  const arch = { "aarch64-apple-darwin": ["aarch64", "arm64"], "x86_64-apple-darwin": ["x64", "x86_64"] }[target];
  if (!arch) throw new Error("Unsupported macOS target");
  const name = `Unfocus_${env.RELEASE_VERSION}_${arch[0]}.dmg`;
  if (JSON.stringify(readdirSync(input)) !== JSON.stringify([name]) || !lstatSync(join(input, name)).isFile()) throw new Error("Expected exactly one regular input DMG for this version/target");
  if (existsSync(output)) throw new Error("Signing output already exists");
  const work = workspace();
  mkdirSync(work, { mode: 0o700 });
  const keychain = join(work, "release.keychain-db");
  const mount = join(work, "mount");
  const stage = join(work, "stage");
  const app = join(stage, "Unfocus.app");
  const password = randomBytes(32).toString("hex");
  const certificate = join(work, "certificate.p12");
  const apiKey = join(work, "AuthKey.p8");
  const entitlements = join(work, "entitlements.plist");
  let failure;
  try {
    writeFileSync(certificate, Buffer.from(env.APPLE_CERTIFICATE_BASE64, "base64"), { mode: 0o600 });
    writeFileSync(apiKey, Buffer.from(env.APPLE_API_KEY_BASE64, "base64"), { mode: 0o600 });
    // WKWebView's system process owns JIT; the Rust host needs no exceptions.
    writeFileSync(entitlements, '<?xml version="1.0"?><!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd"><plist version="1.0"><dict/></plist>');
    command("/usr/bin/security", ["create-keychain", "-p", password, keychain]);
    command("/usr/bin/security", ["set-keychain-settings", "-lut", "21600", keychain]);
    command("/usr/bin/security", ["unlock-keychain", "-p", password, keychain]);
    command("/usr/bin/security", ["import", certificate, "-k", keychain, "-P", env.APPLE_CERTIFICATE_PASSWORD, "-T", "/usr/bin/codesign"]);
    command("/usr/bin/security", ["set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s", "-k", password, keychain]);
    command("/usr/bin/xcrun", ["notarytool", "store-credentials", "unfocus-release", "--key", apiKey, "--key-id", env.APPLE_API_KEY_ID, "--issuer", env.APPLE_API_ISSUER, "--keychain", keychain]);
    rmSync(certificate); rmSync(apiKey);
    mkdirSync(mount); mkdirSync(stage);
    // Tauri DMGs can include an SLA. Conversion avoids an interactive attach
    // license prompt; the app is copied into a newly signed distribution DMG.
    const converted = join(work, "input.cdr");
    command("/usr/bin/hdiutil", ["convert", resolve(input, name), "-format", "UDTO", "-o", converted]);
    command("/usr/bin/hdiutil", ["attach", "-readonly", "-nobrowse", "-mountpoint", mount, converted]);
    if (!lstatSync(join(mount, "Unfocus.app")).isDirectory()) throw new Error("DMG lacks Unfocus.app");
    command("/usr/bin/ditto", [join(mount, "Unfocus.app"), app]);
    command("/usr/bin/hdiutil", ["detach", mount]);
    if (command("/usr/bin/plutil", ["-extract", "CFBundleShortVersionString", "raw", "-o", "-", join(app, "Contents/Info.plist")]).trim() !== env.RELEASE_VERSION) throw new Error("App version mismatch");
    if (command("/usr/bin/lipo", ["-archs", join(app, "Contents/MacOS/unfocus")]).trim() !== arch[1]) throw new Error("App architecture mismatch");
    const nested = [];
    function walk(path) {
      for (const entry of readdirSync(path, { withFileTypes: true })) {
        const file = join(path, entry.name);
        if (entry.isSymbolicLink()) {
          if (!realpathSync(file).startsWith(app + sep)) throw new Error("App symlink escapes bundle");
        } else if (entry.isDirectory()) {
          walk(file);
          if (/\.(app|framework|xpc|appex|bundle)$/.test(entry.name)) nested.push(file);
        } else if (entry.isFile()) {
          if (command("/usr/bin/file", ["-b", file]).includes("Mach-O")) nested.push(file);
        } else throw new Error("Unsupported app filesystem entry");
      }
    }
    walk(app);
    function verify(path, runtime = true) {
      command("/usr/bin/codesign", ["--verify", "--deep", "--strict", "--verbose=4", path]);
      verifySignatureDetails(command("/usr/bin/codesign", ["-dv", "--verbose=4", path]), env.APPLE_TEAM_ID, env.APPLE_SIGNING_IDENTITY, runtime);
    }
    for (const path of [...nested, app]) {
      command("/usr/bin/codesign", ["--force", "--sign", env.APPLE_SIGNING_IDENTITY, "--keychain", keychain, "--options", "runtime", "--timestamp", "--entitlements", entitlements, path]);
    }
    for (const path of [...nested, app]) verify(path);
    function notarize(path) {
      const response = command("/usr/bin/xcrun", ["notarytool", "submit", path, "--keychain-profile", "unfocus-release", "--keychain", keychain, "--wait", "--timeout", "30m", "--output-format", "json"]);
      if (JSON.parse(response).status !== "Accepted") throw new Error("Apple notarization was not accepted");
    }
    const archive = join(work, "app.zip");
    command("/usr/bin/ditto", ["-c", "-k", "--keepParent", app, archive]);
    notarize(archive);
    command("/usr/bin/xcrun", ["stapler", "staple", app]);
    command("/usr/bin/xcrun", ["stapler", "validate", app]);
    command("/usr/sbin/spctl", ["--assess", "--type", "execute", "--verbose=4", app]);
    command("/bin/ln", ["-s", "/Applications", join(stage, "Applications")]);
    const dmg = join(work, name);
    command("/usr/bin/hdiutil", ["create", "-volname", "Unfocus", "-srcfolder", stage, "-format", "UDZO", dmg]);
    command("/usr/bin/codesign", ["--sign", env.APPLE_SIGNING_IDENTITY, "--keychain", keychain, "--timestamp", dmg]);
    notarize(dmg);
    command("/usr/bin/xcrun", ["stapler", "staple", dmg]);
    command("/usr/bin/xcrun", ["stapler", "validate", dmg]);
    verify(dmg, false);
    command("/usr/sbin/spctl", ["--assess", "--type", "open", "--context", "context:primary-signature", "--verbose=4", dmg]);
    command("/usr/bin/hdiutil", ["attach", "-readonly", "-nobrowse", "-mountpoint", mount, dmg]);
    verify(join(mount, "Unfocus.app"));
    command("/usr/bin/xcrun", ["stapler", "validate", join(mount, "Unfocus.app")]);
    command("/usr/sbin/spctl", ["--assess", "--type", "execute", "--verbose=4", join(mount, "Unfocus.app")]);
    command("/usr/bin/hdiutil", ["detach", mount]);
    mkdirSync(output);
    command("/bin/cp", [dmg, resolve(output, name)]);
  } catch (error) {
    failure = error;
    throw error;
  } finally {
    try { cleanup(command); }
    catch (error) {
      if (failure) console.error(error.message);
      else throw error;
    }
  }
}
if (import.meta.main) {
  try {
    if (process.argv[2] === "--cleanup") cleanup();
    else signMacOSRelease(...process.argv.slice(2));
  } catch (error) { console.error(error.message); process.exit(1); }
}
