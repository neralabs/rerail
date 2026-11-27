import { closeSync, openSync } from "node:fs";
import { mkdtemp, open, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { eachLine, eachLineSync, type LastLine, type PositionalRead } from "../../../src/engine/storage/file-lines.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

/**
 * A file's bytes: lines of one-byte, two-byte, three-byte and four-byte characters, an empty line, a line a "\r" ends,
 * bytes that are no UTF-8 (a three-byte character cut short before its newline, a lone 0xff), and a last line with no
 * newline after it.
 */
const BYTES = Buffer.concat([
  Buffer.from('{"type":"event","eventId":"a"}\n'),
  Buffer.from('{"text":"café € \u{1f600}"}\n\n'),
  Buffer.from('{"crlf":true}\r\n'),
  Buffer.from([0x7b, 0xe2, 0x82, 0x0a, 0xff, 0x7d, 0x0a]),
  Buffer.from('{"torn":"ét'),
]);

/** The lines and the last line a decode of the whole file gives, split at each "\n". */
function wholeSplit(bytes: Buffer): { lines: string[]; last: LastLine } {
  const lines = bytes.toString("utf8").split("\n");
  const text = lines.pop()!;
  return { lines, last: { text, start: bytes.lastIndexOf(0x0a) + 1 } };
}

async function fileOf(bytes: Buffer): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-file-lines-"));
  roots.push(root);
  const path = join(root, "events.jsonl");
  await writeFile(path, bytes);
  return path;
}

async function readAsync(path: string, readBytes: number): Promise<{ lines: string[]; last: LastLine }> {
  const lines: string[] = [];
  const handle = await open(path, "r");
  try {
    return { last: await eachLine(handle, line => lines.push(line), readBytes), lines };
  } finally {
    await handle.close();
  }
}

function readSync(path: string, readBytes: number): { lines: string[]; last: LastLine } {
  const lines: string[] = [];
  const descriptor = openSync(path, "r");
  try {
    return { last: eachLineSync(descriptor, line => lines.push(line), readBytes), lines };
  } finally {
    closeSync(descriptor);
  }
}

/** A handle on `bytes` that serves reads as a file does and notes where each read started. */
function noted(bytes: Buffer): { handle: PositionalRead; reads: number[] } {
  const reads: number[] = [];
  return {
    reads,
    handle: {
      read: async (buffer, offset, length, position) => {
        reads.push(position);
        return { bytesRead: bytes.copy(buffer, offset, position, Math.min(bytes.length, position + length)) };
      },
    },
  };
}

describe("a file read a line at a time", () => {
  it.each([1, 2, 3, 4, 5, 7, 16, 1024 * 1024])("gives each line and the last as a decode of the whole file does, in reads of %i bytes", async readBytes => {
    const path = await fileOf(BYTES);
    const expected = wholeSplit(BYTES);
    expect(expected.lines).toHaveLength(6);
    expect(await readAsync(path, readBytes)).toEqual(expected);
    expect(readSync(path, readBytes)).toEqual(expected);
  });

  it("gives an empty file no line and an empty last line at its start, and a file a newline ends an empty last line at its end", async () => {
    expect(await readAsync(await fileOf(Buffer.alloc(0)), 4)).toEqual({ lines: [], last: { text: "", start: 0 } });
    const whole = Buffer.from('{"a":1}\n{"b":2}\n');
    expect(await readAsync(await fileOf(whole), 3)).toEqual({ lines: ['{"a":1}', '{"b":2}'], last: { text: "", start: whole.length } });
  });

  it("hands a line on once the read that holds its newline returns, before the next read, a line over many reads included", async () => {
    const bytes = Buffer.from(`one\n${"x".repeat(10)}\nlast`);
    const { handle, reads } = noted(bytes);
    const readsAtLine: number[] = [];
    const last = await eachLine(handle, () => readsAtLine.push(reads.length), 3);
    // "one\n" ends in the second read (bytes 3 to 5); the line of ten x's, from byte 4, ends in the fifth (12 to 14).
    expect(readsAtLine).toEqual([2, 5]);
    expect(reads).toEqual([0, 3, 6, 9, 12, 15, 18, 19]);
    expect(last).toEqual({ text: "last", start: 15 });
  });

  it("ends the read with the error its line handler throws, reading no further", async () => {
    const { handle, reads } = noted(Buffer.from("first\nsecond\nthird\n"));
    await expect(eachLine(handle, line => {
      if (line === "first") throw new SyntaxError("not JSON");
    }, 4)).rejects.toThrow("not JSON");
    expect(reads).toEqual([0, 4]);
  });
});
