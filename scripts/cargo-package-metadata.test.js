// scripts/cargo-package-metadata.test.js
import { describe, expect, test } from "bun:test";
import { resolveCargoPackageMetadata } from "./cargo-package-metadata.js";
import { verifyVendoredGlib } from "./check-vendored-glib.js";

const panelSource =
  "git+https://github.com/ahkohd/tauri-nspanel.git?rev=a3122e894383aa068ec5365a42994e3ac94ba1b6#a3122e894383aa068ec5365a42994e3ac94ba1b6";

describe("resolveCargoPackageMetadata", () => {
  test("records the verified GLib source and upstream backport without changing its version", () => {
    const backport = verifyVendoredGlib();
    const metadata = resolveCargoPackageMetadata({ name: "glib", version: "0.18.5", source: null, manifest_path: backport.manifestPath, license: "MIT" }, backport);
    expect(metadata.lockedSource).toBe(`vendored:src-tauri/vendor/glib-0.18.5#sha256=${backport.sha256}`);
    expect(metadata.backport.advisory).toBe("RUSTSEC-2024-0429");
    expect(metadata.backport.treeSha256).toBe(backport.sha256);
  });

  test("rejects GLib path metadata that does not select the verified crate", () => {
    const backport = verifyVendoredGlib();
    expect(() => resolveCargoPackageMetadata({ name: "glib", version: "0.18.5", source: null, manifest_path: "/elsewhere/Cargo.toml" }, backport))
      .toThrow("unverified GLib source");
  });

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
