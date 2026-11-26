import { readSync } from "node:fs";

/** How many bytes each read of a file takes; a line longer than this spans reads. */
const LINE_READ_BYTES = 1024 * 1024;

/** What follows a file's last newline once the whole file was read. */
export interface LastLine {
  /** Its text: "" when the file is empty or ends with a newline. */
  readonly text: string;
  /** Where it starts in the file, in bytes: the length of the file less the last line's. */
  readonly start: number;
}

/**
 * Cuts the bytes of a file, given in the order they were read, into lines at each "\n", and hands each line on as text
 * decoded from UTF-8 on its own. A "\n" byte is never part of a multi-byte character, so each line decodes as it would
 * in a decode of the whole file, a "\r" before the "\n" kept. It holds the bytes of one line at most: a file of any
 * size is read without a string longer than its longest line, where the text of the whole file could not be longer
 * than Node's longest string (`buffer.constants.MAX_STRING_LENGTH`, 536,870,888 characters in Node 22 and 24).
 */
export class LineSplitter {
  readonly #onLine: (line: string) => void;
  /** The bytes of the line being read that earlier reads gave, each a copy. */
  #held: Buffer[] = [];
  #lineStart = 0;
  #read = 0;

  constructor(onLine: (line: string) => void) {
    this.#onLine = onLine;
  }

  /** Takes the next bytes read; the caller may reuse `bytes` once this returns. */
  push(bytes: Buffer): void {
    let from = 0;
    for (let newline = bytes.indexOf(0x0a); newline !== -1; newline = bytes.indexOf(0x0a, from)) {
      const part = bytes.subarray(from, newline);
      const line = this.#held.length === 0 ? part.toString("utf8") : Buffer.concat([...this.#held, part]).toString("utf8");
      this.#held = [];
      from = newline + 1;
      this.#lineStart = this.#read + from;
      this.#onLine(line);
    }
    if (from < bytes.length) this.#held.push(Buffer.from(bytes.subarray(from)));
    this.#read += bytes.length;
  }

  /** The last line, once every byte of the file was pushed. */
  end(): LastLine {
    return { text: Buffer.concat(this.#held).toString("utf8"), start: this.#lineStart };
  }
}

/** A file's read at a position, as `FileHandle` gives it. */
export interface PositionalRead {
  read(buffer: Buffer, offset: number, length: number, position: number): Promise<{ readonly bytesRead: number }>;
}

/**
 * Hands each line of the file open at `handle` to `onLine` ({@link LineSplitter}), read by position from its start in
 * reads of `readBytes`, and returns its last line. A line `onLine` throws for ends the read with that error.
 */
export async function eachLine(handle: PositionalRead, onLine: (line: string) => void, readBytes = LINE_READ_BYTES): Promise<LastLine> {
  const lines = new LineSplitter(onLine);
  const chunk = Buffer.allocUnsafe(readBytes);
  for (let position = 0; ;) {
    const { bytesRead } = await handle.read(chunk, 0, readBytes, position);
    if (bytesRead === 0) return lines.end();
    lines.push(chunk.subarray(0, bytesRead));
    position += bytesRead;
  }
}

/** {@link eachLine} on the file open at `descriptor`, for a caller that reads in one synchronous step. */
export function eachLineSync(descriptor: number, onLine: (line: string) => void, readBytes = LINE_READ_BYTES): LastLine {
  const lines = new LineSplitter(onLine);
  const chunk = Buffer.allocUnsafe(readBytes);
  for (let position = 0; ;) {
    const bytesRead = readSync(descriptor, chunk, 0, readBytes, position);
    if (bytesRead === 0) return lines.end();
    lines.push(chunk.subarray(0, bytesRead));
    position += bytesRead;
  }
}
