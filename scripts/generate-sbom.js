import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { resolveCargoPackageMetadata } from "./cargo-package-metadata.js";
import { resolveBunDependency } from "./sbom-dependencies.js";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const output = process.argv[2] ? resolve(root, process.argv[2]) : resolve(root, "unfocus.cdx.json");
const decoder = new TextDecoder();
const compareText = (left, right) => (left < right ? -1 : left > right ? 1 : 0);

const cargo = Bun.spawnSync(
  ["cargo", "metadata", "--manifest-path", "src-tauri/Cargo.toml", "--locked", "--format-version", "1"],
  { cwd: root, stdout: "pipe", stderr: "pipe" }
);
if (cargo.exitCode !== 0) throw new Error(`cargo metadata failed:\n${decoder.decode(cargo.stderr)}`);
const cargoMetadata = JSON.parse(decoder.decode(cargo.stdout));
const application = cargoMetadata.packages.find(
  (pkg) => pkg.name === "unfocus" && pkg.source === null
);
if (!application) throw new Error("cargo metadata did not contain the Unfocus package");
const rootRef = `pkg:generic/unfocus@${application.version}`;

const installedNodePackages = new Map();
function collectNodeModules(directory) {
  if (!existsSync(directory)) return;
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (entry.name.startsWith(".")) continue;
    const path = join(directory, entry.name);
    if (entry.name.startsWith("@")) {
      for (const scoped of readdirSync(path, { withFileTypes: true })) {
        if (scoped.isDirectory() || scoped.isSymbolicLink()) collectNodePackage(join(path, scoped.name));
      }
    } else if (entry.isDirectory() || entry.isSymbolicLink()) {
      collectNodePackage(path);
    }
  }
}

function collectNodePackage(directory) {
  const path = join(directory, "package.json");
  if (!existsSync(path)) return;
  const manifest = JSON.parse(readFileSync(path, "utf8"));
  if (typeof manifest.name !== "string" || typeof manifest.version !== "string") return;
  installedNodePackages.set(`${manifest.name}@${manifest.version}`, manifest);
  collectNodeModules(join(directory, "node_modules"));
}

collectNodeModules(join(root, "node_modules"));

function npmPurl(name, version) {
  const encodedName = name.split("/").map(encodeURIComponent).join("/");
  return `pkg:npm/${encodedName}@${encodeURIComponent(version)}`;
}

function licenseEntry(declared) {
  return declared ? [{ license: { name: declared } }] : undefined;
}

const components = [];
const cargoRefsById = new Map([[application.id, rootRef]]);
for (const pkg of cargoMetadata.packages) {
  if (pkg.id === application.id) continue;
  const metadata = resolveCargoPackageMetadata(pkg);
  const sourceQualifier = metadata.lockedSource?.startsWith("git+")
    ? `?vcs_url=${encodeURIComponent(metadata.lockedSource)}`
    : "";
  const purl =
    `pkg:cargo/${encodeURIComponent(pkg.name)}@${encodeURIComponent(pkg.version)}` +
    sourceQualifier;
  cargoRefsById.set(pkg.id, purl);
  components.push({
    type: "library",
    "bom-ref": purl,
    name: pkg.name,
    version: pkg.version,
    purl,
    licenses: licenseEntry(metadata.declaredLicense),
    externalReferences: metadata.repository
      ? [{ type: "vcs", url: metadata.repository }]
      : undefined,
    properties: [
      { name: "unfocus:lockfile", value: "src-tauri/Cargo.lock" },
      ...(metadata.lockedSource
        ? [{ name: "unfocus:cargo-source", value: metadata.lockedSource }]
        : [])
    ]
  });
}

const bunLock = (await import(pathToFileURL(join(root, "bun.lock")).href)).default;
const bunPackages = [];
const bunPackagesByName = new Map();
const bunPackagesByLockKey = new Map();
for (const [lockKey, entry] of Object.entries(bunLock.packages)) {
  if (!Array.isArray(entry) || typeof entry[0] !== "string") {
    throw new Error("bun.lock contains an invalid package entry");
  }
  const separator = entry[0].lastIndexOf("@");
  if (separator <= 0 || separator === entry[0].length - 1) {
    throw new Error(`could not parse bun.lock package identifier: ${entry[0]}`);
  }
  const name = entry[0].slice(0, separator);
  const version = entry[0].slice(separator + 1);
  const manifest = installedNodePackages.get(`${name}@${version}`);
  const platformLicense =
    name.startsWith("@esbuild/") ||
    name.startsWith("@rollup/") ||
    name.startsWith("@rolldown/binding-") ||
    name === "fsevents"
    ? "MIT"
    : name.startsWith("@typescript/typescript-")
      ? "Apache-2.0"
      : name.startsWith("lightningcss-")
        ? "MPL-2.0"
        : name.startsWith("@tauri-apps/cli-")
          ? "Apache-2.0 OR MIT"
          : name.startsWith("@napi-rs/lzma-")
            ? "MIT"
            : null;
  const purl = npmPurl(name, version);
  const integrity = typeof entry[3] === "string" ? entry[3].match(/^sha512-(.+)$/) : null;
  const metadata = entry[2] ?? {};
  if (!metadata || typeof metadata !== "object" || Array.isArray(metadata)) {
    throw new Error(`bun.lock contains invalid metadata for ${entry[0]}`);
  }
  // An absent or incomplete node_modules leaves `manifest` undefined, which
  // would emit a component with no `licenses` and silently drop license data
  // from the SBOM. Refuse to publish an SBOM that cannot see what it declares.
  const declaredLicense =
    typeof manifest?.license === "string" ? manifest.license : platformLicense;
  if (typeof declaredLicense !== "string" || declaredLicense.trim() === "") {
    throw new Error(
      `no license metadata for ${name}@${version}; run \`bun install\` before generating the SBOM`
    );
  }

  components.push({
    type: "library",
    "bom-ref": purl,
    name,
    version,
    purl,
    licenses: licenseEntry(declaredLicense),
    hashes: integrity
      ? [{ alg: "SHA-512", content: Buffer.from(integrity[1], "base64").toString("hex") }]
      : undefined,
    properties: [{ name: "unfocus:lockfile", value: "bun.lock" }]
  });
  const record = { lockKey, name, version, ref: purl, metadata };
  bunPackages.push(record);
  if (bunPackagesByLockKey.has(lockKey)) {
    throw new Error(`bun.lock contains a duplicate package key: ${lockKey}`);
  }
  bunPackagesByLockKey.set(lockKey, record);
  const versions = bunPackagesByName.get(name) ?? [];
  versions.push(record);
  bunPackagesByName.set(name, versions);
}

const fontRef = "pkg:generic/Fraunces@variable-font";
const fontPath = join(root, "static/fonts/fraunces.woff2");
const fontHash = createHash("sha256").update(readFileSync(fontPath)).digest("hex");
components.push({
  type: "file",
  "bom-ref": fontRef,
  name: "Fraunces",
  version: "variable-font",
  purl: fontRef,
  licenses: [{ license: { id: "OFL-1.1" } }],
  hashes: [{ alg: "SHA-256", content: fontHash }],
  properties: [{ name: "unfocus:source", value: "static/fonts/fraunces.woff2" }]
});

components.sort((left, right) => compareText(left["bom-ref"], right["bom-ref"]));
const componentRefs = new Set();
for (const component of components) {
  const ref = component["bom-ref"];
  if (componentRefs.has(ref)) throw new Error(`duplicate component bom-ref: ${ref}`);
  componentRefs.add(ref);
}

const dependencyGraph = new Map();
const cargoResolveNodes = cargoMetadata.resolve?.nodes;
if (!Array.isArray(cargoResolveNodes)) {
  throw new Error("cargo metadata did not contain a resolved dependency graph");
}
for (const node of cargoResolveNodes) {
  const ref = cargoRefsById.get(node.id);
  if (!ref) throw new Error(`cargo resolve contains an unknown package: ${node.id}`);
  if (dependencyGraph.has(ref)) throw new Error(`cargo resolve contains a duplicate node: ${node.id}`);
  if (!Array.isArray(node.deps)) throw new Error(`cargo resolve node has invalid dependencies: ${node.id}`);
  const dependencies = new Set();
  for (const dependency of node.deps) {
    const dependencyRef = cargoRefsById.get(dependency.pkg);
    if (!dependencyRef) {
      throw new Error(`cargo dependency ${dependency.pkg} from ${node.id} has no component`);
    }
    dependencies.add(dependencyRef);
  }
  dependencyGraph.set(ref, dependencies);
}
for (const [id, ref] of cargoRefsById) {
  if (!dependencyGraph.has(ref)) throw new Error(`cargo package is absent from the resolve graph: ${id}`);
}

function dependencyEntries(manifest, field, owner) {
  const value = manifest[field];
  if (value === undefined) return [];
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`${owner} has an invalid ${field} map in bun.lock`);
  }
  return Object.entries(value).sort(([left], [right]) => compareText(left, right));
}

const bunPackageIndex = { byName: bunPackagesByName, byLockKey: bunPackagesByLockKey };

function collectBunDependencies(owner, manifest, includeDevDependencies = false) {
  const dependencies = new Set();
  const optionalPeersValue = manifest.optionalPeers ?? [];
  if (!Array.isArray(optionalPeersValue) || optionalPeersValue.some((name) => typeof name !== "string")) {
    throw new Error(`${owner} has an invalid optionalPeers list in bun.lock`);
  }
  const optionalPeers = new Set(optionalPeersValue);
  const peerNames = new Set(dependencyEntries(manifest, "peerDependencies", owner).map(([name]) => name));
  for (const name of optionalPeers) {
    if (!peerNames.has(name)) throw new Error(`${owner} marks undeclared peer ${name} as optional`);
  }

  const fields = [
    ["dependencies", false],
    ["optionalDependencies", true],
    ["peerDependencies", null]
  ];
  if (includeDevDependencies) fields.push(["devDependencies", false]);
  for (const [field, optional] of fields) {
    for (const [name, range] of dependencyEntries(manifest, field, owner)) {
      const ref = resolveBunDependency(
        bunPackageIndex,
        owner,
        name,
        range,
        optional === null ? optionalPeers.has(name) : optional
      );
      if (ref) dependencies.add(ref);
    }
  }
  return dependencies;
}

for (const pkg of bunPackages) {
  if (dependencyGraph.has(pkg.ref)) throw new Error(`duplicate dependency graph ref: ${pkg.ref}`);
  dependencyGraph.set(pkg.ref, collectBunDependencies(pkg.lockKey, pkg.metadata));
}

const rootWorkspace = bunLock.workspaces?.[""];
if (!rootWorkspace || typeof rootWorkspace !== "object" || Array.isArray(rootWorkspace)) {
  throw new Error("bun.lock does not contain the root workspace");
}
const rootDependencies = dependencyGraph.get(rootRef);
if (!rootDependencies) throw new Error("cargo resolve does not contain the Unfocus root");
for (const ref of collectBunDependencies("root workspace", rootWorkspace, true)) {
  rootDependencies.add(ref);
}
rootDependencies.add(fontRef);
dependencyGraph.set(fontRef, new Set());

const knownRefs = new Set([rootRef, ...componentRefs]);
for (const ref of knownRefs) {
  if (!dependencyGraph.has(ref)) throw new Error(`component is absent from the dependency graph: ${ref}`);
}
for (const [ref, dependencies] of dependencyGraph) {
  if (!knownRefs.has(ref)) throw new Error(`dependency graph contains an unknown ref: ${ref}`);
  for (const dependencyRef of dependencies) {
    if (!knownRefs.has(dependencyRef)) {
      throw new Error(`dependency graph contains dangling ref ${dependencyRef} from ${ref}`);
    }
    if (dependencyRef === ref) throw new Error(`dependency graph contains a self-reference: ${ref}`);
  }
}

const reachable = new Set([rootRef]);
const pending = [rootRef];
while (pending.length > 0) {
  const ref = pending.pop();
  for (const dependencyRef of dependencyGraph.get(ref)) {
    if (reachable.has(dependencyRef)) continue;
    reachable.add(dependencyRef);
    pending.push(dependencyRef);
  }
}
const unreachable = [...componentRefs].filter((ref) => !reachable.has(ref));
if (unreachable.length > 0) {
  throw new Error(`components are unreachable from the root: ${unreachable.join(", ")}`);
}

const orderedDependencyRefs = [rootRef, ...components.map((component) => component["bom-ref"])];
const dependencies = orderedDependencyRefs.map((ref) => ({
  ref,
  dependsOn: [...dependencyGraph.get(ref)].sort(compareText)
}));
const document = {
  bomFormat: "CycloneDX",
  specVersion: "1.6",
  version: 1,
  metadata: {
    component: {
      type: "application",
      "bom-ref": rootRef,
      name: "Unfocus",
      version: application.version,
      purl: rootRef,
      licenses: [{ license: { id: "MIT" } }]
    }
  },
  components,
  dependencies
};

mkdirSync(dirname(output), { recursive: true });
writeFileSync(output, JSON.stringify(document, null, 2) + "\n");
console.log(`wrote ${output} (${components.length} locked components)`);
