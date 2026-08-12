// scripts/sbom-dependencies.test.js
import { describe, expect, test } from "bun:test";
import { resolveBunDependency } from "./sbom-dependencies.js";

function index(entries) {
  const byLockKey = new Map();
  const byName = new Map();
  for (const [lockKey, name, version] of entries) {
    const record = { lockKey, name, version, ref: `pkg:npm/${name}@${version}` };
    byLockKey.set(lockKey, record);
    byName.set(name, [...(byName.get(name) ?? []), record]);
  }
  return { byName, byLockKey };
}

describe("resolveBunDependency", () => {
  test("resolves a single locked version", () => {
    const packages = index([["vite", "vite", "8.2.1"]]);
    expect(resolveBunDependency(packages, "root workspace", "vite", "^8.2.0", false))
      .toBe("pkg:npm/vite@8.2.1");
  });

  test("prefers the owner-scoped copy when a nested duplicate exists", () => {
    // A frontend bump left @types/node at 26.2.0 for the root while bun-types
    // still pinned 26.1.2 beneath itself. Both satisfy the wildcard range, so
    // resolving by bare name alone is ambiguous and used to throw.
    const packages = index([
      ["@types/node", "@types/node", "26.2.0"],
      ["bun-types/@types/node", "@types/node", "26.1.2"]
    ]);
    expect(resolveBunDependency(packages, "bun-types", "@types/node", "*", false))
      .toBe("pkg:npm/@types/node@26.1.2");
  });

  test("still resolves the root copy for an owner without a nested duplicate", () => {
    const packages = index([
      ["@types/node", "@types/node", "26.2.0"],
      ["bun-types/@types/node", "@types/node", "26.1.2"]
    ]);
    expect(resolveBunDependency(packages, "svelte-check", "@types/node", "^26.2.0", false))
      .toBe("pkg:npm/@types/node@26.2.0");
  });

  test("ignores an owner-scoped copy that does not satisfy the range", () => {
    const packages = index([
      ["@types/node", "@types/node", "26.2.0"],
      ["bun-types/@types/node", "@types/node", "26.1.2"]
    ]);
    expect(resolveBunDependency(packages, "bun-types", "@types/node", "^26.2.0", false))
      .toBe("pkg:npm/@types/node@26.2.0");
  });

  test("an unrelated owner sees the root copy, not another owner's nested one", () => {
    // vite's peer range matched both copies. A nested entry belongs to its
    // owner alone, so vite must resolve to the root copy rather than throw.
    const packages = index([
      ["@types/node", "@types/node", "26.2.0"],
      ["bun-types/@types/node", "@types/node", "26.1.2"]
    ]);
    expect(resolveBunDependency(packages, "vite", "@types/node", "^20.19.0 || >=22.12.0", false))
      .toBe("pkg:npm/@types/node@26.2.0");
  });

  test("a nested owner falls back to an ancestor's copy", () => {
    const packages = index([
      ["@types/node", "@types/node", "26.2.0"],
      ["bun-types/@types/node", "@types/node", "26.1.2"]
    ]);
    expect(resolveBunDependency(packages, "bun-types/inner", "@types/node", "*", false))
      .toBe("pkg:npm/@types/node@26.1.2");
  });

  test("throws when only same-named copies exist and none is reachable by key", () => {
    const packages = index([
      ["a/dep", "dep", "1.0.0"],
      ["b/dep", "dep", "2.0.0"]
    ]);
    expect(() => resolveBunDependency(packages, "c", "dep", "*", false))
      .toThrow("multiple matching locked packages");
  });

  test("throws when nothing matches the range", () => {
    const packages = index([["vite", "vite", "8.2.1"]]);
    expect(() => resolveBunDependency(packages, "root workspace", "vite", "^9.0.0", false))
      .toThrow("no matching locked package");
  });

  test("returns null for an absent optional dependency", () => {
    const packages = index([["vite", "vite", "8.2.1"]]);
    expect(resolveBunDependency(packages, "vite", "fsevents", "~2.3.3", true)).toBeNull();
  });

  test("throws for an absent required dependency", () => {
    const packages = index([["vite", "vite", "8.2.1"]]);
    expect(() => resolveBunDependency(packages, "vite", "fsevents", "~2.3.3", false))
      .toThrow("no matching locked package");
  });

  test("rejects an invalid range", () => {
    const packages = index([["vite", "vite", "8.2.1"]]);
    expect(() => resolveBunDependency(packages, "root workspace", "vite", "", false))
      .toThrow("invalid dependency range");
  });
});
