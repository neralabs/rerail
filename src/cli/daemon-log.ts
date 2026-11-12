/**
 * The daemon's stdout and stderr are `daemon.log`, which every daemon start appends to. Each line Rerail writes there
 * starts with the time it was written, so a crash, a restart or an error can be placed in time and next to what a
 * person saw. Output written to the file descriptor directly, as Node's report of a fatal error and the native code's
 * `eprintln!` are, passes untimed; the timed lines around it bound when it came.
 */

const NEWLINE = 0x0a;

/** The bytes of a write that is text: bytes as they are, a UTF-8 string encoded; none for a string in another encoding. */
function textBytes(chunk: unknown, encoding: unknown): Uint8Array | undefined {
  if (chunk instanceof Uint8Array) return chunk;
  if (typeof chunk !== "string") return undefined;
  const name = typeof encoding === "string" ? encoding.toLowerCase() : "utf8";
  return name === "utf8" || name === "utf-8" ? Buffer.from(chunk, "utf8") : undefined;
}

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
 * A string goes as its UTF-8 bytes and bytes as they are, so a character split across two writes stays whole; a string
 * in another encoding (`base64`, `hex`) is not text and passes as it is.
 */
export function stampLines(stream: Pick<NodeJS.WriteStream, "write">, now: () => Date = () => new Date()): void {
  const write = stream.write.bind(stream) as (chunk: unknown, ...rest: unknown[]) => boolean;
  let atLineStart = true;
  stream.write = ((chunk: unknown, ...rest: unknown[]) => {
    const bytes = textBytes(chunk, rest[0]);
    if (bytes === undefined || bytes.length === 0) return write(chunk, ...rest);
    const stamped = stampedBytes(bytes, Buffer.from(`${now().toISOString()} `, "utf8"), atLineStart);
    atLineStart = bytes[bytes.length - 1] === NEWLINE;
    return write(stamped, ...rest);
  }) as typeof stream.write;
}
