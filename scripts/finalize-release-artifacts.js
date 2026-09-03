#!/usr/bin/env bun

import { createHash } from "node:crypto";
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  createReadStream,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expectedReleaseFilenames, verifyChecksums } from "./assemble-release-artifacts.js";
import { parsePackageEvidence } from "./inspect-linux-packages.js";
import {
  createUpdateEnvelope,
  linuxUpdateEnvelopeName,
  linuxUpdatePayloadName,
  parseUpdateEnvelopeStructure,
  parseUpdatePayload,
  releaseChannel,
} from "./linux-update-envelope.js";

const REPOSITORY_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const CARGO_MANIFEST = join(REPOSITORY_ROOT, "src-tauri", "Cargo.toml");

async function sha256File(path) {
  const hash = createHash("sha256");
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

function regularFile(path, label, maximumBytes = Number.MAX_SAFE_INTEGER) {
  if (!existsSync(path)) throw new Error(`${label} is missing: ${path}`);
  const stat = lstatSync(path);
  if (!stat.isFile()) throw new Error(`${label} must be a regular non-symlink file: ${path}`);
  if (stat.size <= 0 || stat.size > maximumBytes) throw new Error(`${label} has an invalid size`);
  return stat;
}

function exactInventory(directory, expected, label) {
  if (!existsSync(directory) || !lstatSync(directory).isDirectory()) {
    throw new Error(`${label} directory is missing: ${directory}`);
  }
  const actual = readdirSync(directory, { withFileTypes: true }).map((entry) => {
    if (!entry.isFile()) throw new Error(`${label} contains non-file entry ${entry.name}`);
    return entry.name;
  });
  actual.sort();
  const sortedExpected = [...expected].sort();
  if (actual.length !== sortedExpected.length || actual.some((name, index) => name !== sortedExpected[index])) {
    throw new Error(`${label} inventory must be exactly: ${sortedExpected.join(", ")}`);
  }
}

function requireEmptyDestination(path) {
  if (existsSync(path)) {
    if (!lstatSync(path).isDirectory() || readdirSync(path).length !== 0) {
      throw new Error(`release output directory is not empty: ${path}`);
    }
    return;
  }
  const parent = dirname(path);
  if (!existsSync(parent) || !lstatSync(parent).isDirectory()) {
    throw new Error(`release output parent directory is missing: ${parent}`);
  }
  mkdirSync(path);
}

export async function verifyEvidenceFiles(directory, evidence) {
  const checksumPath = join(directory, "SHA256SUMS");
  if ((await sha256File(checksumPath)) !== evidence.candidateChecksumsSha256) {
    throw new Error("Linux package evidence does not match staged base SHA256SUMS bytes");
  }
  for (const package_ of Object.values(evidence.packages)) {
    const path = join(directory, package_.filename);
    const stat = regularFile(path, package_.filename);
    if (stat.size !== package_.sizeBytes || (await sha256File(path)) !== package_.sha256) {
      throw new Error(`Linux package evidence does not match staged ${package_.filename}`);
    }
  }
}

export function verifyPayloadMatchesEvidence(payload, evidence, staging) {
  const targets = [
    ["linux-x86_64-appimage", evidence.packages.appimage, true],
    ["linux-x86_64-deb", evidence.packages.deb, false],
    ["linux-x86_64-rpm", evidence.packages.rpm, false],
  ];
  for (const [target, package_, hasInnerDigest] of targets) {
    const metadata = payload.platforms[target];
    if (metadata.sizeBytes !== package_.sizeBytes || metadata.sha256 !== package_.sha256) {
      throw new Error(`Linux update payload does not match package evidence for ${target}`);
    }
    if (hasInnerDigest && metadata.innerExecutableSha256 !== package_.innerExecutableSha256) {
      throw new Error("Linux update payload inner executable digest does not match package evidence");
    }
    const signaturePath = join(staging, `${package_.filename}.sig`);
    regularFile(signaturePath, `${package_.filename}.sig`, 8 * 1024);
    const signature = readFileSync(signaturePath, "utf8").trimEnd();
    if (metadata.signature !== signature) {
      throw new Error(`Linux update payload signature does not match ${package_.filename}.sig`);
    }
  }
}

export function verifySignatures(publicKey, signedFiles) {
  regularFile(publicKey, "updater public key", 8 * 1024);
  const arguments_ = [
    "run",
    "--quiet",
    "--locked",
    "--manifest-path",
    CARGO_MANIFEST,
    "--example",
    "verify-update-signature",
    "--",
    publicKey,
  ];
  for (const path of signedFiles) arguments_.push(`${path}.sig`, path);
  const environment = { ...process.env };
  delete environment.TAURI_SIGNING_PRIVATE_KEY;
  delete environment.TAURI_SIGNING_PRIVATE_KEY_PASSWORD;
  delete environment.TAURI_SIGNING_PRIVATE_KEY_PATH;
  const result = spawnSync("cargo", arguments_, {
    cwd: REPOSITORY_ROOT,
    encoding: "utf8",
    env: environment,
    maxBuffer: 1024 * 1024,
    timeout: 300_000,
  });
  if (result.error) throw new Error(`updater signature verifier failed to start: ${result.error.message}`);
  if (result.signal) throw new Error(`updater signature verifier was terminated by ${result.signal}`);
  if (result.status !== 0) {
    const stderr = result.stderr?.trim();
    throw new Error(`updater signature verification failed${stderr ? `: ${stderr}` : ""}`);
  }
}

async function writeChecksums(directory, names) {
  const sorted = [...names].sort();
  const lines = [];
  for (const name of sorted) lines.push(`${await sha256File(join(directory, name))}  ${name}`);
  writeFileSync(join(directory, "SHA256SUMS"), `${lines.join("\n")}\n`, { flag: "wx" });
  verifyChecksums(directory, sorted);
}

export async function finalizeBaseRelease(candidateDirectory, outputDirectory, version) {
  const channel = releaseChannel(version);
  if (channel === "beta") throw new Error("beta releases require signed Linux update finalization");
  const candidate = resolve(candidateDirectory);
  const output = resolve(outputDirectory);
  const baseFiles = expectedReleaseFilenames(version);
  exactInventory(candidate, [...baseFiles, "SHA256SUMS"], "release candidate");
  verifyChecksums(candidate, baseFiles);
  requireEmptyDestination(output);
  try {
    for (const name of [...baseFiles, "SHA256SUMS"]) copyFileSync(join(candidate, name), join(output, name));
    verifyChecksums(output, baseFiles);
    exactInventory(output, [...baseFiles, "SHA256SUMS"], "final release");
    return [...baseFiles, "SHA256SUMS"].sort();
  } catch (error) {
    rmSync(output, { recursive: true, force: true });
    throw error;
  }
}

export async function finalizeBetaRelease(
  signedStagingDirectory,
  evidencePath,
  publicKeyPath,
  outputDirectory,
  version,
  options = {},
) {
  if (releaseChannel(version) !== "beta") throw new Error("signed Linux update finalization requires a beta version");
  const staging = resolve(signedStagingDirectory);
  const evidenceFile = resolve(evidencePath);
  const publicKey = resolve(publicKeyPath);
  const output = resolve(outputDirectory);
  regularFile(evidenceFile, "Linux package evidence", 64 * 1024);
  const evidence = parsePackageEvidence(readFileSync(evidenceFile, "utf8"), version);
  const baseFiles = expectedReleaseFilenames(version);
  const packageFiles = Object.values(evidence.packages).map((package_) => package_.filename);
  const packageSignatures = packageFiles.map((name) => `${name}.sig`);
  const payloadName = linuxUpdatePayloadName(version);
  const payloadPath = join(staging, payloadName);
  const signedInventory = [...baseFiles, "SHA256SUMS", ...packageSignatures, payloadName, `${payloadName}.sig`];
  exactInventory(staging, signedInventory, "signed release staging");
  verifyChecksums(staging, baseFiles);
  await verifyEvidenceFiles(staging, evidence);

  regularFile(payloadPath, payloadName, 64 * 1024);
  regularFile(`${payloadPath}.sig`, `${payloadName}.sig`, 8 * 1024);
  const payloadText = readFileSync(payloadPath, "utf8");
  const { payload } = parseUpdatePayload(payloadText, version);
  verifyPayloadMatchesEvidence(payload, evidence, staging);
  const signatureVerifier = options.verifySignatures ?? verifySignatures;
  signatureVerifier(publicKey, [...packageFiles.map((name) => join(staging, name)), payloadPath]);
  const envelopeText = createUpdateEnvelope(payloadText, readFileSync(`${payloadPath}.sig`, "utf8"));
  parseUpdateEnvelopeStructure(envelopeText, version);

  const envelopeName = linuxUpdateEnvelopeName(version);
  const finalFiles = [...baseFiles, ...packageSignatures, envelopeName].sort();
  requireEmptyDestination(output);
  try {
    for (const name of [...baseFiles, ...packageSignatures]) {
      copyFileSync(join(staging, name), join(output, name));
    }
    writeFileSync(join(output, envelopeName), envelopeText, { flag: "wx" });
    await writeChecksums(output, finalFiles);
    exactInventory(output, [...finalFiles, "SHA256SUMS"], "final beta release");
    parseUpdateEnvelopeStructure(readFileSync(join(output, envelopeName), "utf8"), version);
    return [...finalFiles, "SHA256SUMS"].sort();
  } catch (error) {
    rmSync(output, { recursive: true, force: true });
    throw error;
  }
}

function usage() {
  console.error(
    "usage: finalize-release-artifacts.js base <candidate> <empty-output> <non-beta-version> | beta <signed-staging> <package-evidence.json> <public-key> <empty-output> <beta-version>",
  );
}

async function main() {
  const [mode, ...arguments_] = process.argv.slice(2);
  let files;
  if (mode === "base" && arguments_.length === 3) {
    files = await finalizeBaseRelease(...arguments_);
  } else if (mode === "beta" && arguments_.length === 5) {
    files = await finalizeBetaRelease(...arguments_);
  } else {
    usage();
    process.exit(2);
  }
  for (const name of files) console.log(name);
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
