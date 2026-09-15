#!/usr/bin/env bun
import { readFileSync } from "node:fs";
import { releaseChannel } from "./linux-update-envelope.js";
import { expectedFinalReleaseFilenames } from "./verify-final-release.js";

export function validateDraft(release, version, commit) {
  const prerelease = releaseChannel(version) !== "stable";
  if (!/^[0-9a-f]{40}$/.test(commit)) throw new Error("Invalid event commit");
  if (release?.draft !== true || release.prerelease !== prerelease || release.tag_name !== `v${version}`) {
    throw new Error("Refusing a published release or draft with the wrong tag/channel");
  }
  if (release.target_commitish !== commit) throw new Error("Existing draft targets a different commit");
  if (!Number.isSafeInteger(release.id) || release.id <= 0) throw new Error("Invalid draft id");
}

export function validateDraftInventory(assets, version) {
  const expected = expectedFinalReleaseFilenames(version).sort();
  if (!Array.isArray(assets) || assets.length !== expected.length || assets.some((asset) =>
    asset.state !== "uploaded" || !Number.isSafeInteger(asset.id) || asset.id <= 0 ||
    !Number.isSafeInteger(asset.size) || asset.size <= 0)) throw new Error("Incomplete draft inventory");
  if (JSON.stringify(assets.map((asset) => asset.name).sort()) !== JSON.stringify(expected)) {
    throw new Error("Partial or different immutable draft inventory");
  }
}

if (import.meta.main) {
  const [mode, version, commit] = process.argv.slice(2);
  const data = JSON.parse(readFileSync(0, "utf8"));
  if (mode === "release") validateDraft(data, version, commit);
  else if (mode === "inventory") validateDraftInventory(data, version);
  else throw new Error("Expected release or inventory mode");
}
