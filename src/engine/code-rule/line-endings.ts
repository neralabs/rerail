/**
 * How a file compares with a Git blob under Git's line-ending conversion. Where a repository has a checkout write CRLF
 * (`core.autocrlf=true`, which Git for Windows' installer sets for the whole machine, or a `text` or `eol=crlf`
 * attribute), the working tree does not hold a blob's bytes, so a comparison of the two by digest or blob id has to try
 * both forms, or every file Git writes reads as the session's own edit. Which rule a checkout follows for a path is the
 * repository's settings' answer (`gitCrlfPaths`); each function here takes that answer as `rule`, undefined where a
 * checkout writes the blob as stored. That answer costs Git processes, and moves a comparison only for a file that holds
 * a blob in its CRLF form alone (`heldOnlyInCrlf`, `crlfSourceId`), so it is asked for those files only.
 */
import { createHash } from "node:crypto";
import type { CrlfRule } from "../git.js";

/** The bytes a file holds a blob in: the blob's own (`stored`), or those a checkout writes for it by a CRLF rule. */
export type LineForm = "stored" | CrlfRule;

/** A repository's object format, which names the hash Git takes of a blob for its id. */
export type BlobIdAlgorithm = "sha1" | "sha256";

const LF = 0x0a;
const CR = 0x0d;
/** A file's last byte Git leaves out of its count of control bytes: Ctrl-Z, which ended a file on DOS. */
const DOS_END_OF_FILE = 0x1a;
/** The control bytes Git counts as text: backspace, tab, escape and form feed. */
const TEXT_CONTROLS: ReadonlySet<number> = new Set([0x08, 0x09, 0x1b, 0x0c]);
const DELETE = 0x7f;

/**
 * How a file whose bytes have the SHA-256 `digest` (hex) holds `blob`: as stored, as a checkout writes it by `rule`, or
 * not at all (undefined). No blob and no digest, a file absent on both sides, counts as held as stored. The CRLF form
 * is built and hashed only when the stored form misses.
 */
export function heldAs(blob: Buffer | undefined, digest: string | undefined, rule: CrlfRule | undefined): LineForm | undefined {
  if (blob === undefined) return digest === undefined ? "stored" : undefined;
  if (digest === undefined) return undefined;
  if (sha256(blob) === digest) return "stored";
  if (rule === undefined) return undefined;
  const written = crlfCheckout(blob, rule);
  return written !== undefined && sha256(written) === digest ? rule : undefined;
}

/**
 * Whether a file whose bytes have `digest` holds `blob` only in the CRLF form `text` writes, not as stored. Only then
 * does the rule a repository follows change what {@link heldAs} answers: `auto` writes what `text` writes or the blob
 * as stored, and no rule makes a file that holds neither form hold the blob. So no file of which this is false needs
 * the rule asked for. A CRLF form always holds a CR, so a file without one never holds a blob only in it.
 */
export function heldOnlyInCrlf(blob: Buffer | undefined, digest: string | undefined): boolean {
  if (blob === undefined || digest === undefined || sha256(blob) === digest) return false;
  const written = crlfCheckout(blob, "text");
  return written !== undefined && sha256(written) === digest;
}

/** `blob` as UTF-8 text in `form`, so it lines up line by line with a working tree that holds it that way. */
export function textAs(blob: Buffer, form: LineForm): string {
  return (form === "stored" ? blob : crlfCheckout(blob, form) ?? blob).toString("utf8");
}

/**
 * The ids of the blobs a file holding `text` (as UTF-8) can have been checked out from: the id of `text` itself and,
 * where `rule` has a checkout write CRLF, the id of the blob with each CRLF an LF, when a checkout writes that blob as
 * `text` ({@link checkedOutFrom}). Under `text` a blob that holds some of those line breaks as CRLF and the rest as LF
 * checks out as `text` too; one per mix, so these are not looked for.
 */
export function blobIds(text: string, algorithm: BlobIdAlgorithm, rule: CrlfRule | undefined): string[] {
  const source = rule === undefined ? undefined : checkedOutFrom(text, rule);
  return source === undefined ? [blobId(text, algorithm)] : [blobId(text, algorithm), blobId(source, algorithm)];
}

/**
 * The id of the LF blob a checkout by `text` writes as `text`, undefined where it writes none: the one id any rule adds
 * to the text's own in {@link blobIds}, since `auto` writes what `text` writes or the blob as stored. Only where the
 * repository holds that blob does the rule decide whether `text` can have been checked out from it.
 */
export function crlfSourceId(text: string, algorithm: BlobIdAlgorithm): string | undefined {
  const source = checkedOutFrom(text, "text");
  return source === undefined ? undefined : blobId(source, algorithm);
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
 * The bytes a checkout writes for `blob` by `rule`, as Git's `crlf_to_worktree` does: each LF that follows no CR becomes
 * CRLF. Undefined when it writes the blob as it is: no such LF, or under `auto` a blob that holds a CR, which Git leaves
 * alone rather than convert one way only, or that Git takes for binary data ({@link looksBinary}).
 */
function crlfCheckout(blob: Buffer, rule: CrlfRule): Buffer | undefined {
  if (rule === "auto" && (blob.includes(CR) || looksBinary(blob))) return undefined;
  const text = blob.toString("latin1");
  const written = text.replaceAll(/(?<!\r)\n/gu, "\r\n");
  return written.length === text.length ? undefined : Buffer.from(written, "latin1");
}

/**
 * Whether Git takes `blob` for binary data, as its `convert_is_binary` does: a NUL in it, or fewer whole 128s of text
 * bytes than control bytes. LF and CR count as neither, and backspace, tab, escape, form feed and every byte from 128
 * up as text; a final Ctrl-Z is left out. Only `auto` asks, and only of a blob with no CR in it.
 */
function looksBinary(blob: Buffer): boolean {
  let text = 0;
  let control = 0;
  for (const byte of blob) {
    if (byte === 0) return true;
    if (byte === LF || byte === CR) continue;
    if (byte === DELETE || (byte < 0x20 && !TEXT_CONTROLS.has(byte))) control++;
    else text++;
  }
  if (blob.at(-1) === DOS_END_OF_FILE) control--;
  return text >> 7 < control;
}

/**
 * The text of the blob with each CRLF of `text` an LF, when a checkout writes that blob as `text` by `rule`. Undefined
 * otherwise: `text` holds no CRLF, or holds what that checkout never writes, such as an LF without its CR, or under
 * `auto` a lone CR or a NUL.
 */
function checkedOutFrom(text: string, rule: CrlfRule): string | undefined {
  if (!text.includes("\r\n")) return undefined;
  const source = text.replaceAll("\r\n", "\n");
  return crlfCheckout(Buffer.from(source, "utf8"), rule)?.equals(Buffer.from(text, "utf8")) === true ? source : undefined;
}
