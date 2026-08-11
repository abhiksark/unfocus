#!/usr/bin/env bun

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function extractReleaseNotes(changelog, tag) {
  if (typeof tag !== "string" || !/^v(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$/.test(tag)) {
    throw new Error(`release tag is not a canonical v-prefixed version: ${tag}`);
  }
  const version = tag.slice(1);
  const heading = new RegExp(`^## \\[${escapeRegExp(version)}\\] - \\d{4}-\\d{2}-\\d{2}\\s*$`, "m");
  const matches = [...changelog.matchAll(new RegExp(heading.source, "gm"))];
  if (matches.length !== 1) {
    throw new Error(`CHANGELOG.md must contain exactly one dated [${version}] section, found ${matches.length}`);
  }

  const bodyStart = matches[0].index + matches[0][0].length;
  const remainder = changelog.slice(bodyStart);
  const nextHeading = remainder.search(/^## /m);
  const section = nextHeading === -1 ? remainder : remainder.slice(0, nextHeading);
  const references = section.search(/^\[(?:Unreleased|\d+\.\d+\.\d+[^\]]*)\]:\s+https?:/m);
  const body = (references === -1 ? section : section.slice(0, references)).trim();
  if (!body || !/^###\s+\S/m.test(body) || !/^-\s+\S/m.test(body)) {
    throw new Error(`CHANGELOG.md [${version}] section is empty or has no curated change entries`);
  }
  return `${body}\n`;
}

function main() {
  const [tag, changelogPath = "CHANGELOG.md"] = process.argv.slice(2);
  if (!tag || process.argv.length > 4) {
    console.error("usage: extract-release-notes.js <vX.Y.Z[-prerelease]> [CHANGELOG.md]");
    process.exit(2);
  }
  process.stdout.write(extractReleaseNotes(readFileSync(resolve(changelogPath), "utf8"), tag));
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
