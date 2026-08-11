#!/usr/bin/env bun

import { execFileSync } from "node:child_process";
import {
  chmodSync,
  closeSync,
  existsSync,
  fsyncSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  readdirSync,
  readlinkSync,
  renameSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
import { tmpdir } from "node:os";

const CANONICAL_VERSION =
  /^(?<core>(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*))(?:-(?<channel>alpha|beta|rc)\.(?<counter>0|[1-9]\d*))?$/;
const CONTROL_VERSION = /^Version:\s*([^\r\n]+)\s*$/gm;
const DEFAULT_REPOSITORY = "abhiksark/unfocus";

function command(binary, args, options = {}) {
  try {
    return execFileSync(binary, args, {
      encoding: options.encoding ?? "utf8",
      maxBuffer: 64 * 1024 * 1024,
      ...options,
    });
  } catch (error) {
    const stderr = typeof error.stderr === "string" ? error.stderr.trim() : "";
    const detail = stderr ? `: ${stderr}` : "";
    throw new Error(`${binary} ${args.join(" ")} failed${detail}`);
  }
}

export function semverToDebianVersion(version) {
  if (typeof version !== "string") throw new Error("canonical version must be a string");
  const match = CANONICAL_VERSION.exec(version);
  if (!match) {
    throw new Error(
      `unsupported canonical version ${version}: expected X.Y.Z or X.Y.Z-(alpha|beta|rc).N without build metadata or leading zeroes`,
    );
  }

  const { core, channel, counter } = match.groups;
  return channel ? `${core}~${channel}.${counter}-1` : `${core}-1`;
}

export function readDebianField(packagePath, field) {
  const value = command("dpkg-deb", ["--field", resolve(packagePath), field]).trim();
  if (!value) throw new Error(`${basename(packagePath)} has no Debian ${field} field`);
  return value;
}

function hashFile(path) {
  return createHash("sha256").update(readFileSync(path)).digest("hex");
}

function walkTree(root, current = root, out = new Map()) {
  const entries = readdirSync(current, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name));
  for (const entry of entries) {
    const path = join(current, entry.name);
    const name = relative(root, path).split(sep).join("/");
    const stat = lstatSync(path);
    const mode = stat.mode & 0o7777;
    if (entry.isDirectory()) {
      out.set(name, { type: "directory", mode });
      walkTree(root, path, out);
    } else if (entry.isFile()) {
      out.set(name, { type: "file", mode, size: stat.size, sha256: hashFile(path) });
    } else if (entry.isSymbolicLink()) {
      out.set(name, { type: "symlink", mode, target: readlinkSync(path) });
    } else {
      throw new Error(`unsupported package entry type: ${name}`);
    }
  }
  return out;
}

function comparableControl(text) {
  const matches = [...text.matchAll(CONTROL_VERSION)];
  if (matches.length !== 1) throw new Error(`Debian control must contain exactly one Version field, found ${matches.length}`);
  CONTROL_VERSION.lastIndex = 0;
  return text.replace(CONTROL_VERSION, "Version: <normalized>");
}

function replaceControlVersion(text, expectedCurrent, next) {
  const matches = [...text.matchAll(CONTROL_VERSION)];
  if (matches.length !== 1) throw new Error(`Debian control must contain exactly one Version field, found ${matches.length}`);
  if (matches[0][1] !== expectedCurrent) {
    throw new Error(`generated Debian Version is ${matches[0][1]}, expected canonical version ${expectedCurrent}`);
  }
  CONTROL_VERSION.lastIndex = 0;
  return text.replace(CONTROL_VERSION, `Version: ${next}`);
}

function extractPackage(packagePath, directory) {
  mkdirSync(directory, { recursive: true });
  command("dpkg-deb", ["--raw-extract", resolve(packagePath), directory]);
}

function verifyPreservedPackage(originalRoot, rebuiltPath, verificationRoot) {
  extractPackage(rebuiltPath, verificationRoot);

  const originalPayload = walkTree(originalRoot);
  const rebuiltPayload = walkTree(verificationRoot);
  for (const snapshot of [originalPayload, rebuiltPayload]) snapshot.delete("DEBIAN/control");
  if (JSON.stringify([...originalPayload]) !== JSON.stringify([...rebuiltPayload])) {
    throw new Error("Debian package payload or non-Version control metadata changed during finalization");
  }

  const originalControl = readFileSync(join(originalRoot, "DEBIAN", "control"), "utf8");
  const rebuiltControl = readFileSync(join(verificationRoot, "DEBIAN", "control"), "utf8");
  if (comparableControl(originalControl) !== comparableControl(rebuiltControl)) {
    throw new Error("Debian control metadata other than Version changed during finalization");
  }
}

export function verifyDebianPackage(packagePath, canonicalVersion, options = {}) {
  const expectedVersion = semverToDebianVersion(canonicalVersion);
  const fields = {
    Package: readDebianField(packagePath, "Package"),
    Version: readDebianField(packagePath, "Version"),
    Architecture: readDebianField(packagePath, "Architecture"),
    Depends: readDebianField(packagePath, "Depends"),
  };

  const expectedPackage = options.packageName ?? "unfocus";
  const expectedArchitecture = options.architecture ?? "amd64";
  if (fields.Package !== expectedPackage) {
    throw new Error(`${basename(packagePath)} contains Package ${fields.Package}, expected ${expectedPackage}`);
  }
  if (fields.Version !== expectedVersion) {
    throw new Error(`${basename(packagePath)} contains Version ${fields.Version}, expected ${expectedVersion}`);
  }
  if (fields.Architecture !== expectedArchitecture) {
    throw new Error(`${basename(packagePath)} contains Architecture ${fields.Architecture}, expected ${expectedArchitecture}`);
  }
  return fields;
}

export function finalizeDebianPackage(packagePath, canonicalVersion, options = {}) {
  const source = resolve(packagePath);
  if (!existsSync(source) || !statSync(source).isFile()) {
    throw new Error(`Debian package is not a file: ${source}`);
  }

  const mappedVersion = semverToDebianVersion(canonicalVersion);
  const originalMode = statSync(source).mode & 0o7777;
  const work = mkdtempSync(join(dirname(source), ".unfocus-deb-"));
  const unpacked = join(work, "root");
  const verification = join(work, "verification");
  const rebuilt = join(work, basename(source));

  try {
    extractPackage(source, unpacked);
    const controlPath = join(unpacked, "DEBIAN", "control");
    const control = readFileSync(controlPath, "utf8");
    writeFileSync(controlPath, replaceControlVersion(control, canonicalVersion, mappedVersion));

    command("dpkg-deb", ["--build", "--root-owner-group", unpacked, rebuilt]);
    chmodSync(rebuilt, originalMode);
    verifyPreservedPackage(unpacked, rebuilt, verification);
    const fields = verifyDebianPackage(rebuilt, canonicalVersion, options);

    const fd = openSync(rebuilt, "r");
    try {
      fsyncSync(fd);
    } finally {
      closeSync(fd);
    }
    renameSync(rebuilt, source);
    return fields;
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

export function debianVersionIsGreater(candidate, previous) {
  try {
    execFileSync("dpkg", ["--compare-versions", candidate, "gt", previous], { stdio: "ignore" });
    return true;
  } catch (error) {
    if (error?.status === 1) return false;
    const status = error?.status ?? "unknown";
    throw new Error(
      `dpkg --compare-versions ${candidate} gt ${previous} could not run (exit ${status})`,
    );
  }
}

async function githubJson(url, token) {
  const headers = {
    Accept: "application/vnd.github+json",
    "User-Agent": "unfocus-release-qualification",
    "X-GitHub-Api-Version": "2022-11-28",
  };
  if (token) headers.Authorization = `Bearer ${token}`;
  const response = await fetch(url, { headers });
  if (!response.ok) throw new Error(`GitHub API ${response.status} for ${url}`);
  return response.json();
}

async function downloadAsset(asset, destination, token) {
  const headers = { "User-Agent": "unfocus-release-qualification" };
  if (token) headers.Authorization = `Bearer ${token}`;
  const response = await fetch(asset.browser_download_url, { headers, redirect: "follow" });
  if (!response.ok) throw new Error(`download failed for ${asset.name}: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  writeFileSync(destination, bytes);
  if (typeof asset.digest === "string" && asset.digest.startsWith("sha256:")) {
    const actual = createHash("sha256").update(bytes).digest("hex");
    const expected = asset.digest.slice("sha256:".length);
    if (actual !== expected) throw new Error(`${asset.name} digest is ${actual}, expected ${expected}`);
  }
}

export async function qualifyPublishedDebianVersions(packagePath, canonicalVersion, options = {}) {
  const repository = options.repository ?? process.env.GITHUB_REPOSITORY ?? DEFAULT_REPOSITORY;
  if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repository)) {
    throw new Error(`invalid GitHub repository ${repository}`);
  }
  const token = options.token ?? process.env.GH_TOKEN ?? process.env.GITHUB_TOKEN;
  const candidate = verifyDebianPackage(packagePath, canonicalVersion, options).Version;
  const assets = [];

  for (let page = 1; ; page += 1) {
    const releases = await githubJson(
      `https://api.github.com/repos/${repository}/releases?per_page=100&page=${page}`,
      token,
    );
    if (!Array.isArray(releases)) throw new Error("GitHub releases response was not an array");
    for (const release of releases) {
      if (release.draft || !release.published_at || !Array.isArray(release.assets)) continue;
      for (const asset of release.assets) {
        if (typeof asset.name === "string" && asset.name.toLowerCase().endsWith(".deb")) {
          assets.push({ ...asset, tag: release.tag_name });
        }
      }
    }
    if (releases.length < 100) break;
  }

  const work = mkdtempSync(join(tmpdir(), "unfocus-published-debs-"));
  const compared = [];
  try {
    for (const [index, asset] of assets.entries()) {
      const destination = join(work, `${index}-${basename(asset.name)}`);
      await downloadAsset(asset, destination, token);
      const previous = readDebianField(destination, "Version");
      if (!debianVersionIsGreater(candidate, previous)) {
        throw new Error(
          `candidate Debian Version ${candidate} must be greater than published ${previous} from ${asset.tag}/${asset.name}`,
        );
      }
      compared.push({ tag: asset.tag, name: asset.name, version: previous });
    }
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
  return { candidate, compared };
}

function usage() {
  console.error(
    "usage: debian-package.js map <version> | finalize <package.deb> <version> | verify <package.deb> <version> | qualify-published <package.deb> <version> [owner/repo]",
  );
}

async function main() {
  const [action, ...args] = process.argv.slice(2);
  if (action === "map" && args.length === 1) {
    console.log(semverToDebianVersion(args[0]));
  } else if (action === "finalize" && args.length === 2) {
    const fields = finalizeDebianPackage(args[0], args[1]);
    console.log(`finalized ${basename(args[0])}: ${fields.Package} ${fields.Version} ${fields.Architecture}`);
  } else if (action === "verify" && args.length === 2) {
    const fields = verifyDebianPackage(args[0], args[1]);
    console.log(`verified ${basename(args[0])}: ${fields.Package} ${fields.Version} ${fields.Architecture}`);
  } else if (action === "qualify-published" && (args.length === 2 || args.length === 3)) {
    const result = await qualifyPublishedDebianVersions(args[0], args[1], { repository: args[2] });
    for (const item of result.compared) {
      console.log(`ordered after ${item.tag}/${item.name} (${item.version})`);
    }
    console.log(`qualified candidate Debian Version ${result.candidate} against ${result.compared.length} published package(s)`);
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
