import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { PACKAGE_ROOT } from "../../../src/engine/index.js";

/**
 * The model's manifest pins each file's SHA-256, and the engine refuses to open a model whose bytes differ: every
 * session event then fails and nothing is recorded. Git for Windows converts text files to CRLF on checkout by default
 * (core.autocrlf=true), which changes `config.json`'s bytes, so every file the manifest pins must be checked out as is.
 */
describe("the bundled model in a checkout", () => {
  it("has every file its manifest checksums marked -text, so no checkout rewrites its line ends", () => {
    const manifest = JSON.parse(readFileSync(join(PACKAGE_ROOT, "model", "manifest.json"), "utf8")) as { files: Record<string, unknown> };
    const paths = Object.keys(manifest.files).map(file => `model/${file}`);
    const output = execFileSync("git", ["check-attr", "text", "--", ...paths], { cwd: PACKAGE_ROOT, encoding: "utf8" });
    const textAttribute = Object.fromEntries(output.trim().split(/\r?\n/u).map(line => {
      const [path, , value] = line.split(": ");
      return [path, value];
    }));
    expect(textAttribute).toEqual(Object.fromEntries(paths.map(path => [path, "unset"])));
  });
});
