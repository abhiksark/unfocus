import { afterEach, describe, expect, test } from "bun:test";
import { mkdtempSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  inspectAppImageOuter,
  inspectLinuxPackages,
  parseBuildId,
  parseDebianControlArchive,
  parseDebianFields,
  parseDebianMd5Sums,
  parseNewcArchive,
  parsePackageEvidence,
  parseRpmLayout,
  parseRpmMetadata,
  parseSquashfsListing,
  requireEmptyRpmScriptListing,
  validateDebianControlArchive,
  validateSbom,
} from "./inspect-linux-packages.js";

const temporaryDirectories = [];

function temporaryDirectory() {
  const directory = mkdtempSync(join(tmpdir(), "unfocus-package-inspector-test-"));
  temporaryDirectories.push(directory);
  return directory;
}

function syntheticAppImage() {
  const filesystemOffset = 184;
  const bytes = Buffer.alloc(filesystemOffset + 96);
  bytes.set([0x7f, 0x45, 0x4c, 0x46, 2, 1, 1, 0, 0x41, 0x49, 0x02], 0);
  bytes.writeUInt16LE(3, 16);
  bytes.writeUInt16LE(62, 18);
  bytes.writeUInt32LE(1, 20);
  bytes.writeBigUInt64LE(128n, 24);
  bytes.writeBigUInt64LE(64n, 32);
  bytes.writeBigUInt64LE(120n, 40);
  bytes.writeUInt16LE(64, 52);
  bytes.writeUInt16LE(56, 54);
  bytes.writeUInt16LE(1, 56);
  bytes.writeUInt16LE(64, 58);
  bytes.writeUInt16LE(1, 60);
  bytes.writeUInt16LE(0, 62);
  bytes.writeUInt32LE(1, 64);
  bytes.writeUInt32LE(5, 64 + 4);
  bytes.writeBigUInt64LE(0n, 64 + 8);
  bytes.writeBigUInt64LE(0n, 64 + 16);
  bytes.writeBigUInt64LE(BigInt(filesystemOffset), 64 + 32);
  bytes.writeBigUInt64LE(BigInt(filesystemOffset), 64 + 40);
  bytes.writeBigUInt64LE(4_096n, 64 + 48);

  bytes.writeUInt32LE(0x7371_7368, filesystemOffset);
  bytes.writeUInt32LE(1, filesystemOffset + 4);
  bytes.writeUInt32LE(131_072, filesystemOffset + 12);
  bytes.writeUInt16LE(6, filesystemOffset + 20);
  bytes.writeUInt16LE(17, filesystemOffset + 22);
  bytes.writeUInt16LE(4, filesystemOffset + 28);
  bytes.writeUInt16LE(0, filesystemOffset + 30);
  bytes.writeBigUInt64LE(96n, filesystemOffset + 40);
  for (const offset of [48, 56, 64, 72, 80, 88]) {
    bytes.writeBigUInt64LE(0xffff_ffff_ffff_ffffn, filesystemOffset + offset);
  }
  return { bytes, filesystemOffset };
}

function writeSyntheticAppImage(mutate) {
  const directory = temporaryDirectory();
  const path = join(directory, "fixture.AppImage");
  const fixture = syntheticAppImage();
  mutate?.(fixture.bytes, fixture.filesystemOffset);
  writeFileSync(path, fixture.bytes);
  return { path, ...fixture };
}

function newcEntry(name, mode, data = Buffer.alloc(0)) {
  const nameBytes = Buffer.from(`${name}\0`);
  const fields = [1, mode, 0, 0, 1, 0, data.length, 0, 0, 0, 0, nameBytes.length, 0];
  const header = Buffer.from(`070701${fields.map((value) => value.toString(16).padStart(8, "0")).join("")}`);
  const namePadding = Buffer.alloc((4 - ((header.length + nameBytes.length) % 4)) % 4);
  const dataPadding = Buffer.alloc((4 - (data.length % 4)) % 4);
  return Buffer.concat([header, nameBytes, namePadding, data, dataPadding]);
}

function newcArchive(name = "./usr/bin/unfocus") {
  const bytes = Buffer.concat([
    newcEntry(name, 0o100755, Buffer.from("binary")),
    newcEntry("TRAILER!!!", 0),
  ]);
  return Buffer.concat([bytes, Buffer.alloc((512 - (bytes.length % 512)) % 512)]);
}

function tarOctal(header, offset, length, value) {
  header.write(value.toString(8).padStart(length - 1, "0"), offset, length - 1, "ascii");
}

function tarEntry(name, data, type = "0", mode = 0o644) {
  const header = Buffer.alloc(512);
  header.write(name, 0, 100, "utf8");
  tarOctal(header, 100, 8, mode);
  tarOctal(header, 108, 8, 0);
  tarOctal(header, 116, 8, 0);
  tarOctal(header, 124, 12, data.length);
  tarOctal(header, 136, 12, 0);
  header.fill(0x20, 148, 156);
  header.write(type, 156, "ascii");
  header.write("ustar  ", 257, "ascii");
  const checksum = header.reduce((total, byte) => total + byte, 0);
  header.write(checksum.toString(8).padStart(6, "0"), 148, 6, "ascii");
  header[154] = 0;
  header[155] = 0x20;
  return Buffer.concat([header, data, Buffer.alloc((512 - (data.length % 512)) % 512)]);
}

function debianControlArchive(extraEntries = []) {
  return Buffer.concat([
    tarEntry("./", Buffer.alloc(0), "5", 0o755),
    tarEntry("./control", Buffer.from("Package: unfocus\n")),
    tarEntry("./md5sums", Buffer.from("0 usr/bin/unfocus\n")),
    ...extraEntries,
    Buffer.alloc(1024),
  ]);
}

function listingLine(mode, size, path, target) {
  const suffix = target === undefined ? path : `${path} -> ${target}`;
  return `${mode} 0/0 ${String(size).padStart(12)} 2026-09-03 12:00 squashfs-root${suffix ? `/${suffix}` : ""}`;
}

afterEach(() => {
  for (const directory of temporaryDirectories.splice(0)) {
    rmSync(directory, { recursive: true, force: true });
  }
});

describe("AppImage outer inspection", () => {
  test("derives the appended SquashFS offset from bounded ELF tables", () => {
    const { path, filesystemOffset } = writeSyntheticAppImage();
    expect(inspectAppImageOuter(path)).toEqual({
      fileSize: filesystemOffset + 96,
      filesystemOffset,
      squashfs: { bytesUsed: 96, compression: "zstd", inodeCount: 1 },
    });
  });

  test("rejects a forged AppImage marker", () => {
    const { path } = writeSyntheticAppImage((bytes) => bytes.fill(0, 8, 11));
    expect(() => inspectAppImageOuter(path)).toThrow("type-2 marker is missing");
  });

  test("rejects invalid ELF tables, segments, and entry points", () => {
    const { path } = writeSyntheticAppImage((bytes) => bytes.writeBigUInt64LE(10_000n, 32));
    expect(() => inspectAppImageOuter(path)).toThrow("program header table is outside the file");

    const second = writeSyntheticAppImage((bytes) => bytes.writeBigUInt64LE(10_000n, 64 + 32));
    expect(() => inspectAppImageOuter(second.path)).toThrow("program header 0 references bytes outside the file");

    const zeroEntry = writeSyntheticAppImage((bytes) => bytes.writeBigUInt64LE(0n, 24));
    expect(() => inspectAppImageOuter(zeroEntry.path)).toThrow("nonzero entry point");
    const noLoad = writeSyntheticAppImage((bytes) => bytes.writeUInt32LE(0, 64));
    expect(() => inspectAppImageOuter(noLoad.path)).toThrow("loadable executable segment");
    const entryOutsideLoad = writeSyntheticAppImage((bytes) => bytes.writeBigUInt64LE(10_000n, 24));
    expect(() => inspectAppImageOuter(entryOutsideLoad.path)).toThrow("loadable executable segment");
  });

  test("rejects malformed or unsupported SquashFS superblocks and trailing data", () => {
    const malformed = writeSyntheticAppImage((bytes, offset) => bytes.writeUInt32LE(0, offset));
    expect(() => inspectAppImageOuter(malformed.path)).toThrow("is not SquashFS");

    const unsupported = writeSyntheticAppImage((bytes, offset) => bytes.writeUInt16LE(1, offset + 20));
    expect(() => inspectAppImageOuter(unsupported.path)).toThrow("compression must be zstd");

    const directory = temporaryDirectory();
    const trailingPath = join(directory, "trailing.AppImage");
    writeFileSync(trailingPath, Buffer.concat([syntheticAppImage().bytes, Buffer.from([1])]));
    expect(() => inspectAppImageOuter(trailingPath)).toThrow("nonzero bytes after its SquashFS filesystem");
  });

  test("allows only bounded zero padding after the SquashFS filesystem", () => {
    const fixture = syntheticAppImage();
    const directory = temporaryDirectory();
    const nonzero = join(directory, "nonzero.AppImage");
    writeFileSync(nonzero, Buffer.concat([fixture.bytes, Buffer.from([1])]));
    expect(() => inspectAppImageOuter(nonzero)).toThrow("nonzero bytes after");

    const excessive = join(directory, "excessive.AppImage");
    writeFileSync(excessive, Buffer.concat([fixture.bytes, Buffer.alloc(4_096)]));
    expect(() => inspectAppImageOuter(excessive)).toThrow("excessive bytes after");
  });

  test("rejects symlinked AppImage inputs", () => {
    const directory = temporaryDirectory();
    const target = join(directory, "target.AppImage");
    const link = join(directory, "link.AppImage");
    writeFileSync(target, syntheticAppImage().bytes);
    symlinkSync(target, link);
    expect(() => inspectAppImageOuter(link)).toThrow("regular non-symlink file");
  });
});

describe("SquashFS listing constraints", () => {
  test("parses bounded regular files, directories, and relative symlinks", () => {
    const listing = [
      listingLine("drwxr-xr-x", 1, ""),
      listingLine("-rwxr-xr-x", 10, "AppRun"),
      listingLine("drwxr-xr-x", 1, "usr"),
      listingLine("-rwxr-xr-x", 20, "usr/unfocus"),
      listingLine("lrwxrwxrwx", 11, "launch", "usr/unfocus"),
    ].join("\n");
    expect(parseSquashfsListing(`${listing}\n`)).toEqual([
      { path: "AppRun", type: "-", mode: "-rwxr-xr-x", size: 10 },
      { path: "usr", type: "d", mode: "drwxr-xr-x", size: 1 },
      { path: "usr/unfocus", type: "-", mode: "-rwxr-xr-x", size: 20 },
      { path: "launch", type: "l", mode: "lrwxrwxrwx", size: 11, target: "usr/unfocus" },
    ]);
  });

  test("rejects traversal, dangling symlinks, and cycles", () => {
    const root = listingLine("drwxr-xr-x", 1, "");
    expect(() => parseSquashfsListing(`${root}\n${listingLine("-rw-r--r--", 1, "../escape")}\n`)).toThrow(
      "unsafe path",
    );
    expect(() => parseSquashfsListing(`${root}\n${listingLine("lrwxrwxrwx", 7, "link", "missing")}\n`)).toThrow(
      "missing target",
    );
    expect(() =>
      parseSquashfsListing(
        `${root}\n${listingLine("lrwxrwxrwx", 1, "a", "b")}\n${listingLine("lrwxrwxrwx", 1, "b", "a")}\n`,
      ),
    ).toThrow("symlink cycle");
  });

  test("rejects unsupported entry types and duplicate paths", () => {
    expect(() => parseSquashfsListing(`${listingLine("drwxr-xr-x", 1, "")}\nprw-r--r-- bad\n`)).toThrow(
      "could not parse",
    );
    expect(() =>
      parseSquashfsListing(
        `${listingLine("drwxr-xr-x", 1, "")}\n${listingLine("-rw-r--r--", 1, "same")}\n${listingLine("-rw-r--r--", 1, "same")}\n`,
      ),
    ).toThrow("duplicate path");
  });
});

describe("RPM newc payload constraints", () => {
  test("parses one complete bounded archive", () => {
    expect(parseNewcArchive(newcArchive())).toEqual([
      { path: "/usr/bin/unfocus", mode: 0o100755, size: 6 },
    ]);
  });

  test("rejects traversal, truncation, nonzero padding, and bytes after the trailer", () => {
    expect(() => parseNewcArchive(newcArchive("../escape"))).toThrow("unsafe path");
    expect(() => parseNewcArchive(newcArchive("/usr/bin/unfocus"))).toThrow("absolute path");
    const truncated = newcArchive().subarray(0, 120);
    expect(() => parseNewcArchive(truncated)).toThrow();

    const namePadding = newcArchive("ab");
    namePadding[113] = 1;
    expect(() => parseNewcArchive(namePadding)).toThrow("nonzero name padding");
    const dataPadding = newcArchive();
    const nameSize = Number.parseInt(dataPadding.subarray(94, 102).toString("ascii"), 16);
    const dataSize = Number.parseInt(dataPadding.subarray(54, 62).toString("ascii"), 16);
    const dataEnd = ((110 + nameSize + 3) & ~3) + dataSize;
    dataPadding[dataEnd] = 1;
    expect(() => parseNewcArchive(dataPadding)).toThrow("nonzero data padding");

    const trailing = newcArchive();
    trailing[trailing.length - 1] = 1;
    expect(() => parseNewcArchive(trailing)).toThrow("bytes after its trailer");
  });
});

describe("package metadata parsers", () => {
  test("allows only inert Debian control files and no RPM scriptlets", () => {
    const control = debianControlArchive();
    expect(parseDebianControlArchive(control).map((entry) => entry.path)).toEqual([".", "control", "md5sums"]);
    expect(() => validateDebianControlArchive(control)).not.toThrow();
    expect(() =>
      validateDebianControlArchive(
        debianControlArchive([tarEntry("./postinst", Buffer.from("#!/bin/sh\nexit 0\n"), "0", 0o755)]),
      ),
    ).toThrow("must contain only its root, control, and md5sums");
    expect(() => requireEmptyRpmScriptListing("", "install or uninstall scriptlets")).not.toThrow();
    expect(() => requireEmptyRpmScriptListing("postinstall scriptlet\n", "install or uninstall scriptlets")).toThrow(
      "contains install or uninstall scriptlets",
    );

    const md5sums = Buffer.from(`${"a".repeat(32)}  usr/bin/unfocus\n${"b".repeat(32)}  notice.txt\n`);
    expect(parseDebianMd5Sums(md5sums, ["usr/bin/unfocus", "notice.txt"])).toEqual([
      { digest: "a".repeat(32), path: "usr/bin/unfocus" },
      { digest: "b".repeat(32), path: "notice.txt" },
    ]);
    expect(() => parseDebianMd5Sums(md5sums, ["usr/bin/unfocus"])).toThrow(
      "every payload file exactly",
    );
    expect(() =>
      parseDebianMd5Sums(Buffer.from(`${"a".repeat(32)}  usr/bin/unfocus\n${"b".repeat(32)}  usr/bin/unfocus\n`), [
        "usr/bin/unfocus",
        "usr/bin/unfocus",
      ]),
    ).toThrow("duplicate paths");
  });

  test("parses exact Debian and RPM identity fields", () => {
    expect(parseDebianFields("Package: unfocus\nVersion: 0.7.0~beta.1-1\nArchitecture: amd64\n")).toEqual({
      package: "unfocus",
      version: "0.7.0~beta.1-1",
      architecture: "amd64",
    });
    expect(parseRpmMetadata("unfocus\n0.7.0-beta.1\n1\nx86_64\n")).toEqual({
      name: "unfocus",
      version: "0.7.0-beta.1",
      release: "1",
      architecture: "x86_64",
    });
    const rpmLayout = [
      "/usr/bin/unfocus\t100775\t200\troot\troot\t(none)",
      "/usr/lib/Unfocus\t40755\t0\troot\troot\t(none)",
      "/usr/lib/Unfocus/THIRD_PARTY_NOTICES.txt\t100664\t100\troot\troot\t(none)",
      "/usr/share/applications/Unfocus.desktop\t100664\t20\troot\troot\t(none)",
      "/usr/share/icons/hicolor/128x128/apps/unfocus.png\t100664\t20\troot\troot\t(none)",
      "/usr/share/icons/hicolor/256x256@2/apps/unfocus.png\t100664\t20\troot\troot\t(none)",
      "/usr/share/icons/hicolor/32x32/apps/unfocus.png\t100664\t20\troot\troot\t(none)",
    ].join("\n");
    expect(parseRpmLayout(`${rpmLayout}\n`)).toHaveLength(7);
  });

  test("rejects missing, duplicate, and whitespace-bearing metadata", () => {
    expect(() => parseDebianFields("Package: unfocus\nPackage: other\nArchitecture: amd64\n")).toThrow(
      "invalid Debian metadata",
    );
    expect(() => parseRpmMetadata("unfocus\n0.7.0 beta.1\n1\nx86_64\n")).toThrow("four single-token lines");
    const wrongType = [
      "/usr/bin/unfocus\t120777\t200\troot\troot\t(none)",
      "/usr/lib/Unfocus\t40755\t0\troot\troot\t(none)",
      "/usr/lib/Unfocus/THIRD_PARTY_NOTICES.txt\t100664\t100\troot\troot\t(none)",
      "/usr/share/applications/Unfocus.desktop\t100664\t20\troot\troot\t(none)",
      "/usr/share/icons/hicolor/128x128/apps/unfocus.png\t100664\t20\troot\troot\t(none)",
      "/usr/share/icons/hicolor/256x256@2/apps/unfocus.png\t100664\t20\troot\troot\t(none)",
      "/usr/share/icons/hicolor/32x32/apps/unfocus.png\t100664\t20\troot\troot\t(none)",
    ].join("\n");
    expect(() => parseRpmLayout(`${wrongType}\n`)).toThrow("wrong file type or permissions");
    const privileged = wrongType.replace(
      "/usr/bin/unfocus\t120777\t200\troot\troot\t(none)",
      "/usr/bin/unfocus\t100775\t200\troot\troot\tcap_net_admin=ep",
    );
    expect(() => parseRpmLayout(`${privileged}\n`)).toThrow("root-owned and capability-free");
  });

  test("extracts exactly one bounded GNU build ID", () => {
    expect(parseBuildId("Displaying notes\n    Build ID: 0123456789abcdef0123456789abcdef01234567\n")).toBe(
      "0123456789abcdef0123456789abcdef01234567",
    );
    expect(() => parseBuildId("no build id\n")).toThrow("exactly one valid GNU build ID");
    expect(() => parseBuildId(" Build ID: 0123456789abcdef\n Build ID: fedcba9876543210\n")).toThrow(
      "exactly one valid GNU build ID",
    );
  });
});

describe("release dependency metadata", () => {
  test("requires bounded CycloneDX application metadata and the signature verifier", () => {
    const directory = temporaryDirectory();
    const path = join(directory, "unfocus.cdx.json");
    const sbom = {
      bomFormat: "CycloneDX",
      specVersion: "1.6",
      version: 1,
      metadata: {
        component: {
          type: "application",
          "bom-ref": "pkg:generic/unfocus@0.7.0-beta.1",
          name: "Unfocus",
          version: "0.7.0-beta.1",
        },
      },
      components: [{ "bom-ref": "pkg:cargo/minisign-verify@0.2.5" }],
    };
    writeFileSync(path, JSON.stringify(sbom));
    expect(() => validateSbom(path, "0.7.0-beta.1")).not.toThrow();
    sbom.components = [];
    writeFileSync(path, JSON.stringify(sbom));
    expect(() => validateSbom(path, "0.7.0-beta.1")).toThrow("invalid CycloneDX");
  });
});

describe("package evidence", () => {
  test("requires canonical exact-key evidence with matching build IDs", () => {
    const version = "0.7.0-beta.1";
    const hash = "a".repeat(64);
    const buildId = "0123456789abcdef0123456789abcdef01234567";
    const evidence = {
      schemaVersion: 1,
      version,
      channel: "beta",
      candidateChecksumsSha256: "b".repeat(64),
      packages: {
        appimage: {
          filename: `Unfocus_${version}_amd64.AppImage`,
          sizeBytes: 256,
          sha256: hash,
          filesystemOffset: 64,
          inodeCount: 1,
          innerExecutableSha256: "c".repeat(64),
          innerExecutableBuildId: buildId,
        },
        deb: {
          filename: `Unfocus_${version}_amd64.deb`,
          sizeBytes: 256,
          sha256: hash,
          package: "unfocus",
          version: "0.7.0~beta.1-1",
          architecture: "amd64",
          innerExecutableBuildId: buildId,
        },
        rpm: {
          filename: `Unfocus-${version}-1.x86_64.rpm`,
          sizeBytes: 256,
          sha256: hash,
          name: "unfocus",
          version,
          release: "1",
          architecture: "x86_64",
          innerExecutableBuildId: buildId,
        },
      },
    };
    const text = `${JSON.stringify(evidence, null, 2)}\n`;
    expect(parsePackageEvidence(text, version)).toEqual(evidence);

    evidence.packages.rpm.innerExecutableBuildId = "f".repeat(40);
    expect(() => parsePackageEvidence(`${JSON.stringify(evidence, null, 2)}\n`, version)).toThrow(
      "mismatched executable build IDs",
    );
    expect(() => parsePackageEvidence(JSON.stringify(evidence), version)).toThrow("not canonical JSON");
  });
});

describe("credential-free candidate gate", () => {
  test("rejects incomplete inventory before invoking package tools", async () => {
    const directory = temporaryDirectory();
    await expect(inspectLinuxPackages(directory, "0.7.0-beta.1")).rejects.toThrow(
      "release candidate inventory must be exactly",
    );
  });

  test("rejects unsupported release versions before inspecting files", async () => {
    const directory = temporaryDirectory();
    await expect(inspectLinuxPackages(directory, "0.7.0-preview.1")).rejects.toThrow(
      "unsupported release version",
    );
  });
});
