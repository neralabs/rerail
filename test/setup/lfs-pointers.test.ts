import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { isLfsPointer, lfsPointerFiles, LFS_REMEDY, requireLfsContent } from "../../src/setup/lfs-pointers.js";

const POINTER = "version https://git-lfs.github.com/spec/v1\noid sha256:a4264cfe4354253b9332731aa3db0c066f302c8507ebd791ba1b10f64efc8eb5\nsize 7559256\n";
const ATTRIBUTES = [
  "# Binaries are stored in Git LFS.",
  "model/model.safetensors filter=lfs diff=lfs merge=lfs -text",
  "native/src/code_rule_dsl/value/names/names.bin filter=lfs diff=lfs merge=lfs -text",
  "native/grammars/*/src/parser.generated.c.gz filter=lfs diff=lfs merge=lfs -text",
  "packs/*/upstream/**/*.expected.json linguist-generated=true",
].join("\n");

const roots: string[] = [];

function packageRoot(files: Record<string, string | Buffer>): string {
  const root = mkdtempSync(join(tmpdir(), "rerail-lfs-"));
  roots.push(root);
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  return root;
}

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

describe("Git LFS pointers in a package root", () => {
  it("tells a pointer from the file it stands for", () => {
    const root = packageRoot({ pointer: POINTER, real: Buffer.concat([Buffer.from("TNAMES01"), Buffer.alloc(4096)]), small: "{}" });
    expect(isLfsPointer(join(root, "pointer"))).toBe(true);
    expect(isLfsPointer(join(root, "real"))).toBe(false);
    expect(isLfsPointer(join(root, "small"))).toBe(false);
    expect(isLfsPointer(join(root, "absent"))).toBe(false);
  });

  it("finds every LFS path .gitattributes lists that holds a pointer, and none of the real files", () => {
    const root = packageRoot({
      ".gitattributes": ATTRIBUTES,
      "model/model.safetensors": POINTER,
      "native/src/code_rule_dsl/value/names/names.bin": Buffer.concat([Buffer.from("TNAMES01"), Buffer.alloc(2048)]),
      "native/grammars/tree-sitter-sql/src/parser.generated.c.gz": POINTER,
      "packs/x/upstream/a/case.expected.json": POINTER,
    });
    expect(lfsPointerFiles(root)).toEqual(["model/model.safetensors", "native/grammars/tree-sitter-sql/src/parser.generated.c.gz"]);
  });

  it("checks the shipped model in a package that carries no .gitattributes", () => {
    expect(lfsPointerFiles(packageRoot({ "model/model.safetensors": POINTER }))).toEqual(["model/model.safetensors"]);
    expect(lfsPointerFiles(packageRoot({ "model/model.safetensors": Buffer.alloc(4096) }))).toEqual([]);
  });

  it("refuses a root holding pointers, naming each file and how to fetch it", () => {
    const root = packageRoot({ ".gitattributes": ATTRIBUTES, "model/model.safetensors": POINTER, "native/src/code_rule_dsl/value/names/names.bin": POINTER });
    expect(() => requireLfsContent(root)).toThrow(/model\/model\.safetensors.*names\.bin/u);
    expect(() => requireLfsContent(root)).toThrow(LFS_REMEDY);
    expect(() => requireLfsContent(packageRoot({ "model/model.safetensors": Buffer.alloc(4096) }))).not.toThrow();
  });
});
