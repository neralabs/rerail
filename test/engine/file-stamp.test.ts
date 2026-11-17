import { runInNewContext } from "node:vm";
import { setFlagsFromString } from "node:v8";
import { describe, expect, it } from "vitest";
import { fileStamp } from "../../src/engine/file-stamp.js";

setFlagsFromString("--expose-gc");
const gc = runInNewContext("gc") as () => void;

/** Heap bytes `make` leaves behind, after full collections on both sides. */
function retainedBy<T>(make: () => T): { value: T; bytes: number } {
  gc();
  const before = process.memoryUsage().heapUsed;
  const value = make();
  gc();
  return { value, bytes: process.memoryUsage().heapUsed - before };
}

describe("a file's stamp", () => {
  it("names the file's device, inode, size and times", () => {
    expect(fileStamp({ dev: 1, ino: 2, size: 3, mtimeMs: 4.5, ctimeMs: 6 })).toBe("1:2:3:4.5:6");
  });

  it("is one flat string, since a walk keeps one for every file of the checkout in every session and walk", () => {
    // Numbers as Windows reports them: a 64-bit file index and fractional milliseconds.
    const count = 100_000;
    const { value, bytes } = retainedBy(() => Array.from({ length: count }, (_, index) => fileStamp({
      dev: 3_489_660_928 + index, ino: 28_147_497_671_065_600 + index, size: 4_096 + index, mtimeMs: 1_794_240_777_123.617 + index, ctimeMs: 1_794_240_777_163.572 + index,
    })));
    expect(value).toHaveLength(count);
    // A template literal's rope takes about 340 bytes a stamp, a flat string about 70; the array adds 8.
    expect(bytes / count).toBeLessThan(150);
  });
});
