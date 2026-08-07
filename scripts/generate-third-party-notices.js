import {
  existsSync,
  readFileSync,
  readdirSync,
  statSync,
  writeFileSync
} from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const outputPath = resolve(root, "THIRD_PARTY_NOTICES.txt");
const checkOnly = process.argv.slice(2).includes("--check");
const licenseName = /^(licen[cs]e|copying|notice|copyright)([._-].*)?$/i;
const decoder = new TextDecoder();
const compareText = (left, right) => (left < right ? -1 : left > right ? 1 : 0);

function normalizeText(text) {
  return text.replaceAll("\r\n", "\n").trim() + "\n";
}

function licenseFiles(directory, declaredFile) {
  const paths = new Set();
  if (declaredFile) paths.add(resolve(directory, declaredFile));
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.isFile() && licenseName.test(entry.name)) paths.add(join(directory, entry.name));
  }
  return [...paths].filter((path) => existsSync(path) && statSync(path).isFile()).sort();
}

const cargo = Bun.spawnSync(
  ["cargo", "metadata", "--manifest-path", "src-tauri/Cargo.toml", "--locked", "--format-version", "1"],
  { cwd: root, stdout: "pipe", stderr: "pipe" }
);
if (cargo.exitCode !== 0) {
  throw new Error(`cargo metadata failed:\n${decoder.decode(cargo.stderr)}`);
}
const cargoMetadata = JSON.parse(decoder.decode(cargo.stdout));

const components = [];
const cargoPackages = cargoMetadata.packages
  .filter((pkg) => !(pkg.name === "unfocus" && pkg.source === null))
  .map((pkg) => ({
    pkg,
    files: licenseFiles(dirname(pkg.manifest_path), pkg.license_file)
  }));

function standardLicenseText(spdxIdentifier, contentPattern) {
  const candidate = cargoPackages
    .flatMap((record) => record.files)
    .sort()
    .find((path) => {
      const filename = path.split(/[\\/]/).at(-1) ?? "";
      if (spdxIdentifier === "Apache-2.0" && /license[-_.]?apache/i.test(filename)) return true;
      return contentPattern.test(readFileSync(path, "utf8"));
    });
  if (!candidate) throw new Error(`no locked dependency supplied canonical ${spdxIdentifier} text`);
  return candidate;
}

const apacheLicense = standardLicenseText("Apache-2.0", /Apache License\s+Version 2\.0/i);
const mplLicense = standardLicenseText("MPL-2.0", /Mozilla Public License Version 2\.0/i);
const repositoryMitFallbacks = new Map([
  ["https://github.com/madsmtm/objc2", join(root, "scripts", "license-fallbacks", "objc2-mit.txt")],
  ["https://github.com/OpenByteDev/dlopen2", join(root, "scripts", "license-fallbacks", "dlopen2-mit.txt")],
  ["https://github.com/wravery/webview2-rs", join(root, "scripts", "license-fallbacks", "webview2-rs-mit.txt")]
]);

for (const record of cargoPackages) {
  const { pkg } = record;
  let { files } = record;
  if (files.length === 0 && pkg.repository) {
    const sibling = cargoPackages
      .filter(
        (candidate) =>
          candidate.files.length > 0 && candidate.pkg.repository === pkg.repository
      )
      .sort((left, right) => compareText(left.pkg.name, right.pkg.name))[0];
    files = sibling?.files ?? [];
  }
  if (files.length === 0 && pkg.license === "MIT" && repositoryMitFallbacks.has(pkg.repository)) {
    files = [repositoryMitFallbacks.get(pkg.repository)];
  }
  if (files.length === 0 && pkg.license?.includes("Apache-2.0")) files = [apacheLicense];
  if (files.length === 0 && pkg.license === "MPL-2.0") files = [mplLicense];
  if (files.length === 0) {
    throw new Error(
      `no license text found for Rust package ${pkg.name}@${pkg.version} or a locked package from ${pkg.repository ?? "its upstream repository"}`
    );
  }
  components.push({
    ecosystem: "Rust",
    name: pkg.name,
    version: pkg.version,
    declared: pkg.license ?? `file: ${pkg.license_file}`,
    authors: pkg.authors,
    files
  });
}

const installedNodePackages = new Map();
const nodeLicenseFallbacks = new Map([
  ["@polka/url", join(root, "scripts", "license-fallbacks", "sirv-mit.txt")],
  ["bun-types", join(root, "scripts", "license-fallbacks", "bun-types-mit.txt")],
  ["is-reference", join(root, "scripts", "license-fallbacks", "rich-harris-mit.txt")],
  ["locate-character", join(root, "scripts", "license-fallbacks", "rich-harris-mit.txt")],
  ["sirv", join(root, "scripts", "license-fallbacks", "sirv-mit.txt")]
]);

function packageAuthors(manifest) {
  const values = [manifest.author, ...(Array.isArray(manifest.contributors) ? manifest.contributors : [])];
  return values.filter(Boolean).map((value) => {
    if (typeof value === "string") return value;
    if (typeof value === "object" && typeof value.name === "string") return value.name;
    return String(value);
  });
}

function collectNodeModules(nodeModulesDirectory) {
  if (!existsSync(nodeModulesDirectory)) return;
  for (const entry of readdirSync(nodeModulesDirectory, { withFileTypes: true })) {
    if (entry.name.startsWith(".")) continue;
    const entryPath = join(nodeModulesDirectory, entry.name);
    if (entry.name.startsWith("@")) {
      for (const scoped of readdirSync(entryPath, { withFileTypes: true })) {
        if (scoped.isDirectory() || scoped.isSymbolicLink()) collectNodePackage(join(entryPath, scoped.name));
      }
    } else if (entry.isDirectory() || entry.isSymbolicLink()) {
      collectNodePackage(entryPath);
    }
  }
}

function collectNodePackage(directory) {
  const manifestPath = join(directory, "package.json");
  if (!existsSync(manifestPath)) return;
  const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
  if (typeof manifest.name !== "string" || typeof manifest.version !== "string") return;
  const identifier = `${manifest.name}@${manifest.version}`;
  if (!installedNodePackages.has(identifier)) installedNodePackages.set(identifier, { directory, manifest });
  collectNodeModules(join(directory, "node_modules"));
}

collectNodeModules(join(root, "node_modules"));

const bunLock = (await import(pathToFileURL(join(root, "bun.lock")).href)).default;
for (const entry of Object.values(bunLock.packages)) {
  if (!Array.isArray(entry) || typeof entry[0] !== "string") {
    throw new Error("bun.lock contains an invalid package entry");
  }
  const separator = entry[0].lastIndexOf("@");
  if (separator <= 0 || separator === entry[0].length - 1) {
    throw new Error(`could not parse bun.lock package identifier: ${entry[0]}`);
  }
  const name = entry[0].slice(0, separator);
  const version = entry[0].slice(separator + 1);
  const identifier = `${name}@${version}`;
  const installed = installedNodePackages.get(identifier);
  const manifest = installed?.manifest;
  let files = installed ? licenseFiles(installed.directory, manifest.licenseFile) : [];

  if (files.length === 0 && nodeLicenseFallbacks.has(name)) {
    files = [nodeLicenseFallbacks.get(name)];
  }
  if (files.length === 0 && name.startsWith("@esbuild/")) {
    files = licenseFiles(join(root, "node_modules", "esbuild"));
  }
  if (files.length === 0 && name.startsWith("@rollup/")) {
    files = licenseFiles(join(root, "node_modules", "rollup"));
  }
  if (files.length === 0 && name.startsWith("@tauri-apps/cli-")) {
    files = licenseFiles(join(root, "node_modules", "@tauri-apps", "cli"));
  }
  if (files.length === 0 && name.startsWith("@napi-rs/lzma-")) {
    files = [join(root, "scripts", "license-fallbacks", "napi-lzma-mit.txt")];
  }
  if (files.length === 0 && name === "fsevents") {
    files = [join(root, "scripts", "license-fallbacks", "fsevents-mit.txt")];
  }
  if (files.length === 0) throw new Error(`no license text found for JavaScript package ${identifier}`);

  const platformFallback = name.startsWith("@esbuild/")
    ? { license: "MIT", authors: ["Evan Wallace and esbuild contributors"] }
    : name.startsWith("@rollup/")
      ? { license: "MIT", authors: ["Lukas Taegert-Atkinson"] }
      : name.startsWith("@tauri-apps/cli-")
        ? { license: "Apache-2.0 OR MIT", authors: ["Tauri Programme within The Commons Conservancy"] }
        : name.startsWith("@napi-rs/lzma-")
          ? { license: "MIT", authors: ["Brooooooklyn/lzma contributors"] }
          : name === "fsevents"
            ? {
                license: "MIT",
                authors: ["Philipp Dunkel", "Ben Noordhuis", "Elan Shankar", "Miroslav Bajtoš", "Paul Miller"]
              }
            : null;
  components.push({
    ecosystem: "JavaScript",
    name,
    version,
    declared:
      platformFallback
        ? platformFallback.license
        : typeof manifest?.license === "string"
        ? manifest.license
        : JSON.stringify(manifest?.license ?? "unspecified"),
    authors: platformFallback?.authors ?? (manifest ? packageAuthors(manifest) : []),
    files
  });
}

const fontLicense = join(root, "static", "fonts", "OFL.txt");
components.push({
  ecosystem: "Vendored asset",
  name: "Fraunces",
  version: "variable font",
  declared: "OFL-1.1",
  authors: ["The Fraunces Project Authors"],
  files: [fontLicense]
});

components.sort((left, right) =>
  compareText(
    `${left.ecosystem}:${left.name}@${left.version}`,
    `${right.ecosystem}:${right.name}@${right.version}`
  )
);

const texts = new Map();
for (const component of components) {
  component.hashes = [];
  for (const path of component.files) {
    const text = normalizeText(readFileSync(path, "utf8"));
    const hash = new Bun.CryptoHasher("sha256").update(text).digest("hex");
    component.hashes.push(hash);
    const record = texts.get(hash) ?? { text, usedBy: new Set() };
    record.usedBy.add(`${component.ecosystem}: ${component.name}@${component.version}`);
    texts.set(hash, record);
  }
  component.hashes.sort();
}

let output = `THIRD-PARTY SOFTWARE NOTICES
================================

This file is generated from Cargo metadata, the installed Bun dependency tree,
and vendored assets. It is intentionally inclusive of build dependencies.
Regenerate it with: bun run notices:generate

PACKAGE INVENTORY
-----------------

`;
for (const component of components) {
  output += `${component.ecosystem}: ${component.name}@${component.version}\n`;
  output += `Declared license: ${component.declared}\n`;
  if (component.authors?.length) output += `Upstream authors: ${component.authors.join(", ")}\n`;
  output += `License text IDs: ${component.hashes.join(", ")}\n\n`;
}

output += `LICENSE AND NOTICE TEXTS
------------------------

`;
for (const [hash, record] of [...texts].sort(([left], [right]) => compareText(left, right))) {
  output += `===============================================================================\n`;
  output += `License text ID: ${hash}\n`;
  output += `Used by:\n`;
  for (const identifier of [...record.usedBy].sort(compareText)) output += `  - ${identifier}\n`;
  output += `-------------------------------------------------------------------------------\n`;
  output += record.text + "\n";
}

if (checkOnly) {
  if (!existsSync(outputPath) || readFileSync(outputPath, "utf8") !== output) {
    console.error("THIRD_PARTY_NOTICES.txt is stale; run bun run notices:generate");
    process.exit(1);
  }
  console.log(`third-party notices are current (${components.length} packages/assets)`);
} else {
  writeFileSync(outputPath, output);
  console.log(`wrote THIRD_PARTY_NOTICES.txt (${components.length} packages/assets)`);
}
