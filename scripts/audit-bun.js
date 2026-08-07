import { loadDependencyExceptions } from "./check-dependency-exceptions.js";
import { fileURLToPath } from "node:url";

const decoder = new TextDecoder();
const audit = Bun.spawnSync(["bun", "audit", "--json"], {
  cwd: fileURLToPath(new URL("..", import.meta.url)),
  stdout: "pipe",
  stderr: "pipe"
});
const stdout = decoder.decode(audit.stdout).trim();
const stderr = decoder.decode(audit.stderr).trim();

let report;
try {
  report = JSON.parse(stdout);
} catch {
  throw new Error(`bun audit did not return JSON${stderr ? `: ${stderr}` : ""}`);
}

// A scalar, null, or array report would iterate as zero advisories and pass
// silently. Only a package-keyed object can be audited.
if (report === null || typeof report !== "object" || Array.isArray(report)) {
  throw new Error(`bun audit did not return an advisory object${stderr ? `: ${stderr}` : ""}`);
}

const advisories = Object.entries(report).flatMap(([packageName, entries]) => {
  if (!Array.isArray(entries)) throw new Error(`bun audit returned invalid entries for ${packageName}`);
  return entries.map((entry) => {
    const match = typeof entry.url === "string" ? entry.url.match(/\/(GHSA-[\w-]+)$/) : null;
    if (!match) throw new Error(`bun audit returned an advisory without a GHSA identifier`);
    return { ...entry, packageName, advisoryId: match[1] };
  });
});

const exceptions = loadDependencyExceptions().filter(
  (exception) => exception.ecosystem === "javascript"
);
const allowed = new Set(exceptions.map((exception) => exception.id));
const observed = new Set(advisories.map((advisory) => advisory.advisoryId));
const unapproved = advisories.filter((advisory) => !allowed.has(advisory.advisoryId));
const unused = exceptions.filter((exception) => !observed.has(exception.id));

if (unapproved.length > 0) {
  for (const advisory of unapproved) {
    console.error(
      `unapproved Bun advisory: ${advisory.advisoryId} (${advisory.severity}) in ${advisory.packageName}`
    );
  }
  process.exit(1);
}
if (unused.length > 0) {
  throw new Error(`unused JavaScript dependency exception(s): ${unused.map(({ id }) => id).join(", ")}`);
}
if (audit.exitCode !== 0 && advisories.length === 0) {
  throw new Error(`bun audit failed${stderr ? `: ${stderr}` : ""}`);
}

console.log(`Bun audit passed with ${advisories.length} exact, expiring exception(s)`);
