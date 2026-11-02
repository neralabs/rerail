// scripts/build/require-lfs.mjs: build:native stops on a Git LFS pointer in place of an LFS-tracked file.

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { lfsPointerFiles } from "../../scripts/build/require-lfs.mjs";

const POINTER = "version https://git-lfs.github.com/spec/v1\noid sha256:4d7a\nsize 1301504\n";
const roots = [];

function checkout(files) {
  const root = mkdtempSync(join(tmpdir(), "rerail-require-lfs-"));
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

describe("the build's Git LFS preflight", () => {
  const attributes = [
    "model/model.safetensors filter=lfs diff=lfs merge=lfs -text",
    "native/src/code_rule_dsl/value/names/names.bin filter=lfs diff=lfs merge=lfs -text",
    "*.bin.gz filter=lfs",
  ].join("\n");

  it("names each LFS-tracked file that holds its pointer", () => {
    const root = checkout({
      ".gitattributes": attributes,
      "model/model.safetensors": POINTER,
      "native/src/code_rule_dsl/value/names/names.bin": POINTER,
      "deep/er/table.bin.gz": POINTER,
    });
    expect(lfsPointerFiles(root)).toEqual(["deep/er/table.bin.gz", "model/model.safetensors", "native/src/code_rule_dsl/value/names/names.bin"]);
  });

  it("passes the real files, and pointer text outside LFS paths", () => {
    const root = checkout({
      ".gitattributes": attributes,
      "model/model.safetensors": Buffer.alloc(4096),
      "native/src/code_rule_dsl/value/names/names.bin": Buffer.concat([Buffer.from("TNAMES01"), Buffer.alloc(2048)]),
      "docs/lfs.md": POINTER,
    });
    expect(lfsPointerFiles(root)).toEqual([]);
  });
});
