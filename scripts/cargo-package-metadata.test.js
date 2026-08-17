// scripts/cargo-package-metadata.test.js
import { describe, expect, test } from "bun:test";
import { resolveCargoPackageMetadata } from "./cargo-package-metadata.js";

const panelSource =
  "git+https://github.com/ahkohd/tauri-nspanel.git?rev=a3122e894383aa068ec5365a42994e3ac94ba1b6#a3122e894383aa068ec5365a42994e3ac94ba1b6";

describe("resolveCargoPackageMetadata", () => {
  test("applies the panel license fallback only to the exact locked source", () => {
    expect(
      resolveCargoPackageMetadata({
        source: panelSource,
        license: null,
        license_file: null,
        repository: null
      })
    ).toEqual({
      declaredLicense: "MIT OR Apache-2.0",
      lockedSource: panelSource,
      repository: null
    });

    expect(
      resolveCargoPackageMetadata({
        source: panelSource.replace(/.$/, "0"),
        license: null,
        license_file: null,
        repository: null
      }).declaredLicense
    ).toBeNull();
  });

  test("prefers ordinary Cargo metadata over the exact-source fallback", () => {
    expect(
      resolveCargoPackageMetadata({
        source: panelSource,
        license: "BSD-3-Clause",
        license_file: "LICENSE",
        repository: "https://example.com/upstream"
      })
    ).toEqual({
      declaredLicense: "BSD-3-Clause",
      lockedSource: panelSource,
      repository: "https://example.com/upstream"
    });
  });
});
