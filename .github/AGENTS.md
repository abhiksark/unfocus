# GitHub automation

This file applies to `.github/**` and to work that reads or changes pull
requests, checks, Dependabot branches, repository protections, tags, or
releases. Follow the root `AGENTS.md` as well. Read `scripts/AGENTS.md` for
dependency metadata, version/toolchain pins, and release-artifact collection.

## CI and workflow security

- Keep third-party actions pinned to full commit SHAs with a readable version
  comment. Do not replace a SHA with a mutable tag.
- Preserve least-privilege workflow and job permissions. Keep checkout
  `persist-credentials: false` unless a reviewed write step needs credentials.
- Pull-request workflows must not expose release credentials or other secrets
  to untrusted code.
- Keep CI concurrency canceling superseded pull-request and push runs. Release
  jobs must not cancel another run for the same tag after publishing begins.
- Bun setup reads `.bun-version`; Rust uses the exact shared pin checked by
  `bun run toolchains:check`. Do not create independent toolchain pins.
- `Required CI` must depend on every required job and fail unless all succeed.
  Current CI includes version/toolchain checks, the Linux quality gate,
  dependency audits, and macOS/Windows compile checks.
- Do not weaken locked installs, warnings-as-errors, dependency checks,
  generated-notice checks, or platform compilation to make CI pass.

## Branching and SemVer strategy

- `main` is the protected release branch and the GitHub default branch. It
  must always be release-ready and receives normal development only through a
  batch promotion from `dev`.
- `dev` is the protected integration branch and the base for normal work.
- `main` and `dev` are the only long-lived branches. Start short-lived branches
  from current `dev` and name them by purpose: `feature/<slug>`, `fix/<slug>`,
  `docs/<slug>`, `chore/<slug>`, or `refactor/<slug>`.
- Open normal pull requests from the short-lived branch into `dev`. Do not
  target `main` directly. Prefer squash merges into `dev`, then delete the
  merged work branch.
- Because GitHub preselects the default branch as a new pull request's base,
  verify the base explicitly. CI rejects a pull request into `main` unless its
  head branch is `dev`.
- Promote a coherent, reviewed, releasable batch with one pull request from
  `dev` into `main`. Use a merge commit for this promotion so the batch has a
  visible boundary. Do not add unrelated fixes directly to the promotion PR.
- Choose the batch version by the highest-impact included change: major for a
  breaking public change, minor for a backward-compatible feature, and patch
  for a backward-compatible fix. While the project remains below `1.0.0`, a
  breaking change increments the minor version and resets the patch version.
- Set the complete release version on `dev` with `bun run version:set X.Y.Z`
  or an approved prerelease form, run every release-relevant gate, and include
  that version commit in the `dev` → `main` promotion.
- After the promotion merges and current `main` CI passes, tag that exact
  `main` commit as `vX.Y.Z[-prerelease]`. Never tag or release from `dev` or a
  short-lived branch.
- Dependabot updates follow the same integration path and target `dev`. Keep
  `.github/dependabot.yml` aligned.
- Urgent fixes still use a short-lived `fix/<slug>` branch into `dev`, followed
  by an expedited `dev` to `main` promotion. Do not bypass the integration
  branch.

## Pull requests and Dependabot

- Read the current base, head, merge state, commits, files, and check rollup
  before reporting a pull request's status.
- Do not call checks green based on an older run. Distinguish queued,
  in-progress, skipped, stale, failed, and successful checks.
- Keep unrelated work out of Dependabot branches. When a dependency update
  makes generated metadata stale, regenerate only the required metadata on
  that branch and run the complete dependency gate from `scripts/AGENTS.md`.
- If a branch is behind its target branch, update it and require fresh checks
  before merging when repository protection requires an up-to-date head.
- A review, status check, or diagnosis does not authorize comments, branch
  updates, pushes, approvals, closures, or merges.

## Release invariants

- A release tag must point to a commit already contained in `main` and equal
  the complete declared version with a `v` prefix.
- The quality job runs before packaging. Build jobs do not receive release
  credentials; the publisher alone receives narrowly scoped write,
  provenance, and attestation permissions through the `release` environment.
- Collect platform artifacts separately and reject filename collisions before
  staging. Generate and verify checksums, then attest the staged assets.
- Verify the remote tag still resolves to the workflow's event commit before
  release writes and again before uploads. Never tolerate a moved tag.
- Create or update only a draft prerelease. Published releases are immutable:
  never replace their assets, reuse their tag, or overwrite their notes.
- Current packages are not code-signed or notarized. Release notes must say so
  and retain checksum and build-provenance verification guidance.

## Homebrew tap automation boundary

- The `Unfocus Homebrew Updater` GitHub App is installed only on
  `abhiksark/homebrew-unfocus`. Its credentials live in the
  default-branch-restricted `homebrew-tap-automation` environment and mint a
  token scoped only to that tap.
- The source repository may use that token only to send the
  `unfocus-alpha-published` repository dispatch. The tap may use it only to
  push an automation branch and open a reviewable cask pull request.
- The App has only Contents and Pull Requests read/write permissions on the
  tap. It must never be installed on Unfocus or receive Administration,
  Actions, Environments, or Secrets permissions.
- Contents and Pull Requests write access must be used only for the automation
  branch and pull request. The workflows must never alter releases or release
  assets, merge the App's pull request, or grant the App a ruleset bypass.

## APT repository automation boundary

- The APT archive for alpha packages lives in `abhiksark/unfocus-apt` and is
  served from GitHub Pages (`https://apt.abhik.ai/`). Suite
  name is `alpha`; package name is `unfocus`; architecture is `amd64` only.
- The APT GitHub App is installed only on `abhiksark/unfocus-apt`. Its
  credentials live in the default-branch-restricted `apt-repo-automation`
  environment on both repositories. Secret names: `APT_APP_ID`,
  `APT_APP_PRIVATE_KEY` (source dispatch and apt-repo update). Signing secrets
  live only on `unfocus-apt`: `APT_SIGNING_KEY` (armored private key) and
  optional `APT_SIGNING_KEY_PASSPHRASE`.
- The source repository may use the App only to send the
  `unfocus-alpha-published` repository dispatch to `unfocus-apt`. It must never
  push apt tree files, hold the archive private key, or sign indexes.
- The apt repository may use the App only to push an automation branch and open
  a reviewable pull request that updates `pool/`, `dists/`, and `public-key.asc`.
  Workflows must never alter Unfocus releases, merge their own pull request, or
  grant a ruleset bypass.
- Archive GPG signing authenticates **repository metadata** only. Alpha `.deb`
  packages remain application-unsigned; docs must keep that distinction.
- Operator setup steps for the key, App, Pages, and first publish live in
  `abhiksark/unfocus-apt` (`OPERATOR.md`).

## Repository settings

- Do not infer live branch protection, tag rules, environments, immutable
  release settings, or security alerts from workflow files. Query the current
  repository state before making a claim.
- Changing repository settings, secrets, environments, rulesets, tags,
  releases, pull requests, or branches is an external write and requires user
  authorization.

## Checks

- Run `git diff --check` for every change.
- Run `bun run toolchains:check` after workflow toolchain edits and
  `bun run version:check` after release-version logic changes.
- Run every locally available gate whose command or environment the workflow
  changes. State which GitHub-hosted platform jobs remain for CI.
- Validate current remote checks and settings directly before reporting them;
  never treat a local workflow parse as proof of live repository state.
