/**
 * The daemon's stdout and stderr are `daemon.log`, which every daemon start appends to. Each line Rerail writes there
 * starts with the time it was written, so a crash, a restart or an error can be placed in time and next to what a
 * person saw. Output written to the file descriptor directly, as Node's report of a fatal error and the native code's
 * `eprintln!` are, passes untimed; the timed lines around it bound when it came.
 */

const NEWLINE = 0x0a;

/** Encodings whose string is not the text it writes but bytes spelled out (`aGk=`), so the bytes are the text. */
const BYTE_ENCODINGS = new Set(["base64", "base64url", "hex"]);

/** Where the file the streams share stands: at the start of a line, or inside one a write left unfinished. */
interface LineState {
  atStart: boolean;
}

/**
 * What a write puts in the file, as text: a string in a text encoding as it is; bytes, and a string of bytes spelled
 * out, as bytes. Undefined for anything else, and for an encoding Node does not know, which its write then refuses.
 */
function writtenText(chunk: unknown, encoding: unknown): string | Uint8Array | undefined {
  if (typeof chunk !== "string") return chunk instanceof Uint8Array ? chunk : undefined;
  const name = typeof encoding === "string" ? encoding.toLowerCase() : "utf8";
  if (!Buffer.isEncoding(name)) return undefined;
  return BYTE_ENCODINGS.has(name) ? Buffer.from(chunk, name) : chunk;
}

/** `text` with `stamp` after every newline that more text follows, and first when `atLineStart`. */
function stampedString(text: string, stamp: string, atLineStart: boolean): string {
  const ended = text.endsWith("\n");
  const inner = (ended ? text.slice(0, -1) : text).replaceAll("\n", `\n${stamp}`);
  return `${atLineStart ? stamp : ""}${inner}${ended ? "\n" : ""}`;
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

/** Whether `text` ends with a newline. */
function endsLine(text: string | Uint8Array): boolean {
  return typeof text === "string" ? text.endsWith("\n") : text[text.length - 1] === NEWLINE;
}

/**
 * Makes every line written to `streams` start with the time it was written (`now`), in UTC as ISO 8601, then a space.
 * The streams write to one file, so lines are counted across the writes of all of them: a write that ends mid-line
 * leaves the rest of that line to the next write of any of them, unstamped. A string goes as the text it is, in the
 * encoding it is written in (`latin1` and `utf16le` included); bytes, and a string of bytes spelled out (`base64`,
 * `hex`), go as bytes, so a character split across two writes stays whole.
 */
export function stampLines(streams: ReadonlyArray<Pick<NodeJS.WriteStream, "write">>, now: () => Date = () => new Date()): void {
  const line: LineState = { atStart: true };
  for (const stream of streams) stampStream(stream, line, now);
}

/** Stamps the lines `stream` writes ({@link stampLines}); `line` is shared by every stream of its file. */
function stampStream(stream: Pick<NodeJS.WriteStream, "write">, line: LineState, now: () => Date): void {
  const write = stream.write.bind(stream) as (chunk: unknown, ...rest: unknown[]) => boolean;
  stream.write = ((chunk: unknown, ...rest: unknown[]) => {
    const text = writtenText(chunk, rest[0]);
    if (text === undefined || text.length === 0) return write(chunk, ...rest);
    const stamp = `${now().toISOString()} `;
    const stamped = typeof text === "string" ? stampedString(text, stamp, line.atStart) : stampedBytes(text, Buffer.from(stamp, "utf8"), line.atStart);
    line.atStart = endsLine(text);
    return write(stamped, ...rest);
  }) as typeof stream.write;
}
