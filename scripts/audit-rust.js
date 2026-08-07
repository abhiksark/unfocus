import { loadDependencyExceptions } from "./check-dependency-exceptions.js";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
const decoder = new TextDecoder();
const audit = Bun.spawnSync(
  ["cargo", "audit", "--json", "--file", `${root}/src-tauri/Cargo.lock`],
  { cwd: root, stdout: "pipe", stderr: "pipe" }
);
const stdout = decoder.decode(audit.stdout).trim();
const stderr = decoder.decode(audit.stderr).trim();

let report;
try {
  report = JSON.parse(stdout);
} catch {
  throw new Error(`cargo audit did not return JSON${stderr ? `: ${stderr}` : ""}`);
}

const vulnerabilities = report?.vulnerabilities?.list;
if (!Array.isArray(vulnerabilities)) {
  throw new Error("cargo audit returned an invalid vulnerability list");
}
if (!report?.warnings || typeof report.warnings !== "object" || Array.isArray(report.warnings)) {
  throw new Error("cargo audit returned an invalid warning map");
}

// `unmaintained` is dependency-health information rather than a vulnerability
// suppression. Every other advisory-bearing warning category (including
// `unsound`) participates in the exact, expiring exception policy. A new
// warning shape fails closed instead of disappearing silently.
const unmaintainedWarnings = report.warnings.unmaintained;
if (unmaintainedWarnings !== undefined && !Array.isArray(unmaintainedWarnings)) {
  throw new Error("cargo audit returned invalid unmaintained warnings");
}
const unmaintained = unmaintainedWarnings ?? [];
const warningAdvisories = Object.entries(report.warnings).flatMap(([kind, entries]) => {
  if (kind === "unmaintained") return [];
  if (!Array.isArray(entries)) throw new Error(`cargo audit returned invalid ${kind} warnings`);
  for (const entry of entries) {
    if (typeof entry?.advisory?.id !== "string") {
      throw new Error(`cargo audit returned a ${kind} warning without an advisory identifier`);
    }
  }
  return entries;
});
const advisories = [...vulnerabilities, ...warningAdvisories];

const exceptions = loadDependencyExceptions().filter((exception) => exception.ecosystem === "rust");
const allowed = new Set(exceptions.map((exception) => exception.id));
const observed = new Set(advisories.map((entry) => entry?.advisory?.id));
const unapproved = advisories.filter((entry) => !allowed.has(entry?.advisory?.id));
const unused = exceptions.filter((exception) => !observed.has(exception.id));

if (unapproved.length > 0) {
  for (const entry of unapproved) {
    console.error(
      `unapproved Rust advisory: ${entry?.advisory?.id ?? "unknown"} in ${entry?.package?.name ?? "unknown package"}`
    );
  }
  process.exit(1);
}
if (unused.length > 0) {
  throw new Error(`unused Rust dependency exception(s): ${unused.map(({ id }) => id).join(", ")}`);
}
if (audit.exitCode !== 0 && advisories.length === 0) {
  throw new Error(`cargo audit failed${stderr ? `: ${stderr}` : ""}`);
}

console.log(
  `Rust audit passed with ${advisories.length} exact, expiring exception(s); ` +
    `${unmaintained.length} unmaintained-package notice(s) reported separately`
);
