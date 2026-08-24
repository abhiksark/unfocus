import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");

function workflow(name) {
  return readFileSync(join(root, ".github", "workflows", name), "utf8");
}

function occurrences(contents, value) {
  return contents.split(value).length - 1;
}

describe("release dispatch workflow immutability", () => {
  for (const [channel, expected] of [["alpha", 1], ["beta", 1]]) {
    test(`binds the APT ${channel} tag to the release target commit and rechecks it before dispatch`, () => {
      const contents = workflow(`apt-${channel}-dispatch.yml`);

      expect(contents).toContain(`event_type: "unfocus-${channel}-published"`);
      expect(contents).toContain(`TAG_PATTERN='^v(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)-${channel}\\.(0|[1-9][0-9]*)$'`);
      expect(occurrences(contents, "TARGET_COMMITISH=$(jq -r '.target_commitish // empty' <<<\"$RELEASE\")")).toBe(expected);
      expect(occurrences(contents, "Release target_commitish must be an exact lowercase commit SHA.")).toBe(expected);
      expect(occurrences(contents, "[ \"$TAG_COMMIT\" = \"$TARGET_COMMIT\" ] || {")).toBe(2);
      expect(contents).toContain("$TAG does not point to the release target commit.");
      expect(contents).toContain("$TAG_NAME moved after release validation; refusing to dispatch.");
      expect(contents).toContain("echo \"target_commit=$TARGET_COMMIT\"");
      expect(occurrences(contents, "git fetch --no-tags origin \"+refs/tags/$TAG_NAME:refs/tags/$TAG_NAME\"")).toBe(1);
    });
  }

  for (const [channel, expected] of [["alpha", 2], ["beta", 2]]) {
    test(`binds both Homebrew ${channel} paths to the release target commit and rechecks before each dispatch`, () => {
      const contents = workflow(`homebrew-${channel}-dispatch.yml`);

      expect(contents).toContain(`types: [unfocus-homebrew-${channel}-published]`);
      expect(contents).toContain(`event_type: "unfocus-homebrew-${channel}-published"`);
      expect(contents).toContain(`event_type: "unfocus-${channel}-published"`);
      expect(occurrences(contents, "TARGET_COMMITISH=$(jq -r '.target_commitish // empty'")).toBe(expected);
      expect(occurrences(contents, "Release target_commitish must be an exact lowercase commit SHA.")).toBe(expected);
      expect(occurrences(contents, "[ \"$TAG_COMMIT\" = \"$TARGET_COMMIT\" ] || {")).toBe(4);
      expect(contents).toContain("$TAG_NAME moved after release validation; refusing to relay.");
      expect(contents).toContain("$TAG_NAME moved after release validation; refusing to dispatch.");
      expect(occurrences(contents, "echo \"target_commit=$TARGET_COMMIT\"")).toBe(expected);
      expect(occurrences(contents, "git fetch --no-tags origin \"+refs/tags/$TAG_NAME:refs/tags/$TAG_NAME\"")).toBe(expected);
    });
  }
});
