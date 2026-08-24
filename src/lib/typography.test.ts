import { describe, expect, test } from "bun:test";
import { compile, type AST } from "svelte/compiler";

function sha256(bytes: Uint8Array): string {
  return new Bun.CryptoHasher("sha256").update(bytes).digest("hex");
}

type FontProvenance = {
  family: string;
  version: string;
  upstream: { repository: string; commit: string; path: string };
  asset: { path: string; sha256: string };
  license: { path: string; spdx: string; sha256: string };
};

function staticAttribute(element: AST.RegularElement, name: string): string | null {
  const attribute = element.attributes.find(
    (candidate): candidate is AST.Attribute =>
      candidate.type === "Attribute" && candidate.name === name
  );
  if (!attribute || attribute.value === true || !Array.isArray(attribute.value)) return null;
  if (attribute.value.length !== 1 || attribute.value[0].type !== "Text") return null;
  return attribute.value[0].data;
}

function typographyRoles(root: AST.Root): Record<string, string | null> {
  const roles = new Map<string, string | null>();

  function visit(value: unknown) {
    if (Array.isArray(value)) {
      for (const item of value) visit(item);
      return;
    }
    if (value === null || typeof value !== "object") return;

    if ((value as { type?: string }).type === "RegularElement") {
      const element = value as AST.RegularElement;
      const id = staticAttribute(element, "id");
      const classes = new Set((staticAttribute(element, "class") ?? "").split(/\s+/));
      const key =
        id === "break-message"
          ? "message"
          : classes.has("guidance")
            ? "guidance"
            : classes.has("eyebrow")
              ? "eyebrow"
              : element.name === "time"
                ? "clock"
                : classes.has("timer-digits")
                  ? "countdown"
                  : classes.has("display-label")
                    ? "display"
                    : null;
      if (key !== null) roles.set(key, staticAttribute(element, "data-type-role"));
    }

    for (const child of Object.values(value as Record<string, unknown>)) visit(child);
  }

  visit(root);
  return Object.fromEntries(roles);
}

describe("vendored typography", () => {
  test("ships the selected local Newsreader font without the retired Fraunces asset", async () => {
    const font = Bun.file(new URL("../../static/fonts/newsreader.woff2", import.meta.url));
    const retiredFont = Bun.file(
      new URL("../../static/fonts/fraunces.woff2", import.meta.url)
    );
    const license = Bun.file(new URL("../../static/fonts/OFL.txt", import.meta.url));

    expect(await font.exists()).toBe(true);
    expect(await retiredFont.exists()).toBe(false);
    expect(await license.exists()).toBe(true);
    if (!(await font.exists()) || !(await license.exists())) return;

    const bytes = new Uint8Array(await font.arrayBuffer());
    expect(new TextDecoder().decode(bytes.subarray(0, 4))).toBe("wOF2");
    expect(bytes.byteLength).toBeLessThanOrEqual(256_000);

    const licenseText = await license.text();
    expect(licenseText).toContain("The Newsreader Project Authors");
    expect(licenseText).toContain("SIL OPEN FONT LICENSE Version 1.1");
  });

  test("matches the pinned Newsreader source and license provenance", async () => {
    const provenanceFile = Bun.file(
      new URL("../../scripts/asset-sources/newsreader.provenance.json", import.meta.url)
    );

    expect(await provenanceFile.exists()).toBe(true);
    if (!(await provenanceFile.exists())) return;

    const provenance = (await provenanceFile.json()) as FontProvenance;
    const font = new Uint8Array(
      await Bun.file(new URL(`../../${provenance.asset.path}`, import.meta.url)).arrayBuffer()
    );
    const license = new Uint8Array(
      await Bun.file(new URL(`../../${provenance.license.path}`, import.meta.url)).arrayBuffer()
    );

    expect(provenance).toMatchObject({
      family: "Newsreader",
      version: "1.003",
      upstream: {
        repository: "https://github.com/productiontype/Newsreader",
        commit: "cfcb4f7af0e52c25e8df2a2431814c8e5fe2e155",
        path: "fonts/variable/woff2/Newsreader[opsz,wght].woff2"
      },
      asset: { path: "static/fonts/newsreader.woff2" },
      license: { path: "static/fonts/OFL.txt", spdx: "OFL-1.1" }
    });
    expect(sha256(font)).toBe(provenance.asset.sha256);
    expect(sha256(license)).toBe(provenance.license.sha256);
  });
});

describe("typography roles", () => {
  test("marks reflective, interface, and monospace roles on the real break content", async () => {
    const source = await Bun.file(new URL("./BreakOverlay.svelte", import.meta.url)).text();
    const { ast } = compile(source, {
      filename: "src/lib/BreakOverlay.svelte",
      generate: false,
      modernAst: true
    });

    expect(typographyRoles(ast as AST.Root)).toEqual({
      clock: "mono",
      eyebrow: "ui",
      message: "reflective-display",
      guidance: "ui",
      countdown: "mono",
      display: "mono"
    });
  });
});
