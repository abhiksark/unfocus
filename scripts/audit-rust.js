import { loadDependencyExceptions } from "./check-dependency-exceptions.js";
import { fileURLToPath } from "node:url";
import { resolveVerifiedGlib } from "./check-vendored-glib.js";

const root = fileURLToPath(new URL("..", import.meta.url));
// cargo-audit excludes path packages. Verify the source and separately scan
// the authenticated registry identity so future GLib advisories still fail.
const glib = resolveVerifiedGlib(root);
const decoder = new TextDecoder();

function validateAdvisoryEntries(entries, label) {
  if (!Array.isArray(entries)) throw new Error(`cargo audit returned invalid ${label} entries`);
  for (const entry of entries) {
    if (typeof entry?.advisory?.id !== "string" || !entry.advisory.id.trim()) {
      throw new Error(`cargo audit returned a ${label} entry without an advisory identifier`);
    }
  }
}

function auditLockfile(path) {
  const audit = Bun.spawnSync(["cargo", "audit", "--json", "--file", path], {
    cwd: root, stdout: "pipe", stderr: "pipe"
  });
  const stdout = decoder.decode(audit.stdout).trim();
  const stderr = decoder.decode(audit.stderr).trim();
  let report;
  try {
    report = JSON.parse(stdout);
  } catch {
    throw new Error(`cargo audit did not return JSON${stderr ? `: ${stderr}` : ""}`);
  }
  const vulnerabilities = report?.vulnerabilities?.list;
  validateAdvisoryEntries(vulnerabilities, "vulnerability");
  if (!report?.warnings || typeof report.warnings !== "object" || Array.isArray(report.warnings)) {
    throw new Error("cargo audit returned an invalid warning map");
  }
  // Keep unmaintained notices distinct. Every other advisory-bearing warning
  // (including unsoundness) participates in the fail-closed policy.
  const warningAdvisories = Object.entries(report.warnings).flatMap(([kind, entries]) => {
    validateAdvisoryEntries(entries, `${kind} warning`);
    if (kind === "unmaintained") return [];
    return entries;
  });
  const unmaintained = report.warnings.unmaintained ?? [];
  const advisories = [...vulnerabilities, ...warningAdvisories];
  if (audit.exitCode !== 0 && advisories.length === 0) throw new Error(`cargo audit failed${stderr ? `: ${stderr}` : ""}`);
  return { advisories, unmaintained };
}

const application = auditLockfile(`${root}/src-tauri/Cargo.lock`);
const registry = auditLockfile(glib.auditLockPath);
function registryGlib(entries) {
  for (const entry of entries) {
    const pkg = entry?.package;
    if (typeof pkg?.name !== "string" || !pkg.name.trim() || typeof pkg.version !== "string" || !pkg.version.trim() || pkg.source !== "registry+https://github.com/rust-lang/crates.io-index") {
      throw new Error("invalid registry audit package identity");
    }
  }
  return entries.filter((entry) => entry.package.name === "glib");
}
const unmaintained = [...application.unmaintained, ...registryGlib(registry.unmaintained)];
// Other packages in the registry input are scanner scaffolding. Actual app
// transitive versions are covered by the application lockfile audit above.
const all = [...application.advisories, ...registryGlib(registry.advisories)];
const advisories = all.filter((entry) => !(
  entry.kind === "unsound" && entry.advisory?.id === glib.advisory &&
  entry.package?.name === glib.name && entry.package?.version === glib.version &&
  entry.package?.source === "registry+https://github.com/rust-lang/crates.io-index"
));

const exceptions = loadDependencyExceptions().filter((exception) => exception.ecosystem === "rust");
const allowed = new Set(exceptions.map((exception) => exception.id));
const observed = new Set(advisories.map((entry) => entry?.advisory?.id));
const unapproved = advisories.filter((entry) => !allowed.has(entry?.advisory?.id));
const unused = exceptions.filter((exception) => !observed.has(exception.id));
if (unapproved.length > 0) {
  for (const entry of unapproved) {
    console.error(`unapproved Rust advisory: ${entry?.advisory?.id ?? "unknown"} in ${entry?.package?.name ?? "unknown package"}`);
  }
  process.exit(1);
}
if (unused.length > 0) throw new Error(`unused Rust dependency exception(s): ${unused.map(({ id }) => id).join(", ")}`);
console.log(
  `Rust audit passed with ${advisories.length} exact, expiring exception(s); ` +
  `${unmaintained.length} unmaintained-package notice(s) reported separately; ` +
  `${glib.advisory} source backport verified; registry GLib checked for additional advisories`
);
