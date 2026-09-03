#!/usr/bin/env bun

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
import { join, resolve } from "node:path";
import { tmpdir } from "node:os";
import { expectedReleaseFilenames, verifyChecksums } from "./assemble-release-artifacts.js";
import {
  verifyEvidenceFiles,
  verifyPayloadMatchesEvidence,
  verifySignatures,
} from "./finalize-release-artifacts.js";
import { parsePackageEvidence } from "./inspect-linux-packages.js";
import {
  linuxUpdateEnvelopeName,
  linuxUpdatePayloadName,
  parseUpdateEnvelopeStructure,
  releaseChannel,
} from "./linux-update-envelope.js";

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

async function sha256File(path) {
  const hash = createHash("sha256");
  for await (const chunk of Bun.file(path).stream()) hash.update(chunk);
  return hash.digest("hex");
}

export function expectedFinalReleaseFilenames(version) {
  const baseFiles = expectedReleaseFilenames(version);
  if (releaseChannel(version) !== "beta") return [...baseFiles, "SHA256SUMS"].sort();
  return [
    ...baseFiles,
    `Unfocus_${version}_amd64.AppImage.sig`,
    `Unfocus_${version}_amd64.deb.sig`,
    `Unfocus-${version}-1.x86_64.rpm.sig`,
    linuxUpdateEnvelopeName(version),
    "SHA256SUMS",
  ].sort();
}

async function compareBaseAssets(finalDirectory, candidateDirectory, names) {
  for (const name of names) {
    const finalPath = join(finalDirectory, name);
    const candidatePath = join(candidateDirectory, name);
    const finalStat = lstatSync(finalPath);
    const candidateStat = lstatSync(candidatePath);
    if (
      !finalStat.isFile() ||
      !candidateStat.isFile() ||
      finalStat.size !== candidateStat.size ||
      (await sha256File(finalPath)) !== (await sha256File(candidatePath))
    ) {
      throw new Error(`final release base asset differs from validated candidate: ${name}`);
    }
  }
}

export async function verifyFinalRelease(
  finalDirectory,
  candidateDirectory,
  evidencePath,
  publicKeyPath,
  version,
  options = {},
) {
  const final = resolve(finalDirectory);
  const candidate = resolve(candidateDirectory);
  const evidenceFile = resolve(evidencePath);
  const baseFiles = expectedReleaseFilenames(version);
  exactInventory(candidate, [...baseFiles, "SHA256SUMS"], "validated release candidate");
  verifyChecksums(candidate, baseFiles);
  if (!existsSync(evidenceFile) || !lstatSync(evidenceFile).isFile()) {
    throw new Error(`Linux package evidence is missing: ${evidenceFile}`);
  }
  const evidence = parsePackageEvidence(readFileSync(evidenceFile, "utf8"), version);
  await verifyEvidenceFiles(candidate, evidence);

  const channel = releaseChannel(version);
  if (channel !== "beta") {
    exactInventory(final, [...baseFiles, "SHA256SUMS"], "final release");
    verifyChecksums(final, baseFiles);
    await compareBaseAssets(final, candidate, [...baseFiles, "SHA256SUMS"]);
    return [...baseFiles, "SHA256SUMS"].sort();
  }

  const packageFiles = Object.values(evidence.packages).map((package_) => package_.filename);
  const packageSignatures = packageFiles.map((name) => `${name}.sig`);
  const envelopeName = linuxUpdateEnvelopeName(version);
  const nonChecksumFiles = [...baseFiles, ...packageSignatures, envelopeName].sort();
  exactInventory(final, [...nonChecksumFiles, "SHA256SUMS"], "final beta release");

  const envelopePath = join(final, envelopeName);
  const envelopeStat = lstatSync(envelopePath);
  if (envelopeStat.size <= 0 || envelopeStat.size > 64 * 1024) {
    throw new Error("final Linux update envelope has an invalid size");
  }
  const parsed = parseUpdateEnvelopeStructure(readFileSync(envelopePath, "utf8"), version);
  verifyPayloadMatchesEvidence(parsed.payload, evidence, final);
  const temporary = mkdtempSync(join(tmpdir(), "unfocus-final-release-verification-"));
  try {
    const payloadPath = join(temporary, linuxUpdatePayloadName(version));
    writeFileSync(payloadPath, parsed.payloadText, { flag: "wx" });
    writeFileSync(`${payloadPath}.sig`, parsed.envelope.signature, { flag: "wx" });
    const signatureVerifier = options.verifySignatures ?? verifySignatures;
    signatureVerifier(
      resolve(publicKeyPath),
      [...packageFiles.map((name) => join(final, name)), payloadPath],
    );
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
  await compareBaseAssets(final, candidate, baseFiles);
  verifyChecksums(final, nonChecksumFiles);
  return [...nonChecksumFiles, "SHA256SUMS"].sort();
}

function usage() {
  console.error(
    "usage: verify-final-release.js <final-release> <validated-candidate> <package-evidence.json> <public-key-or-dash> <canonical-version>",
  );
}

async function main() {
  const [final, candidate, evidence, publicKey, version, ...extra] = process.argv.slice(2);
  if (!final || !candidate || !evidence || !publicKey || !version || extra.length !== 0) {
    usage();
    process.exit(2);
  }
  const channel = releaseChannel(version);
  if (channel === "beta" && publicKey === "-") throw new Error("beta verification requires the updater public key");
  const names = await verifyFinalRelease(final, candidate, evidence, publicKey, version);
  console.log(`verified ${channel} release with ${names.length} immutable assets`);
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
