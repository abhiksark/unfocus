# Automation and generated artifacts

This file applies to `scripts/**` and to repository-wide generated or locked
metadata: `package.json`, `bun.lock`, `.bun-version`, `rust-toolchain.toml`,
`THIRD_PARTY_NOTICES.txt`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`,
`src-tauri/tauri.conf.json`, and `.github/dependency-exceptions.json`. Follow
the root `AGENTS.md` as well.

Read `src/AGENTS.md` for scene generation, `src-tauri/AGENTS.md` for tray or
Cargo behavior, and `.github/AGENTS.md` for workflow pins, release metadata,
or release-artifact collection.

## Automation contract

- Use Bun for JavaScript tooling. Do not use npm, npx, pnpm, or Yarn.
- Prefer named `package.json` scripts. Keep automation deterministic,
  non-interactive, and runnable from the repository root.
- Validation scripts fail closed: malformed, incomplete, missing, or
  unrecognized tool output is an error, not an empty successful result.
- Emit actionable errors that name the stale file and its supported
  regeneration command. Do not silently repair files in check mode.
- Keep generators deterministic. Review generated diffs for ordering changes,
  dropped entries, duplicates, and platform-only metadata.

## Generated and locked files

- The credentialed first-party break-scene source lives at
  `scripts/asset-sources/break-scene-source.png`. Its exact bytes and the local
  `static/break-scene.jpg` 4K delivery derivative must match the SHA-256 hashes
  and `sips` transform in
  `scripts/asset-sources/break-scene.provenance.json`; the source is not native
  4K. Update the source, derivative, and deterministic provenance record
  together, and never replace them with a runtime download or CDN asset.
- The vendored Newsreader Roman variable font and normalized OFL text under
  `static/fonts/` must match the pinned source, hashes, and documented
  normalization in `scripts/asset-sources/newsreader.provenance.json`. Update
  the font, license, provenance, notice metadata, and SBOM metadata together.
- Tray PNGs under `src-tauri/icons/tray/` come from
  `src-tauri/icons/tray/unfocus-tray.svg` via `bun run tray:generate`. Never
  hand-edit the PNGs.
- `THIRD_PARTY_NOTICES.txt` is generated with `bun run notices:generate` and
  verified with `bun run notices:check`. Never hand-edit it.
- `bun.lock` and `src-tauri/Cargo.lock` are package-manager output. Review
  their diffs, but do not fabricate or partially edit dependency records.
- Generate an SBOM with `bun run sbom:generate <path>` to a temporary path.
  It must represent both locked dependency trees and shipped vendored assets,
  with stable identities, licenses, hashes when available, and no duplicate or
  dangling references. Do not leave an untracked SBOM in the repository.
- Regenerate derived artifacts in the same change as their source.

## Dependencies and advisories

- JavaScript dependency changes require the frontend gate from
  `src/AGENTS.md`, `bun run dependencies:check`, notices regeneration/check,
  and SBOM generation.
- Rust dependency changes require the Rust gate from `src-tauri/AGENTS.md`,
  `bun run dependencies:audit-rust`, notices regeneration/check, and SBOM
  generation.
- Any unresolved advisory needs an exact, justified, expiring entry in
  `.github/dependency-exceptions.json`. Broad package, version, category, or
  non-expiring exceptions are forbidden.
- Keep vulnerability, unsoundness, and unmaintained notices distinct. Do not
  downgrade a finding by moving it into another category.
- Prefer existing dependencies and standard/platform APIs. Review licensing,
  supported targets, maintenance, and lockfile impact before adding one.

## Versions and toolchains

- Bun is pinned in `.bun-version`; Rust is pinned in `rust-toolchain.toml`.
  `bun run toolchains:check` proves those pins agree with `package.json`, CI,
  release automation, and the Linux container. Do not hand-update just one.
- Use `bun run version:set X.Y.Z` for every version change; never edit version
  declarations manually. Then run `bun run version:check`.
- Prerelease versions are allowed. A tag must equal the complete declared
  version with a `v` prefix. The Windows MSI ProductVersion remains the
  numeric core in `bundle.windows.wix.version`; `version:check` enforces it.
- Artifact filenames retain the complete declared version so a prerelease and
  a later final release cannot produce identically named files.
- Build metadata is unsupported. Do not introduce a version form the version
  script rejects.

## Container and release artifacts

- `./scripts/run-linux-spike-container.sh` reaches the host X11 display and
  session bus. It is a development convenience, not a sandbox; run it only on
  a trusted revision.
- Release-artifact collection must consume only paths reported by the build,
  reject missing or unsupported bundles, and preserve platform-distinct files.
  Coordinate changes with `.github/workflows/release.yml` and
  `.github/AGENTS.md`.

## Checks

- Run `git diff --check` for every change.
- Run each changed script's check mode or supported end-to-end command.
- Dependency, notice, SBOM, version, and toolchain work runs every matching
  gate above; a single passing generator is not a substitute for the complete
  metadata set.
