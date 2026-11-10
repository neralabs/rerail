import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { blobIds, heldAs, textAs } from "../../../src/engine/code-rule/line-endings.js";

/** A file's digest as a walk records it: the SHA-256 of its bytes. */
function digestOf(bytes: string): string {
  return createHash("sha256").update(Buffer.from(bytes, "latin1")).digest("hex");
}
const blob = (bytes: string): Buffer => Buffer.from(bytes, "latin1");

describe("how a file holds a Git blob", () => {
  it("holds a blob as stored when its bytes are the blob's, whatever the repository writes", () => {
    for (const crlf of [true, false]) {
      expect(heldAs(blob("a\nb\n"), digestOf("a\nb\n"), crlf)).toBe("stored");
      expect(heldAs(blob("a\r\nb\r\n"), digestOf("a\r\nb\r\n"), crlf)).toBe("stored");
    }
  });

  it("holds an LF blob in CRLF only where the repository has a checkout write CRLF", () => {
    expect(heldAs(blob("a\nb\n"), digestOf("a\r\nb\r\n"), true)).toBe("crlf");
    expect(heldAs(blob("a\nb\n"), digestOf("a\r\nb\r\n"), false)).toBeUndefined();
  });

  it("never holds a blob with a CR already in it in CRLF, since Git writes that blob as it is", () => {
    expect(heldAs(blob("a\r\nb\n"), digestOf("a\r\nb\r\n"), true)).toBeUndefined();
    expect(heldAs(blob("a\r\nb\n"), digestOf("a\r\r\nb\r\n"), true)).toBeUndefined();
  });

  it("never holds binary data in CRLF, since Git writes a blob with a NUL as it is", () => {
    expect(heldAs(blob("a\0\nb\n"), digestOf("a\0\r\nb\r\n"), true)).toBeUndefined();
    expect(heldAs(blob("a\0\nb\n"), digestOf("a\0\nb\n"), true)).toBe("stored");
  });

  it("holds a blob with no line break only as stored", () => {
    expect(heldAs(blob("abc"), digestOf("abc"), true)).toBe("stored");
    expect(heldAs(blob("abc"), digestOf("abc\r\n"), true)).toBeUndefined();
  });

  it("does not hold a blob a file differs from in either form", () => {
    expect(heldAs(blob("a\nb\n"), digestOf("a\nc\n"), true)).toBeUndefined();
  });

  it("counts a file absent on both sides as held, and absent on one side as not", () => {
    expect(heldAs(undefined, undefined, true)).toBe("stored");
    expect(heldAs(undefined, digestOf("a\n"), true)).toBeUndefined();
    expect(heldAs(blob("a\n"), undefined, true)).toBeUndefined();
  });
});

describe("a blob as text in the form a file holds it", () => {
  it("gives an LF blob CRLF line breaks in the CRLF form, and leaves it as it is in the stored form", () => {
    expect(textAs(blob("a\nb\n"), "crlf")).toBe("a\r\nb\r\n");
    expect(textAs(blob("a\nb\n"), "stored")).toBe("a\nb\n");
  });

  it("leaves a blob Git writes as it is unchanged in the CRLF form: a CR in it, a NUL, or no line break", () => {
    expect(textAs(blob("a\r\nb\n"), "crlf")).toBe("a\r\nb\n");
    expect(textAs(blob("a\0\nb\n"), "crlf")).toBe("a\0\nb\n");
    expect(textAs(blob("abc"), "crlf")).toBe("abc");
  });

  it("decodes the blob as UTF-8", () => {
    expect(textAs(Buffer.from("é\n", "utf8"), "crlf")).toBe("é\r\n");
  });
});

describe("the ids of the blobs a file's text can come from", () => {
  // `git hash-object` of these bytes: SHA-1, and SHA-256 in a repository made with `--object-format=sha256`.
  const HELLO_LF_SHA1 = "ce013625030ba8dba906f756967f9e9ca394464a";
  const HELLO_CRLF_SHA1 = "ef0493b275aa2080237f676d2ef6559246f56636";
  const HELLO_LF_SHA256 = "2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4";

  it("names the text's own blob in the repository's object format", () => {
    expect(blobIds("hello\n", "sha1", false)).toEqual([HELLO_LF_SHA1]);
    expect(blobIds("hello\n", "sha256", false)).toEqual([HELLO_LF_SHA256]);
  });

  it("adds the LF blob a checkout writes as CRLF text, where the repository has a checkout write CRLF", () => {
    expect(blobIds("hello\r\n", "sha1", true)).toEqual([HELLO_CRLF_SHA1, HELLO_LF_SHA1]);
    expect(blobIds("hello\r\n", "sha1", false)).toEqual([HELLO_CRLF_SHA1]);
  });

  it("names one blob for text with no CRLF in it", () => {
    expect(blobIds("hello\n", "sha1", true)).toEqual([HELLO_LF_SHA1]);
    expect(blobIds("hello", "sha1", true)).toHaveLength(1);
  });

  it("names one blob for text a checkout never writes: an LF without its CR, a lone CR, or a NUL", () => {
    // A checkout writes every LF of an LF blob as CRLF, and a blob with a CR or a NUL as it is (`heldAs`).
    expect(blobIds("hello\r\nworld\n", "sha1", true)).toHaveLength(1);
    expect(blobIds("hello\rworld\r\n", "sha1", true)).toHaveLength(1);
    expect(blobIds("hello\0\r\n", "sha1", true)).toHaveLength(1);
  });
});
