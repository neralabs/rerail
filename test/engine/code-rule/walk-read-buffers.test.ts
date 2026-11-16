import { mkdtemp, readFile, realpath, writeFile } from "node:fs/promises";
import type * as FileSystem from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { WalkReadBuffers } from "../../../src/engine/code-rule/walk-read-buffers.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/**
 * A failure the next file a test opens meets, as a failing disk gives one: its second `read` (the first filled the first
 * buffer, so the read grew it) or its `close` rejects with `EIO`.
 */
const failing = vi.hoisted(() => ({ next: undefined as "read" | "close" | undefined }));
vi.mock("node:fs/promises", async importOriginal => {
  const actual = await importOriginal<typeof FileSystem>();
  const eio = () => Object.assign(new Error("EIO: i/o error"), { code: "EIO" });
  return {
    ...actual,
    open: async (...args: Parameters<typeof actual.open>) => {
      const handle = await actual.open(...args);
      const call = failing.next;
      failing.next = undefined;
      if (call === "read") {
        const read = handle.read.bind(handle) as (...forwarded: unknown[]) => ReturnType<typeof handle.read>;
        let reads = 0;
        handle.read = ((...forwarded: unknown[]) => ((reads += 1) === 2 ? Promise.reject(eio()) : read(...forwarded))) as typeof handle.read;
      }
      if (call === "close") {
        const close = handle.close.bind(handle);
        handle.close = async () => {
          await close();
          throw eio();
        };
      }
      return handle;
    },
  };
});

let folder: string;

beforeEach(async () => {
  folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-walk-read-buffers-")));
});
afterEach(async () => {
  failing.next = undefined;
  await removeFolder(folder);
});

async function file(name: string, contents: string): Promise<string> {
  const path = join(folder, name);
  await writeFile(path, contents);
  return path;
}

/** What `lend` gave `use`: the bytes as text, and the memory they were read into. */
async function lent(buffers: WalkReadBuffers, path: string, size: number): Promise<{ text: string; memory: ArrayBufferLike }> {
  return await buffers.lend(path, size, async read => ({ text: read.text(), memory: read.bytes.buffer }));
}

/** mulberry32: the same numbers from the same seed on every run. */
function seeded(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let mixed = Math.imul(state ^ (state >>> 15), state | 1);
    mixed ^= mixed + Math.imul(mixed ^ (mixed >>> 7), mixed | 61);
    return ((mixed ^ (mixed >>> 14)) >>> 0) / 4294967296;
  };
}

/** File sizes at and around the edges a read meets: empty, the first buffer's 64 KiB, its doubling, the limits below. */
const EDGE_SIZES = [0, 1, 4_095, 4_096, 65_535, 65_536, 65_537, 131_071, 131_072, 131_073, 100_000, 100_001, 100_002];

/** A file of random bytes, UTF-8 or not, of a size at an edge or anywhere below 200 KB. */
async function randomFile(name: string, next: () => number): Promise<{ path: string; size: number }> {
  const size = next() < 0.5 ? EDGE_SIZES[Math.floor(next() * EDGE_SIZES.length)]! : Math.floor(next() * 200_000);
  const bytes = Buffer.alloc(size);
  for (let index = 0; index < size; index += 1) bytes[index] = Math.floor(next() * 256);
  const path = join(folder, name);
  await writeFile(path, bytes);
  return { path, size };
}

/** The size a walk's stamp could give for a file of `size` bytes: its own, or one it had before it grew or shrank. */
function stampedSize(size: number, next: () => number): number {
  if (next() < 0.6) return size;
  return Math.max(0, size + Math.floor((next() - 0.5) * 140_000));
}

/** Where `lend`'s read of the file at `path` differs from `readFile`'s bytes, cut at `limit` as a walk cuts them; else undefined. */
async function differsFromReadFile(buffers: WalkReadBuffers, path: string, stamp: number, limit: number): Promise<string | undefined> {
  const whole = await readFile(path);
  const expected = whole.subarray(0, Math.min(whole.length, limit));
  const [same, text] = await buffers.lend(path, stamp, async read => [read.bytes.equals(expected), read.text()] as const);
  return same && text === expected.toString("utf8") ? undefined : `${path}: ${whole.length} bytes, stamp ${stamp}, limit ${limit}`;
}

describe("WalkReadBuffers", () => {
  it("reads each file whole into the buffer the read before it gave back", async () => {
    const buffers = new WalkReadBuffers(1_000_001);
    const first = await lent(buffers, await file("a.py", "a = 1\n"), 6);
    const second = await lent(buffers, await file("b.py", "b = 22\n"), 7);
    expect([first.text, second.text]).toEqual(["a = 1\n", "b = 22\n"]);
    expect(second.memory).toBe(first.memory);
  });

  it("gives reads that overlap buffers of their own", async () => {
    const buffers = new WalkReadBuffers(1_000_001);
    const a = await file("a.py", "a\n");
    const b = await file("b.py", "b\n");
    const inner = buffers.lend(a, 2, async outer => {
      const other = await lent(buffers, b, 2);
      return [outer.bytes.buffer === other.memory, outer.text(), other.text];
    });
    expect(await inner).toEqual([false, "a\n", "b\n"]);
  });

  it("reads a file that grew or shrank since its stamp to its end", async () => {
    const buffers = new WalkReadBuffers(1_000_001);
    const grown = "g".repeat(200_000);
    expect((await lent(buffers, await file("grown.py", grown), 10)).text).toBe(grown);
    expect((await lent(buffers, await file("shrunk.py", "short\n"), 5_000)).text).toBe("short\n");
    expect((await lent(buffers, await file("empty.py", ""), 0)).text).toBe("");
  });

  it("reads a file past the limit only to one byte past it", async () => {
    const buffers = new WalkReadBuffers(101);
    const path = await file("large.py", "x".repeat(1_000));
    expect(await buffers.lend(path, 0, async read => read.bytes.length)).toBe(101);
    expect(await buffers.lend(path, 1_000, async read => read.bytes.length)).toBe(101);
  });

  it("grows a buffer to the limit and no further for a file past it, from any stamp", async () => {
    const buffers = new WalkReadBuffers(150_000);
    const bytes = Buffer.alloc(200_000);
    for (let index = 0; index < bytes.length; index += 1) bytes[index] = index % 251;
    const path = join(folder, "large.bin");
    await writeFile(path, bytes);
    // From the first buffer's 64 KiB, doubled until the limit stops it; and from a buffer the stamp sizes at the limit.
    for (const stamp of [10, 200_000]) {
      expect(await buffers.lend(path, stamp, async read => read.bytes.equals(bytes.subarray(0, 150_000))), `stamp ${stamp}`).toBe(true);
    }
  });

  it("reads a file that fills the first buffer exactly whole, with the read after it that finds nothing", async () => {
    const buffers = new WalkReadBuffers(1_000_001);
    const contents = "y".repeat(64 * 1024);
    const path = await file("full.py", contents);
    for (const stamp of [0, 65_535, 65_536]) expect((await lent(buffers, path, stamp)).text, `stamp ${stamp}`).toBe(contents);
  });

  it("gives its buffer back when a read fails, so the read after it takes no new one", async () => {
    const buffers = new WalkReadBuffers(1_000_001);
    const first = await lent(buffers, await file("a.py", "a\n"), 2);
    // A file swapped for a folder: where a folder opens, as on Linux and macOS, the read fails with EISDIR.
    await expect(lent(buffers, folder, 2)).rejects.toThrow();
    const next = await lent(buffers, await file("b.py", "b\n"), 2);
    expect(next.memory).toBe(first.memory);
  });

  it.each(["read", "close"] as const)("keeps the buffer a read grew when its %s then fails, so a read as large after it allocates none", async call => {
    const buffers = new WalkReadBuffers(1_000_001);
    const contents = "z".repeat(100_000);
    const path = await file("large.py", contents);
    // A stamp of 10 bytes starts the read in a 64 KiB buffer, which it fills and leaves for one of 128 KiB.
    failing.next = call;
    await expect(lent(buffers, path, 10)).rejects.toThrow("EIO");
    const allocations = vi.spyOn(Buffer, "allocUnsafeSlow");
    try {
      const next = await lent(buffers, path, 100_000);
      expect(next.text).toBe(contents);
      expect(next.memory.byteLength).toBe(128 * 1024);
      expect(allocations).not.toHaveBeenCalled();
    } finally {
      allocations.mockRestore();
    }
  });

  it("gives the bytes readFile gives, cut at the limit, whatever the file's size, stamp and the reads before it", async () => {
    const differing: string[] = [];
    for (const limit of [1_000_001, 100_001]) {
      // One set of buffers for every read, so each read lands in a buffer an earlier, larger file may have filled.
      const buffers = new WalkReadBuffers(limit);
      const next = seeded(limit);
      for (let round = 0; round < 40; round += 1) {
        // Three reads at once, as a walk runs them, each with a buffer of its own.
        const files = await Promise.all([0, 1, 2].map(async index => await randomFile(`${limit}-${round}-${index}.bin`, next)));
        const stamps = files.map(({ size }) => stampedSize(size, next));
        const found = await Promise.all(files.map(async ({ path }, index) => await differsFromReadFile(buffers, path, stamps[index]!, limit)));
        differing.push(...found.filter(difference => difference !== undefined));
      }
    }
    expect(differing).toEqual([]);
  }, 30_000);
});
