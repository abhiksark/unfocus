#!/usr/bin/env bun

import { createHash } from "node:crypto";
import {
  closeSync,
  createReadStream,
  existsSync,
  lstatSync,
  mkdirSync,
  mkdtempSync,
  openSync,
  readFileSync,
  readlinkSync,
  readSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join, posix, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { expectedReleaseFilenames, verifyChecksums } from "./assemble-release-artifacts.js";
import { semverToDebianVersion } from "./debian-package.js";
import { releaseChannel } from "./linux-update-envelope.js";

const REPOSITORY_ROOT = resolve(fileURLToPath(new URL("..", import.meta.url)));
const EVIDENCE_SCHEMA_VERSION = 1;
const MAX_PACKAGE_BYTES = 536_870_912;
const MAX_COMMAND_OUTPUT = 64 * 1024 * 1024;
const MAX_UNPACKED_BYTES = 1_073_741_824;
const MAX_INODES = 10_000;
const MAX_DEPTH = 64;
const MAX_PATH_BYTES = 4_096;
const MAX_APP_RUN_BYTES = 1_048_576;
const MAX_EXECUTABLE_BYTES = 134_217_728;
const MAX_DEBIAN_CONTROL_ARCHIVE_BYTES = 4 * 1024 * 1024;
const TAR_BLOCK_BYTES = 512;
const REQUIRED_COMMON_PATHS = [
  "usr/bin/unfocus",
  "usr/lib/Unfocus/THIRD_PARTY_NOTICES.txt",
  "usr/share/applications/Unfocus.desktop",
  "usr/share/icons/hicolor/32x32/apps/unfocus.png",
  "usr/share/icons/hicolor/128x128/apps/unfocus.png",
  "usr/share/icons/hicolor/256x256@2/apps/unfocus.png",
];
const RPM_PATHS = [...REQUIRED_COMMON_PATHS.map((path) => `/${path}`), "/usr/lib/Unfocus"].sort();
const DEBIAN_DIRECTORIES = [
  "usr",
  "usr/bin",
  "usr/lib",
  "usr/lib/Unfocus",
  "usr/share",
  "usr/share/applications",
  "usr/share/doc",
  "usr/share/doc/unfocus",
  "usr/share/icons",
  "usr/share/icons/hicolor",
  "usr/share/icons/hicolor/32x32",
  "usr/share/icons/hicolor/32x32/apps",
  "usr/share/icons/hicolor/128x128",
  "usr/share/icons/hicolor/128x128/apps",
  "usr/share/icons/hicolor/256x256@2",
  "usr/share/icons/hicolor/256x256@2/apps",
].sort();
const SHA256 = /^[0-9a-f]{64}$/;
const BUILD_ID = /^[0-9a-f]{16,128}$/;

function canonicalJson(value) {
  return `${JSON.stringify(value, null, 2)}\n`;
}

function exactKeys(value, expected, label) {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error(`${label} must be an object`);
  const actual = Object.keys(value);
  if (actual.length !== expected.length || actual.some((key, index) => key !== expected[index])) {
    throw new Error(`${label} keys must be exactly: ${expected.join(", ")}`);
  }
}

function validEvidenceFile(package_, filename, label) {
  if (package_.filename !== filename) throw new Error(`${label} filename must be ${filename}`);
  if (!Number.isSafeInteger(package_.sizeBytes) || package_.sizeBytes <= 0 || package_.sizeBytes > MAX_PACKAGE_BYTES) {
    throw new Error(`${label} sizeBytes is invalid`);
  }
  if (!SHA256.test(package_.sha256)) throw new Error(`${label} SHA-256 is invalid`);
  if (!BUILD_ID.test(package_.innerExecutableBuildId)) throw new Error(`${label} GNU build ID is invalid`);
}

export function parsePackageEvidence(text, expectedVersion) {
  if (typeof text !== "string" || Buffer.byteLength(text) > 64 * 1024) {
    throw new Error("Linux package evidence must be bounded UTF-8 text");
  }
  let evidence;
  try {
    evidence = JSON.parse(text);
  } catch (error) {
    throw new Error(`Linux package evidence is not valid JSON: ${error.message}`);
  }
  if (canonicalJson(evidence) !== text) throw new Error("Linux package evidence is not canonical JSON");
  exactKeys(
    evidence,
    ["schemaVersion", "version", "channel", "candidateChecksumsSha256", "packages"],
    "Linux package evidence",
  );
  if (evidence.schemaVersion !== EVIDENCE_SCHEMA_VERSION) {
    throw new Error(`Linux package evidence schemaVersion must be ${EVIDENCE_SCHEMA_VERSION}`);
  }
  const channel = releaseChannel(evidence.version);
  if (evidence.channel !== channel) throw new Error(`Linux package evidence channel must be ${channel}`);
  if (expectedVersion !== undefined && evidence.version !== expectedVersion) {
    throw new Error(`Linux package evidence version is ${evidence.version}, expected ${expectedVersion}`);
  }
  if (!SHA256.test(evidence.candidateChecksumsSha256)) {
    throw new Error("Linux package evidence candidate checksum digest is invalid");
  }
  exactKeys(evidence.packages, ["appimage", "deb", "rpm"], "Linux package evidence packages");

  const { appimage, deb, rpm } = evidence.packages;
  exactKeys(
    appimage,
    [
      "filename",
      "sizeBytes",
      "sha256",
      "filesystemOffset",
      "inodeCount",
      "innerExecutableSha256",
      "innerExecutableBuildId",
    ],
    "Linux package evidence AppImage",
  );
  validEvidenceFile(appimage, `Unfocus_${evidence.version}_amd64.AppImage`, "Linux package evidence AppImage");
  if (
    !Number.isSafeInteger(appimage.filesystemOffset) ||
    appimage.filesystemOffset < 64 ||
    appimage.filesystemOffset + 96 > appimage.sizeBytes
  ) {
    throw new Error("Linux package evidence AppImage filesystem offset is invalid");
  }
  if (!Number.isSafeInteger(appimage.inodeCount) || appimage.inodeCount <= 0 || appimage.inodeCount > MAX_INODES) {
    throw new Error("Linux package evidence AppImage inode count is invalid");
  }
  if (!SHA256.test(appimage.innerExecutableSha256)) {
    throw new Error("Linux package evidence AppImage inner executable SHA-256 is invalid");
  }

  exactKeys(
    deb,
    ["filename", "sizeBytes", "sha256", "package", "version", "architecture", "innerExecutableBuildId"],
    "Linux package evidence Debian",
  );
  validEvidenceFile(deb, `Unfocus_${evidence.version}_amd64.deb`, "Linux package evidence Debian");
  if (deb.package !== "unfocus" || deb.version !== semverToDebianVersion(evidence.version) || deb.architecture !== "amd64") {
    throw new Error("Linux package evidence Debian identity is invalid");
  }

  exactKeys(
    rpm,
    ["filename", "sizeBytes", "sha256", "name", "version", "release", "architecture", "innerExecutableBuildId"],
    "Linux package evidence RPM",
  );
  validEvidenceFile(rpm, `Unfocus-${evidence.version}-1.x86_64.rpm`, "Linux package evidence RPM");
  if (rpm.name !== "unfocus" || rpm.version !== evidence.version || rpm.release !== "1" || rpm.architecture !== "x86_64") {
    throw new Error("Linux package evidence RPM identity is invalid");
  }
  if (new Set([appimage.innerExecutableBuildId, deb.innerExecutableBuildId, rpm.innerExecutableBuildId]).size !== 1) {
    throw new Error("Linux package evidence contains mismatched executable build IDs");
  }
  return evidence;
}

function commandEnvironment() {
  return { ...process.env, LANG: "C", LC_ALL: "C", TZ: "UTC" };
}

function run(command, arguments_, options = {}) {
  const result = spawnSync(command, arguments_, {
    cwd: options.cwd,
    encoding: options.encoding === undefined ? "utf8" : options.encoding,
    env: commandEnvironment(),
    input: options.input,
    maxBuffer: options.maxBuffer ?? MAX_COMMAND_OUTPUT,
    stdio: options.stdio,
    timeout: options.timeout ?? 60_000,
  });
  if (result.error) throw new Error(`${command} failed to start: ${result.error.message}`);
  if (result.signal) throw new Error(`${command} was terminated by ${result.signal}`);
  const stderr =
    typeof result.stderr === "string" ? result.stderr.trim() : result.stderr?.toString("utf8").trim();
  if (result.status !== 0) {
    throw new Error(`${command} exited with ${result.status}${stderr ? `: ${stderr}` : ""}`);
  }
  if (options.rejectStderr && stderr) throw new Error(`${command} reported an error: ${stderr}`);
  return result.stdout;
}

function regularFile(path, label, maximumBytes = MAX_PACKAGE_BYTES) {
  if (!existsSync(path)) throw new Error(`${label} is missing: ${path}`);
  const stat = lstatSync(path);
  if (!stat.isFile()) throw new Error(`${label} must be a regular non-symlink file: ${path}`);
  if (stat.size <= 0 || stat.size > maximumBytes) {
    throw new Error(`${label} size must be between 1 and ${maximumBytes} bytes`);
  }
  return stat;
}

async function digestFile(path, algorithm) {
  const hash = createHash(algorithm);
  for await (const chunk of createReadStream(path)) hash.update(chunk);
  return hash.digest("hex");
}

async function sha256File(path) {
  return digestFile(path, "sha256");
}

async function md5File(path) {
  return digestFile(path, "md5");
}

function readRange(file, offset, size, fileSize, label) {
  if (
    !Number.isSafeInteger(offset) ||
    !Number.isSafeInteger(size) ||
    offset < 0 ||
    size < 0 ||
    offset + size > fileSize
  ) {
    throw new Error(`${label} range is outside the file`);
  }
  const buffer = Buffer.alloc(size);
  const count = readSync(file, buffer, 0, size, offset);
  if (count !== size) throw new Error(`${label} could not be read completely`);
  return buffer;
}

function safeNumber(value, label) {
  if (value > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error(`${label} exceeds the safe integer range`);
  return Number(value);
}

function checkedTableEnd(offset, entrySize, count, fileSize, label) {
  const size = entrySize * count;
  if (!Number.isSafeInteger(size) || offset < 0 || offset + size > fileSize) {
    throw new Error(`${label} table is outside the file`);
  }
  return offset + size;
}

function inspectProgramHeaders(file, header, fileSize, label) {
  const { entryPoint, programHeaderOffset, programHeaderSize, programHeaderCount } = header;
  const tableSize = programHeaderSize * programHeaderCount;
  const table = readRange(
    file,
    programHeaderOffset,
    tableSize,
    fileSize,
    `${label} program header table`,
  );
  let maximumEnd = programHeaderOffset + tableSize;
  let loadSegments = 0;
  let executableEntry = false;
  for (let index = 0; index < programHeaderCount; index += 1) {
    const offset = index * programHeaderSize;
    const type = table.readUInt32LE(offset);
    const flags = table.readUInt32LE(offset + 4);
    const fileOffset = safeNumber(table.readBigUInt64LE(offset + 8), `${label} program header ${index} offset`);
    const virtualAddress = table.readBigUInt64LE(offset + 16);
    const fileBytes = safeNumber(
      table.readBigUInt64LE(offset + 32),
      `${label} program header ${index} file size`,
    );
    const memoryBytes = safeNumber(
      table.readBigUInt64LE(offset + 40),
      `${label} program header ${index} memory size`,
    );
    const alignment = table.readBigUInt64LE(offset + 48);
    if (fileOffset + fileBytes > fileSize) {
      throw new Error(`${label} program header ${index} references bytes outside the file`);
    }
    if (type === 1) {
      loadSegments += 1;
      if (fileBytes > memoryBytes) {
        throw new Error(`${label} load segment ${index} is larger on disk than in memory`);
      }
      if (alignment > 1n) {
        if ((alignment & (alignment - 1n)) !== 0n) {
          throw new Error(`${label} load segment ${index} has invalid alignment`);
        }
        if (virtualAddress % alignment !== BigInt(fileOffset) % alignment) {
          throw new Error(`${label} load segment ${index} has inconsistent alignment`);
        }
      }
      const memoryEnd = virtualAddress + BigInt(memoryBytes);
      if ((flags & 1) !== 0 && entryPoint >= virtualAddress && entryPoint < memoryEnd) executableEntry = true;
    }
    maximumEnd = Math.max(maximumEnd, fileOffset + fileBytes);
  }
  if (loadSegments === 0 || !executableEntry) {
    throw new Error(`${label} must contain a loadable executable segment covering its entry point`);
  }
  return maximumEnd;
}

function inspectSectionHeaders(file, header, fileSize) {
  const { sectionHeaderOffset, sectionHeaderSize, sectionHeaderCount } = header;
  const tableSize = sectionHeaderSize * sectionHeaderCount;
  const table = readRange(file, sectionHeaderOffset, tableSize, fileSize, "ELF section header table");
  let maximumEnd = sectionHeaderOffset + tableSize;
  for (let index = 0; index < sectionHeaderCount; index += 1) {
    const offset = index * sectionHeaderSize;
    const type = table.readUInt32LE(offset + 4);
    const fileOffset = safeNumber(table.readBigUInt64LE(offset + 24), `ELF section header ${index} offset`);
    const sectionSize = safeNumber(table.readBigUInt64LE(offset + 32), `ELF section header ${index} size`);
    if (type !== 8 && fileOffset + sectionSize > fileSize) {
      throw new Error(`ELF section header ${index} references bytes outside the file`);
    }
    if (type !== 8) maximumEnd = Math.max(maximumEnd, fileOffset + sectionSize);
  }
  return maximumEnd;
}

function parseElfHeader(bytes, label) {
  if (bytes.length < 64 || !bytes.subarray(0, 4).equals(Buffer.from([0x7f, 0x45, 0x4c, 0x46]))) {
    throw new Error(`${label} is not an ELF file`);
  }
  if (bytes[4] !== 2 || bytes[5] !== 1 || bytes[6] !== 1) {
    throw new Error(`${label} must be ELF64, little-endian, version 1`);
  }
  const type = bytes.readUInt16LE(16);
  if (type !== 2 && type !== 3) throw new Error(`${label} must be an executable or position-independent executable`);
  if (bytes.readBigUInt64LE(24) === 0n) throw new Error(`${label} must have a nonzero entry point`);
  if (bytes.readUInt16LE(18) !== 62 || bytes.readUInt32LE(20) !== 1) {
    throw new Error(`${label} must be an x86_64 ELF version 1 file`);
  }
  if (bytes.readUInt16LE(52) !== 64) throw new Error(`${label} has an unsupported ELF header size`);
  const programHeaderSize = bytes.readUInt16LE(54);
  const programHeaderCount = bytes.readUInt16LE(56);
  const sectionHeaderSize = bytes.readUInt16LE(58);
  const sectionHeaderCount = bytes.readUInt16LE(60);
  if (programHeaderSize !== 56 || programHeaderCount === 0 || programHeaderCount > 128) {
    throw new Error(`${label} has unsupported ELF program headers`);
  }
  if (sectionHeaderSize !== 64 || sectionHeaderCount === 0 || sectionHeaderCount > 4_096) {
    throw new Error(`${label} has unsupported ELF section headers`);
  }
  const sectionNameIndex = bytes.readUInt16LE(62);
  if (sectionNameIndex >= sectionHeaderCount) throw new Error(`${label} has an invalid section-name index`);
  return {
    entryPoint: bytes.readBigUInt64LE(24),
    programHeaderOffset: safeNumber(bytes.readBigUInt64LE(32), `${label} program header offset`),
    sectionHeaderOffset: safeNumber(bytes.readBigUInt64LE(40), `${label} section header offset`),
    programHeaderSize,
    programHeaderCount,
    sectionHeaderSize,
    sectionHeaderCount,
  };
}

function validateSquashfsSuperblock(bytes, filesystemOffset, fileSize) {
  if (bytes.readUInt32LE(0) !== 0x7371_7368) throw new Error("AppImage appended filesystem is not SquashFS");
  const inodeCount = bytes.readUInt32LE(4);
  const blockSize = bytes.readUInt32LE(12);
  const compression = bytes.readUInt16LE(20);
  const blockLog = bytes.readUInt16LE(22);
  const major = bytes.readUInt16LE(28);
  const minor = bytes.readUInt16LE(30);
  const bytesUsed = safeNumber(bytes.readBigUInt64LE(40), "SquashFS bytes used");
  if (inodeCount === 0 || inodeCount > MAX_INODES) throw new Error(`SquashFS inode count exceeds ${MAX_INODES}`);
  if (blockLog > 20 || 2 ** blockLog !== blockSize || blockSize < 4_096 || blockSize > 1_048_576) {
    throw new Error("SquashFS block size is unsupported");
  }
  if (compression !== 6) throw new Error("SquashFS compression must be zstd");
  if (major !== 4 || minor !== 0) throw new Error("SquashFS version must be 4.0");
  if (bytesUsed < 96 || filesystemOffset + bytesUsed > fileSize) {
    throw new Error("SquashFS bytes-used range is outside the AppImage");
  }
  for (const offset of [48, 56, 64, 72, 80, 88]) {
    const table = bytes.readBigUInt64LE(offset);
    if (table !== 0xffff_ffff_ffff_ffffn && table >= BigInt(bytesUsed)) {
      throw new Error("SquashFS table offset is outside the appended filesystem");
    }
  }
  return { bytesUsed, compression: "zstd", inodeCount };
}

export function inspectAppImageOuter(path) {
  const stat = regularFile(path, "AppImage");
  const file = openSync(path, "r");
  try {
    const headerBytes = readRange(file, 0, 64, stat.size, "AppImage ELF header");
    const header = parseElfHeader(headerBytes, "AppImage runtime");
    if (!headerBytes.subarray(8, 11).equals(Buffer.from([0x41, 0x49, 0x02]))) {
      throw new Error("AppImage type-2 marker is missing at ELF offset 8");
    }
    checkedTableEnd(
      header.programHeaderOffset,
      header.programHeaderSize,
      header.programHeaderCount,
      stat.size,
      "ELF program header",
    );
    checkedTableEnd(
      header.sectionHeaderOffset,
      header.sectionHeaderSize,
      header.sectionHeaderCount,
      stat.size,
      "ELF section header",
    );
    const programEnd = inspectProgramHeaders(file, header, stat.size, "AppImage runtime");
    const sectionEnd = inspectSectionHeaders(file, header, stat.size);
    const filesystemOffset = Math.max(64, programEnd, sectionEnd);
    const superblockBytes = readRange(file, filesystemOffset, 96, stat.size, "SquashFS superblock");
    const squashfs = validateSquashfsSuperblock(superblockBytes, filesystemOffset, stat.size);
    const paddingOffset = filesystemOffset + squashfs.bytesUsed;
    const paddingSize = stat.size - paddingOffset;
    if (paddingSize >= 4_096) {
      throw new Error("AppImage has excessive bytes after its SquashFS filesystem");
    }
    if (
      readRange(file, paddingOffset, paddingSize, stat.size, "AppImage SquashFS padding").some(
        (byte) => byte !== 0,
      )
    ) {
      throw new Error("AppImage has nonzero bytes after its SquashFS filesystem");
    }
    return { fileSize: stat.size, filesystemOffset, squashfs };
  } finally {
    closeSync(file);
  }
}

function validArchivePath(path, label) {
  const relative = path.replace(/^\.\//, "").replace(/^\//, "");
  if (!relative || relative.includes("\0") || Buffer.byteLength(relative) > MAX_PATH_BYTES) {
    throw new Error(`${label} contains an invalid path`);
  }
  const components = relative.split("/");
  if (components.length > MAX_DEPTH || components.some((part) => !part || part === "." || part === "..")) {
    throw new Error(`${label} contains an unsafe path: ${path}`);
  }
  return relative;
}

function detectSymlinkCycles(entries) {
  const links = new Map(entries.filter((entry) => entry.type === "l").map((entry) => [entry.path, entry.target]));
  for (const start of links.keys()) {
    const seen = new Set();
    let current = start;
    while (links.has(current)) {
      if (seen.has(current)) throw new Error(`AppImage contains a symlink cycle at ${start}`);
      seen.add(current);
      const target = links.get(current);
      if (target.startsWith("/")) throw new Error(`AppImage symlink ${current} has an absolute target`);
      const resolved = posix.normalize(posix.join(posix.dirname(current), target));
      if (resolved === ".." || resolved.startsWith("../")) {
        throw new Error(`AppImage symlink ${current} escapes the filesystem root`);
      }
      current = resolved;
    }
  }
}

export function parseSquashfsListing(text) {
  if (Buffer.byteLength(text) > MAX_COMMAND_OUTPUT) throw new Error("SquashFS listing exceeds the output limit");
  const entries = [];
  const seen = new Set();
  let unpackedBytes = 0;
  for (const line of text.trimEnd().split("\n")) {
    const match = /^([dl-][rwx-]{9})\s+0\/0\s+(\d+)\s+\d{4}-\d{2}-\d{2} \d{2}:\d{2} squashfs-root(?:\/(.*))?$/.exec(
      line,
    );
    if (!match) throw new Error(`could not parse SquashFS listing line: ${line}`);
    const mode = match[1];
    const size = Number(match[2]);
    const rawPath = match[3];
    if (rawPath === undefined) {
      if (mode[0] !== "d") throw new Error("SquashFS root must be a directory");
      continue;
    }
    const arrow = mode[0] === "l" ? rawPath.lastIndexOf(" -> ") : -1;
    const listedPath = arrow === -1 ? rawPath : rawPath.slice(0, arrow);
    const path = validArchivePath(listedPath, "SquashFS listing");
    if (seen.has(path)) throw new Error(`SquashFS contains duplicate path ${path}`);
    seen.add(path);
    if (!Number.isSafeInteger(size) || size < 0) throw new Error(`SquashFS entry ${path} has an invalid size`);
    if (mode[0] === "-") {
      unpackedBytes += size;
      if (!Number.isSafeInteger(unpackedBytes) || unpackedBytes > MAX_UNPACKED_BYTES) {
        throw new Error(`SquashFS unpacked file bytes exceed ${MAX_UNPACKED_BYTES}`);
      }
    }
    const entry = { path, type: mode[0], mode, size };
    if (mode[0] === "l") {
      if (arrow === -1) throw new Error(`SquashFS symlink ${path} has no target`);
      const target = rawPath.slice(arrow + 4);
      if (!target || Buffer.byteLength(target) > MAX_PATH_BYTES || target.includes("\0")) {
        throw new Error(`SquashFS symlink ${path} has an invalid target`);
      }
      entry.target = target;
    }
    entries.push(entry);
    if (entries.length > MAX_INODES) throw new Error(`SquashFS listing exceeds ${MAX_INODES} entries`);
  }
  detectSymlinkCycles(entries);
  const existingPaths = new Set(entries.map((entry) => entry.path));
  for (const entry of entries.filter((candidate) => candidate.type === "l")) {
    const target = posix.normalize(posix.join(posix.dirname(entry.path), entry.target));
    if (!existingPaths.has(target)) throw new Error(`AppImage symlink ${entry.path} has a missing target ${target}`);
  }
  return entries;
}

function requireListedEntry(entries, path, maximumSize, executable = false) {
  const entry = entries.find((candidate) => candidate.path === path);
  if (!entry || entry.type !== "-") throw new Error(`AppImage requires regular file ${path}`);
  if (entry.size <= 0 || entry.size > maximumSize) {
    throw new Error(`AppImage ${path} size must be between 1 and ${maximumSize} bytes`);
  }
  if (executable && !/[x]/.test(entry.mode.slice(1))) throw new Error(`AppImage ${path} must be executable`);
  return entry;
}

function inspectSquashfsListing(appImagePath, filesystemOffset) {
  const listing = run("unsquashfs", ["-lln", "-o", String(filesystemOffset), appImagePath], {
    maxBuffer: MAX_COMMAND_OUTPUT,
    timeout: 120_000,
  });
  const entries = parseSquashfsListing(listing);
  requireListedEntry(entries, "AppRun", MAX_APP_RUN_BYTES, true);
  requireListedEntry(entries, "usr/bin/unfocus", MAX_EXECUTABLE_BYTES, true);
  requireListedEntry(entries, "usr/lib/Unfocus/THIRD_PARTY_NOTICES.txt", 32 * 1024 * 1024);
  requireListedEntry(entries, "usr/share/applications/Unfocus.desktop", 1024 * 1024);
  for (const path of REQUIRED_COMMON_PATHS.filter((candidate) => candidate.endsWith("/unfocus.png"))) {
    requireListedEntry(entries, path, 16 * 1024 * 1024);
  }
  return entries;
}

function extractAppImageFilesystem(appImagePath, filesystemOffset, output) {
  run("unsquashfs", ["-f", "-d", output, "-o", String(filesystemOffset), appImagePath], {
    timeout: 120_000,
  });
}

function listedPermissions(mode) {
  const masks = [0o400, 0o200, 0o100, 0o040, 0o020, 0o010, 0o004, 0o002, 0o001];
  const expected = ["r", "w", "x", "r", "w", "x", "r", "w", "x"];
  return mode
    .slice(1)
    .split("")
    .reduce((bits, character, index) => (character === expected[index] ? bits | masks[index] : bits), 0);
}

function collectExtractedEntries(root, current = root, entries = []) {
  for (const directoryEntry of readdirSync(current, { withFileTypes: true })) {
    const path = join(current, directoryEntry.name);
    const stat = lstatSync(path);
    const archivePath = relative(root, path).split(sep).join("/");
    validArchivePath(archivePath, "extracted AppImage");
    let type;
    if (stat.isDirectory()) type = "d";
    else if (stat.isFile()) type = "-";
    else if (stat.isSymbolicLink()) type = "l";
    else throw new Error(`extracted AppImage contains unsupported entry ${archivePath}`);
    const entry = { path: archivePath, type, mode: stat.mode & 0o777, size: stat.size };
    if (type === "l") entry.target = readlinkSync(path);
    entries.push(entry);
    if (entries.length > MAX_INODES) throw new Error(`extracted AppImage exceeds ${MAX_INODES} entries`);
    if (type === "d") collectExtractedEntries(root, path, entries);
  }
  return entries;
}

function reconcileExtractedAppImage(root, listedEntries) {
  const expected = new Map(listedEntries.map((entry) => [entry.path, entry]));
  const extracted = collectExtractedEntries(root);
  if (extracted.length !== expected.size) throw new Error("extracted AppImage does not match its SquashFS listing");
  for (const actual of extracted) {
    const listed = expected.get(actual.path);
    if (
      !listed ||
      actual.type !== listed.type ||
      actual.mode !== listedPermissions(listed.mode) ||
      (actual.type === "-" && actual.size !== listed.size) ||
      (actual.type === "l" && actual.target !== listed.target)
    ) {
      throw new Error(`extracted AppImage entry ${actual.path} does not match its SquashFS listing`);
    }
  }
}

function extractedRegularFile(root, relativePath, maximumSize, executable = false) {
  const path = join(root, relativePath);
  const stat = regularFile(path, `extracted ${relativePath}`, maximumSize);
  if (executable && (stat.mode & 0o111) === 0) throw new Error(`extracted ${relativePath} is not executable`);
  return path;
}

function validateDesktopEntry(path) {
  const text = readFileSync(path, "utf8");
  if (text.includes("\r") || text.includes("\0") || !text.endsWith("\n")) {
    throw new Error("packaged desktop entry is not canonical UTF-8 text");
  }
  const lines = text.trimEnd().split("\n");
  if (lines.shift() !== "[Desktop Entry]") throw new Error("packaged desktop entry has the wrong section");
  const fields = new Map();
  for (const line of lines) {
    const match = /^([A-Za-z]+)=(.*)$/.exec(line);
    if (!match || fields.has(match[1])) throw new Error(`packaged desktop entry has invalid field ${line}`);
    fields.set(match[1], match[2]);
  }
  const expected = {
    Categories: "Utility;",
    Comment: "Local-first break and reflection app",
    Exec: "unfocus",
    StartupWMClass: "unfocus",
    Icon: "unfocus",
    Name: "Unfocus",
    Terminal: "false",
    Type: "Application",
  };
  if (
    fields.size !== Object.keys(expected).length ||
    Object.entries(expected).some(([key, value]) => fields.get(key) !== value)
  ) {
    throw new Error("packaged desktop entry identity or launch fields are invalid");
  }
}

function validatePng(path) {
  const bytes = readFileSync(path);
  if (bytes.length < 24 || !bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) {
    throw new Error(`packaged icon is not a PNG: ${path}`);
  }
  const width = bytes.readUInt32BE(16);
  const height = bytes.readUInt32BE(20);
  if (width === 0 || height === 0 || width > 2_048 || height > 2_048) {
    throw new Error(`packaged icon has invalid dimensions: ${path}`);
  }
}

function validateExtractedPresentation(root) {
  validateDesktopEntry(extractedRegularFile(root, "usr/share/applications/Unfocus.desktop", 1024 * 1024));
  for (const icon of REQUIRED_COMMON_PATHS.filter((candidate) => candidate.endsWith("/unfocus.png"))) {
    validatePng(extractedRegularFile(root, icon, 16 * 1024 * 1024));
  }
}

export function parseBuildId(text, label = "ELF executable") {
  const matches = [...text.matchAll(/\bBuild ID: ([0-9a-f]+)\s*$/gm)].map((match) => match[1]);
  if (matches.length !== 1 || !BUILD_ID.test(matches[0])) {
    throw new Error(`${label} must contain exactly one valid GNU build ID`);
  }
  return matches[0];
}

function inspectExecutable(path, label) {
  const stat = regularFile(path, label, MAX_EXECUTABLE_BYTES);
  if ((stat.mode & 0o111) === 0) throw new Error(`${label} must be executable`);
  const file = openSync(path, "r");
  try {
    const header = parseElfHeader(readRange(file, 0, 64, stat.size, `${label} ELF header`), label);
    checkedTableEnd(
      header.programHeaderOffset,
      header.programHeaderSize,
      header.programHeaderCount,
      stat.size,
      `${label} program header`,
    );
    checkedTableEnd(
      header.sectionHeaderOffset,
      header.sectionHeaderSize,
      header.sectionHeaderCount,
      stat.size,
      `${label} section header`,
    );
    inspectProgramHeaders(file, header, stat.size, label);
    inspectSectionHeaders(file, header, stat.size);
  } finally {
    closeSync(file);
  }
  return parseBuildId(run("readelf", ["--notes", "--wide", path], { rejectStderr: true }), label);
}

function parseTarOctal(field, label) {
  if (field.some((byte) => byte > 0x7f)) throw new Error(`Debian control archive ${label} is not octal`);
  const terminator = field.indexOf(0);
  const end = terminator === -1 ? field.length : terminator;
  if (field.subarray(end).some((byte) => byte !== 0 && byte !== 0x20)) {
    throw new Error(`Debian control archive ${label} has bytes after its terminator`);
  }
  const text = field.subarray(0, end).toString("ascii").trim();
  if (!/^[0-7]+$/.test(text)) throw new Error(`Debian control archive ${label} is not octal`);
  const value = Number.parseInt(text, 8);
  if (!Number.isSafeInteger(value)) throw new Error(`Debian control archive ${label} is too large`);
  return value;
}

function parseTarText(field, label) {
  const terminator = field.indexOf(0);
  const end = terminator === -1 ? field.length : terminator;
  if (field.subarray(end).some((byte) => byte !== 0)) {
    throw new Error(`Debian control archive ${label} has bytes after its terminator`);
  }
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(field.subarray(0, end));
  } catch {
    throw new Error(`Debian control archive ${label} is not UTF-8`);
  }
}

function validateTarChecksum(header) {
  const expected = parseTarOctal(header.subarray(148, 156), "header checksum");
  let actual = 0;
  for (let index = 0; index < header.length; index += 1) {
    actual += index >= 148 && index < 156 ? 0x20 : header[index];
  }
  if (actual !== expected) throw new Error("Debian control archive header checksum is invalid");
}

export function parseDebianControlArchive(archive) {
  if (
    !Buffer.isBuffer(archive) ||
    archive.length < TAR_BLOCK_BYTES * 2 ||
    archive.length > MAX_DEBIAN_CONTROL_ARCHIVE_BYTES ||
    archive.length % TAR_BLOCK_BYTES !== 0
  ) {
    throw new Error("Debian control archive has an invalid size");
  }

  const entries = [];
  const paths = new Set();
  let offset = 0;
  while (offset + TAR_BLOCK_BYTES <= archive.length) {
    const header = archive.subarray(offset, offset + TAR_BLOCK_BYTES);
    if (header.every((byte) => byte === 0)) {
      if (offset + TAR_BLOCK_BYTES * 2 > archive.length || archive.subarray(offset).some((byte) => byte !== 0)) {
        throw new Error("Debian control archive has an invalid end marker");
      }
      return entries;
    }
    validateTarChecksum(header);
    if (header.subarray(257, 262).toString("ascii") !== "ustar") {
      throw new Error("Debian control archive must use ustar headers");
    }
    const name = parseTarText(header.subarray(0, 100), "entry name");
    const prefix = parseTarText(header.subarray(345, 500), "entry prefix");
    const rawPath = prefix ? `${prefix}/${name}` : name;
    if (rawPath.startsWith("/")) {
      throw new Error(`Debian control archive contains an unsafe path: ${rawPath}`);
    }
    const path = rawPath === "./" ? "." : validArchivePath(rawPath, "Debian control archive");
    if (paths.has(path)) throw new Error(`Debian control archive contains duplicate path ${path}`);
    paths.add(path);
    const mode = parseTarOctal(header.subarray(100, 108), `${path} mode`);
    const uid = parseTarOctal(header.subarray(108, 116), `${path} uid`);
    const gid = parseTarOctal(header.subarray(116, 124), `${path} gid`);
    const size = parseTarOctal(header.subarray(124, 136), `${path} size`);
    const typeByte = header[156];
    const type = typeByte === 0 ? "0" : String.fromCharCode(typeByte);
    const linkName = parseTarText(header.subarray(157, 257), `${path} link name`);
    const dataStart = offset + TAR_BLOCK_BYTES;
    const dataEnd = dataStart + size;
    const nextOffset = dataStart + Math.ceil(size / TAR_BLOCK_BYTES) * TAR_BLOCK_BYTES;
    if (dataEnd > archive.length || nextOffset > archive.length) {
      throw new Error(`Debian control archive entry ${path} is truncated`);
    }
    if (archive.subarray(dataEnd, nextOffset).some((byte) => byte !== 0)) {
      throw new Error(`Debian control archive entry ${path} has nonzero padding`);
    }
    entries.push({
      path,
      mode,
      uid,
      gid,
      size,
      type,
      linkName,
      data: Buffer.from(archive.subarray(dataStart, dataEnd)),
    });
    if (entries.length > 32) throw new Error("Debian control archive contains too many entries");
    offset = nextOffset;
  }
  throw new Error("Debian control archive has no end marker");
}

export function validateDebianControlArchive(archive) {
  const entries = parseDebianControlArchive(archive).sort((left, right) => left.path.localeCompare(right.path));
  const expected = [
    { path: ".", type: "5", mode: 0o755, minimumSize: 0, maximumSize: 0 },
    { path: "control", type: "0", mode: 0o644, minimumSize: 1, maximumSize: 64 * 1024 },
    { path: "md5sums", type: "0", mode: 0o644, minimumSize: 1, maximumSize: 2 * 1024 * 1024 },
  ];
  if (entries.length !== expected.length) {
    throw new Error("Debian control archive must contain only its root, control, and md5sums");
  }
  for (let index = 0; index < expected.length; index += 1) {
    const entry = entries[index];
    const wanted = expected[index];
    if (
      entry.path !== wanted.path ||
      entry.type !== wanted.type ||
      entry.linkName !== "" ||
      entry.mode !== wanted.mode ||
      entry.uid !== 0 ||
      entry.gid !== 0 ||
      entry.size < wanted.minimumSize ||
      entry.size > wanted.maximumSize
    ) {
      throw new Error(`Debian control archive has invalid ${wanted.path} metadata`);
    }
  }
  return entries;
}

export function parseDebianMd5Sums(bytes, expectedPaths) {
  let text;
  try {
    text = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    throw new Error("Debian md5sums is not UTF-8");
  }
  if (!text.endsWith("\n") || text.includes("\r") || text.includes("\0")) {
    throw new Error("Debian md5sums must be canonical LF-terminated text");
  }
  const entries = text.slice(0, -1).split("\n").map((line) => {
    const match = /^([0-9a-f]{32})  ([^\r\n]+)$/.exec(line);
    if (!match) throw new Error(`invalid Debian md5sums line: ${line}`);
    const path = validArchivePath(match[2], "Debian md5sums");
    if (path !== match[2]) throw new Error(`Debian md5sums contains a noncanonical path: ${match[2]}`);
    return { digest: match[1], path };
  });
  const paths = entries.map((entry) => entry.path);
  if (new Set(paths).size !== paths.length) throw new Error("Debian md5sums contains duplicate paths");
  const sortedPaths = [...paths].sort();
  const sortedExpected = [...expectedPaths].sort();
  if (
    sortedPaths.length !== sortedExpected.length ||
    sortedPaths.some((path, index) => path !== sortedExpected[index])
  ) {
    throw new Error("Debian md5sums must cover every payload file exactly");
  }
  return entries;
}

export function requireEmptyRpmScriptListing(text, label) {
  if (typeof text !== "string" || Buffer.byteLength(text) > 1024 * 1024) {
    throw new Error(`RPM ${label} listing is invalid`);
  }
  if (text !== "") throw new Error(`RPM package contains ${label}`);
}

function validateRpmScriptlets(path) {
  for (const [option, label] of [
    ["--scripts", "install or uninstall scriptlets"],
    ["--triggers", "trigger scriptlets"],
    ["--filetriggers", "file trigger scriptlets"],
  ]) {
    requireEmptyRpmScriptListing(run("rpm", ["-qp", option, "--", path], { maxBuffer: 1024 * 1024 }), label);
  }
}

export function parseDebianFields(text) {
  const fields = new Map();
  for (const line of text.trimEnd().split("\n")) {
    const match = /^(Package|Version|Architecture): (\S+)$/.exec(line);
    if (!match || fields.has(match[1])) throw new Error(`invalid Debian metadata line: ${line}`);
    fields.set(match[1], match[2]);
  }
  if (fields.size !== 3) throw new Error("Debian metadata is incomplete");
  return Object.fromEntries([...fields.entries()].map(([key, value]) => [key.toLowerCase(), value]));
}

function parseDebianContents(text) {
  const entries = [];
  for (const line of text.trimEnd().split("\n")) {
    const match = /^([dl-][rwx-]{9}) 0\/0\s+(\d+) \d{4}-\d{2}-\d{2} \d{2}:\d{2} (.+)$/.exec(line);
    if (!match) throw new Error(`invalid Debian contents line: ${line}`);
    const type = match[1][0];
    const rawPath = match[3];
    if ((rawPath === "." || rawPath === "./") && type === "d") continue;
    if (rawPath.startsWith("/")) throw new Error(`Debian package contains an absolute path: ${rawPath}`);
    const path = validArchivePath(rawPath.endsWith("/") ? rawPath.slice(0, -1) : rawPath, "Debian package");
    if (type !== "d" && type !== "-") throw new Error(`Debian package contains unsupported entry ${path}`);
    const size = Number(match[2]);
    if (!Number.isSafeInteger(size) || size < 0) throw new Error(`Debian package entry ${path} has an invalid size`);
    entries.push({ mode: match[1], size, path, type });
    if (entries.length > MAX_INODES) throw new Error(`Debian package exceeds ${MAX_INODES} entries`);
  }
  const paths = entries.map((entry) => entry.path);
  if (new Set(paths).size !== paths.length) throw new Error("Debian package contains duplicate paths");
  const unpackedBytes = entries
    .filter((entry) => entry.type === "-")
    .reduce((total, entry) => total + entry.size, 0);
  if (!Number.isSafeInteger(unpackedBytes) || unpackedBytes > MAX_UNPACKED_BYTES) {
    throw new Error(`Debian package unpacked file bytes exceed ${MAX_UNPACKED_BYTES}`);
  }
  return entries;
}

function validateDebianLayout(entries) {
  const files = new Set(entries.filter((entry) => entry.type === "-").map((entry) => entry.path));
  const directories = entries.filter((entry) => entry.type === "d").map((entry) => entry.path).sort();
  for (const path of REQUIRED_COMMON_PATHS) {
    if (!files.has(path)) throw new Error(`Debian package is missing ${path}`);
  }
  for (const path of files) {
    if (!REQUIRED_COMMON_PATHS.includes(path) && !path.startsWith("usr/share/doc/unfocus/")) {
      throw new Error(`Debian package contains unexpected file ${path}`);
    }
  }
  if (
    directories.length !== DEBIAN_DIRECTORIES.length ||
    directories.some((path, index) => path !== DEBIAN_DIRECTORIES[index])
  ) {
    throw new Error(`Debian package directories must be exactly: ${DEBIAN_DIRECTORIES.join(", ")}`);
  }
  for (const entry of entries) {
    const expectedMode =
      entry.type === "d"
        ? "drwxr-xr-x"
        : entry.path === "usr/bin/unfocus"
          ? "-rwxr-xr-x"
          : "-rw-r--r--";
    if (entry.mode !== expectedMode) throw new Error(`Debian package entry ${entry.path} has unexpected permissions`);
  }
}

export function parseRpmMetadata(text) {
  const lines = text.trimEnd().split("\n");
  if (lines.length !== 4 || lines.some((line) => !line || /\s/.test(line))) {
    throw new Error("RPM metadata must contain exactly four single-token lines");
  }
  return { name: lines[0], version: lines[1], release: lines[2], architecture: lines[3] };
}

export function parseRpmLayout(text) {
  const entries = text.trimEnd().split("\n").map((line) => {
    const match = /^(\S+)\t([0-7]+)\t(\d+)\t([^\t\r\n]+)\t([^\t\r\n]+)\t([^\t\r\n]+)$/.exec(
      line,
    );
    if (!match) throw new Error(`invalid RPM layout line: ${line}`);
    const path = `/${validArchivePath(match[1], "RPM package")}`;
    const mode = Number.parseInt(match[2], 8);
    const size = Number(match[3]);
    if (match[4] !== "root" || match[5] !== "root" || match[6] !== "(none)") {
      throw new Error(`RPM package entry ${path} must be root-owned and capability-free`);
    }
    if (!Number.isSafeInteger(mode) || !Number.isSafeInteger(size) || size < 0) {
      throw new Error(`RPM package entry ${path} has invalid metadata`);
    }
    const expectedMode =
      path === "/usr/lib/Unfocus" ? 0o040755 : path === "/usr/bin/unfocus" ? 0o100775 : 0o100664;
    if (mode !== expectedMode) {
      throw new Error(`RPM package entry ${path} has the wrong file type or permissions`);
    }
    return { path, mode, size };
  });
  const paths = entries.map((entry) => entry.path).sort();
  if (new Set(paths).size !== paths.length) throw new Error("RPM package contains duplicate paths");
  if (paths.length !== RPM_PATHS.length || paths.some((path, index) => path !== RPM_PATHS[index])) {
    throw new Error(`RPM package paths must be exactly: ${RPM_PATHS.join(", ")}`);
  }
  const unpackedBytes = entries.reduce((total, entry) => total + entry.size, 0);
  if (!Number.isSafeInteger(unpackedBytes) || unpackedBytes > MAX_UNPACKED_BYTES) {
    throw new Error(`RPM package unpacked file bytes exceed ${MAX_UNPACKED_BYTES}`);
  }
  return entries;
}

function newcHex(header, offset, label) {
  const value = header.subarray(offset, offset + 8).toString("ascii");
  if (!/^[0-9a-fA-F]{8}$/.test(value)) throw new Error(`RPM cpio ${label} is not hexadecimal`);
  return Number.parseInt(value, 16);
}

function aligned4(value) {
  return (value + 3) & ~3;
}

export function parseNewcArchive(cpio) {
  if (!Buffer.isBuffer(cpio) || cpio.length === 0 || cpio.length > MAX_PACKAGE_BYTES) {
    throw new Error("RPM cpio archive has an invalid size");
  }
  const entries = [];
  let offset = 0;
  let foundTrailer = false;
  while (offset < cpio.length) {
    if (offset + 110 > cpio.length) throw new Error("RPM cpio header is truncated");
    const header = cpio.subarray(offset, offset + 110);
    const magic = header.subarray(0, 6).toString("ascii");
    if (magic !== "070701" && magic !== "070702") throw new Error("RPM cpio archive is not newc format");
    const mode = newcHex(header, 14, "mode");
    const links = newcHex(header, 38, "link count");
    const size = newcHex(header, 54, "file size");
    const nameSize = newcHex(header, 94, "name size");
    const expectedChecksum = newcHex(header, 102, "checksum");
    if (links === 0 || nameSize < 2 || nameSize > MAX_PATH_BYTES + 1) {
      throw new Error("RPM cpio entry has invalid link or name metadata");
    }
    const nameStart = offset + 110;
    const nameEnd = nameStart + nameSize;
    if (nameEnd > cpio.length) throw new Error("RPM cpio entry name is truncated");
    const nameBytes = cpio.subarray(nameStart, nameEnd);
    if (nameBytes.at(-1) !== 0 || nameBytes.subarray(0, -1).includes(0)) {
      throw new Error("RPM cpio entry name is not one NUL-terminated string");
    }
    const name = new TextDecoder("utf-8", { fatal: true }).decode(nameBytes.subarray(0, -1));
    if (name.startsWith("/")) throw new Error(`RPM cpio archive contains an absolute path: ${name}`);
    const dataStart = aligned4(nameEnd);
    if (cpio.subarray(nameEnd, dataStart).some((byte) => byte !== 0)) {
      throw new Error(`RPM cpio entry ${name} has nonzero name padding`);
    }
    const dataEnd = dataStart + size;
    if (dataEnd > cpio.length) throw new Error(`RPM cpio entry ${name} data is truncated`);
    const data = cpio.subarray(dataStart, dataEnd);
    if (magic === "070702") {
      const checksum = data.reduce((total, byte) => (total + byte) >>> 0, 0);
      if (checksum !== expectedChecksum) throw new Error(`RPM cpio entry ${name} checksum is invalid`);
    } else if (expectedChecksum !== 0) {
      throw new Error(`RPM cpio entry ${name} has a checksum in no-check format`);
    }
    const nextOffset = aligned4(dataEnd);
    if (cpio.subarray(dataEnd, nextOffset).some((byte) => byte !== 0)) {
      throw new Error(`RPM cpio entry ${name} has nonzero data padding`);
    }
    offset = nextOffset;
    if (name === "TRAILER!!!") {
      if (size !== 0) throw new Error("RPM cpio trailer contains data");
      foundTrailer = true;
      break;
    }
    const path = `/${validArchivePath(name, "RPM cpio archive")}`;
    entries.push({ path, mode, size });
    if (entries.length > MAX_INODES) throw new Error(`RPM cpio archive exceeds ${MAX_INODES} entries`);
  }
  if (!foundTrailer) throw new Error("RPM cpio archive has no trailer");
  if (cpio.subarray(offset).some((byte) => byte !== 0)) throw new Error("RPM cpio archive has bytes after its trailer");
  return entries;
}

function rpmToCpio(rpmPath) {
  const result = spawnSync("rpm2cpio", [rpmPath], {
    encoding: null,
    env: commandEnvironment(),
    maxBuffer: MAX_PACKAGE_BYTES,
    timeout: 120_000,
  });
  if (result.error) throw new Error(`rpm2cpio failed to start: ${result.error.message}`);
  if (result.signal) throw new Error(`rpm2cpio was terminated by ${result.signal}`);
  const cpio = result.stdout;
  const stderr = result.stderr?.toString("utf8").trim() ?? "";
  const hasCompleteNewcArchive =
    Buffer.isBuffer(cpio) &&
    cpio.subarray(0, 6).toString("ascii") === "070701" &&
    cpio.lastIndexOf(Buffer.from("TRAILER!!!", "ascii")) >= Math.max(0, cpio.length - 1024);
  // rpm2cpio from RPM 4.18 can return one after emitting a complete archive.
  // A complete newc trailer and successful cpio extraction remain mandatory.
  if ((result.status !== 0 && result.status !== 1) || !hasCompleteNewcArchive || stderr) {
    throw new Error(`rpm2cpio failed with status ${result.status}${stderr ? `: ${stderr}` : ""}`);
  }
  return cpio;
}

function extractRpm(rpmPath, output, expectedEntries) {
  const cpio = rpmToCpio(rpmPath);
  const archiveEntries = parseNewcArchive(cpio).sort((left, right) => left.path.localeCompare(right.path));
  const layoutEntries = [...expectedEntries].sort((left, right) => left.path.localeCompare(right.path));
  if (
    archiveEntries.length !== layoutEntries.length ||
    archiveEntries.some(
      (entry, index) =>
        entry.path !== layoutEntries[index].path ||
        entry.mode !== layoutEntries[index].mode ||
        entry.size !== layoutEntries[index].size,
    )
  ) {
    throw new Error("RPM cpio payload does not match signed RPM header layout");
  }
  run("cpio", ["--extract", "--make-directories", "--no-absolute-filenames", "--preserve-modification-time"], {
    cwd: output,
    encoding: null,
    input: cpio,
    maxBuffer: MAX_COMMAND_OUTPUT,
    timeout: 120_000,
  });
}

export function validateSbom(path, version) {
  regularFile(path, "release SBOM", 16 * 1024 * 1024);
  let sbom;
  try {
    sbom = JSON.parse(readFileSync(path, "utf8"));
  } catch (error) {
    throw new Error(`release SBOM is not valid JSON: ${error.message}`);
  }
  const component = sbom?.metadata?.component;
  if (
    sbom?.bomFormat !== "CycloneDX" ||
    sbom?.specVersion !== "1.6" ||
    sbom?.version !== 1 ||
    component?.type !== "application" ||
    component?.name !== "Unfocus" ||
    component?.version !== version ||
    component?.["bom-ref"] !== `pkg:generic/unfocus@${version}` ||
    !Array.isArray(sbom.components) ||
    sbom.components.length === 0 ||
    sbom.components.length > MAX_INODES
  ) {
    throw new Error("release SBOM has invalid CycloneDX application metadata");
  }
  const references = sbom.components.map((entry) => entry?.["bom-ref"]);
  if (references.some((reference) => typeof reference !== "string" || !reference)) {
    throw new Error("release SBOM contains a component without a bom-ref");
  }
  if (new Set(references).size !== references.length) throw new Error("release SBOM contains duplicate components");
  if (!references.includes("pkg:cargo/minisign-verify@0.2.5")) {
    throw new Error("release SBOM does not contain the updater signature verifier dependency");
  }
}

function requireExactInventory(directory, version) {
  const expected = [...expectedReleaseFilenames(version), "SHA256SUMS"].sort();
  const actual = readdirSync(directory, { withFileTypes: true }).map((entry) => {
    if (!entry.isFile()) throw new Error(`release candidate contains non-file entry ${entry.name}`);
    return entry.name;
  });
  actual.sort();
  if (actual.length !== expected.length || actual.some((name, index) => name !== expected[index])) {
    throw new Error(`release candidate inventory must be exactly: ${expected.join(", ")}`);
  }
  verifyChecksums(directory, expectedReleaseFilenames(version));
}

async function inspectAppImage(directory, version, temporaryRoot, noticesHash) {
  const filename = `Unfocus_${version}_amd64.AppImage`;
  const path = join(directory, filename);
  const outer = inspectAppImageOuter(path);
  const entries = inspectSquashfsListing(path, outer.filesystemOffset);
  if (entries.length + 1 !== outer.squashfs.inodeCount) {
    throw new Error("SquashFS listing count does not match its superblock inode count");
  }
  const extraction = join(temporaryRoot, "appimage");
  extractAppImageFilesystem(path, outer.filesystemOffset, extraction);
  reconcileExtractedAppImage(extraction, entries);
  const executable = extractedRegularFile(extraction, "usr/bin/unfocus", MAX_EXECUTABLE_BYTES, true);
  const appRun = extractedRegularFile(extraction, "AppRun", MAX_APP_RUN_BYTES, true);
  if (lstatSync(appRun).size === 0) throw new Error("AppImage AppRun is empty");
  const notices = extractedRegularFile(
    extraction,
    "usr/lib/Unfocus/THIRD_PARTY_NOTICES.txt",
    32 * 1024 * 1024,
  );
  validateExtractedPresentation(extraction);
  if ((await sha256File(notices)) !== noticesHash) throw new Error("AppImage notices differ from release notices");
  return {
    filename,
    sizeBytes: outer.fileSize,
    sha256: await sha256File(path),
    filesystemOffset: outer.filesystemOffset,
    inodeCount: entries.length + 1,
    innerExecutableSha256: await sha256File(executable),
    innerExecutableBuildId: inspectExecutable(executable, "AppImage usr/bin/unfocus"),
  };
}

function listDebianDataArchive(path) {
  return run(
    "bash",
    [
      "-c",
      'set -euo pipefail; dpkg-deb --fsys-tarfile -- "$1" | tar --numeric-owner --list --verbose --file=-',
      "unfocus-debian-list",
      path,
    ],
    { timeout: 120_000 },
  );
}

async function validateDebianMd5Sums(controlEntries, dataEntries, extraction) {
  const md5sums = controlEntries.find((entry) => entry.path === "md5sums");
  if (!md5sums) throw new Error("Debian control archive has no md5sums file");
  const regularEntries = dataEntries.filter((entry) => entry.type === "-");
  const checksums = parseDebianMd5Sums(
    md5sums.data,
    regularEntries.map((entry) => entry.path),
  );
  const digestByPath = new Map(checksums.map((entry) => [entry.path, entry.digest]));
  for (const entry of regularEntries) {
    const path = join(extraction, entry.path);
    const stat = lstatSync(path);
    if (!stat.isFile() || stat.size !== entry.size) {
      throw new Error(`extracted Debian file ${entry.path} does not match its archive metadata`);
    }
    if ((await md5File(path)) !== digestByPath.get(entry.path)) {
      throw new Error(`Debian md5sums digest does not match ${entry.path}`);
    }
  }
}

async function inspectDeb(directory, version, temporaryRoot, noticesHash) {
  const filename = `Unfocus_${version}_amd64.deb`;
  const path = join(directory, filename);
  const stat = regularFile(path, "Debian package");
  const metadata = parseDebianFields(run("dpkg-deb", ["--field", path, "Package", "Version", "Architecture"]));
  const expectedVersion = semverToDebianVersion(version);
  if (metadata.package !== "unfocus" || metadata.version !== expectedVersion || metadata.architecture !== "amd64") {
    throw new Error(`Debian metadata must be unfocus ${expectedVersion} amd64`);
  }
  const dataEntries = parseDebianContents(listDebianDataArchive(path));
  validateDebianLayout(dataEntries);
  const controlEntries = validateDebianControlArchive(
    run("dpkg-deb", ["--ctrl-tarfile", path], {
      encoding: null,
      maxBuffer: MAX_DEBIAN_CONTROL_ARCHIVE_BYTES,
    }),
  );
  const extraction = join(temporaryRoot, "deb");
  run("dpkg-deb", ["--extract", path, extraction], { timeout: 120_000 });
  await validateDebianMd5Sums(controlEntries, dataEntries, extraction);
  const executable = extractedRegularFile(extraction, "usr/bin/unfocus", MAX_EXECUTABLE_BYTES, true);
  const notices = extractedRegularFile(extraction, "usr/lib/Unfocus/THIRD_PARTY_NOTICES.txt", 32 * 1024 * 1024);
  validateExtractedPresentation(extraction);
  if ((await sha256File(notices)) !== noticesHash) throw new Error("Debian notices differ from release notices");
  return {
    filename,
    sizeBytes: stat.size,
    sha256: await sha256File(path),
    package: metadata.package,
    version: metadata.version,
    architecture: metadata.architecture,
    innerExecutableBuildId: inspectExecutable(executable, "Debian usr/bin/unfocus"),
  };
}

async function inspectRpm(directory, version, temporaryRoot, noticesHash) {
  const filename = `Unfocus-${version}-1.x86_64.rpm`;
  const path = join(directory, filename);
  const stat = regularFile(path, "RPM package");
  const metadata = parseRpmMetadata(
    run("rpm", ["-qp", "--queryformat", "%{NAME}\\n%{VERSION}\\n%{RELEASE}\\n%{ARCH}\\n", "--", path]),
  );
  if (
    metadata.name !== "unfocus" ||
    metadata.version !== version ||
    metadata.release !== "1" ||
    metadata.architecture !== "x86_64"
  ) {
    throw new Error(`RPM metadata must be unfocus ${version} release 1 x86_64`);
  }
  const digestResult = run("rpm", ["-K", path]).trim();
  if (digestResult !== `${path}: digests OK`) throw new Error("RPM package payload digests are not valid");
  validateRpmScriptlets(path);
  const layout = parseRpmLayout(
    run("rpm", [
      "-qp",
      "--queryformat",
      "[%{FILENAMES}\\t%{FILEMODES:octal}\\t%{FILESIZES}\\t%{FILEUSERNAME}\\t%{FILEGROUPNAME}\\t%{FILECAPS}\\n]",
      "--",
      path,
    ]),
  );
  const extraction = join(temporaryRoot, "rpm");
  mkdirSync(extraction);
  extractRpm(path, extraction, layout);
  const executable = extractedRegularFile(extraction, "usr/bin/unfocus", MAX_EXECUTABLE_BYTES, true);
  const notices = extractedRegularFile(extraction, "usr/lib/Unfocus/THIRD_PARTY_NOTICES.txt", 32 * 1024 * 1024);
  validateExtractedPresentation(extraction);
  if ((await sha256File(notices)) !== noticesHash) throw new Error("RPM notices differ from release notices");
  return {
    filename,
    sizeBytes: stat.size,
    sha256: await sha256File(path),
    name: metadata.name,
    version: metadata.version,
    release: metadata.release,
    architecture: metadata.architecture,
    innerExecutableBuildId: inspectExecutable(executable, "RPM usr/bin/unfocus"),
  };
}

export async function inspectLinuxPackages(directory, version) {
  releaseChannel(version);
  semverToDebianVersion(version);
  const source = resolve(directory);
  if (!existsSync(source) || !lstatSync(source).isDirectory()) {
    throw new Error(`release candidate directory is missing: ${source}`);
  }
  requireExactInventory(source, version);
  const noticesPath = join(source, "THIRD_PARTY_NOTICES.txt");
  regularFile(noticesPath, "release notices", 32 * 1024 * 1024);
  const noticesHash = await sha256File(noticesPath);
  const repositoryNoticesHash = await sha256File(join(REPOSITORY_ROOT, "THIRD_PARTY_NOTICES.txt"));
  if (noticesHash !== repositoryNoticesHash) throw new Error("release notices differ from the validated source tree");
  validateSbom(join(source, "unfocus.cdx.json"), version);
  const temporaryRoot = mkdtempSync(join(tmpdir(), "unfocus-package-inspection-"));
  try {
    const appimage = await inspectAppImage(source, version, temporaryRoot, noticesHash);
    const deb = await inspectDeb(source, version, temporaryRoot, noticesHash);
    const rpm = await inspectRpm(source, version, temporaryRoot, noticesHash);
    const buildIds = new Set([
      appimage.innerExecutableBuildId,
      deb.innerExecutableBuildId,
      rpm.innerExecutableBuildId,
    ]);
    if (buildIds.size !== 1) throw new Error("AppImage, Debian, and RPM executables have different GNU build IDs");
    const evidence = {
      schemaVersion: EVIDENCE_SCHEMA_VERSION,
      version,
      channel: releaseChannel(version),
      candidateChecksumsSha256: await sha256File(join(source, "SHA256SUMS")),
      packages: { appimage, deb, rpm },
    };
    for (const package_ of Object.values(evidence.packages)) {
      if (!SHA256.test(package_.sha256)) throw new Error(`${package_.filename} has an invalid SHA-256 digest`);
    }
    const text = canonicalJson(evidence);
    parsePackageEvidence(text, version);
    return text;
  } finally {
    rmSync(temporaryRoot, { recursive: true, force: true });
  }
}

function usage() {
  console.error("usage: inspect-linux-packages.js <release-candidate-directory> <canonical-version> <new-output.json>");
}

async function main() {
  const [directory, version, output, ...extra] = process.argv.slice(2);
  if (!directory || !version || !output || extra.length !== 0) {
    usage();
    process.exit(2);
  }
  const outputPath = resolve(output);
  if (existsSync(outputPath)) throw new Error(`refusing to overwrite existing package evidence: ${outputPath}`);
  const evidence = await inspectLinuxPackages(directory, version);
  writeFileSync(outputPath, evidence, { flag: "wx" });
  console.log(`wrote credential-free Linux package evidence ${outputPath}`);
}

if (import.meta.main) {
  main().catch((error) => {
    console.error(error.message);
    process.exit(1);
  });
}
