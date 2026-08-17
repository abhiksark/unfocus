// scripts/cargo-package-metadata.js

const licenseFallbacks = new Map([
  [
    "git+https://github.com/ahkohd/tauri-nspanel.git?rev=a3122e894383aa068ec5365a42994e3ac94ba1b6#a3122e894383aa068ec5365a42994e3ac94ba1b6",
    "MIT OR Apache-2.0"
  ]
]);

export function resolveCargoPackageMetadata(pkg) {
  return {
    declaredLicense:
      pkg.license ??
      (pkg.license_file ? `file: ${pkg.license_file}` : licenseFallbacks.get(pkg.source) ?? null),
    lockedSource: pkg.source ?? null,
    repository: pkg.repository ?? null
  };
}
