/**
 * How a file compares with a Git blob under Git's line-ending conversion. With `core.autocrlf=true`, which Git for
 * Windows' installer sets for the whole machine, a checkout writes each LF of a blob as CRLF, and an add stores CRLF as
 * LF again. The working tree then never holds a blob's bytes, so a comparison of the two by digest or blob id has to try
 * both forms, or every file Git writes reads as the session's own edit. Whether a checkout writes a path with CRLF is the
 * repository's settings' answer (`gitCrlfPaths`); each function here takes that answer as `crlf`.
 */
import { createHash } from "node:crypto";

/** The bytes a file holds a blob in: the blob's own (`stored`), or the CRLF line breaks a checkout writes (`crlf`). */
export type LineForm = "stored" | "crlf";

/** A repository's object format, which names the hash Git takes of a blob for its id. */
export type BlobIdAlgorithm = "sha1" | "sha256";

const LF = 0x0a;
const CR = 0x0d;

/**
 * How a file whose bytes have the SHA-256 `digest` (hex) holds `blob`: as stored, as a checkout writes it with CRLF
 * line breaks where `crlf` says the repository has it do so, or not at all (undefined). No blob and no digest, a file
 * absent on both sides, counts as held as stored. The CRLF form is built and hashed only when the stored form misses.
 */
export function heldAs(blob: Buffer | undefined, digest: string | undefined, crlf: boolean): LineForm | undefined {
  if (blob === undefined) return digest === undefined ? "stored" : undefined;
  if (digest === undefined) return undefined;
  if (sha256(blob) === digest) return "stored";
  const checkedOut = crlf ? crlfCheckout(blob) : undefined;
  return checkedOut !== undefined && sha256(checkedOut) === digest ? "crlf" : undefined;
}

/** `blob` as UTF-8 text in `form`, so it lines up line by line with a working tree that holds it that way. */
export function textAs(blob: Buffer, form: LineForm): string {
  return (form === "crlf" ? crlfCheckout(blob) ?? blob : blob).toString("utf8");
}

/**
 * The ids of the blobs a file holding `text` (as UTF-8) can have been checked out from: the id of `text` itself and,
 * where `crlf` says a checkout writes CRLF, the id of the LF blob a checkout writes as `text` ({@link checkedOutFrom}).
 */
export function blobIds(text: string, algorithm: BlobIdAlgorithm, crlf: boolean): string[] {
  const source = crlf ? checkedOutFrom(text) : undefined;
  return source === undefined ? [blobId(text, algorithm)] : [blobId(text, algorithm), blobId(source, algorithm)];
}

/** Git names a blob by the hash of a `blob <size>\0` header followed by its bytes. */
function blobId(text: string, algorithm: BlobIdAlgorithm): string {
  const bytes = Buffer.from(text, "utf8");
  return createHash(algorithm).update(`blob ${bytes.length}\0`).update(bytes).digest("hex");
}

function sha256(data: Buffer): string {
  return createHash("sha256").update(data).digest("hex");
}

/**
 * The bytes a `core.autocrlf=true` checkout writes for `blob`: every LF a CRLF. Undefined when it writes the blob as it
 * is: no line break, binary data (a NUL), or a CR already there, which Git leaves alone rather than convert one way only.
 */
function crlfCheckout(blob: Buffer): Buffer | undefined {
  if (!blob.includes(LF) || blob.includes(CR) || blob.includes(0)) return undefined;
  return Buffer.from(blob.toString("latin1").replaceAll("\n", "\r\n"), "latin1");
}

/**
 * The text of the blob a `core.autocrlf=true` checkout writes as `text`: every CRLF an LF. Undefined when no blob checks
 * out as `text`: it holds no CRLF, or holds an LF without its CR, a lone CR or a NUL, none of which `crlfCheckout` writes.
 */
function checkedOutFrom(text: string): string | undefined {
  if (!text.includes("\r\n")) return undefined;
  const source = text.replaceAll("\r\n", "\n");
  return crlfCheckout(Buffer.from(source, "utf8"))?.equals(Buffer.from(text, "utf8")) === true ? source : undefined;
}
