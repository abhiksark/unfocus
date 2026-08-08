#!/usr/bin/env bun
// Keeps the four declared versions of Unfocus in sync.
//
//   bun run scripts/version.js check                   verify all four agree
//   bun run scripts/version.js check --expect vX.Y.Z   ...and match a tag
//   bun run scripts/version.js set X.Y.Z               rewrite all four
//
// Sources of truth, in the order a release consumes them:
//   src-tauri/tauri.conf.json  what tauri-action bundles and names artifacts with
//   src-tauri/Cargo.toml       what the Rust crate reports
//   src-tauri/Cargo.lock       must follow Cargo.toml or `--locked` builds fail
//   package.json               what the JS tooling reports

import { existsSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const CRATE = "unfocus";
// The numeric core, which is all a Windows MSI ProductVersion can express.
const CORE = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

// The declared version, which may carry a prerelease label so that artifacts
// are named for the release they belong to. Build metadata stays out: it does
// not order, and nothing downstream reads it.
const VERSION =
  /^(?<core>(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*))(?:-(?<prerelease>[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/;

const FILES = {
  packageJson: join(ROOT, "package.json"),
  tauriConf: join(ROOT, "src-tauri", "tauri.conf.json"),
  cargoToml: join(ROOT, "src-tauri", "Cargo.toml"),
  cargoLock: join(ROOT, "src-tauri", "Cargo.lock"),
};

const LABELS = {
  packageJson: "package.json",
  tauriConf: "src-tauri/tauri.conf.json",
  cargoToml: "src-tauri/Cargo.toml",
  cargoLock: "src-tauri/Cargo.lock",
};

// ---------------------------------------------------------------- extractors

// Top-level "version" key of a 2-space-indented JSON document.
const JSON_VERSION = /^(\s{2}"version"\s*:\s*")([^"]+)("\s*,?)\s*$/m;

function readJsonVersion(text, label) {
  const match = text.match(JSON_VERSION);
  if (!match) throw new Error(`${label}: no top-level "version" key found`);
  return match[2];
}

function writeJsonVersion(text, next, label) {
  if (!JSON_VERSION.test(text)) throw new Error(`${label}: no top-level "version" key found`);
  const out = text.replace(JSON_VERSION, (_, before, __, after) => `${before}${next}${after}`);
  JSON.parse(out); // refuse to emit invalid JSON
  return out;
}

// bundle.windows.wix.version, read by parsing so nesting cannot fool it, and
// written against the wix block alone so the top-level version is untouched.
const WIX_VERSION = /("wix"\s*:\s*\{\s*)"version"(\s*:\s*")([^"]+)(")/;

function readWixVersion(text, label) {
  let parsed;
  try {
    parsed = JSON.parse(text);
  } catch (error) {
    throw new Error(`${label}: not valid JSON: ${error.message}`);
  }
  const version = parsed?.bundle?.windows?.wix?.version;
  if (typeof version !== "string") {
    throw new Error(`${label}: no bundle.windows.wix.version string`);
  }
  return version;
}

function writeWixVersion(text, next, label) {
  if (!WIX_VERSION.test(text)) {
    throw new Error(`${label}: bundle.windows.wix.version must be the first key of the wix block`);
  }
  const out = text.replace(WIX_VERSION, (_, open, mid, __, close) => `${open}"version"${mid}${next}${close}`);
  if (readWixVersion(out, label) !== next) throw new Error(`${label}: wix version did not update`);
  return out;
}

// The `version` line inside the [package] table only — never a dependency's.
function cargoTomlRange(text, label) {
  const start = text.search(/^\[package\]\s*$/m);
  if (start === -1) throw new Error(`${label}: no [package] table found`);
  const after = text.slice(start + 1);
  const rel = after.search(/^\[/m);
  return [start, rel === -1 ? text.length : start + 1 + rel];
}

function readCargoTomlVersion(text, label) {
  const [start, end] = cargoTomlRange(text, label);
  const match = text.slice(start, end).match(/^version\s*=\s*"([^"]+)"\s*$/m);
  if (!match) throw new Error(`${label}: no version in [package]`);
  return match[1];
}

function writeCargoTomlVersion(text, next, label) {
  const [start, end] = cargoTomlRange(text, label);
  const block = text.slice(start, end);
  if (!/^version\s*=\s*"([^"]+)"\s*$/m.test(block)) throw new Error(`${label}: no version in [package]`);
  const patched = block.replace(/^version\s*=\s*"([^"]+)"\s*$/m, `version = "${next}"`);
  return text.slice(0, start) + patched + text.slice(end);
}

// The [[package]] block whose name is exactly `unfocus`, out of hundreds.
function cargoLockRange(text, label) {
  const re = /^\[\[package\]\]\s*$/gm;
  let match;
  while ((match = re.exec(text)) !== null) {
    const start = match.index;
    const rest = text.slice(start + match[0].length);
    const relEnd = rest.search(/^\[\[package\]\]\s*$/m);
    const end = relEnd === -1 ? text.length : start + match[0].length + relEnd;
    const block = text.slice(start, end);
    if (new RegExp(`^name\\s*=\\s*"${CRATE}"\\s*$`, "m").test(block)) return [start, end];
  }
  throw new Error(`${label}: no [[package]] block for name = "${CRATE}"`);
}

function readCargoLockVersion(text, label) {
  const [start, end] = cargoLockRange(text, label);
  const match = text.slice(start, end).match(/^version\s*=\s*"([^"]+)"\s*$/m);
  if (!match) throw new Error(`${label}: no version in the ${CRATE} package block`);
  return match[1];
}

function writeCargoLockVersion(text, next, label) {
  const [start, end] = cargoLockRange(text, label);
  const block = text.slice(start, end);
  if (!/^version\s*=\s*"([^"]+)"\s*$/m.test(block)) throw new Error(`${label}: no version in the ${CRATE} block`);
  const patched = block.replace(/^version\s*=\s*"([^"]+)"\s*$/m, `version = "${next}"`);
  return text.slice(0, start) + patched + text.slice(end);
}

const READERS = {
  packageJson: readJsonVersion,
  tauriConf: readJsonVersion,
  cargoToml: readCargoTomlVersion,
  cargoLock: readCargoLockVersion,
};

const WRITERS = {
  packageJson: writeJsonVersion,
  tauriConf: writeJsonVersion,
  cargoToml: writeCargoTomlVersion,
};

// ------------------------------------------------------------------- helpers

async function readAll() {
  const out = {};
  for (const [key, path] of Object.entries(FILES)) {
    if (!existsSync(path)) throw new Error(`missing ${LABELS[key]}`);
    const text = await readFile(path, "utf8");
    out[key] = { path, text, version: READERS[key](text, LABELS[key]) };
  }
  return out;
}

function report(state) {
  const width = Math.max(...Object.values(LABELS).map((label) => label.length));
  for (const key of Object.keys(FILES)) {
    console.error(`  ${LABELS[key].padEnd(width)}  ${state[key].version}`);
  }
}

async function restore(snapshot) {
  for (const [, { path, text }] of Object.entries(snapshot)) {
    await writeFile(path, text);
  }
  console.error("rolled back; working tree is unchanged");
}

// `cargo update -p unfocus` re-resolves the path package to whatever
// Cargo.toml now declares. Cargo.toml must already be written.
async function syncLockWithCargo(next) {
  const args = ["update", "-p", CRATE, "--offline", "--manifest-path", FILES.cargoToml];
  const run = (argv) => Bun.spawnSync(["cargo", ...argv], { cwd: ROOT, stderr: "pipe", stdout: "pipe" });
  let proc = run(args);
  if (proc.exitCode !== 0) {
    // A cold registry cache can defeat --offline; retry allowed to hit the network.
    proc = run(args.filter((arg) => arg !== "--offline"));
  }
  if (proc.exitCode !== 0) throw new Error(`cargo update failed:\n${new TextDecoder().decode(proc.stderr)}`);
  // cargo exits 0 even when it changes nothing, so confirm by reading it back.
  const text = await readFile(FILES.cargoLock, "utf8");
  const got = readCargoLockVersion(text, LABELS.cargoLock);
  if (got !== next) throw new Error(`cargo update left Cargo.lock at ${got}, expected ${next}`);
}

function haveCargo() {
  try {
    // Bun.spawnSync throws (rather than returning nonzero) when the binary is absent.
    return Bun.spawnSync(["cargo", "--version"], { stderr: "ignore", stdout: "ignore" }).exitCode === 0;
  } catch {
    return false;
  }
}

// ---------------------------------------------------------------- subcommands

async function check(expect) {
  const state = await readAll();
  const versions = Object.keys(FILES).map((key) => state[key].version);
  const unique = [...new Set(versions)];

  if (unique.length !== 1) {
    console.error("version declarations disagree:");
    report(state);
    console.error("\nfix with: bun run version:set <X.Y.Z>");
    process.exit(1);
  }

  const declared = unique[0];
  const parsed = VERSION.exec(declared);
  if (!parsed) {
    console.error(`declared version ${declared} is not X.Y.Z or X.Y.Z-prerelease`);
    process.exit(1);
  }
  const { core, prerelease } = parsed.groups;

  // The MSI cannot carry the prerelease label the other declarations do, so it
  // gets the numeric core and has to stay pinned to it.
  const wix = readWixVersion(state.tauriConf.text, LABELS.tauriConf);
  if (wix !== core) {
    console.error(`bundle.windows.wix.version is ${wix}, expected the numeric core ${core}`);
    report(state);
    console.error(`\nfix with: bun run version:set ${declared}`);
    process.exit(1);
  }

  if (expect !== undefined) {
    if (expect !== `v${declared}`) {
      console.error(`tag ${expect} does not match the declared version ${declared}`);
      report(state);
      console.error(`\nthe tag must be v${declared}, or the declared version must match the tag`);
      process.exit(1);
    }
    console.log(`ok: ${expect} matches all four declarations (${declared}); MSI ships as ${wix}`);
    return;
  }

  console.log(
    prerelease
      ? `ok: all four declarations are ${declared}; MSI ships as ${wix}`
      : `ok: all four declarations are ${declared}`
  );
}

async function set(next) {
  const parsed = VERSION.exec(next);
  if (!parsed) {
    console.error(`refusing ${next}: version must be X.Y.Z or X.Y.Z-prerelease`);
    console.error("build metadata does not order and nothing downstream reads it");
    process.exit(1);
  }
  const core = parsed.groups.core;
  if (!CORE.test(core)) {
    console.error(`refusing ${next}: ${core} is not a numeric X.Y.Z core`);
    process.exit(1);
  }

  const snapshot = await readAll();
  const current = snapshot.tauriConf.version;
  if (current === next) {
    console.error(`already at ${next}`);
    process.exit(1);
  }

  try {
    for (const [key, writer] of Object.entries(WRITERS)) {
      let text = writer(snapshot[key].text, next, LABELS[key]);
      // The MSI takes the numeric core; it cannot express the label.
      if (key === "tauriConf") text = writeWixVersion(text, core, LABELS[key]);
      await writeFile(FILES[key], text);
    }
    if (haveCargo()) {
      await syncLockWithCargo(next);
    } else {
      console.error("warning: cargo not found; patching the Cargo.lock block directly.");
      console.error("         run `cargo check --manifest-path src-tauri/Cargo.toml` before tagging.");
      await writeFile(
        FILES.cargoLock,
        writeCargoLockVersion(snapshot.cargoLock.text, next, LABELS.cargoLock),
      );
    }
    const after = await readAll();
    const bad = Object.keys(FILES).filter((key) => after[key].version !== next);
    if (bad.length) throw new Error(`post-write check failed for: ${bad.map((key) => LABELS[key]).join(", ")}`);
    const wroteWix = readWixVersion(after.tauriConf.text, LABELS.tauriConf);
    if (wroteWix !== core) throw new Error(`post-write check failed: wix version is ${wroteWix}, expected ${core}`);
  } catch (error) {
    console.error(`\n${error.message}`);
    await restore(snapshot);
    process.exit(1);
  }

  console.log(`${current} -> ${next}`);
  for (const key of Object.keys(FILES)) console.log(`  updated ${LABELS[key]}`);
  console.log(`\nnext: review the diff, commit, then tag v${next}`);
}

// --------------------------------------------------------------------- entry

const [command, ...rest] = process.argv.slice(2);

try {
  if (command === "check") {
    if (rest.length === 0) {
      await check(undefined);
    } else if (rest.length === 2 && rest[0] === "--expect" && rest[1]) {
      await check(rest[1]);
    } else {
      throw new Error("usage: version.js check [--expect vX.Y.Z]");
    }
  } else if (command === "set") {
    if (rest.length !== 1 || !rest[0]) throw new Error("usage: version.js set <X.Y.Z>");
    await set(rest[0]);
  } else {
    console.error("usage: version.js check [--expect vX.Y.Z] | version.js set <X.Y.Z>");
    process.exit(2);
  }
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
