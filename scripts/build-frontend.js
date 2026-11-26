import { rmSync, statSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("..", import.meta.url));
// A stale entry must never hide an incomplete SvelteKit build.
rmSync(join(root, "build"), { recursive: true, force: true });
// Run the CLI under Bun even when Node happens to be installed on the host.
const result = Bun.spawnSync([
  process.execPath, join(root, "node_modules/vite/bin/vite.js"), "build", ...process.argv.slice(2)
], { cwd: root, stdout: "inherit", stderr: "inherit" });
if (result.signalCode || result.exitCode !== 0) process.exit(result.exitCode || 1);

const entry = statSync(join(root, "build/index.html"), { throwIfNoEntry: false });
if (!entry?.isFile() || entry.size === 0) {
  throw new Error("Vite did not produce a non-empty build/index.html; use the Bun version in .bun-version and rerun bun run build");
}
