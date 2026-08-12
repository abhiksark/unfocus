// scripts/sbom-dependencies.js
//
// Dependency-edge resolution for the CycloneDX SBOM. Kept separate from
// generate-sbom.js so it can be tested without that script's start-up work,
// which shells out to cargo and reads the real lockfile on import.

/**
 * Lock keys a dependency could resolve to, nearest first.
 *
 * bun.lock keys a nested copy under `<owner>/<name>`, mirroring node module
 * resolution: that copy is visible only to its owner and the owner's own
 * descendants. Everything else sees the root copy, keyed by the bare name. So
 * walk up the owner's path and finish at the root, exactly as a resolver would.
 */
function resolutionOrder(owner, name) {
  const segments = owner.split("/");
  const keys = [];
  for (let depth = segments.length; depth > 0; depth -= 1) {
    keys.push(`${segments.slice(0, depth).join("/")}/${name}`);
  }
  keys.push(name);
  return keys;
}

/**
 * Resolve one bun.lock dependency edge to the locked package it refers to.
 *
 * A package can appear twice in the lockfile: once at the root and once beneath
 * a dependent that pinned an older version. Matching on the bare name alone is
 * ambiguous the moment that happens, because a loose range such as `*` matches
 * both copies. Resolve by lock key first so each owner gets the copy it would
 * actually load, then fall back to the previous name-based search, which still
 * covers aliased entries whose lock key differs from the package name.
 *
 * @param {{ byName: Map<string, Array<object>>, byLockKey: Map<string, object> }} packages
 * @param {string} owner    lock key of the depending package, or a label for the root workspace
 * @param {string} name     dependency name as written in the manifest
 * @param {string} range    semver range as written in the manifest
 * @param {boolean} optional whether an unresolved dependency is tolerated
 * @returns {string | null} the locked package ref, or null for an absent optional dependency
 */
export function resolveBunDependency(packages, owner, name, range, optional) {
  if (typeof range !== "string" || range.length === 0) {
    throw new Error(`${owner} has an invalid dependency range for ${name}`);
  }

  for (const key of resolutionOrder(owner, name)) {
    const record = packages.byLockKey.get(key);
    if (record && Bun.semver.satisfies(record.version, range)) return record.ref;
  }

  const candidates = packages.byName.get(name) ??
    (packages.byLockKey.has(name) ? [packages.byLockKey.get(name)] : []);
  const matches = candidates.filter((candidate) => Bun.semver.satisfies(candidate.version, range));
  if (matches.length === 1) return matches[0].ref;
  if (matches.length === 0 && optional && candidates.length === 0) return null;
  const detail = matches.length === 0 ? "no matching locked package" : "multiple matching locked packages";
  throw new Error(`${owner} dependency ${name}@${range} has ${detail}`);
}
