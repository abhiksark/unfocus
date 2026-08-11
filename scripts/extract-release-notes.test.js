import { describe, expect, test } from "bun:test";
import { extractReleaseNotes } from "./extract-release-notes.js";

const changelog = `# Changelog

## [Unreleased]

### Added

- Future work.

## [0.2.0-alpha.1] - 2026-08-11

### Added

- Consumer dashboard.

### Changed

- Debian ordering.

## [0.1.0-alpha.1] - 2026-08-07

### Added

- First alpha.

[Unreleased]: https://github.com/abhiksark/unfocus/compare/v0.2.0-alpha.1...dev
[0.2.0-alpha.1]: https://github.com/abhiksark/unfocus/releases/tag/v0.2.0-alpha.1
`;

describe("release-note extraction", () => {
  test("returns only the curated versioned changelog body", () => {
    expect(extractReleaseNotes(changelog, "v0.2.0-alpha.1")).toBe(
      "### Added\n\n- Consumer dashboard.\n\n### Changed\n\n- Debian ordering.\n",
    );
  });

  test("fails when the versioned section is missing", () => {
    expect(() => extractReleaseNotes(changelog, "v0.2.0-alpha.2")).toThrow(
      "exactly one dated [0.2.0-alpha.2] section, found 0",
    );
  });

  test("fails when the versioned section has no curated entries", () => {
    const empty = "# Changelog\n\n## [0.2.0-alpha.1] - 2026-08-11\n\n## [0.1.0] - 2026-08-01\n";
    expect(() => extractReleaseNotes(empty, "v0.2.0-alpha.1")).toThrow(
      "section is empty or has no curated change entries",
    );
  });

  test("requires a canonical v-prefixed tag", () => {
    expect(() => extractReleaseNotes(changelog, "0.2.0-alpha.1")).toThrow("canonical v-prefixed version");
  });

  test("does not include changelog comparison references after the final section", () => {
    expect(extractReleaseNotes(changelog, "v0.1.0-alpha.1")).toBe("### Added\n\n- First alpha.\n");
  });
});
