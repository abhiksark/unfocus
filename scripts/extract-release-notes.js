#!/usr/bin/env bun

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

function isCalendarDate(year, month, day) {
  const date = new Date(Date.UTC(year, month - 1, day));
  return date.getUTCFullYear() === year && date.getUTCMonth() === month - 1 && date.getUTCDate() === day;
}

export function extractReleaseNotes(changelog, tag) {
  if (typeof tag !== "string" || !/^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-(?:0|[1-9]\d*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*)(?:\.(?:0|[1-9]\d*|[0-9A-Za-z-]*[A-Za-z-][0-9A-Za-z-]*))*)?$/.test(tag)) {
    throw new Error(`release tag is not a canonical v-prefixed version: ${tag}`);
  }
  const version = tag.slice(1);
  const heading = new RegExp(`^## \\[${escapeRegExp(version)}\\] - (\\d{4})-(\\d{2})-(\\d{2})\\s*$`, "m");
  const matches = [...changelog.matchAll(new RegExp(heading.source, "gm"))];
  if (matches.length !== 1) {
    throw new Error(`CHANGELOG.md must contain exactly one dated [${version}] section, found ${matches.length}`);
  }
  const [, year, month, day] = matches[0];
  if (!isCalendarDate(Number(year), Number(month), Number(day))) {
    throw new Error(`CHANGELOG.md has an invalid date for [${version}]`);
  }

  const bodyStart = matches[0].index + matches[0][0].length;
  const remainder = changelog.slice(bodyStart);
  const nextHeading = remainder.search(/^## /m);
  const section = nextHeading === -1 ? remainder : remainder.slice(0, nextHeading);
  const references = section.search(/^\[(?:Unreleased|\d+\.\d+\.\d+[^\]]*)\]:\s+https?:/m);
  const body = (references === -1 ? section : section.slice(0, references)).trim();
  const subsections = [...body.matchAll(/^###\s+(\S(?:.*\S)?)\s*$/gm)];
  if (!body || subsections.length === 0) {
    throw new Error(`CHANGELOG.md [${version}] section is empty or has no curated change entries`);
  }
  for (const [index, subsection] of subsections.entries()) {
    const next = subsections[index + 1];
    const entries = body.slice(subsection.index + subsection[0].length, next?.index);
    if (!/^\s*-\s+\S/m.test(entries)) {
      throw new Error(`CHANGELOG.md [${version}] subsection "${subsection[1]}" has no curated change entries`);
    }
  }
  return `${body}\n`;
}

export function composeReleaseNotes(changelog, tag) {
  return "These early builds are not code-signed or notarized. Verify downloads with SHA256SUMS and the GitHub build-provenance attestations.\n" +
    "The release also includes a CycloneDX SBOM and the bundled third-party notices.\n\n" +
    "- **Linux**: X11 is qualified. APT archive metadata is signed; application binaries are unsigned. Wayland is unsupported.\n" +
    "- **macOS**: Preview and unnotarized. Multi-monitor behavior is not yet qualified.\n" +
    "- **Windows**: Idle and fullscreen probes are implemented, but interactive multi-monitor qualification is pending.\n\n" +
    `## Changes\n\n${extractReleaseNotes(changelog, tag)}`;
}

function main() {
  const [tag, changelogPath = "CHANGELOG.md"] = process.argv.slice(2);
  if (!tag || process.argv.length > 4) {
    console.error("usage: extract-release-notes.js <vX.Y.Z[-prerelease]> [CHANGELOG.md]");
    process.exit(2);
  }
  process.stdout.write(composeReleaseNotes(readFileSync(resolve(changelogPath), "utf8"), tag));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
