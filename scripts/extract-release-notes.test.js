import { describe, expect, test } from "bun:test";
import * as releaseNotes from "./extract-release-notes.js";

const { extractReleaseNotes } = releaseNotes;

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
  test("composes complete platform-qualified release notes", () => {
    const composeReleaseNotes = releaseNotes.composeReleaseNotes ?? (() => "");
    expect(composeReleaseNotes(changelog, "v0.2.0-alpha.1")).toBe(
      "These prerelease builds are not notarized. macOS app bundles are ad-hoc signed rather than Developer ID-signed, and Windows installers are not code-signed. Verify downloads with SHA256SUMS and the GitHub build-provenance attestations.\n" +
        "The release also includes a CycloneDX SBOM and the bundled third-party notices.\n\n" +
        "- **Linux**: X11 is qualified. APT archive metadata is signed; application binaries are unsigned. Wayland is unsupported.\n" +
        "- **macOS 11+**: Preview, ad-hoc signed, and not notarized. Uses the system-provided AppKit and WebKit frameworks; multi-monitor behavior is not yet qualified.\n" +
        "- **Windows**: Idle and fullscreen probes are implemented, but interactive multi-monitor qualification is pending.\n\n" +
        "## Changes\n\n" +
        "### Added\n\n- Consumer dashboard.\n\n### Changed\n\n- Debian ordering.\n",
    );
  });

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

  test("rejects impossible calendar dates", () => {
    const invalid = changelog.replace("2026-08-11", "2026-02-31");
    expect(() => extractReleaseNotes(invalid, "v0.2.0-alpha.1")).toThrow(
      "CHANGELOG.md has an invalid date for [0.2.0-alpha.1]",
    );
  });

  test("rejects a changelog subsection without an entry", () => {
    const incomplete = changelog.replace("### Changed\n\n- Debian ordering.", "### Changed\n");
    expect(() => extractReleaseNotes(incomplete, "v0.2.0-alpha.1")).toThrow(
      'CHANGELOG.md [0.2.0-alpha.1] subsection "Changed" has no curated change entries',
    );
  });

  test("requires a canonical v-prefixed tag", () => {
    expect(() => extractReleaseNotes(changelog, "0.2.0-alpha.1")).toThrow("canonical v-prefixed version");
  });

  test("rejects a numeric prerelease identifier with a leading zero", () => {
    expect(() => extractReleaseNotes(changelog, "v0.4.0-01")).toThrow("canonical v-prefixed version");
  });

  test("accepts a prerelease identifier that contains letters after a leading zero", () => {
    const alphanumeric = changelog.replace("[0.2.0-alpha.1] - 2026-08-11", "[0.2.0-01alpha] - 2026-08-11");
    expect(extractReleaseNotes(alphanumeric, "v0.2.0-01alpha")).toBe(
      "### Added\n\n- Consumer dashboard.\n\n### Changed\n\n- Debian ordering.\n",
    );
  });

  test("does not include changelog comparison references after the final section", () => {
    expect(extractReleaseNotes(changelog, "v0.1.0-alpha.1")).toBe("### Added\n\n- First alpha.\n");
  });
});
