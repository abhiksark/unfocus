import { afterEach, expect, test } from "bun:test";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const roots = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });

function buildFixture(source) {
  const root = mkdtempSync(join(tmpdir(), "unfocus-build-test-"));
  roots.push(root);
  const { scripts: { build } } = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"));
  writeFileSync(join(root, "package.json"), JSON.stringify({ type: "module", scripts: { build } }));
  mkdirSync(join(root, "scripts"));
  const wrapper = new URL("./build-frontend.js", import.meta.url);
  if (existsSync(wrapper)) cpSync(wrapper, join(root, "scripts/build-frontend.js"));
  mkdirSync(join(root, "node_modules/vite/bin"), { recursive: true });
  mkdirSync(join(root, "node_modules/.bin"));
  const executable = `#!${process.execPath}\n${source}\n`;
  writeFileSync(join(root, "node_modules/vite/bin/vite.js"), executable);
  writeFileSync(join(root, "node_modules/.bin/vite"), executable, { mode: 0o755 });
  writeFileSync(join(root, "node_modules/.bin/vite.cmd"), `@"${process.execPath}" "%~dp0\\..\\vite\\bin\\vite.js" %*\r\n`);
  return root;
}

function runBuild(root, args = []) {
  const result = Bun.spawnSync([process.execPath, "run", "build", ...args], { cwd: root, stdout: "pipe", stderr: "pipe" });
  return { status: result.exitCode, output: result.stdout.toString() + result.stderr.toString() };
}

test("build runs Vite with Bun and forwards its arguments", () => {
  const root = buildFixture(`import { mkdirSync, writeFileSync } from "node:fs";
mkdirSync("build", { recursive: true });
writeFileSync("build/index.html", JSON.stringify({ bun: typeof Bun !== "undefined", args: process.argv.slice(2) }));`);
  expect(runBuild(root, ["--mode", "test"]).status).toBe(0);
  expect(JSON.parse(readFileSync(join(root, "build/index.html"), "utf8"))).toEqual({ bun: true, args: ["build", "--mode", "test"] });
});

test("build rejects a successful Vite exit without fresh frontend output", () => {
  const root = buildFixture("");
  mkdirSync(join(root, "build"));
  writeFileSync(join(root, "build/index.html"), "stale frontend");
  const result = runBuild(root);
  expect(result.status).not.toBe(0);
  expect(result.output).toContain("build/index.html");
  expect(existsSync(join(root, "build/index.html"))).toBe(false);
});

test("build rejects an empty or non-file frontend entry", () => {
  for (const source of [
    'import { mkdirSync, writeFileSync } from "node:fs"; mkdirSync("build"); writeFileSync("build/index.html", "");',
    'import { mkdirSync } from "node:fs"; mkdirSync("build/index.html", { recursive: true });'
  ]) {
    const result = runBuild(buildFixture(source));
    expect(result.status).not.toBe(0);
    expect(result.output).toContain("build/index.html");
  }
});

test("build preserves Vite failure status", () => {
  expect(runBuild(buildFixture("process.exit(7);")).status).toBe(7);
});
