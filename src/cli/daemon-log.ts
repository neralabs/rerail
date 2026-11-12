/**
 * The daemon's stdout and stderr are `daemon.log`, which every daemon start appends to. Each line Rerail writes there
 * starts with the time it was written, so a crash, a restart or an error can be placed in time and next to what a
 * person saw. Output written to the file descriptor directly, as Node's report of a fatal error and the native code's
 * `eprintln!` are, passes untimed; the timed lines around it bound when it came.
 */

/** A newline with more of the same write after it: where the next line's time goes. */
const LINE_INSIDE_TEXT = /\n(?=[^])/gu;
const NEWLINE = 0x0a;

/** `bytes` with `stamp` after every newline that more bytes follow, and first when `atLineStart`. */
function stampedBytes(bytes: Uint8Array, stamp: Buffer, atLineStart: boolean): Buffer {
  const parts: Uint8Array[] = atLineStart ? [stamp] : [];
  let start = 0;
  for (let index = bytes.indexOf(NEWLINE); index !== -1 && index < bytes.length - 1; index = bytes.indexOf(NEWLINE, start)) {
    parts.push(bytes.subarray(start, index + 1), stamp);
    start = index + 1;
  }
  parts.push(bytes.subarray(start));
  return Buffer.concat(parts);
}

/**
 * Makes every line written to `stream` start with the time it was written (`now`), in UTC as ISO 8601, then a space.
 * Lines are counted across writes: a write that ends mid-line leaves the rest of that line to the next one, unstamped.
 * Bytes are stamped as bytes, so a character split across two writes stays whole; a string in another encoding
 * (`base64`, `hex`) is not text and passes as it is.
 */
export function stampLines(stream: Pick<NodeJS.WriteStream, "write">, now: () => Date = () => new Date()): void {
  const write = stream.write.bind(stream) as (chunk: unknown, ...rest: unknown[]) => boolean;
  let atLineStart = true;
  stream.write = ((chunk: unknown, ...rest: unknown[]) => {
    const encoding = typeof rest[0] === "string" ? rest[0].toLowerCase() : "utf8";
    if (typeof chunk === "string" && chunk !== "" && (encoding === "utf8" || encoding === "utf-8")) {
      const stamp = `${now().toISOString()} `;
      const stamped = `${atLineStart ? stamp : ""}${chunk.replace(LINE_INSIDE_TEXT, `\n${stamp}`)}`;
      atLineStart = chunk.endsWith("\n");
      return write(stamped, ...rest);
    }
    if (chunk instanceof Uint8Array && chunk.length > 0) {
      const stamped = stampedBytes(chunk, Buffer.from(`${now().toISOString()} `, "utf8"), atLineStart);
      atLineStart = chunk[chunk.length - 1] === NEWLINE;
      return write(stamped, ...rest);
    }
    return write(chunk, ...rest);
  }) as typeof stream.write;
}
