import { afterEach, describe, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import {
  chmodSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readlinkSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  debianVersionIsGreater,
  finalizeDebianPackage,
  readDebianField,
  semverToDebianVersion,
  verifyDebianPackage,
} from "./debian-package.js";

const temporaryDirectories = [];
const linuxPackaging = process.platform === "linux" ? test : test.skip;
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

function temporaryDirectory() {
  const directory = mkdtempSync(join(tmpdir(), "unfocus-debian-test-"));
  temporaryDirectories.push(directory);
  return directory;
}

function buildFixture(version = "0.2.0-alpha.1", packageDirectory) {
  const work = temporaryDirectory();
  const root = join(work, "root");
  const debian = join(root, "DEBIAN");
  const binary = join(root, "usr", "bin", "unfocus");
  const desktop = join(root, "usr", "share", "applications", "Unfocus.desktop");
  const icon = join(root, "usr", "share", "icons", "hicolor", "32x32", "apps", "unfocus.png");
  const notices = join(root, "usr", "lib", "Unfocus", "THIRD_PARTY_NOTICES.txt");
  for (const directory of [
    debian,
    join(root, "usr", "bin"),
    join(root, "usr", "share", "applications"),
    join(root, "usr", "share", "icons", "hicolor", "32x32", "apps"),
    join(root, "usr", "lib", "Unfocus"),
  ]) {
    mkdirSync(directory, { recursive: true });
  }
  writeFileSync(
    join(debian, "control"),
    `Package: unfocus\nVersion: ${version}\nArchitecture: amd64\nDepends: libgtk-3-0, libwebkit2gtk-4.1-0\nMaintainer: Unfocus\nDescription: fixture\n`,
  );
  writeFileSync(join(debian, "md5sums"), "fixture  usr/bin/unfocus\n");
  writeFileSync(binary, "#!/bin/sh\nexit 0\n");
  chmodSync(binary, 0o755);
  writeFileSync(desktop, "[Desktop Entry]\nName=Unfocus\nExec=unfocus\n");
  writeFileSync(icon, "fixture-icon\n");
  writeFileSync(notices, "fixture notices\n");
  symlinkSync("unfocus", join(root, "usr", "bin", "unfocus-link"));

  const destination = packageDirectory ?? work;
  mkdirSync(destination, { recursive: true });
  const packagePath = join(destination, `Unfocus_${version}_amd64.deb`);
  execFileSync("dpkg-deb", ["--build", "--root-owner-group", root, packagePath]);
  return { work, packagePath };
}

describe("SemVer to Debian mapping", () => {
  test.each([
    ["0.2.0", "0.2.0-1"],
    ["0.2.0-alpha.1", "0.2.0~alpha.1-1"],
    ["10.20.30-beta.2", "10.20.30~beta.2-1"],
    ["1.0.0-rc.9", "1.0.0~rc.9-1"],
  ])("maps %s", (canonical, expected) => {
    expect(semverToDebianVersion(canonical)).toBe(expected);
  });

  test.each([
    "1.2",
    "1.2.3+build.1",
    "1.2.3-preview.1",
    "1.2.3-alpha",
    "1.2.3-alpha.01",
    "01.2.3-alpha.1",
    "1.02.3",
    "1.2.03",
    "1.2.3-alpha.1.extra",
    "1.2.3-ALPHA.1",
    "1.2.3-alpha-1",
  ])("rejects %s", (canonical) => {
    expect(() => semverToDebianVersion(canonical)).toThrow("unsupported canonical version");
  });
});

describe("Debian ordering", () => {
  linuxPackaging("proves legacy, prerelease, stable, patch, and minor transitions", () => {
    const ordered = [
      "0.1.0-alpha.1",
      "0.2.0~alpha.1-1",
      "0.2.0~alpha.2-1",
      "0.2.0~beta.1-1",
      "0.2.0~rc.1-1",
      "0.2.0-1",
      "0.2.1~alpha.1-1",
      "0.2.1-1",
      "0.3.0~alpha.1-1",
    ];
    for (let index = 1; index < ordered.length; index += 1) {
      expect(debianVersionIsGreater(ordered[index], ordered[index - 1])).toBe(true);
      expect(debianVersionIsGreater(ordered[index - 1], ordered[index])).toBe(false);
      expect(debianVersionIsGreater(ordered[index], ordered[index])).toBe(false);
    }
  });
});

describe("Debian package finalization", () => {
  linuxPackaging("changes only Version and preserves the package payload", () => {
    const { work, packagePath } = buildFixture();

    const fields = finalizeDebianPackage(packagePath, "0.2.0-alpha.1");
    expect(fields).toEqual({
      Package: "unfocus",
      Version: "0.2.0~alpha.1-1",
      Architecture: "amd64",
      Depends: "libgtk-3-0, libwebkit2gtk-4.1-0",
    });
    expect(verifyDebianPackage(packagePath, "0.2.0-alpha.1")).toEqual(fields);

    const extracted = join(work, "extracted");
    execFileSync("dpkg-deb", ["--raw-extract", packagePath, extracted]);
    const binary = join(extracted, "usr", "bin", "unfocus");
    expect(readFileSync(binary, "utf8")).toBe("#!/bin/sh\nexit 0\n");
    expect(lstatSync(binary).mode & 0o777).toBe(0o755);
    expect(readlinkSync(join(extracted, "usr", "bin", "unfocus-link"))).toBe("unfocus");
    expect(readFileSync(join(extracted, "usr", "share", "applications", "Unfocus.desktop"), "utf8")).toContain("Exec=unfocus");
    expect(readFileSync(join(extracted, "usr", "share", "icons", "hicolor", "32x32", "apps", "unfocus.png"), "utf8")).toBe("fixture-icon\n");
    expect(readFileSync(join(extracted, "usr", "lib", "Unfocus", "THIRD_PARTY_NOTICES.txt"), "utf8")).toBe("fixture notices\n");
    expect(readDebianField(packagePath, "Depends")).toBe("libgtk-3-0, libwebkit2gtk-4.1-0");
  });

  linuxPackaging("refuses a package whose generated Version is not canonical", () => {
    const { packagePath } = buildFixture("0.1.0-alpha.1");
    expect(() => finalizeDebianPackage(packagePath, "0.2.0-alpha.1")).toThrow(
      "generated Debian Version is 0.1.0-alpha.1",
    );
  });

  linuxPackaging("finalizes the Tauri Debian artifact before collecting it", () => {
    const canonicalVersion = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
    const work = temporaryDirectory();
    const bundle = join(work, "target", "release", "bundle", "deb");
    const { packagePath } = buildFixture(canonicalVersion, bundle);
    const output = join(work, "release-artifacts");
    execFileSync("bun", ["run", "scripts/collect-release-artifacts.js", output], {
      cwd: root,
      env: { ...process.env, TAURI_ARTIFACT_PATHS: JSON.stringify([packagePath]) },
    });

    const collected = join(output, `Unfocus_${canonicalVersion}_amd64.deb`);
    expect(readDebianField(packagePath, "Version")).toBe(semverToDebianVersion(canonicalVersion));
    expect(verifyDebianPackage(collected, canonicalVersion).Version).toBe(
      semverToDebianVersion(canonicalVersion),
    );
  });
});
