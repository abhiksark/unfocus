import { describe, expect, test } from "bun:test";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { render } from "svelte/server";
import { createServer, type ViteDevServer } from "vite";

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
  test("renders reflective, interface, and monospace roles on the real break content", async () => {
    const projectRoot = fileURLToPath(new URL("../..", import.meta.url));
    const libDirectory = fileURLToPath(new URL(".", import.meta.url));
    const cacheDirectory = await mkdtemp(join(tmpdir(), "unfocus-vite-"));
    let vite: ViteDevServer | undefined;

    try {
      vite = await createServer({
        appType: "custom",
        cacheDir: cacheDirectory,
        configFile: false,
        logLevel: "silent",
        plugins: [svelte({ compilerOptions: { dev: false } })],
        resolve: { alias: { $lib: libDirectory } },
        root: projectRoot,
        server: { middlewareMode: true }
      });

      const module = await vite.ssrLoadModule("/src/lib/BreakOverlay.svelte");
      const { body } = render(module.default, {
        props: {
          runId: 1,
          monitorIndex: 0,
          monitorCount: 1,
          durationSeconds: 20,
          deadlineMs: Date.now() + 20_000,
          onClose: async () => {}
        }
      });

      const roles = new Map<string, string | null>();
      const captureRole = (
        key: string
      ): HTMLRewriterTypes.HTMLRewriterElementContentHandlers => ({
        element(element) {
          roles.set(key, element.getAttribute("data-type-role"));
        }
      });

      await new HTMLRewriter()
        .on("#break-message", captureRole("message"))
        .on(".guidance", captureRole("guidance"))
        .on(".eyebrow", captureRole("eyebrow"))
        .on(".overlay-header time", captureRole("clock"))
        .on(".timer-digits", captureRole("countdown"))
        .on(".display-label", captureRole("display"))
        .transform(new Response(body))
        .text();

      expect(Object.fromEntries(roles)).toEqual({
        clock: "mono",
        eyebrow: "ui",
        message: "reflective-display",
        guidance: "ui",
        countdown: "mono",
        display: "mono"
      });
    } finally {
      try {
        await vite?.close();
      } finally {
        await rm(cacheDirectory, { recursive: true, force: true });
      }
    }
  });
});
