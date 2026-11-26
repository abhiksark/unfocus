// scripts/cargo-package-metadata.js
import { resolve } from "node:path";

const licenseFallbacks = new Map([
  [
    "git+https://github.com/ahkohd/tauri-nspanel.git?rev=a3122e894383aa068ec5365a42994e3ac94ba1b6#a3122e894383aa068ec5365a42994e3ac94ba1b6",
    "MIT OR Apache-2.0"
  ]
]);

export function resolveCargoPackageMetadata(pkg, verifiedGlib) {
  const metadata = {
    declaredLicense:
      pkg.license ??
      (pkg.license_file ? `file: ${pkg.license_file}` : licenseFallbacks.get(pkg.source) ?? null),
    lockedSource: pkg.source ?? null,
    repository: pkg.repository ?? null
  };
  if (pkg.name === "glib" && pkg.source === null) {
    if (!verifiedGlib || pkg.version !== verifiedGlib.version || !pkg.manifest_path || resolve(pkg.manifest_path) !== verifiedGlib.manifestPath) {
      throw new Error("unverified GLib source in Cargo metadata");
    }
    metadata.lockedSource = `vendored:${verifiedGlib.vendor}#sha256=${verifiedGlib.sha256}`;
    metadata.backport = {
      advisory: verifiedGlib.advisory,
      upstreamCommit: verifiedGlib.upstreamCommit,
      upstreamUrl: verifiedGlib.upstreamUrl,
      archiveUrl: verifiedGlib.archiveUrl,
      archiveSha256: verifiedGlib.archiveSha256,
      treeSha256: verifiedGlib.sha256,
      treeDigestFormat: verifiedGlib.treeDigestFormat
    };
  }
  return metadata;
}
