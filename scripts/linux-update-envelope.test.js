import { afterEach, describe, expect, test } from "bun:test";
import { createHash } from "node:crypto";
import {
  mkdirSync,
  mkdtempSync,
  rmSync,
  symlinkSync,
  truncateSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  LINUX_UPDATE_TARGETS,
  createUpdateEnvelope,
  createUpdatePayloadFromDirectory,
  linuxUpdateEnvelopeName,
  linuxUpdatePackageNames,
  linuxUpdatePayloadName,
  parseUpdateEnvelopeStructure,
  parseUpdatePayload,
  releaseChannel,
  requireBetaVersion,
  validateTauriSignatureEncoding,
} from "./linux-update-envelope.js";

const temporaryDirectories = [];
const VERSION = "0.7.0-beta.1";
const INNER_HASH = "1".repeat(64);

function canonicalJson(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

function signature(filename, keyByte = 7, algorithm = "ED") {
  const packet = Buffer.alloc(74, 0x42);
  packet.write(algorithm, 0, "ascii");
  packet.fill(keyByte, 2, 10);
  const global = Buffer.alloc(64, 0x24);
  const inner =
    "untrusted comment: signature from tauri secret key\n" +
    `${packet.toString("base64")}\n` +
    `trusted comment: timestamp:1788385296\tfile:${filename}\n` +
    `${global.toString("base64")}\n`;
  return Buffer.from(inner, "utf8").toString("base64");
}

function temporaryDirectory() {
  const directory = mkdtempSync(join(tmpdir(), "unfocus-update-envelope-test-"));
  temporaryDirectories.push(directory);
  return directory;
}

function populateSignedPackages(version = VERSION, keyByte = 7) {
  const directory = temporaryDirectory();
  const names = linuxUpdatePackageNames(version);
  for (const name of Object.values(names)) {
    writeFileSync(join(directory, name), `fixture bytes for ${name}\n`);
    writeFileSync(join(directory, `${name}.sig`), signature(name, keyByte));
  }
  return { directory, names };
}

async function payloadFixture(version = VERSION, keyByte = 7) {
  const { directory, names } = populateSignedPackages(version, keyByte);
  const payloadText = await createUpdatePayloadFromDirectory(directory, version, INNER_HASH);
  return { directory, names, payloadText, payload: JSON.parse(payloadText), keyByte };
}

function envelopeText(payloadText, payloadSignature) {
  return canonicalJson({
    schemaVersion: 1,
    payload: Buffer.from(payloadText, "utf8").toString("base64"),
    signature: payloadSignature,
  });
}

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

describe("Linux update release versions", () => {
  test.each([
    ["1.2.3", "stable"],
    ["1.2.3-alpha.0", "alpha"],
    ["1.2.3-beta.12", "beta"],
    ["1.2.3-rc.4", "rc"],
  ])("classifies %s as %s", (version, expected) => {
    expect(releaseChannel(version)).toBe(expected);
  });

  test.each([
    "1.2",
    "01.2.3-beta.1",
    "1.2.3-beta.01",
    "1.2.3-beta",
    "1.2.3-preview.1",
    "1.2.3-beta.1+build",
  ])("rejects unsupported release version %s", (version) => {
    expect(() => releaseChannel(version)).toThrow("unsupported release version");
  });

  test("allows update envelopes only for exact beta versions", () => {
    expect(requireBetaVersion(VERSION)).toBe(VERSION);
    expect(() => requireBetaVersion("0.7.0-alpha.1")).toThrow("require an exact beta version");
    expect(() => requireBetaVersion("0.7.0")).toThrow("require an exact beta version");
  });

  test("derives exact package, payload, and envelope names", () => {
    expect(linuxUpdatePackageNames(VERSION)).toEqual({
      appimage: `Unfocus_${VERSION}_amd64.AppImage`,
      deb: `Unfocus_${VERSION}_amd64.deb`,
      rpm: `Unfocus-${VERSION}-1.x86_64.rpm`,
    });
    expect(linuxUpdatePayloadName(VERSION)).toBe(`Unfocus_${VERSION}_linux_x86_64.update.payload.json`);
    expect(linuxUpdateEnvelopeName(VERSION)).toBe(`Unfocus_${VERSION}_linux_x86_64.update.json`);
  });
});

describe("Tauri updater signature structure", () => {
  test("accepts a canonical prehashed signature and returns its key ID", () => {
    expect(validateTauriSignatureEncoding(signature("fixture.AppImage"), "fixture.AppImage")).toEqual({
      encoded: signature("fixture.AppImage"),
      keyId: "0707070707070707",
    });
  });

  test("rejects legacy signatures and the wrong trusted-comment filename", () => {
    expect(() => validateTauriSignatureEncoding(signature("fixture.AppImage", 7, "Ed"), "fixture.AppImage")).toThrow(
      "must use a prehashed minisign signature packet",
    );
    expect(() => validateTauriSignatureEncoding(signature("other.AppImage"), "fixture.AppImage")).toThrow(
      "trusted comment names the wrong file",
    );
  });

  test.each(["", "not base64", "AAAA\nAAAA", " AAAA"])("rejects malformed outer signature %p", (value) => {
    expect(() => validateTauriSignatureEncoding(value, "fixture.AppImage")).toThrow();
  });
});

describe("Linux update payload and envelope", () => {
  test("builds deterministic canonical metadata from exact signed package bytes", async () => {
    const { names, payloadText, payload, keyByte } = await payloadFixture();
    const second = await createUpdatePayloadFromDirectory(
      temporaryDirectories.at(-1),
      VERSION,
      INNER_HASH,
    );
    expect(second).toBe(payloadText);
    expect(payload.schemaVersion).toBe(1);
    expect(payload.channel).toBe("beta");
    expect(payload.version).toBe(VERSION);
    expect(Object.keys(payload.platforms)).toEqual(LINUX_UPDATE_TARGETS);

    const appimage = payload.platforms["linux-x86_64-appimage"];
    expect(appimage.url).toBe(
      `https://github.com/abhiksark/unfocus/releases/download/v${VERSION}/${names.appimage}`,
    );
    expect(appimage.innerExecutableSha256).toBe(INNER_HASH);
    expect(appimage.sha256).toBe(
      createHash("sha256").update(`fixture bytes for ${names.appimage}\n`).digest("hex"),
    );

    const payloadSignature = signature(linuxUpdatePayloadName(VERSION), keyByte);
    const envelope = createUpdateEnvelope(payloadText, payloadSignature);
    expect(envelope).toBe(canonicalJson(JSON.parse(envelope)));
    const parsed = parseUpdateEnvelopeStructure(envelope, VERSION);
    expect(parsed.payload).toEqual(payload);
    expect(parsed.payloadText).toBe(payloadText);
    expect(parsed.packageKeyId).toBe("0707070707070707");
  });

  test("rejects package signatures from different updater key IDs", async () => {
    const { directory, names } = populateSignedPackages();
    writeFileSync(join(directory, `${names.rpm}.sig`), signature(names.rpm, 8));
    await expect(createUpdatePayloadFromDirectory(directory, VERSION, INNER_HASH)).rejects.toThrow(
      "package signatures do not use one updater key ID",
    );
  });

  test("rejects a payload signature from a different updater key ID", async () => {
    const { payloadText } = await payloadFixture();
    expect(() => createUpdateEnvelope(payloadText, signature(linuxUpdatePayloadName(VERSION), 8))).toThrow(
      "payload and package signatures do not use one updater key ID",
    );
  });

  test("rejects unsafe or generic target metadata", async () => {
    const { payload } = await payloadFixture();
    payload.platforms["linux-x86_64-appimage"].url = "https://example.com/update.AppImage";
    const unsafe = canonicalJson(payload);
    expect(() => parseUpdatePayload(unsafe)).toThrow(".url must be exactly");

    payload.platforms["linux-x86_64-appimage"].url =
      `https://github.com/abhiksark/unfocus/releases/download/v${VERSION}/Unfocus_${VERSION}_amd64.AppImage`;
    payload.platforms["linux-x86_64"] = payload.platforms["linux-x86_64-appimage"];
    const generic = canonicalJson(payload);
    expect(() => parseUpdatePayload(generic)).toThrow("platforms keys must be exactly");
  });

  test("rejects noncanonical JSON, unknown fields, and version mismatch", async () => {
    const { payloadText, payload } = await payloadFixture();
    expect(() => parseUpdatePayload(JSON.stringify(payload))).toThrow("canonical two-space JSON form");

    payload.notes = "remote text";
    expect(() => parseUpdatePayload(canonicalJson(payload))).toThrow("payload keys must be exactly");
    expect(() => parseUpdatePayload(payloadText, "0.7.0-beta.2")).toThrow(
      `version is ${VERSION}, expected 0.7.0-beta.2`,
    );
  });

  test("rejects unknown envelope fields while leaving byte authenticity to cryptographic verification", async () => {
    const { payloadText, payload, keyByte } = await payloadFixture();
    const payloadSignature = signature(linuxUpdatePayloadName(VERSION), keyByte);
    const envelope = JSON.parse(createUpdateEnvelope(payloadText, payloadSignature));
    envelope.extra = true;
    expect(() => parseUpdateEnvelopeStructure(canonicalJson(envelope))).toThrow("envelope keys must be exactly");

    delete envelope.extra;
    payload.platforms["linux-x86_64-deb"].sha256 = "f".repeat(64);
    envelope.payload = Buffer.from(canonicalJson(payload), "utf8").toString("base64");
    expect(parseUpdateEnvelopeStructure(canonicalJson(envelope)).payload.platforms["linux-x86_64-deb"].sha256).toBe(
      "f".repeat(64),
    );
    // Structural parsing intentionally cannot claim cryptographic verification.
    expect(envelopeText(payloadText, payloadSignature)).toBe(createUpdateEnvelope(payloadText, payloadSignature));
  });

  test("rejects oversized package files and metadata", async () => {
    const { directory, names } = populateSignedPackages();
    truncateSync(join(directory, names.appimage), 536_870_913);
    await expect(createUpdatePayloadFromDirectory(directory, VERSION, INNER_HASH)).rejects.toThrow(
      "must be a positive safe integer no greater than 536870912",
    );

    const { payload } = await payloadFixture();
    for (const target of LINUX_UPDATE_TARGETS) {
      const oversized = structuredClone(payload);
      oversized.platforms[target].sizeBytes = 536_870_913;
      expect(() => parseUpdatePayload(canonicalJson(oversized))).toThrow(
        "must be a positive safe integer no greater than 536870912",
      );
    }
  });

  test("rejects package and signature symlinks", async () => {
    const { directory, names } = populateSignedPackages();
    rmSync(join(directory, names.deb));
    symlinkSync(names.rpm, join(directory, names.deb));
    await expect(createUpdatePayloadFromDirectory(directory, VERSION, INNER_HASH)).rejects.toThrow(
      "must be a regular non-symlink file",
    );

    const other = temporaryDirectory();
    mkdirSync(join(other, "assets"));
    const populated = populateSignedPackages();
    rmSync(join(populated.directory, `${populated.names.deb}.sig`));
    symlinkSync(`${populated.names.rpm}.sig`, join(populated.directory, `${populated.names.deb}.sig`));
    await expect(createUpdatePayloadFromDirectory(populated.directory, VERSION, INNER_HASH)).rejects.toThrow(
      "must be a regular non-symlink file",
    );
  });
});
