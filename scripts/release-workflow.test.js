import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const workflow = readFileSync(resolve(".github/workflows/release.yml"), "utf8");

function position(text) {
  const index = workflow.indexOf(text);
  expect(index).toBeGreaterThanOrEqual(0);
  return index;
}

describe("release workflow signing boundary", () => {
  test("keeps ordinary Tauri builds credential-free and updater-json-free", () => {
    expect(workflow).toContain("uploadUpdaterJson: false");
    expect(position("Build packages without release credentials")).toBeLessThan(
      position("Inspect Linux package identities and contents"),
    );
    expect(position("Inspect Linux package identities and contents")).toBeLessThan(
      position("Sign a fresh beta candidate"),
    );
  });

  test("maps production updater secrets only on the fresh beta signing step", () => {
    expect(workflow.match(/secrets\.TAURI_SIGNING_PRIVATE_KEY/g)).toHaveLength(2);
    const signingStep = workflow.slice(
      position("- name: Sign a fresh beta candidate"),
      position("- name: Finalize a fresh candidate without signing credentials"),
    );
    expect(signingStep).toContain("secrets.TAURI_SIGNING_PRIVATE_KEY");
    expect(signingStep).toContain("secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD");
    expect(workflow.slice(0, position("- name: Sign a fresh beta candidate"))).not.toContain(
      "secrets.TAURI_SIGNING_PRIVATE_KEY",
    );
    expect(workflow.slice(position("- name: Finalize a fresh candidate without signing credentials"))).not.toContain(
      "secrets.TAURI_SIGNING_PRIVATE_KEY",
    );
  });

  test("discovers and fully verifies a reusable draft before signing", () => {
    expect(position("Discover and download a complete reusable draft")).toBeLessThan(
      position("Sign a fresh beta candidate"),
    );
    expect(position("Verify the complete reusable draft before any signing secret")).toBeLessThan(
      position("Sign a fresh beta candidate"),
    );
    expect(workflow).toContain("Existing draft has a partial or different immutable asset inventory.");
    expect(workflow).toContain('EXPECTED_CHECKSUM_SIZE=$(awk');
    expect(workflow).toContain('[ "$SIZE" -le 8192 ]');
    expect(workflow).toContain('[ "$SIZE" -le 65536 ]');
    expect(workflow).toContain('CANDIDATE_PATH="validated-release/release-assets/$NAME"');
    expect(workflow).toContain('ulimit -f "$FILE_BLOCK_LIMIT"');
    expect(workflow).not.toContain("releases/assets/$ASSET_ID\" -X DELETE");
    expect(workflow).not.toContain("gh release upload");
    expect(workflow).toContain("uploaded-assets.tsv");
    expect(workflow).toContain("Fresh draft assets changed during upload.");
    expect(workflow).toContain("uploaded and reconciled $UPLOADED immutable assets");
  });

  test("attests only after finalization and final verification", () => {
    expect(position("Finalize a fresh candidate without signing credentials")).toBeLessThan(
      position("Attest final immutable release assets"),
    );
    expect(position("Verify the fresh final release")).toBeLessThan(
      position("Attest final immutable release assets"),
    );
    expect(workflow).toContain("subject-path: final-release/*");
    expect(workflow).not.toContain("subject-path: release-assets/*");
  });

  test("runs a production-secret-free ephemeral promotion rehearsal", () => {
    const rehearsal = workflow.slice(position("finalize-rehearsal:"), position("publish:"));
    expect(rehearsal).toContain("unfocus-ephemeral-updater.key");
    expect(rehearsal).toContain("release:verify-final");
    expect(rehearsal).not.toContain("${{ secrets.");
  });

  test("derives the release channel once and transports validated package evidence", () => {
    expect(workflow.match(/releaseChannel\(process\.argv\[1\]\)/g)).toHaveLength(1);
    expect(workflow).toContain("name: validated-release-candidate");
    expect(workflow).toContain("validated-release/linux-package-evidence.json");
    expect(workflow).toContain("UPDATER_PUBLIC_KEY: src-tauri/update-keys/linux-beta.pub");
    expect(workflow).toContain('release:verify-update-signature --check-public-key "$UPDATER_PUBLIC_KEY"');
  });
});
