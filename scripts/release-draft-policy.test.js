import { describe, expect, test } from "bun:test";
import { validateDraft, validateDraftInventory } from "./release-draft-policy.js";
import { expectedFinalReleaseFilenames } from "./verify-final-release.js";
const commit = "a".repeat(40);
function draft(version, prerelease) { return { id: 123, draft: true, prerelease, tag_name: `v${version}`, target_commitish: commit }; }
describe("immutable draft channel policy", () => {
  test("stable is not a prerelease; every prerelease remains a prerelease", () => {
    for (const [version, prerelease] of [["0.7.0", false], ["0.7.0-alpha.1", true], ["0.7.0-beta.1", true], ["0.7.0-rc.1", true]]) {
      expect(() => validateDraft(draft(version, prerelease), version, commit)).not.toThrow();
      expect(() => validateDraft(draft(version, !prerelease), version, commit)).toThrow();
    }
  });
  test("rejects published, moved-target, wrong-tag and malformed releases", () => {
    for (const patch of [{ draft: false }, { target_commitish: "b".repeat(40) }, { tag_name: "v0.8.0" }, { id: -1 }, { prerelease: "false" }]) {
      expect(() => validateDraft({ ...draft("0.7.0", false), ...patch }, "0.7.0", commit)).toThrow();
    }
  });
  test("stable exact inventory excludes updater files and rejects incomplete uploads", () => {
    const assets = expectedFinalReleaseFilenames("0.7.0").map((name, i) => ({ name, id: i + 1, size: 100, state: "uploaded" }));
    expect(assets).toHaveLength(10);
    expect(() => validateDraftInventory(assets, "0.7.0")).not.toThrow();
    for (const invalid of [assets.slice(1), [...assets, { name: "update.json", id: 999, size: 10, state: "uploaded" }],
      assets.map((asset, i) => i ? asset : { ...asset, state: "new" }),
      assets.map((asset, i) => i ? asset : { ...asset, size: 0 }),
      assets.map((asset, i) => i ? asset : { ...asset, name: assets[1].name })]) {
      expect(() => validateDraftInventory(invalid, "0.7.0")).toThrow();
    }
  });
});
