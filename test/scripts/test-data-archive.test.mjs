// scripts/s3/test-data-archive.mjs: the canonical tar a tool-pack's test data is archived as (the same files always
// make the same bytes, which system tar reads), the tars readArchive refuses, and the locks parseLock refuses.

import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { archiveSizeBound, buildArchive, compressArchive, decompressArchive, lockText, parseLock, readArchive, sha256 } from "../../scripts/s3/test-data-archive.mjs";

const scratch = [];
afterEach(() => {
  for (const directory of scratch.splice(0)) rmSync(directory, { recursive: true, force: true });
});

const entry = (path, text) => ({ path, bytes: Buffer.from(text, "utf8") });

/**
 * Paths with a control character, and one in NFD ("é" as "e" and a combining acute), which a filesystem that
 * normalizes names would make the same file as its NFC form.
 */
const UNPRINTABLE = ["fixtures/a\u0001.py", "fixtures/a\n.py", "fixtures/a\u001f.py", "fixtures/a\u007f.py", "fixtures/caf\u0065\u0301.py"];

/** Paths Windows cannot hold: a reserved character, or a segment it would strip a trailing dot or space from. */
const WINDOWS_RESERVED = ["fixtures/a:b.py", "fixtures/a*.py", "fixtures/a?.py", 'fixtures/"a".py'];
const WINDOWS_TRAILING = ["fixtures/a.", "fixtures/a ", "fixtures./a.py", "fixtures /a.py"];

/** The message `run` throws; fails the test when it does not throw. */
function failure(run) {
  try {
    run();
  } catch (error) {
    return error.message;
  }
  throw new Error("it did not fail");
}

/** The text of a header field: up to its first NUL. */
const field = (header, offset, length) => {
  const raw = header.subarray(offset, offset + length);
  const end = raw.indexOf(0);
  return raw.subarray(0, end === -1 ? length : end).toString("utf8");
};

/** A ustar header naming `name` with `type` and `size`, its checksum filled in, written the way any tar writer would. */
function rawHeader(name, type, size) {
  const header = Buffer.alloc(512);
  header.write(name, 0, 100, "utf8");
  header.write("0000644\0", 100, "ascii");
  header.write("0000000\0", 108, "ascii");
  header.write("0000000\0", 116, "ascii");
  header.write(`${size.toString(8).padStart(11, "0")}\0`, 124, "ascii");
  header.write("00000000000\0", 136, "ascii");
  header.fill(" ", 148, 156);
  header.write(type, 156, 1, "ascii");
  header.write("ustar\0", 257, "ascii");
  header.write("00", 263, "ascii");
  let sum = 0;
  for (const byte of header) sum += byte;
  header.write(`${sum.toString(8).padStart(6, "0")}\0 `, 148, 8, "ascii");
  return header;
}

/** A tar of `members` (each `{ name, type, body }`), with its end-of-archive blocks. */
function rawTar(members) {
  const parts = [];
  for (const { name, type, body = Buffer.alloc(0) } of members) {
    parts.push(rawHeader(name, type, body.length), body, Buffer.alloc((512 - (body.length % 512)) % 512));
  }
  return Buffer.concat([...parts, Buffer.alloc(1024)]);
}

describe("buildArchive", () => {
  const files = [entry("fixtures/b.py", "b\n"), entry("fixtures.expected.json", "{}\n"), entry("fixtures/a/z.py", "z\n"), entry("cases.json", "[]\n")];

  it("makes the same tar, sha256 and byte count from the same files in any order", () => {
    const first = buildArchive(files);
    for (const order of [[...files].reverse(), [files[2], files[0], files[3], files[1]]]) {
      const again = buildArchive(order);
      expect(again.tar.equals(first.tar)).toBe(true);
      expect({ sha256: again.sha256, files: again.files, bytes: again.bytes }).toEqual({ sha256: first.sha256, files: 4, bytes: 10 });
    }
    expect(first.sha256).toBe(sha256(first.tar));
    expect(buildArchive([...files.slice(1), entry("fixtures/b.py", "B\n")]).sha256).not.toBe(first.sha256);
  });

  it("writes every entry as a regular file with mode 0644, uid and gid 0, no owner names and mtime 0", () => {
    const { tar } = buildArchive(files);
    const headers = [];
    for (let offset = 0; offset < tar.length; ) {
      const header = tar.subarray(offset, offset + 512);
      if (header.every((byte) => byte === 0)) break;
      headers.push(header);
      const size = Number.parseInt(field(header, 124, 12), 8);
      offset += 512 + Math.ceil(size / 512) * 512;
    }
    expect(headers.map((header) => field(header, 0, 100))).toEqual(["cases.json", "fixtures.expected.json", "fixtures/a/z.py", "fixtures/b.py"]);
    for (const header of headers) {
      expect({
        mode: field(header, 100, 8),
        uid: field(header, 108, 8),
        gid: field(header, 116, 8),
        mtime: field(header, 136, 12),
        type: field(header, 156, 1),
        uname: field(header, 265, 32),
        gname: field(header, 297, 32),
      }).toEqual({ mode: "0000644", uid: "0000000", gid: "0000000", mtime: "00000000000", type: "0", uname: "", gname: "" });
    }
  });

  const split = `fixtures/${"d".repeat(80)}/${"f".repeat(60)}.py`;
  const segment = `${"x".repeat(150)}.expected.json`;
  const longLeaf = `fixtures/${"y".repeat(120)}.py`;
  const long = [entry(split, "split\n"), entry(segment, "pax\n"), entry(longLeaf, "leaf\n")];

  it("round-trips a path longer than 100 bytes through ustar's prefix field, or through a pax record when no `/` splits it", () => {
    const { tar } = buildArchive(long);
    const prefixed = Array.from({ length: tar.length / 512 }, (_, index) => tar.subarray(index * 512, index * 512 + 512)).find((block) => field(block, 345, 155) !== "");
    expect(field(prefixed, 345, 155)).toBe(`fixtures/${"d".repeat(80)}`);
    expect(field(prefixed, 0, 100)).toBe(`${"f".repeat(60)}.py`);
    expect(readArchive(decompressArchive(compressArchive(tar))).map(({ path, bytes }) => [path, bytes.toString("utf8")])).toEqual([
      [split, "split\n"],
      [longLeaf, "leaf\n"],
      [segment, "pax\n"],
    ]);
  });

  const hasTar = spawnSync("tar", ["--version"]).error === undefined;

  it.skipIf(!hasTar)("is read by system tar, long paths included", () => {
    const base = mkdtempSync(join(tmpdir(), "test-data-archive-"));
    scratch.push(base);
    const file = join(base, "data.tar");
    writeFileSync(file, buildArchive([...files, ...long]).tar);
    const listed = spawnSync("tar", ["-tf", file], { encoding: "utf8" });
    expect(listed.status).toBe(0);
    expect(listed.stdout.trim().split("\n").sort()).toEqual([...files, ...long].map(({ path }) => path).sort());
    const out = join(base, "out");
    mkdirSync(out);
    expect(spawnSync("tar", ["-xf", file, "-C", out]).status).toBe(0);
    for (const { path, bytes } of [...files, ...long]) expect(readFileSync(join(out, path)).equals(bytes)).toBe(true);
  });

  it("refuses an unsafe path and a path twice", () => {
    for (const path of ["../x.json", "/abs/x.json", "fixtures/../x.py", "fixtures//x.py", "fixtures\\x.py", ""]) {
      expect(() => buildArchive([entry(path, "x\n")])).toThrow();
    }
    expect(() => buildArchive([entry("cases.json", "a"), entry("cases.json", "b")])).toThrow();
  });

  it("refuses a path with a control character or not in NFC", () => {
    for (const path of UNPRINTABLE) expect(() => buildArchive([entry(path, "x\n")])).toThrow(JSON.stringify(path));
    expect(buildArchive([entry("fixtures/caf\u00e9.py", "x\n")]).files).toBe(1);
  });

  it("refuses a path Windows cannot hold, naming it and why, and keeps a dot or space inside a name", () => {
    for (const path of WINDOWS_RESERVED) {
      expect(failure(() => buildArchive([entry(path, "x\n")]))).toBe(`archive path ${JSON.stringify(path)} holds :, *, ? or ", which Windows cannot hold in a file name`);
    }
    for (const path of WINDOWS_TRAILING) {
      expect(failure(() => buildArchive([entry(path, "x\n")]))).toBe(`archive path ${JSON.stringify(path)} has a segment ending in . or a space, which Windows drops from a file name`);
    }
    expect(buildArchive([entry("fixtures/x y.expected.json", "{}\n"), entry(".github/a.b.py", "x\n")]).files).toBe(2);
  });

  it("refuses two paths equal but for case, naming both", () => {
    const message = failure(() => buildArchive([entry("fixtures/Cases.json", "a"), entry("fixtures/cases.json", "b")]));
    expect(message).toContain("fixtures/Cases.json");
    expect(message).toContain("fixtures/cases.json");
  });
});

describe("readArchive", () => {
  const body = Buffer.from("x\n");

  it("reads a tar of regular files another writer made", () => {
    expect(readArchive(rawTar([{ name: "fixtures/a.py", type: "0", body }]))).toEqual([{ path: "fixtures/a.py", bytes: body }]);
  });

  it("refuses a `..` or absolute path, in the header or a pax record", () => {
    for (const name of ["../a.py", "fixtures/../../a.py", "/etc/a.py"]) expect(() => readArchive(rawTar([{ name, type: "0", body }]))).toThrow();
    const pax = Buffer.from("19 path=../../a.py\n");
    expect(() => readArchive(rawTar([{ name: "PaxHeader", type: "x", body: pax }, { name: "a.py", type: "0", body }]))).toThrow();
  });

  it("refuses a symlink, a hard link or a directory entry", () => {
    for (const type of ["1", "2", "5"]) expect(() => readArchive(rawTar([{ name: "fixtures/a.py", type, body: Buffer.alloc(0) }]))).toThrow();
  });

  it("refuses a path with a control character or not in NFC", () => {
    for (const name of UNPRINTABLE) expect(() => readArchive(rawTar([{ name, type: "0", body }]))).toThrow(JSON.stringify(name));
  });

  it("refuses a path Windows cannot hold, in the header or a pax record", () => {
    for (const name of [...WINDOWS_RESERVED, ...WINDOWS_TRAILING]) expect(() => readArchive(rawTar([{ name, type: "0", body }])), name).toThrow(JSON.stringify(name));
    const pax = Buffer.from("20 path=fixtures/a:\n");
    expect(() => readArchive(rawTar([{ name: "PaxHeader", type: "x", body: pax }, { name: "a.py", type: "0", body }]))).toThrow("which Windows cannot hold in a file name");
  });

  it("refuses two paths equal but for case, naming both", () => {
    const message = failure(() => readArchive(rawTar([{ name: "fixtures/A.py", type: "0", body }, { name: "fixtures/a.py", type: "0", body }])));
    expect(message).toContain("fixtures/A.py");
    expect(message).toContain("fixtures/a.py");
  });

  it("refuses a header with a bad checksum", () => {
    const tar = buildArchive([entry("fixtures/a.py", "x\n")]).tar;
    tar[0] ^= 1;
    expect(() => readArchive(tar)).toThrow();
  });

  it("refuses a truncated tar", () => {
    const { tar } = buildArchive([entry("fixtures/a.py", "x".repeat(1000))]);
    expect(() => readArchive(tar.subarray(0, 512 + 600))).toThrow();
    expect(() => readArchive(tar.subarray(0, tar.length - 1024))).toThrow();
  });
});

describe("archiveSizeBound", () => {
  it("holds any canonical tar of that many files and bytes, long paths included", () => {
    const files = [entry(`${"x".repeat(150)}.expected.json`, "pax\n"), entry(`fixtures/${"d".repeat(80)}/${"f".repeat(60)}.py`, "split\n"), entry("cases.json", "[]\n")];
    const archive = buildArchive(files);
    const bound = archiveSizeBound(archive);
    expect(archive.tar.length).toBeLessThanOrEqual(bound);
    expect(decompressArchive(compressArchive(archive.tar), bound).equals(archive.tar)).toBe(true);
  });

  it("stops decompression of a tar larger than the lock allows", () => {
    const { tar } = buildArchive([entry("cases.json", "x".repeat(1 << 20))]);
    expect(() => decompressArchive(compressArchive(tar), archiveSizeBound({ files: 1, bytes: 16 }))).toThrow();
  });
});

describe("parseLock", () => {
  const label = "packs/p/upstream/t/test-data.lock";
  const lock = { format: 1, sha256: "a".repeat(64), files: 3, bytes: 120 };
  const text = (value) => JSON.stringify(value);

  it("reads the lock lockText writes", () => {
    expect(lockText(lock)).toBe(`${JSON.stringify(lock, null, 2)}\n`);
    expect(parseLock(lockText(lock), label)).toEqual({ sha256: lock.sha256, files: 3, bytes: 120 });
  });

  it.each([
    ["no JSON", "{ format: 1 }"],
    ["a JSON array", "[]"],
    ["a missing key", text({ format: 1, sha256: lock.sha256, files: 3 })],
    ["an extra key", text({ ...lock, tool: "t" })],
    ["format 2", text({ ...lock, format: 2 })],
    ["a non-hex sha256", text({ ...lock, sha256: "g".repeat(64) })],
    ["an uppercase sha256", text({ ...lock, sha256: "A".repeat(64) })],
    ["a short sha256", text({ ...lock, sha256: "a".repeat(63) })],
    ["negative files", text({ ...lock, files: -1 })],
    ["non-integer bytes", text({ ...lock, bytes: 1.5 })],
    ["string bytes", text({ ...lock, bytes: "120" })],
  ])("refuses %s, naming the lock", (_, value) => {
    expect(() => parseLock(value, label)).toThrow(label);
  });
});
