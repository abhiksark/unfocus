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
    expect(position("Verify the complete reusable draft before any updater signing secret")).toBeLessThan(
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

describe("stable macOS protected signing", () => {
  const signing = workflow.slice(workflow.indexOf("  sign-macos:"), workflow.indexOf("  assemble:"));
  test("confines Apple secrets to a protected stable-only read-only job", () => {
    expect(signing).toContain("environment: release");
    expect(signing).toContain("contents: read");
    expect(signing).not.toContain("contents: write");
    expect(signing).not.toContain("id-token:");
    expect(signing).not.toContain("attestations:");
    expect(signing).toContain("!inputs.rehearsal");
    expect(signing).toContain("github.event_name == 'push'");
    expect(signing).toContain("outputs.channel == 'stable'");
    expect(signing).toContain("aarch64-apple-darwin");
    expect(signing).toContain("x86_64-apple-darwin");
    expect(workflow.replace(signing, "")).not.toContain("secrets.APPLE_");
    expect(signing).toContain("if: ${{ always() }}");
    expect(signing).toContain("release:sign-macos --cleanup");
    expect(signing.indexOf("Check immutable tag and draft metadata")).toBeLessThan(signing.indexOf("secrets.APPLE_"));
  });
  test("requires successful stable signing before checksums and preserves channel metadata", () => {
    expect(workflow).toContain("needs: [build, release-context, sign-macos]");
    expect(workflow).toContain("needs.sign-macos.result == 'success'");
    expect(workflow).toContain("inputs.rehearsal || needs.release-context.outputs.channel != 'stable'");
    expect(signing).toContain("name: unsigned-${{ matrix.name }}");
    expect(signing).toContain("path: signed-macos/*.dmg");
    expect(workflow).toContain("RELEASE_PRERELEASE: ${{ needs.release-context.outputs.channel != 'stable' }}");
    expect(workflow).toContain("draft:true, prerelease:$prerelease");
    expect(workflow).not.toContain("prerelease:true}");
    expect(workflow).toContain('release-draft-policy.js release "$RELEASE_VERSION" "$EVENT_SHA"');
  });
});

describe("release dependency graph after intentionally skipped macOS signing", () => {
  const jobs = Bun.YAML.parse(workflow).jobs;
  function eligible(name, results, { channel = "stable", version = "1.0.0", rehearsal = false, cancelled = false, event = "push", ref = "refs/tags/v0.7.0" } = {}) {
    const expression = jobs[name].if;
    // Without an explicit status function, Actions applies implicit success()
    // and may propagate the skipped signing ancestor through successful jobs.
    expect(expression).toContain("!cancelled()");
    const needs = Object.fromEntries(jobs[name].needs.map((id) => [id, {
      result: results[id] ?? "success", outputs: { channel, version },
    }]));
    const javascript = expression.slice(3, -2).replace(/needs\.([\w-]+)/g, 'needs["$1"]');
    return new Function("needs", "inputs", "github", "cancelled", "startsWith", `return (${javascript});`)(
      needs, { rehearsal }, { event_name: event, ref }, () => cancelled, (value, prefix) => value.startsWith(prefix),
    );
  }
  function graph(options, overrides = {}) {
    const results = { "sign-macos": options.channel === "stable" && !(options.version ?? "1.0.0").startsWith("0.") && !options.rehearsal ? "success" : "skipped", ...overrides };
    for (const job of ["assemble", "validate-linux-packages", "finalize-rehearsal", "publish"]) {
      const runs = eligible(job, results, options);
      results[job] = runs ? (overrides[job] ?? "success") : "skipped";
    }
    return results;
  }
  test("stable and every prerelease reach validation and draft publication", () => {
    for (const channel of ["stable", "alpha", "beta", "rc"]) {
      const results = graph({ channel });
      expect(results["validate-linux-packages"]).toBe("success");
      expect(results.publish).toBe("success");
      expect(results["finalize-rehearsal"]).toBe("skipped");
    }
  });
  test("pre-1.x stable deliberately skips signing but failures still block publication", () => {
    for (const version of ["0.7.0", "0.99.0"]) {
      const results = graph({ channel: "stable", version });
      expect(results["sign-macos"]).toBe("skipped");
      expect(results.publish).toBe("success");
      expect(graph({ channel: "stable", version }, { "sign-macos": "failure" }).publish).toBe("skipped");
    }
    for (const version of ["1.0.0", "2.0.0", "10.0.0"]) {
      expect(graph({ channel: "stable", version }, { "sign-macos": "skipped" }).publish).toBe("skipped");
    }
    const gate = "needs.release-context.outputs.channel == 'stable' && !startsWith(needs.release-context.outputs.version, '0.')";
    expect(jobs["sign-macos"].if).toContain(gate);
    expect(JSON.stringify(jobs.build)).toContain(gate);
  });
  test("all promotion rehearsals reach finalization without publication", () => {
    for (const channel of ["stable", "alpha", "beta", "rc"]) {
      const results = graph({ channel, rehearsal: true, event: "pull_request", ref: "refs/pull/1/merge" });
      expect(results["sign-macos"]).toBe("skipped");
      expect(results["validate-linux-packages"]).toBe("success");
      expect(results["finalize-rehearsal"]).toBe("success");
      expect(results.publish).toBe("skipped");
    }
  });
  test("failed or cancelled required jobs never produce a draft", () => {
    for (const prerequisite of ["build", "release-context", "sign-macos", "assemble", "validate-linux-packages"]) {
      for (const failure of ["failure", "cancelled"]) {
        expect(graph({ channel: "stable" }, { [prerequisite]: failure }).publish).toBe("skipped");
      }
    }
    expect(graph({ channel: "stable" }, { "sign-macos": "skipped" }).publish).toBe("skipped");
    for (const channel of ["stable", "alpha", "beta", "rc"]) {
      expect(graph({ channel, cancelled: true }).publish).toBe("skipped");
      expect(graph({ channel, rehearsal: true }, { "validate-linux-packages": "failure" })["finalize-rehearsal"]).toBe("skipped");
    }
  });
  test("each downstream job requires every direct prerequisite to succeed", () => {
    for (const job of ["validate-linux-packages", "finalize-rehearsal", "publish"]) {
      for (const prerequisite of jobs[job].needs) {
        for (const result of ["failure", "cancelled", "skipped"]) {
          expect(eligible(job, { [prerequisite]: result }, { rehearsal: job === "finalize-rehearsal" })).toBe(false);
        }
      }
    }
  });
  test("publication retains tag-push restrictions after the status override", () => {
    expect(graph({ channel: "stable", event: "workflow_dispatch" }).publish).toBe("skipped");
    expect(graph({ channel: "stable", ref: "refs/heads/main" }).publish).toBe("skipped");
  });
});
