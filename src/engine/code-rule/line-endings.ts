/**
 * The two forms of a text file under Git's line-ending conversion. With `core.autocrlf=true`, which Git for Windows'
 * installer sets for the whole machine, a checkout writes each LF of a blob as CRLF, and an add stores CRLF as LF again.
 * The working tree then never holds a blob's bytes, so a comparison of the two by digest or blob id has to try both
 * forms, or every file Git writes reads as the session's own edit.
 */

const LF = 0x0a;
const CR = 0x0d;

/**
 * The bytes a `core.autocrlf=true` checkout writes for `blob`: every LF a CRLF. Undefined when it writes the blob as it
 * is: no line break, binary data (a NUL), or a CR already there, which Git leaves alone rather than convert one way only.
 */
export function crlfCheckout(blob: Buffer): Buffer | undefined {
  if (!blob.includes(LF) || blob.includes(CR) || blob.includes(0)) return undefined;
  return Buffer.from(blob.toString("latin1").replaceAll("\n", "\r\n"), "latin1");
}

/** The text a `core.autocrlf=true` add stores for `text`: every CRLF an LF. Undefined when `text` holds none. */
export function lfStored(text: string): string | undefined {
  return text.includes("\r\n") ? text.replaceAll("\r\n", "\n") : undefined;
}
