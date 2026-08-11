import { afterEach, describe, expect, test } from "bun:test";
import { gunzipSync } from "node:zlib";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { generateDebianMetadata, renderDebianChangelog } from "./generate-debian-metadata.js";

const changelog = `# Changelog

## [0.2.0-alpha.1] - 2026-08-11

### Added

- Consumer dashboard.
`;
const temporaryDirectories = [];

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

describe("Debian metadata generation", () => {
  test("renders a valid deterministic Debian changelog entry", () => {
    expect(renderDebianChangelog(changelog, "0.2.0-alpha.1")).toBe(
      "unfocus (0.2.0~alpha.1-1) unstable; urgency=medium\n\n" +
        "  * Release v0.2.0-alpha.1. See changelog.gz for curated release notes.\n\n" +
        " -- Unfocus contributors <14940119+abhiksark@users.noreply.github.com>  Tue, 11 Aug 2026 00:00:00 +0000\n",
    );
  });

  test("writes reproducible compressed Debian and upstream changelogs", () => {
    const first = mkdtempSync(join(tmpdir(), "unfocus-debian-metadata-"));
    const second = mkdtempSync(join(tmpdir(), "unfocus-debian-metadata-"));
    temporaryDirectories.push(first, second);
    generateDebianMetadata(changelog, "0.2.0-alpha.1", first);
    generateDebianMetadata(changelog, "0.2.0-alpha.1", second);

    for (const name of ["changelog.Debian.gz", "changelog.gz"]) {
      expect(readFileSync(join(first, name))).toEqual(readFileSync(join(second, name)));
    }
    expect(gunzipSync(readFileSync(join(first, "changelog.gz"))).toString()).toBe(changelog);
    expect(gunzipSync(readFileSync(join(first, "changelog.Debian.gz"))).toString()).toContain(
      "unfocus (0.2.0~alpha.1-1)",
    );
  });

  test("fails if the released changelog section is missing", () => {
    expect(() => renderDebianChangelog(changelog, "0.2.0-alpha.2")).toThrow(
      "exactly one dated [0.2.0-alpha.2] section",
    );
  });

  test("rejects an invalid calendar date", () => {
    const invalid = changelog.replace("2026-08-11", "2026-02-31");
    expect(() => renderDebianChangelog(invalid, "0.2.0-alpha.1")).toThrow(
      "CHANGELOG.md has an invalid date for [0.2.0-alpha.1]",
    );
  });
});
