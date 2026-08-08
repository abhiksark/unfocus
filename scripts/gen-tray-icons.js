// scripts/gen-tray-icons.js — renders the tray icon PNGs from unfocus-tray.svg.
// The committed PNGs are generated artifacts: regenerate with
// `bun run tray:generate`, never edit them by hand. The light variant is the
// same glyph with fill="#000" recoloured to fill="#fff", which is why the
// source SVG must express everything as plainly filled paths.

import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const trayDir = join(repoRoot, "src-tauri", "icons", "tray");
const sourcePath = join(trayDir, "unfocus-tray.svg");
const SIZE = 32;

try {
  execFileSync("rsvg-convert", ["--version"], { stdio: "ignore" });
} catch {
  console.error(
    "rsvg-convert not found. Install librsvg first: `brew install librsvg` (macOS) or `apt install librsvg2-bin` (Debian/Ubuntu).",
  );
  process.exit(1);
}

const source = readFileSync(sourcePath, "utf8");
if (!source.includes('fill="#000"')) {
  console.error(`${sourcePath} must draw with fill="#000" so the light variant can be derived.`);
  process.exit(1);
}
if (source.includes("<mask")) {
  console.error(`${sourcePath} must not use <mask>: recolouring would corrupt mask luminance values.`);
  process.exit(1);
}

function render(svgPath, outName) {
  const outPath = join(trayDir, outName);
  execFileSync("rsvg-convert", ["-w", String(SIZE), "-h", String(SIZE), "-o", outPath, svgPath]);
  console.log(`wrote ${outPath}`);
}

render(sourcePath, "tray-template.png");

const tempDir = mkdtempSync(join(tmpdir(), "unfocus-tray-"));
try {
  const lightSvgPath = join(tempDir, "unfocus-tray-light.svg");
  writeFileSync(lightSvgPath, source.replaceAll('fill="#000"', 'fill="#fff"'));
  render(lightSvgPath, "tray-light.png");
} finally {
  rmSync(tempDir, { recursive: true, force: true });
}
