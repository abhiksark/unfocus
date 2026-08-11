import { copyFileSync, existsSync, mkdirSync, readFileSync, statSync } from "node:fs";
import { basename, resolve, sep } from "node:path";
import { finalizeDebianPackage } from "./debian-package.js";

const outputDirectory = resolve(process.argv[2] ?? "release-artifacts");
const encodedPaths = process.env.TAURI_ARTIFACT_PATHS;

if (!encodedPaths) throw new Error("TAURI_ARTIFACT_PATHS is required");

const artifactPaths = JSON.parse(encodedPaths);
if (!Array.isArray(artifactPaths)) throw new Error("Tauri artifact paths must be a JSON array");

const releaseExtension = /\.(appimage|deb|dmg|exe|msi|rpm)$/i;
const selected = artifactPaths.filter((artifactPath) => {
  if (typeof artifactPath !== "string") return false;
  const resolvedPath = resolve(artifactPath);
  const pathParts = resolvedPath.split(sep);
  return pathParts.includes("bundle") && releaseExtension.test(resolvedPath);
});

if (selected.length === 0) {
  throw new Error("Tauri did not report any distributable bundle artifacts");
}

mkdirSync(outputDirectory, { recursive: true });
const copiedNames = new Set();
const canonicalVersion = JSON.parse(readFileSync(resolve("package.json"), "utf8")).version;
const artifacts = selected.map((artifactPath) => {
  const source = resolve(artifactPath);
  if (!statSync(source).isFile()) throw new Error(`release artifact is not a file: ${source}`);

  const filename = basename(source);
  if (copiedNames.has(filename)) throw new Error(`duplicate release artifact name: ${filename}`);
  if (existsSync(resolve(outputDirectory, filename))) {
    throw new Error(`release artifact destination already exists: ${filename}`);
  }
  copiedNames.add(filename);
  return { filename, source };
});

for (const { filename, source } of artifacts) {
  if (filename.toLowerCase().endsWith(".deb")) {
    finalizeDebianPackage(source, canonicalVersion);
  }
  copyFileSync(source, resolve(outputDirectory, filename));
  console.log(`collected ${filename}`);
}
