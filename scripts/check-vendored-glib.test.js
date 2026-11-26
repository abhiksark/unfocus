import { afterEach, describe, expect, test } from "bun:test";
import { cpSync, mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { verifyResolvedGlib, verifyVendoredGlib } from "./check-vendored-glib.js";

const roots = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
function fixture() {
  const root = mkdtempSync(join(tmpdir(), "unfocus-glib-test-"));
  roots.push(root);
  mkdirSync(join(root, "scripts"), { recursive: true });
  mkdirSync(join(root, "src-tauri"));
  cpSync(new URL("./vendor-sources", import.meta.url), join(root, "scripts/vendor-sources"), { recursive: true });
  cpSync(new URL("../src-tauri/vendor", import.meta.url), join(root, "src-tauri/vendor"), { recursive: true });
  writeFileSync(join(root, "src-tauri/Cargo.toml"), '[patch.crates-io]\nglib = { path = "vendor/glib-0.18.5" }\n');
  writeFileSync(join(root, "src-tauri/Cargo.lock"), 'version = 4\n[[package]]\nname = "glib"\nversion = "0.18.5"\n');
  return root;
}
function edit(root, path, transform) {
  const fullPath = join(root, path);
  writeFileSync(fullPath, transform(readFileSync(fullPath, "utf8")));
}
const vendor = "src-tauri/vendor/glib-0.18.5/";

describe("verified GLib backport", () => {
  test("rejects effective Cargo source redirection and duplicate resolved GLib packages", () => {
    const verified = verifyVendoredGlib(fixture());
    const pkg = { name: "glib", version: "0.18.5", source: null, manifest_path: verified.manifestPath };
    expect(() => verifyResolvedGlib({ packages: [pkg] }, verified)).not.toThrow();
    for (const packages of [[], [{ ...pkg, manifest_path: "/redirected/Cargo.toml" }], [{ ...pkg, source: "registry+https://github.com/rust-lang/crates.io-index" }], [pkg, pkg]]) {
      expect(() => verifyResolvedGlib({ packages }, verified)).toThrow("resolved GLib source");
    }
  });
  test("preserves authenticated bytes when Git checks out with core.autocrlf=true", () => {
    const root = fixture();
    cpSync(new URL("../.gitattributes", import.meta.url), join(root, ".gitattributes"));
    for (const args of [
      ["init", "--quiet"],
      ["-c", "core.autocrlf=true", "add", "."],
      ["-c", "core.autocrlf=true", "checkout-index", "--all", "--force"]
    ]) {
      if (args.includes("checkout-index")) {
        rmSync(join(root, "src-tauri/vendor"), { recursive: true });
        rmSync(join(root, "scripts/vendor-sources"), { recursive: true });
      }
      const result = Bun.spawnSync(["git", "-C", root, ...args], { stdout: "pipe", stderr: "pipe" });
      expect(result.exitCode).toBe(0);
    }
    expect(() => verifyVendoredGlib(root)).not.toThrow();
  });
  test("accepts exactly the upstream backport and reports a source identity", () => {
    const result = verifyVendoredGlib(fixture());
    expect(result.advisory).toBe("RUSTSEC-2024-0429");
    expect(result.version).toBe("0.18.5");
    expect(result.sha256).toMatch(/^[a-f0-9]{64}$/);
    expect(result.files).toBe(121);
  });
  test("rejects reverting the output pointer fix", () => {
    const root = fixture();
    edit(root, vendor + "src/variant_iter.rs", (s) => s.replace("let mut p: *mut libc::c_char", "let p: *mut libc::c_char").replace("                &mut p,", "                &p,"));
    expect(() => verifyVendoredGlib(root)).toThrow("vendor file differs");
  });
  test("rejects an unrelated source change", () => {
    const root = fixture();
    edit(root, vendor + "src/lib.rs", (s) => s + "\n// changed\n");
    expect(() => verifyVendoredGlib(root)).toThrow("vendor file differs");
  });
  test("rejects missing license files", () => {
    const root = fixture();
    rmSync(join(root, vendor + "LICENSE"));
    expect(() => verifyVendoredGlib(root)).toThrow("vendor file inventory");
  });
  test("rejects extra source files", () => {
    const root = fixture();
    writeFileSync(join(root, vendor + "src/injected.rs"), "");
    expect(() => verifyVendoredGlib(root)).toThrow("vendor file inventory");
  });
  test.skipIf(process.platform === "win32")("rejects symlinked source", () => {
    const root = fixture();
    const path = join(root, vendor + "LICENSE");
    rmSync(path);
    symlinkSync(join(root, vendor + "COPYRIGHT"), path);
    expect(() => verifyVendoredGlib(root)).toThrow("symlink");
  });
  test("rejects archive tampering", () => {
    const root = fixture();
    writeFileSync(join(root, "scripts/vendor-sources/glib-0.18.5.crate"), "altered");
    expect(() => verifyVendoredGlib(root)).toThrow("archive checksum");
  });
  test("rejects provenance tampering", () => {
    const root = fixture();
    edit(root, "scripts/vendor-sources/glib-0.18.5.json", (s) => s.replace("b5a4071e", "00000000"));
    expect(() => verifyVendoredGlib(root)).toThrow("provenance");
  });
  test("rejects a replaced upstream patch", () => {
    const root = fixture();
    edit(root, "scripts/vendor-sources/glib-variant-str-iter.patch", (s) => s + "\n");
    expect(() => verifyVendoredGlib(root)).toThrow("upstream patch checksum");
  });
  test("rejects an audit input that no longer scans the original registry GLib", () => {
    const root = fixture();
    edit(root, "scripts/vendor-sources/glib-0.18.5.audit.lock", (s) => s.replace('name = "glib"\nversion = "0.18.5"', 'name = "glib"\nversion = "0.18.4"'));
    expect(() => verifyVendoredGlib(root)).toThrow("registry audit GLib");
  });
  test("rejects an absent or redirected Cargo patch", () => {
    const root = fixture();
    edit(root, "src-tauri/Cargo.toml", () => "");
    expect(() => verifyVendoredGlib(root)).toThrow("Cargo patch");
    edit(root, "src-tauri/Cargo.toml", () => '[patch.crates-io]\nglib = { path = "elsewhere" }\n');
    expect(() => verifyVendoredGlib(root)).toThrow("Cargo patch");
  });
  test("rejects a registry lock entry or second GLib copy", () => {
    const root = fixture();
    edit(root, "src-tauri/Cargo.lock", (s) => s + 'source = "registry+https://github.com/rust-lang/crates.io-index"\n');
    expect(() => verifyVendoredGlib(root)).toThrow("locked GLib");
    edit(root, "src-tauri/Cargo.lock", (s) => s + '[[package]]\nname = "glib"\nversion = "0.20.0"\n');
    expect(() => verifyVendoredGlib(root)).toThrow("locked GLib");
  });
});

describe.skipIf(process.platform === "win32")("Rust audit source verification", () => {
  function auditFixture(report, { applicationExit = 0, registryExit = 0 } = {}) {
    const root = fixture();
    for (const file of ["audit-rust.js", "check-dependency-exceptions.js", "check-vendored-glib.js"]) {
      cpSync(new URL(file, import.meta.url), join(root, "scripts", file));
    }
    mkdirSync(join(root, ".github"));
    writeFileSync(join(root, ".github/dependency-exceptions.json"), '{"exceptions":[]}');
    const bin = join(root, "bin");
    mkdirSync(bin);
    const output = report.application
      ? `process.argv.at(-1).endsWith(".audit.lock") ? ${JSON.stringify(report.registry)} : ${JSON.stringify(report.application)}`
      : JSON.stringify(report);
    const metadata = { packages: [{ name: "glib", version: "0.18.5", source: null, manifest_path: join(root, vendor, "Cargo.toml") }] };
    writeFileSync(join(bin, "cargo"), `#!${process.execPath}\nconst metadata = process.argv.includes("metadata");\nconsole.log(JSON.stringify(metadata ? ${JSON.stringify(metadata)} : (${output})));\nif (!metadata) process.exit(process.argv.at(-1).endsWith(".audit.lock") ? ${registryExit} : ${applicationExit});\n`, { mode: 0o755 });
    return { root, env: { ...process.env, PATH: bin + ":" + process.env.PATH } };
  }
  function runAudit({ root, env }) {
    const result = Bun.spawnSync([process.execPath, join(root, "scripts/audit-rust.js")], { env, stdout: "pipe", stderr: "pipe" });
    return { status: result.exitCode, output: new TextDecoder().decode(result.stdout) + new TextDecoder().decode(result.stderr) };
  }
  test("fails even if cargo-audit reports clean when the source fix is reverted", () => {
    const config = auditFixture({ vulnerabilities: { list: [] }, warnings: {} });
    edit(config.root, vendor + "src/variant_iter.rs", (s) => s.replace("let mut p: *mut libc::c_char", "let p: *mut libc::c_char"));
    const result = runAudit(config);
    expect(result.status).not.toBe(0);
    expect(result.output).toContain("vendor file differs");
  });
  test("keeps unrelated Rust advisories as failures", () => {
    const config = auditFixture({ vulnerabilities: { list: [{ advisory: { id: "RUSTSEC-2099-0001" }, package: { name: "other", version: "1.0.0", source: "registry+https://github.com/rust-lang/crates.io-index" } }] }, warnings: {} });
    const result = runAudit(config);
    expect(result.status).not.toBe(0);
    expect(result.output).toContain("unapproved Rust advisory: RUSTSEC-2099-0001");
  });
  test("accepts only the exact fixed GLib advisory after source verification", () => {
    const config = auditFixture({
      application: { vulnerabilities: { list: [] }, warnings: {} },
      registry: { vulnerabilities: { list: [] }, warnings: { unsound: [{ kind: "unsound", advisory: { id: "RUSTSEC-2024-0429" }, package: { name: "glib", version: "0.18.5", source: "registry+https://github.com/rust-lang/crates.io-index" } }] } }
    });
    const result = runAudit(config);
    expect(result.status).toBe(0);
    expect(result.output).toContain("source backport verified");
  });
  test("fails on a new GLib advisory even with the original fix present", () => {
    const config = auditFixture({
      application: { vulnerabilities: { list: [] }, warnings: {} },
      registry: { vulnerabilities: { list: [{ advisory: { id: "RUSTSEC-2099-0002" }, package: { name: "glib", version: "0.18.5", source: "registry+https://github.com/rust-lang/crates.io-index" } }] }, warnings: {} }
    });
    const result = runAudit(config);
    expect(result.status).not.toBe(0);
    expect(result.output).toContain("RUSTSEC-2099-0002");
  });
  test("rejects registry findings without a complete package identity", () => {
    for (const pkg of [undefined, { name: "" }, { name: "glib" }, { name: "glib", version: "0.18.5", source: null }]) {
      const config = auditFixture({
        application: { vulnerabilities: { list: [] }, warnings: {} },
        registry: { vulnerabilities: { list: [{ advisory: { id: "RUSTSEC-2099-0003" }, package: pkg }] }, warnings: {} }
      });
      const result = runAudit(config);
      expect(result.status).not.toBe(0);
      expect(result.output).toContain("invalid registry audit package identity");
    }
  });
  test("rejects malformed registry vulnerabilities before filtering scanner-only packages", () => {
    for (const advisory of [undefined, {}, { id: null }, { id: 42 }, { id: "" }, { id: "  " }]) {
      const config = auditFixture({
        application: { vulnerabilities: { list: [] }, warnings: {} },
        registry: { vulnerabilities: { list: [{ advisory, package: { name: "scanner-only", version: "1.0.0", source: "registry+https://github.com/rust-lang/crates.io-index" } }] }, warnings: {} }
      }, { registryExit: 1 });
      const result = runAudit(config);
      expect(result.status).not.toBe(0);
      expect(result.output).toContain("without an advisory identifier");
    }
  });
  test("rejects malformed warning lists and unmaintained notices", () => {
    for (const warnings of [{ unmaintained: null }, { unmaintained: [{}] }, { unsound: [{ advisory: { id: " " } }] }]) {
      const config = auditFixture({
        application: { vulnerabilities: { list: [] }, warnings: {} },
        registry: { vulnerabilities: { list: [] }, warnings }
      });
      const result = runAudit(config);
      expect(result.status).not.toBe(0);
    }
  });
});
