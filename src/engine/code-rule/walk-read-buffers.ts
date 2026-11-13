import { type FileHandle, open } from "node:fs/promises";

/** The size a walk's read buffer starts at: most sources fit, and a larger file grows the buffer it is read into. */
const FIRST_BUFFER_BYTES = 64 * 1024;

/** A file's bytes as a walk read them into a buffer it lends, valid only until the read's `use` settles. */
export interface LentBytes {
  /** The file's bytes: a view of the lent buffer. */
  readonly bytes: Buffer;
  /** The bytes as UTF-8 text, the one copy of them a walk makes, for a file it keeps the text of. */
  text(): string;
}

/** A buffer and how many of its bytes a read filled. */
interface FilledBuffer {
  readonly buffer: Buffer;
  readonly length: number;
}

/**
 * The buffers one walk reads files into, each lent to one read at a time and taken back once that read's caller is done
 * with it. A walk reads every file it holds no digest for and keeps the digest alone of most of them. A buffer of its
 * own per file, as `readFile` makes, is memory outside the heap, which V8 counts apart and answers with a full
 * collection each time it grows by a set amount; a walk that kept no text then ran several times the full collections
 * of one that did. With these, a walk holds one buffer per read it runs at once, grown to the largest file it read.
 */
export class WalkReadBuffers {
  readonly #free: Buffer[] = [];
  /** The most a read takes: one byte past what can be a source, so its caller can tell a file too large to be one. */
  readonly #limit: number;

  constructor(limit: number) {
    this.#limit = limit;
  }

  /**
   * `use` with the bytes of the file at `absolute`, read to its end or the limit. `size`, the size its stamp gave, sizes
   * the buffer, so a file that still holds it takes one read. The file is closed before `use` runs.
   */
  async lend<T>(absolute: string, size: number, use: (read: LentBytes) => Promise<T>): Promise<T> {
    const { buffer, length } = await this.#read(absolute, size);
    try {
      return await use({ bytes: buffer.subarray(0, length), text: () => buffer.toString("utf8", 0, length) });
    } finally {
      this.#free.push(buffer);
    }
  }

  async #read(absolute: string, size: number): Promise<FilledBuffer> {
    const handle = await open(absolute);
    try {
      return await fill(handle, this.#take(Math.min(size + 1, this.#limit)), size, this.#limit);
    } finally {
      await handle.close();
    }
  }

  /** A free buffer of at least `size` bytes, else a new one: a smaller free one is dropped, so the buffers grow. */
  #take(size: number): Buffer {
    const free = this.#free.pop();
    return free !== undefined && free.length >= size ? free : Buffer.allocUnsafeSlow(Math.max(size, FIRST_BUFFER_BYTES));
  }
}

/**
 * Reads `handle` into `start` to the file's end, or to `limit` bytes, moving to a larger buffer when it fills first. A
 * read that stops short once the file holds `size` bytes is its end, as `readFile` takes the size it finds for the end;
 * a shorter file is read on until a read finds nothing.
 */
async function fill(handle: FileHandle, start: Buffer, size: number, limit: number): Promise<FilledBuffer> {
  let buffer = start;
  let length = 0;
  for (;;) {
    if (length === buffer.length) buffer = grown(buffer, length, limit);
    const wanted = buffer.length - length;
    const { bytesRead } = await handle.read(buffer, length, wanted, length);
    length += bytesRead;
    if (bytesRead === 0 || (bytesRead < wanted && length >= size)) return { buffer, length };
  }
}

/** A buffer twice as large as `full`, at most `limit` bytes, holding its first `length` bytes; `full` at the limit. */
function grown(full: Buffer, length: number, limit: number): Buffer {
  if (full.length >= limit) return full;
  const next = Buffer.allocUnsafeSlow(Math.min(full.length * 2, limit));
  full.copy(next, 0, 0, length);
  return next;
}
