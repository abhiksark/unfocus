import { describe, expect, test } from "bun:test";
import { signingConfiguration, verifySignatureDetails } from "./sign-macos-release.js";

const env = {
  RELEASE_VERSION: "0.7.0", REHEARSAL: "false", RUNNER_TEMP: "/tmp",
  APPLE_CERTIFICATE_BASE64: "dGVzdA==", APPLE_CERTIFICATE_PASSWORD: "fixture-only",
  APPLE_API_KEY_BASE64: "dGVzdA==", APPLE_API_KEY_ID: "ABCDEFGHIJ",
  APPLE_API_ISSUER: "12345678-1234-1234-1234-123456789012", APPLE_TEAM_ID: "0123456789",
  APPLE_SIGNING_IDENTITY: "Developer ID Application: Test (0123456789)",
};
const details = `Authority=${env.APPLE_SIGNING_IDENTITY}\nTeamIdentifier=${env.APPLE_TEAM_ID}\nTimestamp=Sep 15, 2026 at 10:00:00 AM\nCodeDirectory v=20500 size=100 flags=0x10000(runtime) hashes=1+1 location=embedded\n`;

describe("stable Apple signing fails closed", () => {
  test("accepts complete configuration only for stable production", () => {
    expect(signingConfiguration(env)).toEqual(env);
    for (const RELEASE_VERSION of ["0.7.0-alpha.1", "0.7.0-beta.1", "0.7.0-rc.1", "0.7.0+local", "bad"]) {
      expect(() => signingConfiguration({ ...env, RELEASE_VERSION })).toThrow();
    }
    expect(() => signingConfiguration({ ...env, REHEARSAL: "true" })).toThrow();
  });
  test("rejects every missing Apple credential or identity field", () => {
    for (const key of Object.keys(env).filter((key) => key.startsWith("APPLE_") || key === "RUNNER_TEMP")) {
      expect(() => signingConfiguration({ ...env, [key]: "" })).toThrow(`Missing required ${key}`);
    }
    expect(() => signingConfiguration({ ...env, APPLE_CERTIFICATE_BASE64: "%%%" })).toThrow();
    expect(() => signingConfiguration({ ...env, APPLE_TEAM_ID: "WRONGTEAM1" })).toThrow();
    expect(() => signingConfiguration({ ...env, APPLE_SIGNING_IDENTITY: "-" })).toThrow();
  });
  test("requires expected public identity, timestamp and hardened runtime", () => {
    expect(() => verifySignatureDetails(details, env.APPLE_TEAM_ID, env.APPLE_SIGNING_IDENTITY)).not.toThrow();
    for (const bad of [details.replace("TeamIdentifier=0123456789", "TeamIdentifier=WRONGTEAM1"),
      details.replace("Authority=Developer ID Application", "Authority=Apple Development"),
      details.replace(/^Timestamp=.*\n/m, ""), details.replace("(runtime)", ""),
      details + "Signature=adhoc\n", details + "linker-signed\n"]) {
      expect(() => verifySignatureDetails(bad, env.APPLE_TEAM_ID, env.APPLE_SIGNING_IDENTITY)).toThrow();
    }
  });
  test("disk images still require Developer ID and timestamp without a runtime flag", () => {
    expect(() => verifySignatureDetails(details.replace("(runtime)", ""), env.APPLE_TEAM_ID, env.APPLE_SIGNING_IDENTITY, false)).not.toThrow();
    expect(() => verifySignatureDetails("Signature=adhoc", env.APPLE_TEAM_ID, env.APPLE_SIGNING_IDENTITY, false)).toThrow();
  });
});

import { mkdtempSync, mkdirSync, writeFileSync, rmSync, existsSync, cpSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { cleanup, signMacOSRelease } from "./sign-macos-release.js";

function signingFixture(run) {
  const root = mkdtempSync(join(tmpdir(), "apple-signing-test-"));
  const original = { ...process.env };
  Object.assign(process.env, env, { RUNNER_TEMP: root });
  const input = join(root, "input"); const output = join(root, "output");
  mkdirSync(input); writeFileSync(join(input, "Unfocus_0.7.0_aarch64.dmg"), "fixture");
  const calls = [];
  function execute(tool, args) {
    calls.push([tool, args]);
    if (tool.endsWith("security") && args[0] === "create-keychain") writeFileSync(args.at(-1), "fixture");
    if (tool.endsWith("hdiutil") && args[0] === "attach") {
      const mount = args[args.indexOf("-mountpoint") + 1];
      mkdirSync(join(mount, "Unfocus.app/Contents/MacOS"), { recursive: true });
      writeFileSync(join(mount, "Unfocus.app/Contents/MacOS/unfocus"), "fixture");
    }
    if (tool.endsWith("hdiutil") && args[0] === "detach") {
      rmSync(args[1], { recursive: true }); mkdirSync(args[1]);
    }
    if (tool.endsWith("ditto") && !args.includes("-c")) cpSync(args[0], args[1], { recursive: true });
    if (tool.endsWith("plutil")) return "0.7.0\n";
    if (tool.endsWith("lipo")) return "arm64\n";
    if (tool.endsWith("file")) return "Mach-O 64-bit executable arm64";
    if (tool.endsWith("codesign") && args[0] === "-dv") return details;
    if (tool.endsWith("xcrun") && args[1] === "submit") return '{"status":"Accepted"}';
    if (tool.endsWith("cp")) writeFileSync(args[1], "signed fixture");
    return "";
  }
  try { run({ root, input, output, calls, execute }); }
  finally { process.env = original; rmSync(root, { recursive: true, force: true }); }
}

test("signing orchestrates inner-to-outer signatures, notarizes app and final DMG, then releases output", () => {
  signingFixture(({ root, input, output, execute, calls }) => {
    signMacOSRelease(input, output, "aarch64-apple-darwin", execute);
    expect(existsSync(join(output, "Unfocus_0.7.0_aarch64.dmg"))).toBe(true);
    expect(existsSync(join(root, "unfocus-apple-signing"))).toBe(false);
    const signed = calls.filter(([tool, args]) => tool.endsWith("codesign") && args.includes("--sign"));
    expect(signed).toHaveLength(3);
    expect(signed[0][1].at(-1)).toEndWith("Contents/MacOS/unfocus");
    expect(signed[1][1].at(-1)).toEndWith("Unfocus.app");
    expect(calls.filter(([tool, args]) => tool.endsWith("xcrun") && args[1] === "submit")).toHaveLength(2);
    expect(calls.filter(([tool]) => tool.endsWith("spctl"))).toHaveLength(3);
    expect(calls.findIndex(([, args]) => args[0] === "convert")).toBeLessThan(calls.findIndex(([, args]) => args[0] === "attach"));
  });
});

test("notarization rejection prevents final output and still deletes the keychain", () => {
  signingFixture(({ root, input, output, execute, calls }) => {
    expect(() => signMacOSRelease(input, output, "aarch64-apple-darwin", (tool, args) => {
      if (tool.endsWith("xcrun") && args[1] === "submit") return '{"status":"Invalid"}';
      return execute(tool, args);
    })).toThrow("not accepted");
    expect(existsSync(output)).toBe(false);
    expect(existsSync(join(root, "unfocus-apple-signing"))).toBe(false);
    expect(calls.some(([, args]) => args[0] === "delete-keychain")).toBe(true);
  });
});

test("cleanup independently attempts volume detach and keychain deletion", () => {
  signingFixture(({ root }) => {
    const work = join(root, "unfocus-apple-signing");
    mkdirSync(join(work, "mount"), { recursive: true });
    writeFileSync(join(work, "mount/file"), "fixture");
    writeFileSync(join(work, "release.keychain-db"), "fixture");
    writeFileSync(join(work, "AuthKey.p8"), "fixture");
    const operations = [];
    expect(() => cleanup((tool, args) => { operations.push(args[0]); throw new Error("fixture failure"); })).toThrow("Apple cleanup failed");
    expect(operations).toEqual(["detach", "delete-keychain"]);
    expect(existsSync(join(work, "AuthKey.p8"))).toBe(false);
    expect(existsSync(join(work, "mount/file"))).toBe(true);
  });
});

test("Gatekeeper failure blocks output and cleanup never masks the original failure", () => {
  signingFixture(({ input, output, execute }) => {
    expect(() => signMacOSRelease(input, output, "aarch64-apple-darwin", (tool, args) => {
      if (tool.endsWith("spctl")) throw new Error("Gatekeeper rejected fixture");
      if (tool.endsWith("security") && args[0] === "delete-keychain") throw new Error("cleanup fixture error");
      return execute(tool, args);
    })).toThrow("Gatekeeper rejected fixture");
    expect(existsSync(output)).toBe(false);
  });
});
