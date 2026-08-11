import { afterEach, describe, expect, test } from "bun:test";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  assembleReleaseArtifacts,
  expectedReleaseFilenames,
  verifyChecksums,
} from "./assemble-release-artifacts.js";
import { finalizeDebianPackage } from "./debian-package.js";

const temporaryDirectories = [];
const linuxPackaging = process.platform === "linux" ? test : test.skip;

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

function temporaryDirectory() {
  const directory = mkdtempSync(join(tmpdir(), "unfocus-assembly-test-"));
  temporaryDirectories.push(directory);
  return directory;
}

function populateInventory(version) {
  const work = temporaryDirectory();
  const downloads = join(work, "downloads");
  const output = join(work, "output");
  mkdirSync(downloads, { recursive: true });

  for (const [index, name] of expectedReleaseFilenames(version).entries()) {
    const artifactDirectory = join(downloads, `artifact-${index}`);
    mkdirSync(artifactDirectory, { recursive: true });
    if (name.endsWith(".deb")) {
      const root = join(work, "debian-root");
      mkdirSync(join(root, "DEBIAN"), { recursive: true });
      mkdirSync(join(root, "usr", "bin"), { recursive: true });
      writeFileSync(
        join(root, "DEBIAN", "control"),
        `Package: unfocus\nVersion: ${version}\nArchitecture: amd64\nDepends: libgtk-3-0\nMaintainer: Unfocus\nDescription: fixture\n`,
      );
      writeFileSync(join(root, "usr", "bin", "unfocus"), "fixture\n");
      const packagePath = join(artifactDirectory, name);
      execFileSync("dpkg-deb", ["--build", "--root-owner-group", root, packagePath]);
      finalizeDebianPackage(packagePath, version);
    } else {
      writeFileSync(join(artifactDirectory, name), `${name}\n`);
    }
  }
  return { downloads, output, work };
}

describe("release artifact assembly", () => {
  linuxPackaging("requires the exact inventory and verifies generated checksums", () => {
    const version = "0.2.0-alpha.1";
    const { downloads, output } = populateInventory(version);
    const assembled = assembleReleaseArtifacts(downloads, output, version);
    expect(assembled).toEqual([...expectedReleaseFilenames(version), "SHA256SUMS"].sort());
    verifyChecksums(output, expectedReleaseFilenames(version));
    expect(readFileSync(join(output, "SHA256SUMS"), "utf8")).toContain(
      `  Unfocus_${version}_amd64.deb`,
    );
  });

  linuxPackaging("rejects unexpected files", () => {
    const version = "0.2.0-alpha.1";
    const { downloads, output } = populateInventory(version);
    writeFileSync(join(downloads, "unexpected.zip"), "unexpected\n");
    expect(() => assembleReleaseArtifacts(downloads, output, version)).toThrow("unexpected: unexpected.zip");
  });

  linuxPackaging("rejects missing files", () => {
    const version = "0.2.0-alpha.1";
    const { downloads, output } = populateInventory(version);
    const [missing] = expectedReleaseFilenames(version);
    rmSync(join(downloads, "artifact-0"), { recursive: true, force: true });
    expect(() => assembleReleaseArtifacts(downloads, output, version)).toThrow(`missing: ${missing}`);
  });

  linuxPackaging("rejects basename collisions from separate build jobs", () => {
    const version = "0.2.0-alpha.1";
    const { downloads, output } = populateInventory(version);
    const duplicateDirectory = join(downloads, "duplicate");
    mkdirSync(duplicateDirectory);
    writeFileSync(join(duplicateDirectory, "THIRD_PARTY_NOTICES.txt"), "duplicate\n");
    expect(() => assembleReleaseArtifacts(downloads, output, version)).toThrow(
      "two build jobs produced an asset named THIRD_PARTY_NOTICES.txt",
    );
  });
});
