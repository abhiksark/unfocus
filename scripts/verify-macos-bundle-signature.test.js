import { describe, expect, test } from "bun:test";
import { mkdtempSync, mkdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  CODESIGN_PATH,
  selectMacOSAppBundle,
  verifyMacOSBundleSignature,
} from "./verify-macos-bundle-signature.js";

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

describe("macOS bundle selection", () => {
  test("selects the single Tauri-reported app bundle", () => {
    const app = createFixtureApp();
    expect(
      selectMacOSAppBundle([
        join(tmpdir(), "outside", "bundle", "macos", "Unfocus.dmg"),
        app,
      ]),
    ).toBe(app);
  });

  test("rejects when Tauri reports no app bundle", () => {
    expect(() => selectMacOSAppBundle([join(tmpdir(), "bundle", "macos", "Unfocus.dmg")])).toThrow(
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
