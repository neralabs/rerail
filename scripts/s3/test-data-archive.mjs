// A tool-pack's test data as one content-pinned archive (CONTRIBUTING.md, "Pack test data"): the canonical tar of its
// files, compressed with zstd, which packs/<pack>/upstream/<tool>/test-data.lock names by the tar's sha256.
//
// The tar is canonical, so the same files always make the same bytes: one regular-file entry per file, sorted by its
// path's UTF-8 bytes, at its path relative to the tool directory (in NFC, with no control character, and no two paths
// equal but for case), with mode 0644, uid and gid 0, empty user and group names, mtime 0, and no directory entries. A
// path longer than ustar's 100-byte name field is split into its prefix field at a `/`, or failing that carried by a
// pax `path` record. The sha256 is the tar's, not the compressed file's, so the pin depends only on the files: a
// different zstd build may compress the same tar to other bytes, and a pull still verifies what it extracts.

import { createHash } from "node:crypto";
import { constants, zstdCompressSync, zstdDecompressSync } from "node:zlib";

/** The lock's file name in a tool directory. */
export const TEST_DATA_LOCK = "test-data.lock";
export const LOCK_FORMAT = 1;

const BLOCK = 512;
const MODE = 0o644;
const SHA256 = /^[0-9a-f]{64}$/u;

const byBytes = (a, b) => Buffer.compare(Buffer.from(a.path, "utf8"), Buffer.from(b.path, "utf8"));

/** A numeric header field: octal digits, zero-padded, then NUL. */
function octal(header, offset, length, value) {
  header.write(`${value.toString(8).padStart(length - 1, "0")}\0`, offset, length, "ascii");
}

function headerBlock(name, prefix, size, type) {
  const header = Buffer.alloc(BLOCK);
  header.write(name, 0, 100, "utf8");
  octal(header, 100, 8, MODE);
  octal(header, 108, 8, 0);
  octal(header, 116, 8, 0);
  octal(header, 124, 12, size);
  octal(header, 136, 12, 0);
  header.fill(" ", 148, 156);
  header.write(type, 156, 1, "ascii");
  header.write("ustar\0", 257, 6, "ascii");
  header.write("00", 263, 2, "ascii");
  header.write(prefix, 345, 155, "utf8");
  let sum = 0;
  for (const byte of header) sum += byte;
  header.write(`${sum.toString(8).padStart(6, "0")}\0 `, 148, 8, "ascii");
  return header;
}

const padding = (size) => Buffer.alloc((BLOCK - (size % BLOCK)) % BLOCK);

/** ustar's name and prefix fields for `path`, or undefined when no `/` splits it into fields that fit. */
function ustarFields(path) {
  if (Buffer.byteLength(path) <= 100) return { name: path, prefix: "" };
  for (let at = path.indexOf("/"); at > 0; at = path.indexOf("/", at + 1)) {
    const prefix = path.slice(0, at);
    const name = path.slice(at + 1);
    if (Buffer.byteLength(prefix) <= 155 && Buffer.byteLength(name) <= 100) return { name, prefix };
  }
  return undefined;
}

/** A pax record, `<length> path=<value>\n`, whose length counts its own digits. */
function paxRecord(key, value) {
  const body = ` ${key}=${value}\n`;
  let length = Buffer.byteLength(body);
  while (String(length).length + Buffer.byteLength(body) !== length) length = String(length).length + Buffer.byteLength(body);
  return Buffer.from(`${length}${body}`, "utf8");
}

/**
 * Throws unless `path` is a safe relative POSIX path in NFC: no empty, `.` or `..` segment, no backslash, no control
 * character (U+0000 to U+001F, U+007F), no `` ` ``, `<`, `>` or `|`, no `:`, `*`, `?` or `"`, and no segment ending in
 * `.` or a space. A path in another normalization would name a second file on a filesystem that keeps names as given
 * and the same file on one that normalizes them.
 */
export function checkEntryPath(path) {
  if (path === "" || path.startsWith("/") || /[\\\u0000-\u001f\u007f]/u.test(path) || path.split("/").some((segment) => segment === "" || segment === "." || segment === "..")) {
    throw new Error(`unsafe archive path ${JSON.stringify(path)}`);
  }
  // The approver's publish summary shows paths in markdown code spans: none may break out of one or into a table cell.
  if (/[`<>|]/u.test(path)) throw new Error(`archive path ${JSON.stringify(path)} holds \`, <, > or |, which the publish summary cannot show safely`);
  // Windows cannot create such a name (`:` also opens an NTFS stream), or silently drops the trailing dot or space.
  if (/[:*?"]/u.test(path)) throw new Error(`archive path ${JSON.stringify(path)} holds :, *, ? or ", which Windows cannot hold in a file name`);
  if (path.split("/").some((segment) => /[. ]$/u.test(segment))) throw new Error(`archive path ${JSON.stringify(path)} has a segment ending in . or a space, which Windows drops from a file name`);
  if (path.normalize("NFC") !== path) throw new Error(`archive path ${JSON.stringify(path)} is not in Unicode normalization form C`);
}

/**
 * A check that throws, naming both, when a path is equal case-insensitively (in NFC, lowercased) to one it was given
 * before: a case-insensitive filesystem would extract the two to one file.
 */
function caseCollisions(what) {
  const seen = new Map();
  return (path) => {
    const folded = path.normalize("NFC").toLowerCase();
    const other = seen.get(folded);
    if (other === path) throw new Error(`${what} ${path} twice`);
    if (other !== undefined) throw new Error(`${what} ${other} and ${path}, which differ only in case`);
    seen.set(folded, path);
  };
}

/**
 * The canonical tar of `entries` (each `{ path, bytes }`, path relative to the tool directory), its sha256, how many
 * files it holds and their total size in bytes.
 */
export function buildArchive(entries) {
  const sorted = [...entries].sort(byBytes);
  const collides = caseCollisions("the archive would hold");
  const parts = [];
  let bytes = 0;
  for (let index = 0; index < sorted.length; index += 1) {
    const { path, bytes: content } = sorted[index];
    checkEntryPath(path);
    collides(path);
    const fields = ustarFields(path);
    if (fields === undefined) {
      const record = paxRecord("path", path);
      parts.push(headerBlock("PaxHeader", "", record.length, "x"), record, padding(record.length));
    }
    parts.push(headerBlock(fields?.name ?? "PaxPath", fields?.prefix ?? "", content.length, "0"), content, padding(content.length));
    bytes += content.length;
  }
  parts.push(Buffer.alloc(BLOCK * 2));
  const tar = Buffer.concat(parts);
  return { tar, sha256: sha256(tar), files: sorted.length, bytes };
}

export function sha256(buffer) {
  return createHash("sha256").update(buffer).digest("hex");
}

/** The tar compressed with zstd at level 19, with zstd's content checksum. */
export function compressArchive(tar) {
  return zstdCompressSync(tar, { params: { [constants.ZSTD_c_compressionLevel]: 19, [constants.ZSTD_c_checksumFlag]: 1 } });
}

/**
 * The most a canonical tar of `files` files holding `bytes` bytes can take: per file a header, a pax header and its
 * record (a path of up to 4 KiB, padded), and the padding of its content; then the two end blocks.
 */
export function archiveSizeBound({ files, bytes }) {
  return bytes + files * (3 * BLOCK + 4096 + BLOCK) + 2 * BLOCK;
}

/** The tar inside a compressed archive; throws on bytes that are not one, or that decompress to more than `maxLength`. */
export function decompressArchive(compressed, maxLength = undefined) {
  return zstdDecompressSync(compressed, maxLength === undefined ? {} : { maxOutputLength: maxLength });
}

const field = (header, offset, length) => {
  const raw = header.subarray(offset, offset + length);
  const end = raw.indexOf(0);
  return raw.subarray(0, end === -1 ? length : end).toString("utf8");
};

const number = (header, offset, length) => {
  const text = field(header, offset, length).trim();
  if (!/^[0-7]+$/u.test(text)) throw new Error(`a tar header field holds ${JSON.stringify(text)}, not an octal number`);
  return Number.parseInt(text, 8);
};

/**
 * The `{ path, bytes }` entries of a tar buildArchive made; throws on anything else (a link, a directory, a bad checksum,
 * a path twice or twice but for case).
 */
export function readArchive(tar) {
  const entries = [];
  const collides = caseCollisions("the tar holds");
  let offset = 0;
  let paxPath;
  while (offset + BLOCK <= tar.length) {
    const header = tar.subarray(offset, offset + BLOCK);
    if (header.every((byte) => byte === 0)) return entries;
    let sum = 0;
    for (let index = 0; index < BLOCK; index += 1) sum += index >= 148 && index < 156 ? 32 : header[index];
    if (sum !== number(header, 148, 8)) throw new Error(`the tar header at byte ${offset} has a bad checksum`);
    const size = number(header, 124, 12);
    const type = String.fromCharCode(header[156]);
    const body = tar.subarray(offset + BLOCK, offset + BLOCK + size);
    if (body.length !== size) throw new Error(`the tar entry at byte ${offset} is truncated`);
    offset += BLOCK + size + padding(size).length;
    if (type === "x") {
      const match = /^\d+ path=(.*)\n$/su.exec(body.toString("utf8"));
      if (match === null) throw new Error("a pax header holds something other than one path record");
      paxPath = match[1];
      continue;
    }
    if (type !== "0") throw new Error(`the tar holds an entry of type ${JSON.stringify(type)}; a test data archive holds regular files only`);
    const prefix = field(header, 345, 155);
    const path = paxPath ?? (prefix === "" ? field(header, 0, 100) : `${prefix}/${field(header, 0, 100)}`);
    paxPath = undefined;
    checkEntryPath(path);
    collides(path);
    entries.push({ path, bytes: Buffer.from(body) });
  }
  throw new Error("the tar ends without its end-of-archive blocks");
}

/** A lock's text: its fields in a fixed order, two-space indented, with a final newline. */
export function lockText({ sha256: digest, files, bytes }) {
  return `${JSON.stringify({ format: LOCK_FORMAT, sha256: digest, files, bytes }, null, 2)}\n`;
}

/** A lock's fields; throws, naming `label` (the lock's repository path), on anything but a well-formed format-1 lock. */
export function parseLock(text, label) {
  let value;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new Error(`${label} is not JSON (${error instanceof Error ? error.message : String(error)})`);
  }
  const problems = [];
  if (value === null || typeof value !== "object" || Array.isArray(value)) problems.push("it is not an object");
  else {
    const keys = Object.keys(value).sort();
    if (keys.join(",") !== "bytes,files,format,sha256") problems.push(`its keys are ${keys.join(", ") || "none"}, not format, sha256, files and bytes`);
    if (value.format !== LOCK_FORMAT) problems.push(`its format is ${JSON.stringify(value.format)}, not ${LOCK_FORMAT}`);
    if (typeof value.sha256 !== "string" || !SHA256.test(value.sha256)) problems.push("its sha256 is not 64 lowercase hex digits");
    for (const key of ["files", "bytes"]) if (!Number.isSafeInteger(value[key]) || value[key] < 0) problems.push(`its ${key} is not a non-negative integer`);
  }
  if (problems.length > 0) throw new Error(`${label} is malformed: ${problems.join("; ")}`);
  return { sha256: value.sha256, files: value.files, bytes: value.bytes };
}
