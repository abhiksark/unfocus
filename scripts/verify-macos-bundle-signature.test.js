import { describe, expect, test } from "bun:test";
import { spawnSync } from "node:child_process";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";
import {
  CODESIGN_PATH,
  resolveArtifactPaths,
  selectMacOSAppBundle,
  verifyMacOSBundleSignature,
} from "./verify-macos-bundle-signature.js";

const SCRIPT_PATH = fileURLToPath(new URL("./verify-macos-bundle-signature.js", import.meta.url));
const REPO_ROOT = dirname(dirname(SCRIPT_PATH));

function createFixtureApp() {
  const root = mkdtempSync(join(tmpdir(), "unfocus-macos-bundle-test-"));
  const app = join(root, "target", "release", "bundle", "macos", "Unfocus.app");
  mkdirSync(join(app, "Contents", "_CodeSignature"), { recursive: true });
  mkdirSync(join(app, "Contents", "MacOS"), { recursive: true });
  writeFileSync(join(app, "Contents", "Info.plist"), "<plist/>");
  writeFileSync(join(app, "Contents", "_CodeSignature", "CodeResources"), "sealed");
  writeFileSync(join(app, "Contents", "MacOS", "Unfocus"), "binary");
  return app;
}

function createFixtureAppFile() {
  const root = mkdtempSync(join(tmpdir(), "unfocus-macos-bundle-file-test-"));
  const app = join(root, "target", "release", "bundle", "macos", "Unfocus.app");
  mkdirSync(dirname(app), { recursive: true });
  writeFileSync(app, "not a bundle");
  return app;
}

function createNonBundleAppDirectory() {
  const root = mkdtempSync(join(tmpdir(), "unfocus-macos-nonbundle-test-"));
  const app = join(root, "artifacts", "macos", "Unfocus.app");
  mkdirSync(join(app, "Contents", "_CodeSignature"), { recursive: true });
  writeFileSync(join(app, "Contents", "_CodeSignature", "CodeResources"), "sealed");
  return app;
}

function createFixtureArtifact(relativePath, contents = "artifact") {
  const root = mkdtempSync(join(tmpdir(), "unfocus-macos-artifact-test-"));
  const artifact = join(root, "target", "release", relativePath);
  mkdirSync(dirname(artifact), { recursive: true });
  writeFileSync(artifact, contents);
  return artifact;
}

function runVerifierWithArtifacts(artifactPaths) {
  return spawnSync("bun", [SCRIPT_PATH], {
    cwd: REPO_ROOT,
    encoding: "utf8",
    env: {
      ...process.env,
      TAURI_ARTIFACT_PATHS: artifactPaths,
    },
  });
}

describe("artifact path parsing", () => {
  test("rejects malformed JSON input", () => {
    expect(() => resolveArtifactPaths("{")).toThrow("TAURI_ARTIFACT_PATHS must be valid JSON");
  });

  test("rejects non-string artifact entries", () => {
    const app = createFixtureApp();
    expect(() => resolveArtifactPaths(JSON.stringify([null, app]))).toThrow(
      "Tauri artifact paths must contain only non-empty strings",
    );
  });

  test("rejects empty artifact entries", () => {
    const app = createFixtureApp();
    expect(() => resolveArtifactPaths(JSON.stringify(["", app]))).toThrow(
      "Tauri artifact paths must contain only non-empty strings",
    );
  });

  test("surfaces malformed JSON input from the command line", () => {
    const result = runVerifierWithArtifacts("{");
    expect(result.status).toBe(1);
    expect(result.stderr).toContain("TAURI_ARTIFACT_PATHS must be valid JSON");
  });
});

describe("macOS bundle selection", () => {
  test("selects the single Tauri-reported app bundle", () => {
    const app = createFixtureApp();
    const dmg = createFixtureArtifact(join("bundle", "dmg", "Unfocus.dmg"));
    expect(
      selectMacOSAppBundle([
        dmg,
        app,
      ]),
    ).toBe(app);
  });

  test("rejects when Tauri reports no app bundle", () => {
    const dmg = createFixtureArtifact(join("bundle", "dmg", "Unfocus.dmg"));
    expect(() => selectMacOSAppBundle([dmg])).toThrow(
      "Expected exactly one macOS .app bundle from Tauri, found 0",
    );
  });

  test("rejects when Tauri reports more than one app bundle", () => {
    const first = createFixtureApp();
    const second = createFixtureApp();
    expect(() => selectMacOSAppBundle([first, second])).toThrow(
      "Expected exactly one macOS .app bundle from Tauri, found 2",
    );
  });

  test("rejects relative app bundle paths", () => {
    const app = createFixtureApp();
    const relativeApp = relative(process.cwd(), app);
    expect(() =>
      selectMacOSAppBundle([
        app,
        relativeApp,
      ])).toThrow("Tauri artifact path must be absolute");
  });

  test("rejects normalized duplicate app bundle paths", () => {
    const app = createFixtureApp();
    const sameAppViaTraversal = join(dirname(app), ".", "..", "macos", "Unfocus.app");
    expect(() => selectMacOSAppBundle([app, sameAppViaTraversal])).toThrow(
      "Expected exactly one macOS .app bundle from Tauri, found 2",
    );
  });

  test("rejects missing app bundle paths", () => {
    const app = createFixtureApp();
    const missingDmg = join(tmpdir(), "unfocus-missing-bundle", "bundle", "dmg", "Unfocus.dmg");
    expect(() => selectMacOSAppBundle([app, missingDmg])).toThrow("Tauri artifact path does not exist");
  });

  test("rejects app bundle paths that point to files", () => {
    const validApp = createFixtureApp();
    const app = createFixtureAppFile();
    expect(() => selectMacOSAppBundle([validApp, app])).toThrow(`macOS app bundle is not a directory: ${app}`);
  });

  test("rejects app directories outside the Tauri bundle path", () => {
    const validApp = createFixtureApp();
    const app = createNonBundleAppDirectory();
    expect(() => selectMacOSAppBundle([validApp, app])).toThrow(
      `macOS app bundle must be inside a Tauri bundle path: ${app}`,
    );
  });
});

describe("macOS bundle signature verification", () => {
  test("accepts a strictly verified ad-hoc bundle with bound Info.plist and sealed resources", () => {
    const app = createFixtureApp();
    const invocations = [];

    verifyMacOSBundleSignature({
      artifactPaths: [app],
      runCodesign(command, args) {
        invocations.push([command, ...args]);
        if (args.includes("--verify")) {
          return { success: true, output: `${app}: valid on disk\n${app}: satisfies its Designated Requirement\n` };
        }
        return {
          success: true,
          output: "Executable=/tmp/Unfocus.app/Contents/MacOS/Unfocus\nSignature=adhoc\nInfo.plist entries=17\nTeamIdentifier=not set\nSealed Resources version=2 rules=13 files=7\n",
        };
      },
    });

    expect(invocations).toEqual([
      [CODESIGN_PATH, "--verify", "--deep", "--strict", "--verbose=4", app],
      [CODESIGN_PATH, "-dv", "--verbose=4", app],
    ]);
  });

  test("rejects a linker-only bundle even when codesign verify passes", () => {
    const app = createFixtureApp();

    expect(() =>
      verifyMacOSBundleSignature({
        artifactPaths: [app],
        runCodesign(_command, args) {
          if (args.includes("--verify")) {
            return { success: true, output: "verified\n" };
          }
          return {
            success: true,
            output: "CodeDirectory v=20400 size=97400 flags=0x20002(adhoc,linker-signed) hashes=3040+0 location=embedded\nSignature=adhoc\nInfo.plist=not bound\nSealed Resources=none\n",
          };
        },
      }),
    ).toThrow("Bundle is linker-signed instead of fully app-signed");
  });

  test("rejects linker-signed details even when resources are present", () => {
    const app = createFixtureApp();

    expect(() =>
      verifyMacOSBundleSignature({
        artifactPaths: [app],
        runCodesign(_command, args) {
          if (args.includes("--verify")) {
            return { success: true, output: "verified\n" };
          }
          return {
            success: true,
            output: "CodeDirectory v=20400 size=97400 flags=0x20002(adhoc,linker-signed) hashes=3040+0 location=embedded\nSignature=adhoc\nInfo.plist entries=17\nSealed Resources version=2 rules=13 files=7\n",
          };
        },
      }),
    ).toThrow("Bundle is linker-signed instead of fully app-signed");
  });

  test("surfaces strict verification failures from codesign", () => {
    const app = createFixtureApp();

    expect(() =>
      verifyMacOSBundleSignature({
        artifactPaths: [app],
        runCodesign(_command, args) {
          if (args.includes("--verify")) {
            return { success: false, output: "code object is not signed at all\nIn architecture: arm64\n" };
          }
          return { success: true, output: "" };
        },
      }),
    ).toThrow("Strict codesign verification failed");
  });
});
