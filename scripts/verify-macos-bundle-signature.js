#!/usr/bin/env bun

import { existsSync, statSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

export const CODESIGN_PATH = "/usr/bin/codesign";

function resolveArtifactPaths(encodedPaths) {
  if (!encodedPaths) {
    throw new Error("TAURI_ARTIFACT_PATHS is required");
  }
  const artifactPaths = JSON.parse(encodedPaths);
  if (!Array.isArray(artifactPaths)) {
    throw new Error("Tauri artifact paths must be a JSON array");
  }
  return artifactPaths;
}

export function selectMacOSAppBundle(artifactPaths) {
  if (!Array.isArray(artifactPaths)) {
    throw new Error("artifact paths must be an array");
  }

  const selected = artifactPaths.filter((artifactPath) => {
    if (typeof artifactPath !== "string") return false;
    const resolvedPath = resolve(artifactPath);
    const pathParts = resolvedPath.split(sep);
    return pathParts.includes("bundle") && resolvedPath.toLowerCase().endsWith(".app");
  });

  if (selected.length !== 1) {
    throw new Error(`Expected exactly one macOS .app bundle from Tauri, found ${selected.length}`);
  }

  const [appPath] = selected.map((artifactPath) => resolve(artifactPath));
  if (!statSync(appPath).isDirectory()) {
    throw new Error(`macOS app bundle is not a directory: ${appPath}`);
  }
  return appPath;
}

export function runCodesign(command, args) {
  const result = spawnSync(command, args, { encoding: "utf8" });
  const output = `${result.stdout ?? ""}${result.stderr ?? ""}`;
  if (result.error) {
    throw new Error(`${command} could not run: ${result.error.message}`);
  }
  return {
    success: result.status === 0,
    output,
  };
}

function requireMatch(output, pattern, message) {
  if (!pattern.test(output)) {
    throw new Error(message);
  }
}

export function verifyMacOSBundleSignature({ artifactPaths, runCodesign: executeCodesign = runCodesign }) {
  const appPath = selectMacOSAppBundle(artifactPaths);
  const strictVerification = executeCodesign(CODESIGN_PATH, ["--verify", "--deep", "--strict", "--verbose=4", appPath]);
  if (!strictVerification.success) {
    throw new Error(`Strict codesign verification failed for ${appPath}:\n${strictVerification.output.trimEnd()}`);
  }

  const signatureDetails = executeCodesign(CODESIGN_PATH, ["-dv", "--verbose=4", appPath]);
  if (!signatureDetails.success) {
    throw new Error(`codesign signature inspection failed for ${appPath}:\n${signatureDetails.output.trimEnd()}`);
  }

  if (/linker-signed/.test(signatureDetails.output)) {
    throw new Error(`Bundle is linker-signed instead of fully app-signed: ${appPath}`);
  }
  requireMatch(signatureDetails.output, /^Signature=adhoc$/m, `Expected an ad-hoc app signature for ${appPath}`);
  if (/^Info\.plist=not bound$/m.test(signatureDetails.output)) {
    throw new Error(`Info.plist is not bound into the app signature for ${appPath}`);
  }
  requireMatch(
    signatureDetails.output,
    /^Info\.plist(?: entries=\d+|=.*)$/m,
    `codesign did not report a bound Info.plist for ${appPath}`,
  );
  if (/^Sealed Resources=none$/m.test(signatureDetails.output)) {
    throw new Error(`Sealed resources are missing from the app signature for ${appPath}`);
  }
  requireMatch(
    signatureDetails.output,
    /^Sealed Resources version=\d+.*$/m,
    `codesign did not report sealed resources for ${appPath}`,
  );

  const codeResourcesPath = resolve(appPath, "Contents", "_CodeSignature", "CodeResources");
  if (!existsSync(codeResourcesPath)) {
    throw new Error(`Sealed resource manifest is missing: ${codeResourcesPath}`);
  }

  return {
    appPath,
    strictVerification: strictVerification.output.trim(),
    signatureDetails: signatureDetails.output.trim(),
    codeResourcesPath,
  };
}

function main() {
  const result = verifyMacOSBundleSignature({
    artifactPaths: resolveArtifactPaths(process.env.TAURI_ARTIFACT_PATHS),
  });
  console.log(`Verified macOS app bundle: ${result.appPath}`);
  console.log(result.strictVerification);
  console.log(result.signatureDetails);
  console.log(`Sealed resource manifest: ${result.codeResourcesPath}`);
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
