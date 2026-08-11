#!/usr/bin/env bun

import { createHash } from "node:crypto";
import {
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { basename, join, resolve } from "node:path";
import { semverToDebianVersion, verifyDebianPackage } from "./debian-package.js";

export function expectedReleaseFilenames(version) {
  semverToDebianVersion(version);
  return [
    "THIRD_PARTY_NOTICES.txt",
    `Unfocus-${version}-1.x86_64.rpm`,
    `Unfocus_${version}_aarch64.dmg`,
    `Unfocus_${version}_amd64.AppImage`,
    `Unfocus_${version}_amd64.deb`,
    `Unfocus_${version}_x64-setup.exe`,
    `Unfocus_${version}_x64.dmg`,
    `Unfocus_${version}_x64_en-US.msi`,
    "unfocus.cdx.json",
  ].sort();
}

function regularFiles(root, current = root, out = []) {
  for (const entry of readdirSync(current, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
    const path = join(current, entry.name);
    if (entry.isDirectory()) {
      regularFiles(root, path, out);
    } else if (entry.isFile()) {
      out.push(path);
    } else {
      throw new Error(`release input must contain only regular files and directories: ${path}`);
    }
  }
  return out;
}

function sha256(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

export function verifyChecksums(directory, expectedNames) {
  const checksumPath = join(directory, "SHA256SUMS");
  const lines = readFileSync(checksumPath, "utf8").trimEnd().split("\n");
  if (lines.length !== expectedNames.length) {
    throw new Error(`SHA256SUMS contains ${lines.length} entries, expected ${expectedNames.length}`);
  }

  const seen = new Set();
  for (const line of lines) {
    const match = /^([0-9a-f]{64})  ([^\r\n]+)$/.exec(line);
    if (!match) throw new Error(`malformed SHA256SUMS line: ${line}`);
    const [, expectedHash, name] = match;
    if (seen.has(name)) throw new Error(`duplicate SHA256SUMS entry: ${name}`);
    if (!expectedNames.includes(name)) throw new Error(`unexpected SHA256SUMS entry: ${name}`);
    seen.add(name);
    const path = join(directory, name);
    if (!existsSync(path) || !lstatSync(path).isFile()) throw new Error(`checksummed asset is missing: ${name}`);
    const actualHash = sha256(path);
    if (actualHash !== expectedHash) {
      throw new Error(`${name} checksum is ${actualHash}, expected ${expectedHash}`);
    }
  }
  for (const name of expectedNames) {
    if (!seen.has(name)) throw new Error(`SHA256SUMS is missing ${name}`);
  }
}

export function assembleReleaseArtifacts(inputDirectory, outputDirectory, version) {
  const input = resolve(inputDirectory);
  const output = resolve(outputDirectory);
  if (!existsSync(input) || !lstatSync(input).isDirectory()) {
    throw new Error(`release download directory is missing: ${input}`);
  }
  if (existsSync(output) && readdirSync(output).length !== 0) {
    throw new Error(`release assembly directory is not empty: ${output}`);
  }

  const expected = expectedReleaseFilenames(version);
  const byName = new Map();
  for (const path of regularFiles(input)) {
    const name = basename(path);
    if (byName.has(name)) throw new Error(`two build jobs produced an asset named ${name}`);
    byName.set(name, path);
  }

  const actual = [...byName.keys()].sort();
  const missing = expected.filter((name) => !byName.has(name));
  const unexpected = actual.filter((name) => !expected.includes(name));
  if (missing.length || unexpected.length) {
    const details = [];
    if (missing.length) details.push(`missing: ${missing.join(", ")}`);
    if (unexpected.length) details.push(`unexpected: ${unexpected.join(", ")}`);
    throw new Error(`release package inventory does not match (${details.join("; ")})`);
  }

  mkdirSync(output, { recursive: true });
  for (const name of expected) copyFileSync(byName.get(name), join(output, name));

  verifyDebianPackage(join(output, `Unfocus_${version}_amd64.deb`), version);
  const checksumText = expected.map((name) => `${sha256(join(output, name))}  ${name}`).join("\n") + "\n";
  writeFileSync(join(output, "SHA256SUMS"), checksumText);
  verifyChecksums(output, expected);
  return [...expected, "SHA256SUMS"].sort();
}

function main() {
  const [input, output, suppliedVersion] = process.argv.slice(2);
  if (!input || !output || process.argv.length > 5) {
    console.error("usage: assemble-release-artifacts.js <downloads> <output> [canonical-version]");
    process.exit(2);
  }
  const version = suppliedVersion ?? JSON.parse(readFileSync(resolve("package.json"), "utf8")).version;
  for (const name of assembleReleaseArtifacts(input, output, version)) console.log(name);
}

if (import.meta.main) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
