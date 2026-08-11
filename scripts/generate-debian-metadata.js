#!/usr/bin/env bun

import { gzipSync } from "node:zlib";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { semverToDebianVersion } from "./debian-package.js";
import { extractReleaseNotes } from "./extract-release-notes.js";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const MAINTAINER = "Unfocus contributors <14940119+abhiksark@users.noreply.github.com>";

export function renderDebianChangelog(changelog, version) {
  extractReleaseNotes(changelog, `v${version}`);
  const heading = new RegExp(`^## \\[${version.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}\\] - (\\d{4})-(\\d{2})-(\\d{2})\\s*$`, "m");
  const match = changelog.match(heading);
  if (!match) throw new Error(`CHANGELOG.md has no dated [${version}] section`);
  const date = new Date(Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])));
  const timestamp = date.toUTCString().replace("GMT", "+0000");
  return `unfocus (${semverToDebianVersion(version)}) unstable; urgency=medium\n\n  * Release v${version}. See changelog.gz for curated release notes.\n\n -- ${MAINTAINER}  ${timestamp}\n`;
}

export function generateDebianMetadata(changelog, version, outputDirectory) {
  const output = resolve(outputDirectory);
  mkdirSync(output, { recursive: true });
  const options = { level: 9, mtime: 0 };
  writeFileSync(join(output, "changelog.Debian.gz"), gzipSync(renderDebianChangelog(changelog, version), options));
  writeFileSync(join(output, "changelog.gz"), gzipSync(changelog, options));
}

function main() {
  const output = process.argv[2] ?? join(ROOT, "src-tauri", "target", "release", "debian-metadata");
  if (process.argv.length > 3) {
    console.error("usage: generate-debian-metadata.js [output-directory]");
    process.exit(2);
  }
  const changelog = readFileSync(join(ROOT, "CHANGELOG.md"), "utf8");
  const version = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8")).version;
  generateDebianMetadata(changelog, version, output);
  console.log(`generated Debian changelogs for ${version} in ${output}`);
}

if (import.meta.main) {
  try {
    main();
  } catch (error) {
    console.error(error.message);
    process.exit(1);
  }
}
