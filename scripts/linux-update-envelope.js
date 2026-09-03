#!/usr/bin/env bun

import { createHash } from "node:crypto";
import { createReadStream, existsSync, lstatSync, readFileSync, writeFileSync } from "node:fs";
import { basename, join, resolve } from "node:path";

const UPDATE_SCHEMA_VERSION = 1;
const UPDATE_CHANNEL = "beta";
const UPDATE_REPOSITORY = "abhiksark/unfocus";
const MAX_SIGNATURE_BYTES = 8 * 1024;
const MAX_ENVELOPE_BYTES = 64 * 1024;
const MAX_PACKAGE_BYTES = 536_870_912;
const SHA256 = /^[0-9a-f]{64}$/;
const BETA_VERSION = /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)-beta\.(?:0|[1-9]\d*)$/;
const CANONICAL_BASE64 = /^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/;
const TAURI_UNTRUSTED_COMMENT = "untrusted comment: signature from tauri secret key";

export const LINUX_UPDATE_TARGETS = [
  "linux-x86_64-appimage",
  "linux-x86_64-deb",
  "linux-x86_64-rpm",
];

export function releaseChannel(version) {
  if (typeof version !== "string") throw new Error("release version must be a string");
  const match = /^(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)(?:-(alpha|beta|rc)\.(?:0|[1-9]\d*))?$/.exec(
    version,
  );
  if (!match) {
    throw new Error(
      `unsupported release version ${version}: expected X.Y.Z or X.Y.Z-(alpha|beta|rc).N without build metadata or leading zeroes`,
    );
  }
  return match[1] ?? "stable";
}

export function requireBetaVersion(version) {
  releaseChannel(version);
  if (!BETA_VERSION.test(version)) {
    throw new Error(`Linux update envelopes require an exact beta version, received ${version}`);
  }
  return version;
}

export function linuxUpdatePackageNames(version) {
  requireBetaVersion(version);
  return {
    appimage: `Unfocus_${version}_amd64.AppImage`,
    deb: `Unfocus_${version}_amd64.deb`,
    rpm: `Unfocus-${version}-1.x86_64.rpm`,
  };
}

export function linuxUpdatePayloadName(version) {
  requireBetaVersion(version);
  return `Unfocus_${version}_linux_x86_64.update.payload.json`;
}

export function linuxUpdateEnvelopeName(version) {
  requireBetaVersion(version);
  return `Unfocus_${version}_linux_x86_64.update.json`;
}

function exactKeys(value, expected, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${label} must be an object`);
  }
  const actual = Object.keys(value);
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    throw new Error(`${label} keys must be exactly: ${expected.join(", ")}`);
  }
}

function canonicalJson(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

function parseCanonicalJson(text, label, maximumBytes) {
  if (typeof text !== "string") throw new Error(`${label} must be UTF-8 text`);
  if (Buffer.byteLength(text) > maximumBytes) {
    throw new Error(`${label} exceeds ${maximumBytes} bytes`);
  }
  let value;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new Error(`${label} is not valid JSON: ${error.message}`);
  }
  if (canonicalJson(value) !== text) {
    throw new Error(`${label} is not in canonical two-space JSON form with one trailing LF`);
  }
  return value;
}

function decodeCanonicalBase64(encoded, label, maximumDecodedBytes = MAX_SIGNATURE_BYTES) {
  if (typeof encoded !== "string" || encoded.length === 0 || !CANONICAL_BASE64.test(encoded)) {
    throw new Error(`${label} must be canonical standard base64`);
  }
  const decoded = Buffer.from(encoded, "base64");
  if (decoded.length > maximumDecodedBytes) {
    throw new Error(`${label} decodes to more than ${maximumDecodedBytes} bytes`);
  }
  if (decoded.toString("base64") !== encoded) {
    throw new Error(`${label} is not canonical standard base64`);
  }
  return decoded;
}

function decodeUtf8(bytes, label) {
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new Error(`${label} does not decode as UTF-8`);
  }
}

export function validateTauriSignatureEncoding(text, expectedFilename) {
  if (typeof text !== "string") throw new Error(`${expectedFilename}.sig must be text`);
  const encoded = text.endsWith("\n") ? text.slice(0, -1) : text;
  if (encoded !== encoded.trim() || encoded.includes("\n") || Buffer.byteLength(encoded) > MAX_SIGNATURE_BYTES) {
    throw new Error(`${expectedFilename}.sig must contain one trimmed base64 value`);
  }

  const inner = decodeUtf8(decodeCanonicalBase64(encoded, `${expectedFilename}.sig`), `${expectedFilename}.sig`);
  if (!inner.endsWith("\n") || inner.includes("\r")) {
    throw new Error(`${expectedFilename}.sig decoded minisign text must end with one LF and contain no CR`);
  }
  const lines = inner.slice(0, -1).split("\n");
  if (lines.length !== 4 || lines[0] !== TAURI_UNTRUSTED_COMMENT) {
    throw new Error(`${expectedFilename}.sig is not a Tauri updater signature`);
  }

  const signaturePacket = decodeCanonicalBase64(lines[1], `${expectedFilename}.sig packet`, 74);
  if (signaturePacket.length !== 74 || signaturePacket.subarray(0, 2).toString("ascii") !== "ED") {
    throw new Error(`${expectedFilename}.sig must use a prehashed minisign signature packet`);
  }

  const escapedFilename = expectedFilename.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  if (!new RegExp(`^trusted comment: timestamp:(?:0|[1-9]\\d*)\\tfile:${escapedFilename}$`).test(lines[2])) {
    throw new Error(`${expectedFilename}.sig trusted comment names the wrong file`);
  }
  const globalSignature = decodeCanonicalBase64(lines[3], `${expectedFilename}.sig global signature`, 64);
  if (globalSignature.length !== 64) {
    throw new Error(`${expectedFilename}.sig has an invalid global signature packet`);
  }

  return {
    encoded,
    keyId: signaturePacket.subarray(2, 10).toString("hex"),
  };
}

function validateHash(value, label) {
  if (typeof value !== "string" || !SHA256.test(value)) {
    throw new Error(`${label} must be 64 lowercase hexadecimal characters`);
  }
}

function validateSize(value, label, maximum = Number.MAX_SAFE_INTEGER) {
  if (!Number.isSafeInteger(value) || value <= 0 || value > maximum) {
    throw new Error(`${label} must be a positive safe integer no greater than ${maximum}`);
  }
}

function expectedDownloadUrl(version, filename) {
  return `https://github.com/${UPDATE_REPOSITORY}/releases/download/v${version}/${filename}`;
}

function validateTargetMetadata(metadata, expected, label) {
  exactKeys(metadata, expected.keys, label);
  if (metadata.url !== expected.url) {
    throw new Error(`${label}.url must be exactly ${expected.url}`);
  }
  validateSize(metadata.sizeBytes, `${label}.sizeBytes`, expected.maximumSize);
  validateHash(metadata.sha256, `${label}.sha256`);
  if (expected.innerExecutable) {
    validateHash(metadata.innerExecutableSha256, `${label}.innerExecutableSha256`);
  }
  return validateTauriSignatureEncoding(metadata.signature, expected.filename).keyId;
}

export function parseUpdatePayload(text, expectedVersion) {
  const payload = parseCanonicalJson(text, "Linux update payload", MAX_ENVELOPE_BYTES);
  exactKeys(payload, ["schemaVersion", "channel", "version", "platforms"], "Linux update payload");
  if (payload.schemaVersion !== UPDATE_SCHEMA_VERSION) {
    throw new Error(`Linux update payload schemaVersion must be ${UPDATE_SCHEMA_VERSION}`);
  }
  if (payload.channel !== UPDATE_CHANNEL) throw new Error(`Linux update payload channel must be ${UPDATE_CHANNEL}`);
  requireBetaVersion(payload.version);
  if (expectedVersion !== undefined && payload.version !== expectedVersion) {
    throw new Error(`Linux update payload version is ${payload.version}, expected ${expectedVersion}`);
  }
  exactKeys(payload.platforms, LINUX_UPDATE_TARGETS, "Linux update payload platforms");

  const names = linuxUpdatePackageNames(payload.version);
  const expected = {
    "linux-x86_64-appimage": {
      filename: names.appimage,
      url: expectedDownloadUrl(payload.version, names.appimage),
      maximumSize: MAX_PACKAGE_BYTES,
      innerExecutable: true,
      keys: ["url", "sizeBytes", "sha256", "innerExecutableSha256", "signature"],
    },
    "linux-x86_64-deb": {
      filename: names.deb,
      url: expectedDownloadUrl(payload.version, names.deb),
      maximumSize: MAX_PACKAGE_BYTES,
      innerExecutable: false,
      keys: ["url", "sizeBytes", "sha256", "signature"],
    },
    "linux-x86_64-rpm": {
      filename: names.rpm,
      url: expectedDownloadUrl(payload.version, names.rpm),
      maximumSize: MAX_PACKAGE_BYTES,
      innerExecutable: false,
      keys: ["url", "sizeBytes", "sha256", "signature"],
    },
  };

  const keyIds = LINUX_UPDATE_TARGETS.map((target) =>
    validateTargetMetadata(payload.platforms[target], expected[target], `Linux update payload ${target}`),
  );
  if (new Set(keyIds).size !== 1) {
    throw new Error("Linux update package signatures do not use one updater key ID");
  }
  return { payload, packageKeyId: keyIds[0] };
}

export function createUpdateEnvelope(payloadText, payloadSignatureText) {
  const { payload, packageKeyId } = parseUpdatePayload(payloadText);
  const payloadFilename = linuxUpdatePayloadName(payload.version);
  const payloadSignature = validateTauriSignatureEncoding(payloadSignatureText, payloadFilename);
  if (payloadSignature.keyId !== packageKeyId) {
    throw new Error("Linux update payload and package signatures do not use one updater key ID");
  }
  return canonicalJson({
    schemaVersion: UPDATE_SCHEMA_VERSION,
    payload: Buffer.from(payloadText, "utf8").toString("base64"),
    signature: payloadSignature.encoded,
  });
}

export function parseUpdateEnvelopeStructure(text, expectedVersion) {
  const envelope = parseCanonicalJson(text, "Linux update envelope", MAX_ENVELOPE_BYTES);
  exactKeys(envelope, ["schemaVersion", "payload", "signature"], "Linux update envelope");
  if (envelope.schemaVersion !== UPDATE_SCHEMA_VERSION) {
    throw new Error(`Linux update envelope schemaVersion must be ${UPDATE_SCHEMA_VERSION}`);
  }
  const payloadText = decodeUtf8(
    decodeCanonicalBase64(envelope.payload, "Linux update envelope payload", MAX_ENVELOPE_BYTES),
    "Linux update envelope payload",
  );
  const parsed = parseUpdatePayload(payloadText, expectedVersion);
  const payloadFilename = linuxUpdatePayloadName(parsed.payload.version);
  const payloadSignature = validateTauriSignatureEncoding(envelope.signature, payloadFilename);
  if (payloadSignature.keyId !== parsed.packageKeyId) {
    throw new Error("Linux update envelope and package signatures do not use one updater key ID");
  }
  return { ...parsed, payloadText, envelope };
}

async function sha256File(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

function requireRegularFile(path, label) {
  if (!existsSync(path)) throw new Error(`${label} is missing: ${path}`);
  const stat = lstatSync(path);
  if (!stat.isFile()) throw new Error(`${label} must be a regular non-symlink file: ${path}`);
  validateSize(stat.size, `${label} size`);
  return stat;
}

async function packageMetadata(directory, filename, maximumSize, innerExecutableSha256) {
  const packagePath = join(directory, filename);
  const signaturePath = `${packagePath}.sig`;
  const stat = requireRegularFile(packagePath, filename);
  validateSize(stat.size, `${filename} size`, maximumSize);
  requireRegularFile(signaturePath, `${filename}.sig`);
  const signature = validateTauriSignatureEncoding(readFileSync(signaturePath, "utf8"), filename).encoded;
  const metadata = {
    url: expectedDownloadUrl(extractVersionFromFilename(filename), filename),
    sizeBytes: stat.size,
    sha256: await sha256File(packagePath),
  };
  if (innerExecutableSha256 !== undefined) metadata.innerExecutableSha256 = innerExecutableSha256;
  metadata.signature = signature;
  return metadata;
}

function extractVersionFromFilename(filename) {
  const match = /^(?:Unfocus_|Unfocus-)(.+?)(?:_amd64\.(?:AppImage|deb)|-1\.x86_64\.rpm)$/.exec(filename);
  if (!match) throw new Error(`cannot extract canonical version from ${filename}`);
  return requireBetaVersion(match[1]);
}

export async function createUpdatePayloadFromDirectory(directory, version, innerExecutableSha256) {
  const source = resolve(directory);
  requireBetaVersion(version);
  validateHash(innerExecutableSha256, "AppImage inner executable SHA-256");
  const names = linuxUpdatePackageNames(version);
  const platforms = {
    "linux-x86_64-appimage": await packageMetadata(
      source,
      names.appimage,
      MAX_PACKAGE_BYTES,
      innerExecutableSha256,
    ),
    "linux-x86_64-deb": await packageMetadata(source, names.deb, MAX_PACKAGE_BYTES),
    "linux-x86_64-rpm": await packageMetadata(source, names.rpm, MAX_PACKAGE_BYTES),
  };
  const payloadText = canonicalJson({
    schemaVersion: UPDATE_SCHEMA_VERSION,
    channel: UPDATE_CHANNEL,
    version,
    platforms,
  });
  parseUpdatePayload(payloadText, version);
  return payloadText;
}

function writeNew(path, text) {
  const output = resolve(path);
  if (existsSync(output)) throw new Error(`refusing to overwrite existing output: ${output}`);
  writeFileSync(output, text, { flag: "wx" });
}

function usage() {
  console.error(
    "usage: linux-update-envelope.js payload <release-assets> <beta-version> <inner-executable-sha256> <output.payload.json> | envelope <payload.json> <payload.json.sig> <output.update.json> | validate-structure <update.json> [beta-version]",
  );
}

async function main() {
  const [action, ...args] = process.argv.slice(2);
  if (action === "payload" && args.length === 4) {
    const [directory, version, innerExecutableSha256, output] = args;
    const text = await createUpdatePayloadFromDirectory(directory, version, innerExecutableSha256);
    if (basename(output) !== linuxUpdatePayloadName(version)) {
      throw new Error(`payload output must be named ${linuxUpdatePayloadName(version)}`);
    }
    writeNew(output, text);
    console.log(`wrote canonical Linux update payload ${resolve(output)}`);
  } else if (action === "envelope" && args.length === 3) {
    const [payloadPath, signaturePath, output] = args;
    const payloadText = readFileSync(resolve(payloadPath), "utf8");
    const { payload } = parseUpdatePayload(payloadText);
    if (basename(payloadPath) !== linuxUpdatePayloadName(payload.version)) {
      throw new Error(`payload input must be named ${linuxUpdatePayloadName(payload.version)}`);
    }
    if (resolve(signaturePath) !== `${resolve(payloadPath)}.sig`) {
      throw new Error("payload signature must be the adjacent .sig file generated by the Tauri signer");
    }
    if (basename(output) !== linuxUpdateEnvelopeName(payload.version)) {
      throw new Error(`envelope output must be named ${linuxUpdateEnvelopeName(payload.version)}`);
    }
    const text = createUpdateEnvelope(payloadText, readFileSync(resolve(signaturePath), "utf8"));
    writeNew(output, text);
    console.log(`wrote structurally validated Linux update envelope ${resolve(output)}`);
  } else if (action === "validate-structure" && (args.length === 1 || args.length === 2)) {
    const [path, expectedVersion] = args;
    const parsed = parseUpdateEnvelopeStructure(readFileSync(resolve(path), "utf8"), expectedVersion);
    console.log(
      `validated Linux update envelope structure for ${parsed.payload.version}; cryptographic verification remains required`,
    );
  } else {
    usage();
    process.exit(2);
  }
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
