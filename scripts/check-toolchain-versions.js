import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const read = (path) => readFileSync(resolve(root, path), "utf8");
const exactVersion = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/;

const bunVersion = read(".bun-version").trim();
const packageJson = JSON.parse(read("package.json"));
if (!exactVersion.test(bunVersion)) throw new Error(`invalid .bun-version: ${bunVersion}`);
if (packageJson.packageManager !== `bun@${bunVersion}`) {
  throw new Error(`packageManager must be bun@${bunVersion}`);
}
if (packageJson.engines?.bun !== bunVersion) {
  throw new Error(`engines.bun must be ${bunVersion}`);
}

const toolchain = read("rust-toolchain.toml");
const rustMatch = toolchain.match(/^channel\s*=\s*"([^"]+)"\s*$/m);
if (!rustMatch || !exactVersion.test(rustMatch[1])) {
  throw new Error("rust-toolchain.toml must contain an exact channel version");
}
const rustVersion = rustMatch[1];

for (const workflow of [".github/workflows/ci.yml", ".github/workflows/release.yml"]) {
  const contents = read(workflow);
  const workflowVersion = contents.match(/^  RUST_VERSION:\s*([^\s#]+)\s*$/m)?.[1];
  if (workflowVersion !== rustVersion) {
    throw new Error(`${workflow} RUST_VERSION must be ${rustVersion}`);
  }
  if (/^\s*bun-version:\s*/m.test(contents)) {
    throw new Error(`${workflow} must use bun-version-file instead of a second Bun pin`);
  }
}

const dockerfile = read("Dockerfile.linux-spike");
const dockerRust = dockerfile.match(/^FROM rust:([^-\s]+)-bookworm\s*$/m)?.[1];
if (dockerRust !== rustVersion) {
  throw new Error(`Dockerfile.linux-spike Rust version must be ${rustVersion}`);
}

console.log(`toolchain pins agree: Bun ${bunVersion}, Rust ${rustVersion}`);
