import { describe, expect, test } from "bun:test";

function jpegDimensions(bytes: Uint8Array): { width: number; height: number } {
  if (bytes[0] !== 0xff || bytes[1] !== 0xd8) {
    throw new Error("break scene is not a JPEG");
  }

  let offset = 2;
  while (offset + 8 < bytes.length) {
    if (bytes[offset] !== 0xff) {
      offset += 1;
      continue;
    }

    const marker = bytes[offset + 1];
    offset += 2;
    if (marker === 0xd8 || marker === 0xd9) continue;

    const segmentLength = (bytes[offset] << 8) | bytes[offset + 1];
    const isStartOfFrame =
      marker >= 0xc0 && marker <= 0xcf && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc;
    if (isStartOfFrame) {
      return {
        height: (bytes[offset + 3] << 8) | bytes[offset + 4],
        width: (bytes[offset + 5] << 8) | bytes[offset + 6]
      };
    }
    offset += segmentLength;
  }

  throw new Error("break scene JPEG dimensions are unavailable");
}

function sha256(bytes: Uint8Array): string {
  return new Bun.CryptoHasher("sha256").update(bytes).digest("hex");
}

type BreakSceneProvenance = {
  source: { path: string; sha256: string };
  derivative: { path: string; sha256: string; width: number; height: number };
};

describe("break scene asset", () => {
  test("ships a local 4K delivery JPEG within the overlay asset budget", async () => {
    const image = Bun.file(new URL("../../static/break-scene.jpg", import.meta.url));
    const exists = await image.exists();

    expect(exists).toBe(true);
    if (!exists) return;

    const bytes = new Uint8Array(await image.arrayBuffer());
    expect(jpegDimensions(bytes)).toEqual({ width: 3840, height: 2160 });
    expect(bytes.byteLength).toBeLessThanOrEqual(2_500_000);
  });

  test("matches the retained source and derivative provenance", async () => {
    const provenance = (await Bun.file(
      new URL("../../scripts/asset-sources/break-scene.provenance.json", import.meta.url)
    ).json()) as BreakSceneProvenance;
    const source = new Uint8Array(
      await Bun.file(
        new URL("../../scripts/asset-sources/break-scene-source.png", import.meta.url)
      ).arrayBuffer()
    );
    const derivative = new Uint8Array(
      await Bun.file(new URL("../../static/break-scene.jpg", import.meta.url)).arrayBuffer()
    );

    expect(provenance.source.path).toBe("scripts/asset-sources/break-scene-source.png");
    expect(sha256(source)).toBe(provenance.source.sha256);
    expect(provenance.derivative).toMatchObject({
      path: "static/break-scene.jpg",
      width: 3840,
      height: 2160
    });
    expect(sha256(derivative)).toBe(provenance.derivative.sha256);
  });
});

type BreakSceneModule = {
  BREAK_SCENE_IMAGE_URL: string;
  breakScenePeriodAt: (date: Date) => "dawn" | "day" | "dusk" | "night";
  breakScenePeriodForRun: (
    deadlineMs: number,
    durationSeconds: number
  ) => "dawn" | "day" | "dusk" | "night";
  breakScenePhase: (state: { complete: boolean; finalSeconds: boolean }) =>
    | "resting"
    | "returning";
};

async function loadBreakScene(): Promise<BreakSceneModule | null> {
  const modulePath = "./break-scene.ts";
  return import(modulePath).catch(() => null) as Promise<BreakSceneModule | null>;
}

describe("break scene presentation", () => {
  test("uses the bundled scene asset", async () => {
    const scene = await loadBreakScene();

    expect(scene).not.toBeNull();
    if (!scene) return;

    expect(scene.BREAK_SCENE_IMAGE_URL).toBe("/break-scene.jpg");
  });

  test("enters the returning phase for final seconds or completion", async () => {
    const scene = await loadBreakScene();

    expect(scene).not.toBeNull();
    if (!scene) return;

    expect(scene.breakScenePhase({ complete: false, finalSeconds: false })).toBe("resting");
    expect(scene.breakScenePhase({ complete: false, finalSeconds: true })).toBe("returning");
    expect(scene.breakScenePhase({ complete: true, finalSeconds: false })).toBe("returning");
  });

  test("selects the four scene periods at their exact local-time boundaries", async () => {
    const scene = await loadBreakScene();

    expect(scene).not.toBeNull();
    if (!scene) return;

    const cases: Array<[hour: number, minute: number, expected: "dawn" | "day" | "dusk" | "night"]> = [
      [4, 59, "night"],
      [5, 0, "dawn"],
      [8, 59, "dawn"],
      [9, 0, "day"],
      [16, 59, "day"],
      [17, 0, "dusk"],
      [20, 59, "dusk"],
      [21, 0, "night"]
    ];

    for (const [hour, minute, expected] of cases) {
      const localDate = new Date(2026, 7, 24, hour, minute);
      expect(scene.breakScenePeriodAt(localDate)).toBe(expected);
    }
  });

  test("falls back to the neutral day scene when local time is invalid", async () => {
    const scene = await loadBreakScene();

    expect(scene).not.toBeNull();
    if (!scene) return;

    expect(scene.breakScenePeriodAt(new Date(Number.NaN))).toBe("day");
  });

  test("selects from the shared run start and not the later deadline", async () => {
    const scene = await loadBreakScene();

    expect(scene).not.toBeNull();
    if (!scene) return;

    const dayStartsMs = new Date(2026, 7, 24, 9, 0).getTime();
    expect(scene.breakScenePeriodForRun(dayStartsMs + 20_000, 20)).toBe("day");
    expect(scene.breakScenePeriodForRun(dayStartsMs + 19_999, 20)).toBe("dawn");
  });
});
