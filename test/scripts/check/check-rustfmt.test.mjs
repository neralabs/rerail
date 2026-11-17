// scripts/check/check-rustfmt.mjs: the Rust files scripts/check/rustfmt-files.txt lists are formatted as rustfmt lays
// them out, each checked alone.

import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { LIST, listedFiles, rustfmtProblems } from "../../../scripts/check/check-rustfmt.mjs";
import { removeFolderSync } from "../../support/platform/remove-folder.ts";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..", "..");

let root;
afterEach(() => {
  if (root !== undefined) removeFolderSync(root);
  root = undefined;
});

/** A folder holding `files` (path to content). */
function folder(files) {
  root = mkdtempSync(join(tmpdir(), "rerail-rustfmt-"));
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  return root;
}

/** A stand-in for rustfmt: trims the spaces that end a line. */
const trimLines = (source) => source.replace(/ +\n/gu, "\n");

describe("check-rustfmt", () => {
  it("reads a path per line, skipping blank lines and comments", () => {
    expect(listedFiles("# The files.\nnative/src/a.rs\n\n  native/src/b.rs  \n# More.\n")).toEqual(["native/src/a.rs", "native/src/b.rs"]);
  });

  it("flags a listed file that is not formatted and one that is gone, and passes a formatted one", () => {
    const at = folder({ "src/clean.rs": "fn a() {}\n", "src/drifted.rs": "fn b() {} \n" });
    const problems = rustfmtProblems(at, ["src/clean.rs", "src/drifted.rs", "src/gone.rs"], trimLines);
    expect(problems.map(({ path, problem }) => [path, problem])).toEqual([
      ["src/drifted.rs", "is not formatted: node scripts/check/check-rustfmt.mjs --write"],
      ["src/gone.rs", `is listed in ${LIST} but gone: take it off the list`],
    ]);
    expect(problems[0].formatted).toBe("fn b() {}\n");
  });

  it("lists only Rust files this checkout has, each once", () => {
    const files = listedFiles(readFileSync(join(ROOT, LIST), "utf8"));
    expect(files.length).toBeGreaterThan(0);
    expect(files.filter((path) => !path.endsWith(".rs"))).toEqual([]);
    expect(new Set(files).size).toBe(files.length);
    expect(rustfmtProblems(ROOT, files, (source) => source)).toEqual([]);
  });
});
