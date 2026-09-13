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
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { expectedReleaseFilenames, verifyChecksums } from "./assemble-release-artifacts.js";
import { parsePackageEvidence } from "./inspect-linux-packages.js";
import {
  createUpdatePayloadFromDirectory,
  linuxUpdatePayloadName,
  requireBetaVersion,
  validateTauriSignatureEncoding,
} from "./linux-update-envelope.js";

const MAX_SIGNATURE_BYTES = 8 * 1024;

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

function exactCandidateInventory(directory, version) {
  const expected = [...expectedReleaseFilenames(version), "SHA256SUMS"].sort();
  const actual = readdirSync(directory, { withFileTypes: true }).map((entry) => {
    if (!entry.isFile()) throw new Error(`release candidate contains non-file entry ${entry.name}`);
    return entry.name;
  });
  actual.sort();
  if (actual.length !== expected.length || actual.some((name, index) => name !== expected[index])) {
    throw new Error(`release candidate inventory must be exactly: ${expected.join(", ")}`);
  }
  verifyChecksums(directory, expectedReleaseFilenames(version));
  return expected;
}

async function verifyEvidenceAgainstCandidate(directory, evidence) {
  const checksumPath = join(directory, "SHA256SUMS");
  if ((await sha256File(checksumPath)) !== evidence.candidateChecksumsSha256) {
    throw new Error("package evidence does not match the candidate SHA256SUMS bytes");
  }
  for (const package_ of Object.values(evidence.packages)) {
    const path = join(directory, package_.filename);
    const stat = regularFile(path, package_.filename);
    if (stat.size !== package_.sizeBytes || (await sha256File(path)) !== package_.sha256) {
      throw new Error(`package evidence does not match ${package_.filename}`);
    }
  }
}

function signingEnvironment(privateKey, password) {
  const environment = {
    HOME: process.env.HOME,
    PATH: process.env.PATH,
    TMPDIR: process.env.TMPDIR,
    TAURI_SIGNING_PRIVATE_KEY: privateKey,
    TAURI_SIGNING_PRIVATE_KEY_PASSWORD: password,
  };
  return Object.fromEntries(Object.entries(environment).filter(([, value]) => value !== undefined));
}

function signFile(path, repositoryRoot, environment) {
  const signaturePath = `${path}.sig`;
  if (existsSync(signaturePath)) throw new Error(`refusing to overwrite existing signature: ${signaturePath}`);
  const result = spawnSync(process.execPath, ["run", "tauri", "signer", "sign", path], {
    cwd: repositoryRoot,
    encoding: "utf8",
    env: environment,
    maxBuffer: 1024 * 1024,
    timeout: 120_000,
  });
  if (result.error) throw new Error(`Tauri signer failed to start: ${result.error.message}`);
  if (result.signal) throw new Error(`Tauri signer was terminated by ${result.signal}`);
  if (result.status !== 0) {
    const stderr = result.stderr?.trim();
    throw new Error(`Tauri signer exited with ${result.status}${stderr ? `: ${stderr}` : ""}`);
  }
  regularFile(signaturePath, "generated updater signature", MAX_SIGNATURE_BYTES);
  validateTauriSignatureEncoding(readFileSync(signaturePath, "utf8"), basename(path));
}

function requireEmptyDestination(path) {
  if (existsSync(path)) {
    const stat = lstatSync(path);
    if (!stat.isDirectory() || readdirSync(path).length !== 0) {
      throw new Error(`signed staging directory is not empty: ${path}`);
    }
    return;
  }
  const parent = dirname(path);
  if (!existsSync(parent) || !lstatSync(parent).isDirectory()) {
    throw new Error(`signed staging parent directory is missing: ${parent}`);
  }
  mkdirSync(path);
}

export async function signLinuxUpdate(candidateDirectory, evidencePath, outputDirectory, version, options = {}) {
  requireBetaVersion(version);
  const candidate = resolve(candidateDirectory);
  const evidenceFile = resolve(evidencePath);
  const output = resolve(outputDirectory);
  if (!existsSync(candidate) || !lstatSync(candidate).isDirectory()) {
    throw new Error(`release candidate directory is missing: ${candidate}`);
  }
  regularFile(evidenceFile, "Linux package evidence", 64 * 1024);
  const evidence = parsePackageEvidence(readFileSync(evidenceFile, "utf8"), version);
  const inventory = exactCandidateInventory(candidate, version);
  await verifyEvidenceAgainstCandidate(candidate, evidence);

  const privateKey = options.privateKey ?? process.env.TAURI_SIGNING_PRIVATE_KEY;
  const password = options.password ?? process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD;
  if (!privateKey || !password) throw new Error("Linux beta signing requires the protected updater key and password");
  if (process.env.TAURI_SIGNING_PRIVATE_KEY_PATH) {
    throw new Error("Linux beta signing accepts the protected key value, not a private-key path");
  }
  const repositoryRoot = resolve(fileURLToPath(new URL("..", import.meta.url)));
  const environment = signingEnvironment(privateKey, password);
  const signer = options.signFile ?? signFile;
  delete process.env.TAURI_SIGNING_PRIVATE_KEY;
  delete process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD;

  requireEmptyDestination(output);
  try {
    for (const name of inventory) copyFileSync(join(candidate, name), join(output, name));
    for (const package_ of Object.values(evidence.packages)) {
      signer(join(output, package_.filename), repositoryRoot, environment);
    }
    const payloadName = linuxUpdatePayloadName(version);
    const payloadPath = join(output, payloadName);
    const payload = await createUpdatePayloadFromDirectory(
      output,
      version,
      evidence.packages.appimage.innerExecutableSha256,
    );
    writeFileSync(payloadPath, payload, { flag: "wx" });
    signer(payloadPath, repositoryRoot, environment);
    return [
      ...inventory,
      ...Object.values(evidence.packages).map((package_) => `${package_.filename}.sig`),
      payloadName,
      `${payloadName}.sig`,
    ].sort();
  } catch (error) {
    rmSync(output, { recursive: true, force: true });
    throw error;
  }
}

function usage() {
  console.error(
    "usage: sign-linux-update.js <release-candidate> <package-evidence.json> <empty-signed-staging> <beta-version>",
  );
}

async function main() {
  const [candidate, evidence, output, version, ...extra] = process.argv.slice(2);
  if (!candidate || !evidence || !output || !version || extra.length !== 0) {
    usage();
    process.exit(2);
  }
  const names = await signLinuxUpdate(candidate, evidence, output, version);
  for (const name of names) console.log(name);
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
