import { createHash } from "node:crypto";
import { lstatSync, readFileSync, readdirSync, realpathSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { gunzipSync } from "node:zlib";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export const glibBackport = Object.freeze({
  name: "glib",
  version: "0.18.5",
  advisory: "RUSTSEC-2024-0429",
  archive: "scripts/vendor-sources/glib-0.18.5.crate",
  archiveUrl: "https://static.crates.io/crates/glib/glib-0.18.5.crate",
  archiveSha256: "233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5",
  patch: "scripts/vendor-sources/glib-variant-str-iter.patch",
  patchSha256: "a742bf59f1b00af0f434623b34e2fd969dc8697d37c796efdf909dd9eb19b854",
  upstreamCommit: "b5a4071e439bef2b5eea76c3aa25e5ae84839e34",
  upstreamUrl: "https://github.com/gtk-rs/gtk-rs-core/commit/b5a4071e439bef2b5eea76c3aa25e5ae84839e34",
  vendor: "src-tauri/vendor/glib-0.18.5"
});
const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");

function regularFile(path) {
  const stat = lstatSync(path);
  if (stat.isSymbolicLink() || !stat.isFile()) throw new Error(`expected regular file, no symlink: ${path}`);
  return readFileSync(path);
}

// Only parse the archive after matching its entire published checksum. All 121
// members of this specific crates.io tarball are regular files, with short names.
function originalFiles(archive) {
  const tar = gunzipSync(archive);
  const files = new Map();
  for (let offset = 0; offset + 512 <= tar.length;) {
    const header = tar.subarray(offset, offset + 512);
    if (header.every((byte) => byte === 0)) break;
    const name = header.subarray(0, 100).toString().split("\0")[0];
    const size = Number.parseInt(header.subarray(124, 136).toString().replace(/\0/g, "").trim(), 8);
    if (!name.startsWith("glib-0.18.5/") || name.includes("..") || ![0, 48].includes(header[156]) || !Number.isSafeInteger(size) || size < 0 || offset + 512 + size > tar.length) {
      throw new Error("unexpected original GLib archive member");
    }
    const path = name.slice("glib-0.18.5/".length);
    if (files.has(path)) throw new Error("duplicate original GLib archive member");
    files.set(path, tar.subarray(offset + 512, offset + 512 + size));
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  if (files.size !== 121) throw new Error("unexpected original GLib archive inventory");
  return files;
}

function vendorFiles(directory, prefix = "") {
  if (lstatSync(directory).isSymbolicLink()) throw new Error(`vendor directory is a symlink: ${directory}`);
  const files = [];
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    const name = prefix + entry.name;
    if (entry.isSymbolicLink()) throw new Error(`vendor file is a symlink: ${name}`);
    if (entry.isDirectory()) files.push(...vendorFiles(path, name + "/"));
    else if (entry.isFile()) files.push(name);
    else throw new Error(`unexpected vendor file type: ${name}`);
  }
  return files.sort();
}

export function verifyVendoredGlib(root = repositoryRoot) {
  const archive = regularFile(join(root, glibBackport.archive));
  if (digest(archive) !== glibBackport.archiveSha256) throw new Error("GLib archive checksum mismatch");
  if (digest(regularFile(join(root, glibBackport.patch))) !== glibBackport.patchSha256) {
    throw new Error("GLib upstream patch checksum mismatch");
  }
  const expected = originalFiles(archive);
  const sourcePath = "src/variant_iter.rs";
  const source = expected.get(sourcePath).toString();
  const before = ["            let p: *mut libc::c_char = std::ptr::null_mut();", "                &p,"];
  const after = ["            let mut p: *mut libc::c_char = std::ptr::null_mut();", "                &mut p,"];
  let patched = source;
  for (let i = 0; i < before.length; i++) {
    if (patched.split(before[i]).length !== 2) throw new Error("unexpected original GLib pointer implementation");
    patched = patched.replace(before[i], after[i]);
  }
  expected.set(sourcePath, Buffer.from(patched));
  const vendor = join(root, glibBackport.vendor);
  // Parent indirection would otherwise bypass the recursive directory checks.
  for (const path of [join(root, "src-tauri"), join(root, "src-tauri/vendor")]) {
    if (lstatSync(path).isSymbolicLink()) throw new Error(`vendor parent is a symlink: ${path}`);
  }
  const names = [...expected.keys()].sort();
  if (JSON.stringify(vendorFiles(vendor)) !== JSON.stringify(names)) throw new Error("GLib vendor file inventory mismatch");
  for (const path of names) {
    if (!regularFile(join(vendor, path)).equals(expected.get(path))) throw new Error(`GLib vendor file differs: ${path}`);
  }
  // Explicit, cross-platform digest format: JSON array of sorted [relative path,
  // SHA-256 of exact file bytes] pairs, UTF-8, without a trailing newline.
  const sha256 = digest(JSON.stringify(names.map((path) => [path, digest(expected.get(path))])));
  const provenance = { ...glibBackport, treeDigestFormat: "sha256-json-sorted-path-file-sha256-v1", treeSha256: sha256, files: names.length };
  const recorded = JSON.parse(regularFile(join(root, "scripts/vendor-sources/glib-0.18.5.json")));
  if (JSON.stringify(recorded) !== JSON.stringify(provenance)) throw new Error("GLib provenance mismatch");
  const manifest = Bun.TOML.parse(regularFile(join(root, "src-tauri/Cargo.toml")).toString());
  const patch = manifest.patch?.["crates-io"]?.glib;
  if (JSON.stringify(patch) !== JSON.stringify({ path: "vendor/glib-0.18.5" })) throw new Error("GLib Cargo patch must select the verified vendor path");
  const lock = Bun.TOML.parse(regularFile(join(root, "src-tauri/Cargo.lock")).toString());
  const packages = lock.package?.filter((pkg) => pkg.name === "glib") ?? [];
  if (packages.length !== 1 || packages[0].version !== "0.18.5" || packages[0].source !== undefined || packages[0].checksum !== undefined) {
    throw new Error("locked GLib must be the single verified path package glib 0.18.5");
  }
  const auditLockPath = join(root, "scripts/vendor-sources/glib-0.18.5.audit.lock");
  const auditLock = Bun.TOML.parse(regularFile(auditLockPath).toString());
  const auditPackages = auditLock.package?.filter((pkg) => pkg.name === "glib") ?? [];
  if (auditPackages.length !== 1 || auditPackages[0].version !== "0.18.5" || auditPackages[0].source !== "registry+https://github.com/rust-lang/crates.io-index" || auditPackages[0].checksum !== glibBackport.archiveSha256) {
    throw new Error("registry audit GLib must retain the authenticated crates.io identity");
  }
  return { ...provenance, sha256, manifestPath: resolve(vendor, "Cargo.toml"), auditLockPath };
}

export function verifyResolvedGlib(metadata, verified) {
  const packages = metadata?.packages?.filter((pkg) => pkg.name === "glib") ?? [];
  const pkg = packages[0];
  let manifestPath;
  try {
    manifestPath = realpathSync(pkg?.manifest_path);
  } catch {
    throw new Error("resolved GLib source has no readable vendor manifest");
  }
  if (packages.length !== 1 || pkg.source !== null || pkg.version !== verified.version || manifestPath !== realpathSync(verified.manifestPath)) {
    throw new Error("resolved GLib source must select the verified vendor manifest");
  }
  return verified;
}

export function resolveVerifiedGlib(root = repositoryRoot) {
  const verified = verifyVendoredGlib(root);
  const result = Bun.spawnSync([
    "cargo", "metadata", "--manifest-path", join(root, "src-tauri/Cargo.toml"),
    "--locked", "--all-features", "--format-version", "1"
  ], { cwd: root, stdout: "pipe", stderr: "pipe" });
  if (result.exitCode !== 0) throw new Error(`cargo metadata failed: ${new TextDecoder().decode(result.stderr)}`);
  return verifyResolvedGlib(JSON.parse(new TextDecoder().decode(result.stdout)), verified);
}

if (import.meta.main) {
  const result = resolveVerifiedGlib();
  console.log(`verified ${result.name} ${result.version}: ${result.advisory} backport, ${result.files} files, tree SHA-256 ${result.sha256}`);
}
