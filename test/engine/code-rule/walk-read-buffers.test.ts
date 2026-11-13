import { mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { WalkReadBuffers } from "../../../src/engine/code-rule/walk-read-buffers.js";

let folder: string;

beforeEach(async () => {
  folder = await realpath(await mkdtemp(join(tmpdir(), "rerail-walk-read-buffers-")));
});
afterEach(async () => {
  await rm(folder, { recursive: true, force: true });
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
});
