import { afterEach, describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import {
  existsSync,
  lstatSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { expectedReleaseFilenames, verifyChecksums } from "./assemble-release-artifacts.js";
import { finalizeBaseRelease, finalizeBetaRelease } from "./finalize-release-artifacts.js";
import { parsePackageEvidence } from "./inspect-linux-packages.js";
import {
  linuxUpdateEnvelopeName,
  linuxUpdatePayloadName,
  parseUpdateEnvelopeStructure,
} from "./linux-update-envelope.js";
import { signLinuxUpdate } from "./sign-linux-update.js";
import { verifyFinalRelease } from "./verify-final-release.js";

const VERSION = "0.7.0-beta.1";
const BUILD_ID = "0123456789abcdef0123456789abcdef01234567";
const INNER_HASH = "1".repeat(64);
const temporaryDirectories = [];

function canonicalJson(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

function sha256(value) {
  return createHash("sha256").update(value).digest("hex");
}

function temporaryDirectory(label = "root") {
  const directory = mkdtempSync(join(tmpdir(), `unfocus-release-${label}-test-`));
  temporaryDirectories.push(directory);
  return directory;
}

function signature(filename, keyByte = 7) {
  const packet = Buffer.alloc(74, 0x42);
  packet.write("ED", 0, "ascii");
  packet.fill(keyByte, 2, 10);
  const global = Buffer.alloc(64, 0x24);
  const inner =
    "untrusted comment: signature from tauri secret key\n" +
    `${packet.toString("base64")}\n` +
    `trusted comment: timestamp:1788385296\tfile:${filename}\n` +
    `${global.toString("base64")}\n`;
  return Buffer.from(inner, "utf8").toString("base64");
}

function baseCandidate(version = VERSION) {
  const directory = temporaryDirectory("candidate");
  const files = expectedReleaseFilenames(version);
  for (const name of files) {
    const bytes = name.endsWith(".AppImage") ? Buffer.alloc(256, 0x41) : Buffer.from(`fixture ${name}\n`);
    writeFileSync(join(directory, name), bytes);
  }
  const checksums = files
    .map((name) => `${sha256(readFileSync(join(directory, name)))}  ${name}`)
    .join("\n");
  writeFileSync(join(directory, "SHA256SUMS"), `${checksums}\n`);
  return { directory, files };
}

function packageEvidence(candidate, version = VERSION) {
  const appimageName = `Unfocus_${version}_amd64.AppImage`;
  const debName = `Unfocus_${version}_amd64.deb`;
  const rpmName = `Unfocus-${version}-1.x86_64.rpm`;
  const packageFile = (name) => ({
    filename: name,
    sizeBytes: lstatSync(join(candidate, name)).size,
    sha256: sha256(readFileSync(join(candidate, name))),
  });
  const evidence = {
    schemaVersion: 1,
    version,
    channel: version.includes("-beta.") ? "beta" : version.includes("-alpha.") ? "alpha" : "stable",
    candidateChecksumsSha256: sha256(readFileSync(join(candidate, "SHA256SUMS"))),
    packages: {
      appimage: {
        ...packageFile(appimageName),
        filesystemOffset: 64,
        inodeCount: 1,
        innerExecutableSha256: INNER_HASH,
        innerExecutableBuildId: BUILD_ID,
      },
      deb: {
        ...packageFile(debName),
        package: "unfocus",
        version: version.includes("-beta.")
          ? version.replace("-beta.", "~beta.") + "-1"
          : version.includes("-alpha.")
            ? version.replace("-alpha.", "~alpha.") + "-1"
            : `${version}-1`,
        architecture: "amd64",
        innerExecutableBuildId: BUILD_ID,
      },
      rpm: {
        ...packageFile(rpmName),
        name: "unfocus",
        version,
        release: "1",
        architecture: "x86_64",
        innerExecutableBuildId: BUILD_ID,
      },
    },
  };
  const path = join(temporaryDirectory("evidence"), "linux-package-evidence.json");
  writeFileSync(path, canonicalJson(evidence));
  return { evidence, path };
}

function structuralSigner(path) {
  writeFileSync(`${path}.sig`, signature(basename(path)));
}

async function signedFixture() {
  const candidate = baseCandidate();
  const evidence = packageEvidence(candidate.directory);
  const staging = join(temporaryDirectory("signed-parent"), "signed");
  await signLinuxUpdate(candidate.directory, evidence.path, staging, VERSION, {
    privateKey: "test-only key value",
    password: "test-only password",
    signFile: structuralSigner,
  });
  const publicKey = join(temporaryDirectory("public-key"), "linux-beta.pub");
  writeFileSync(publicKey, "test-only public key\n");
  return { candidate, evidence, staging, publicKey };
}

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

describe("Linux update signing boundary", () => {
  test("stages base assets, signs three packages, then signs one canonical payload", async () => {
    const { candidate, evidence, staging } = await signedFixture();
    const expected = [
      ...candidate.files,
      "SHA256SUMS",
      ...Object.values(evidence.evidence.packages).map((package_) => `${package_.filename}.sig`),
      linuxUpdatePayloadName(VERSION),
      `${linuxUpdatePayloadName(VERSION)}.sig`,
    ].sort();
    expect(readdirSync(staging).sort()).toEqual(expected);
    expect(parsePackageEvidence(readFileSync(evidence.path, "utf8"), VERSION)).toEqual(evidence.evidence);
  });

  test("passes only the key and minimal process context to signer children", async () => {
    const candidate = baseCandidate();
    const evidence = packageEvidence(candidate.directory);
    const output = join(temporaryDirectory("environment-parent"), "signed");
    const environments = [];
    await signLinuxUpdate(candidate.directory, evidence.path, output, VERSION, {
      privateKey: "test-only key value",
      password: "test-only password",
      signFile: (path, _root, environment) => {
        environments.push(environment);
        structuralSigner(path);
      },
    });
    expect(environments).toHaveLength(4);
    for (const environment of environments) {
      expect(environment.TAURI_SIGNING_PRIVATE_KEY).toBe("test-only key value");
      expect(environment.TAURI_SIGNING_PRIVATE_KEY_PASSWORD).toBe("test-only password");
      expect(environment.GH_TOKEN).toBeUndefined();
      expect(environment.GITHUB_TOKEN).toBeUndefined();
      expect(environment.HTTP_PROXY).toBeUndefined();
      expect(environment.HTTPS_PROXY).toBeUndefined();
    }
  });

  test("rejects candidate drift before invoking the signer", async () => {
    const candidate = baseCandidate();
    const evidence = packageEvidence(candidate.directory);
    writeFileSync(join(candidate.directory, `Unfocus_${VERSION}_amd64.AppImage`), "changed package");
    let called = false;
    const output = join(temporaryDirectory("drift-parent"), "signed");
    await expect(
      signLinuxUpdate(candidate.directory, evidence.path, output, VERSION, {
        privateKey: "test-only key value",
        password: "test-only password",
        signFile: () => {
          called = true;
        },
      }),
    ).rejects.toThrow();
    expect(called).toBe(false);
    expect(existsSync(output)).toBe(false);
  });

  test("removes partial local staging when the signer fails", async () => {
    const candidate = baseCandidate();
    const evidence = packageEvidence(candidate.directory);
    const output = join(temporaryDirectory("failure-parent"), "signed");
    await expect(
      signLinuxUpdate(candidate.directory, evidence.path, output, VERSION, {
        privateKey: "test-only key value",
        password: "test-only password",
        signFile: () => {
          throw new Error("injected signer failure");
        },
      }),
    ).rejects.toThrow("injected signer failure");
    expect(existsSync(output)).toBe(false);
  });

  test("requires beta channel and protected signing values", async () => {
    const candidate = baseCandidate();
    const evidence = packageEvidence(candidate.directory);
    await expect(
      signLinuxUpdate(candidate.directory, evidence.path, join(temporaryDirectory(), "missing"), VERSION),
    ).rejects.toThrow("requires the protected updater key and password");
    await expect(
      signLinuxUpdate(candidate.directory, evidence.path, join(temporaryDirectory(), "alpha"), "0.7.0-alpha.1", {
        privateKey: "key",
        password: "password",
      }),
    ).rejects.toThrow("require an exact beta version");
  });
});

describe("release finalization", () => {
  test("emits the exact 14-file beta inventory and checksums after signature verification", async () => {
    const { candidate, evidence, staging, publicKey } = await signedFixture();
    const output = join(temporaryDirectory("final-parent"), "final");
    const calls = [];
    const names = await finalizeBetaRelease(staging, evidence.path, publicKey, output, VERSION, {
      verifySignatures: (key, paths) => calls.push({ key, paths }),
    });

    expect(calls).toHaveLength(1);
    expect(calls[0].key).toBe(publicKey);
    expect(calls[0].paths).toHaveLength(4);
    expect(names).toHaveLength(14);
    expect(readdirSync(output).sort()).toEqual(names);
    expect(existsSync(join(output, linuxUpdatePayloadName(VERSION)))).toBe(false);
    expect(existsSync(join(output, `${linuxUpdatePayloadName(VERSION)}.sig`))).toBe(false);
    expect(existsSync(join(output, linuxUpdateEnvelopeName(VERSION)))).toBe(true);
    verifyChecksums(output, names.filter((name) => name !== "SHA256SUMS"));
    expect(
      parseUpdateEnvelopeStructure(readFileSync(join(output, linuxUpdateEnvelopeName(VERSION)), "utf8"), VERSION)
        .payload.version,
    ).toBe(VERSION);
    expect(
      await verifyFinalRelease(output, candidate.directory, evidence.path, publicKey, VERSION, {
        verifySignatures: () => {},
      }),
    ).toEqual(names);
    expect(candidate.files).toHaveLength(9);
  });

  test("fails before signature verification when a sidecar differs from the signed payload", async () => {
    const { evidence, staging, publicKey } = await signedFixture();
    writeFileSync(join(staging, `${evidence.evidence.packages.deb.filename}.sig`), signature(evidence.evidence.packages.deb.filename, 8));
    let verified = false;
    await expect(
      finalizeBetaRelease(
        staging,
        evidence.path,
        publicKey,
        join(temporaryDirectory("tamper-parent"), "final"),
        VERSION,
        { verifySignatures: () => (verified = true) },
      ),
    ).rejects.toThrow("payload signature does not match");
    expect(verified).toBe(false);
  });

  test("preserves the base inventory unchanged for non-beta channels", async () => {
    const version = "0.7.0-alpha.1";
    const candidate = baseCandidate(version);
    const output = join(temporaryDirectory("base-parent"), "final");
    const names = await finalizeBaseRelease(candidate.directory, output, version);
    expect(names).toEqual([...candidate.files, "SHA256SUMS"].sort());
    for (const name of names) {
      expect(readFileSync(join(output, name))).toEqual(readFileSync(join(candidate.directory, name)));
    }
  });

  test("verifies reusable-draft signatures before accepting final checksums", async () => {
    const { candidate, evidence, staging, publicKey } = await signedFixture();
    const output = join(temporaryDirectory("checksum-order-parent"), "final");
    await finalizeBetaRelease(staging, evidence.path, publicKey, output, VERSION, {
      verifySignatures: () => {},
    });
    const checksumPath = join(output, "SHA256SUMS");
    const reordered = readFileSync(checksumPath, "utf8").trimEnd().split("\n").reverse().join("\n");
    writeFileSync(checksumPath, `${reordered}\n`);
    let verified = false;
    await expect(
      verifyFinalRelease(output, candidate.directory, evidence.path, publicKey, VERSION, {
        verifySignatures: () => (verified = true),
      }),
    ).rejects.toThrow("sorted canonical");
    expect(verified).toBe(true);
  });

  test("rejects final bytes that differ from the validated candidate", async () => {
    const { candidate, evidence, staging, publicKey } = await signedFixture();
    const output = join(temporaryDirectory("verify-tamper-parent"), "final");
    await finalizeBetaRelease(staging, evidence.path, publicKey, output, VERSION, {
      verifySignatures: () => {},
    });
    writeFileSync(join(output, `Unfocus_${VERSION}_x64.dmg`), "tampered final asset");
    await expect(
      verifyFinalRelease(output, candidate.directory, evidence.path, publicKey, VERSION, {
        verifySignatures: () => {},
      }),
    ).rejects.toThrow();
  });

  test("never routes beta through unsigned base finalization", async () => {
    const candidate = baseCandidate();
    await expect(
      finalizeBaseRelease(candidate.directory, join(temporaryDirectory("wrong-parent"), "final"), VERSION),
    ).rejects.toThrow("beta releases require signed Linux update finalization");
  });
});
