import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { blobIds, heldAs, textAs } from "../../../src/engine/code-rule/line-endings.js";
import { GitReads, type CrlfRule } from "../../../src/engine/git.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/** A file's digest as a walk records it: the SHA-256 of its bytes. */
function digestOf(bytes: string | Buffer): string {
  return createHash("sha256").update(typeof bytes === "string" ? Buffer.from(bytes, "latin1") : bytes).digest("hex");
}
const blob = (bytes: string): Buffer => Buffer.from(bytes, "latin1");

describe("how a file holds a Git blob", () => {
  it("holds a blob as stored when its bytes are the blob's, whatever the repository writes", () => {
    for (const rule of ["auto", "text", undefined] as const) {
      expect(heldAs(blob("a\nb\n"), digestOf("a\nb\n"), rule)).toBe("stored");
      expect(heldAs(blob("a\r\nb\r\n"), digestOf("a\r\nb\r\n"), rule)).toBe("stored");
    }
  });

  it("holds an LF blob in CRLF only where the repository has a checkout write CRLF, by the rule it does so", () => {
    expect(heldAs(blob("a\nb\n"), digestOf("a\r\nb\r\n"), "auto")).toBe("auto");
    expect(heldAs(blob("a\nb\n"), digestOf("a\r\nb\r\n"), "text")).toBe("text");
    expect(heldAs(blob("a\nb\n"), digestOf("a\r\nb\r\n"), undefined)).toBeUndefined();
  });

  it("holds a blob with a CR already in it in CRLF only by `text`, since `auto` writes that blob as it is", () => {
    expect(heldAs(blob("a\r\nb\n"), digestOf("a\r\nb\r\n"), "auto")).toBeUndefined();
    expect(heldAs(blob("a\r\nb\n"), digestOf("a\r\nb\r\n"), "text")).toBe("text");
    expect(heldAs(blob("a\rb\n"), digestOf("a\rb\r\n"), "text")).toBe("text");
    // `text` keeps a CRLF as it is and writes only an LF without its CR as CRLF.
    expect(heldAs(blob("a\r\nb\n"), digestOf("a\r\r\nb\r\n"), "text")).toBeUndefined();
  });

  it("holds binary data in CRLF only by `text`: `auto` writes a blob with a NUL or too many control bytes as it is", () => {
    expect(heldAs(blob("a\0\nb\n"), digestOf("a\0\r\nb\r\n"), "auto")).toBeUndefined();
    expect(heldAs(blob("a\0\nb\n"), digestOf("a\0\nb\n"), "auto")).toBe("stored");
    expect(heldAs(blob("a\x01\nb\n"), digestOf("a\x01\r\nb\r\n"), "auto")).toBeUndefined();
    expect(heldAs(blob("a\0\nb\n"), digestOf("a\0\r\nb\r\n"), "text")).toBe("text");
    expect(heldAs(blob("a\x01\nb\n"), digestOf("a\x01\r\nb\r\n"), "text")).toBe("text");
  });

  it("takes a blob for binary data under `auto` as Git does: one control byte per 128 printable ones is still text", () => {
    const text = `${"x".repeat(128)}\x01\n`;
    expect(heldAs(blob(text), digestOf(`${"x".repeat(128)}\x01\r\n`), "auto")).toBe("auto");
    expect(heldAs(blob(`x${text}\x01`), digestOf(`x${"x".repeat(128)}\x01\r\n\x01`), "auto")).toBeUndefined();
    // Backspace, tab, escape and form feed are text, DEL is not, and a final Ctrl-Z is not counted.
    expect(heldAs(blob("\b\t\x1b\f\n"), digestOf("\b\t\x1b\f\r\n"), "auto")).toBe("auto");
    expect(heldAs(blob("a\x7f\n"), digestOf("a\x7f\r\n"), "auto")).toBeUndefined();
    expect(heldAs(blob("a\n\x1a"), digestOf("a\r\n\x1a"), "auto")).toBe("auto");
  });

  it("holds a blob with no line break only as stored", () => {
    for (const rule of ["auto", "text"] as const) {
      expect(heldAs(blob("abc"), digestOf("abc"), rule)).toBe("stored");
      expect(heldAs(blob("abc"), digestOf("abc\r\n"), rule)).toBeUndefined();
    }
  });

  it("does not hold a blob a file differs from in either form", () => {
    expect(heldAs(blob("a\nb\n"), digestOf("a\nc\n"), "text")).toBeUndefined();
  });

  it("counts a file absent on both sides as held, and absent on one side as not", () => {
    expect(heldAs(undefined, undefined, "auto")).toBe("stored");
    expect(heldAs(undefined, digestOf("a\n"), "auto")).toBeUndefined();
    expect(heldAs(blob("a\n"), undefined, "auto")).toBeUndefined();
  });
});

describe("a blob as text in the form a file holds it", () => {
  it("gives an LF blob CRLF line breaks in either CRLF form, and leaves it as it is in the stored form", () => {
    expect(textAs(blob("a\nb\n"), "auto")).toBe("a\r\nb\r\n");
    expect(textAs(blob("a\nb\n"), "text")).toBe("a\r\nb\r\n");
    expect(textAs(blob("a\nb\n"), "stored")).toBe("a\nb\n");
  });

  it("leaves a blob `auto` writes as it is unchanged: a CR in it, a NUL, or no line break", () => {
    expect(textAs(blob("a\r\nb\n"), "auto")).toBe("a\r\nb\n");
    expect(textAs(blob("a\0\nb\n"), "auto")).toBe("a\0\nb\n");
    expect(textAs(blob("abc"), "auto")).toBe("abc");
  });

  it("gives each LF without its CR a CR in the `text` form, whatever else the blob holds", () => {
    expect(textAs(blob("a\r\nb\n"), "text")).toBe("a\r\nb\r\n");
    expect(textAs(blob("a\rb\0\n"), "text")).toBe("a\rb\0\r\n");
  });

  it("decodes the blob as UTF-8", () => {
    expect(textAs(Buffer.from("é\n", "utf8"), "auto")).toBe("é\r\n");
  });
});

describe("the ids of the blobs a file's text can come from", () => {
  // `git hash-object` of these bytes: SHA-1, and SHA-256 in a repository made with `--object-format=sha256`.
  const HELLO_LF_SHA1 = "ce013625030ba8dba906f756967f9e9ca394464a";
  const HELLO_CRLF_SHA1 = "ef0493b275aa2080237f676d2ef6559246f56636";
  const HELLO_LF_SHA256 = "2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4";

  it("names the text's own blob in the repository's object format", () => {
    expect(blobIds("hello\n", "sha1", undefined)).toEqual([HELLO_LF_SHA1]);
    expect(blobIds("hello\n", "sha256", undefined)).toEqual([HELLO_LF_SHA256]);
  });

  it("adds the LF blob a checkout writes as CRLF text, where the repository has a checkout write CRLF", () => {
    expect(blobIds("hello\r\n", "sha1", "auto")).toEqual([HELLO_CRLF_SHA1, HELLO_LF_SHA1]);
    expect(blobIds("hello\r\n", "sha1", "text")).toEqual([HELLO_CRLF_SHA1, HELLO_LF_SHA1]);
    expect(blobIds("hello\r\n", "sha1", undefined)).toEqual([HELLO_CRLF_SHA1]);
  });

  it("names one blob for text with no CRLF in it", () => {
    expect(blobIds("hello\n", "sha1", "auto")).toEqual([HELLO_LF_SHA1]);
    expect(blobIds("hello", "sha1", "text")).toHaveLength(1);
  });

  it("names one blob for text no checkout writes: an LF without its CR, or under `auto` a lone CR or a NUL", () => {
    for (const rule of ["auto", "text"] as const) expect(blobIds("hello\r\nworld\n", "sha1", rule)).toHaveLength(1);
    expect(blobIds("hello\rworld\r\n", "sha1", "auto")).toHaveLength(1);
    expect(blobIds("hello\0\r\n", "sha1", "auto")).toHaveLength(1);
  });

  it("adds the LF blob `text` writes as text with a lone CR or a NUL in it", () => {
    expect(blobIds("hello\rworld\r\n", "sha1", "text")).toHaveLength(2);
    expect(blobIds("hello\0\r\n", "sha1", "text")).toHaveLength(2);
  });
});

describe("each form against the bytes Git's own checkout writes", () => {
  // Blobs that tell Git's rules apart: what `auto` leaves as it is (a CR, a NUL, too many control bytes for its text)
  // and `text` still converts, and the bytes Git's binary test counts as text.
  const BLOBS: Readonly<Record<string, string>> = {
    "lf.txt": "a\nb\n",
    "crlf.txt": "a\r\nb\r\n",
    "mixed.txt": "a\r\nb\n",
    "cr.txt": "a\rb\n",
    "zero-byte.txt": "a\0\nb\n",
    "control.txt": "a\x01\nb\n",
    "long-control.txt": `${"x".repeat(128)}\x01\n`,
    "delete.txt": "a\x7f\n",
    "escape.txt": "\x1b[1ma\x1b[0m\n",
    "end-of-file.txt": "a\nb\n\x1a",
    "none.txt": "abc",
    "utf8.txt": "é\n",
  };
  const paths = Object.keys(BLOBS);
  let root = "";
  const ids = new Map<string, string>();
  const git = (...args: string[]): string => execFileSync("git", args, { cwd: root, encoding: "utf8" });

  beforeAll(async () => {
    root = await mkdtemp(join(tmpdir(), "rerail-line-endings-"));
    git("init", "--quiet");
    // Stored as written: no attribute and core.autocrlf=false convert nothing on add.
    git("config", "core.autocrlf", "false");
    for (const [path, text] of Object.entries(BLOBS)) await writeFile(join(root, path), text, "utf8");
    git("add", ".");
    git("-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "blobs");
    // `<mode> blob <id>\t<path>`, NUL-terminated.
    for (const entry of git("ls-tree", "-z", "HEAD").split("\0").filter(line => line !== "")) {
      const [meta, path] = entry.split("\t");
      ids.set(path!, meta!.split(" ")[2]!);
    }
  });
  afterAll(async () => {
    await removeFolder(root);
  });

  it.each<{ attributes: string; autocrlf: string; eol: string; rule: CrlfRule | undefined }>([
    { attributes: "# none", autocrlf: "true", eol: "native", rule: "auto" },
    { attributes: "* text=auto", autocrlf: "false", eol: "crlf", rule: "auto" },
    { attributes: "* text=auto eol=crlf", autocrlf: "false", eol: "lf", rule: "auto" },
    { attributes: "* eol=crlf", autocrlf: "false", eol: "lf", rule: "text" },
    { attributes: "* text", autocrlf: "false", eol: "crlf", rule: "text" },
    { attributes: "* text", autocrlf: "true", eol: "lf", rule: "text" },
    // The `crlf` attribute Git still reads where no `text` attribute is set.
    { attributes: "* crlf", autocrlf: "true", eol: "native", rule: "text" },
    // core.eol=native writes CRLF on Windows alone.
    { attributes: "* text", autocrlf: "false", eol: "native", rule: process.platform === "win32" ? "text" : undefined },
    { attributes: "# none", autocrlf: "false", eol: "crlf", rule: undefined },
    { attributes: "* -text", autocrlf: "true", eol: "crlf", rule: undefined },
    { attributes: "* text eol=lf", autocrlf: "true", eol: "crlf", rule: undefined },
    { attributes: "* text=auto eol=lf", autocrlf: "true", eol: "crlf", rule: undefined },
    { attributes: "* text", autocrlf: "input", eol: "crlf", rule: undefined },
    { attributes: "* crlf=input", autocrlf: "true", eol: "crlf", rule: undefined },
    { attributes: "* -crlf", autocrlf: "true", eol: "crlf", rule: undefined },
  ])("names the rule of `$attributes` under core.autocrlf=$autocrlf and core.eol=$eol, and writes what Git writes", async ({ attributes, autocrlf, eol, rule }) => {
    git("config", "core.autocrlf", autocrlf);
    git("config", "core.eol", eol);
    await writeFile(join(root, ".git", "info", "attributes"), `${attributes}\n`);
    await Promise.all(paths.map(path => rm(join(root, path))));
    git("checkout", "--", ".");
    const rules = await new GitReads().crlfPaths(root, paths);
    for (const [path, text] of Object.entries(BLOBS)) {
      const stored = Buffer.from(text, "utf8");
      const written = await readFile(join(root, path));
      expect(rules.get(path), path).toBe(rule);
      expect(textAs(stored, rule ?? "stored"), path).toBe(written.toString("utf8"));
      expect(heldAs(stored, digestOf(written), rule), path).toBe(written.equals(stored) ? "stored" : rule);
      // Of the blobs `text` writes as CRLF text, blobIds names the text's own and the all-LF one: not one that holds
      // a CRLF beside an LF, which `text` writes as it writes the all-LF one.
      if (rule === "text" && path === "mixed.txt") continue;
      expect(blobIds(written.toString("utf8"), "sha1", rule), path).toContain(ids.get(path));
    }
  });
});
