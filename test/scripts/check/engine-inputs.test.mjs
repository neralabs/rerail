// scripts/check/ci/engine-inputs.mjs: the files the release engine is built from, whose hash names the engine artifact a
// later build reuses (.github/actions/engine-artifact). Markdown the build never reads is left out, so a README change
// reuses the engine; Markdown a source embeds, and every file under native/src/ (hashed into the build id), stay in.

import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { engineInputs, unreadDoc } from "../../../scripts/check/ci/engine-inputs.mjs";
import { removeFolderSync } from "../../support/platform/remove-folder.ts";

const ROOT = fileURLToPath(new URL("../../..", import.meta.url));

/** Git with none of an enclosing hook's index or directory. */
const ENV = { ...process.env };
for (const key of ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE"]) delete ENV[key];
const git = (cwd, ...args) => execFileSync("git", args, { cwd, env: ENV, encoding: "utf8" });

/** The paths of engineInputs' file entries (its last two are the locked tool versions). */
const paths = (root) => engineInputs(root).slice(0, -2).map((entry) => entry.slice(entry.indexOf(" ") + 1));

const roots = [];
afterEach(() => {
  for (const root of roots.splice(0)) removeFolderSync(root);
});

/** A repository whose index holds `files` (path to content). */
function repository(files) {
  const root = mkdtempSync(join(tmpdir(), "rerail-engine-inputs-"));
  roots.push(root);
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(join(root, dirname(path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  git(root, "init", "--quiet");
  git(root, "add", "--all");
  return root;
}

describe("unreadDoc", () => {
  it("is Markdown outside native/src/", () => {
    expect(unreadDoc("native/README.md")).toBe(true);
    expect(unreadDoc("native/vendor/simdjson-5.0.5/README.md")).toBe(true);
    expect(unreadDoc("native/grammars/tree-sitter-sql/PATCHES.md")).toBe(true);
    expect(unreadDoc("languages/README.md")).toBe(true);
    expect(unreadDoc("accel/README.md")).toBe(true);
  });

  it("is not a note under native/src/, which the build id hashes, nor a file of another kind", () => {
    expect(unreadDoc("native/src/code_rule_dsl/semver/UPSTREAM.md")).toBe(false);
    expect(unreadDoc("native/build.rs")).toBe(false);
    expect(unreadDoc("languages/python.json")).toBe(false);
    expect(unreadDoc("native/grammars/tree-sitter-vue/LICENSE")).toBe(false);
  });
});

describe("engineInputs", () => {
  it("hashes the engine's files and what its sources embed, leaving out the Markdown the build never reads", () => {
    const root = repository({
      "Cargo.lock": "",
      "package-lock.json": JSON.stringify({ packages: { "node_modules/@napi-rs/cli": { version: "3.0.0" } } }),
      "accel/README.md": "",
      "accel/build.rs": "",
      "languages/README.md": "",
      "languages/python.json": "{}",
      "native/README.md": "",
      "native/src/lib.rs": 'const RULES: &str = include_str!("../../packs/p/rules.rerail");\n',
      "native/src/code_rule_dsl/semver/UPSTREAM.md": "",
      "native/vendor/v/PATCHES.md": "",
      "native/vendor/v/README.md": "",
      "native/vendor/v/lib.rs": '#![doc = include_str!("./README.md")]\n',
      "packs/p/README.md": "",
      "packs/p/rules.rerail": "",
      "src/index.ts": "",
    });
    expect(paths(root)).toEqual([
      "Cargo.lock",
      "accel/build.rs",
      "languages/python.json",
      "native/src/code_rule_dsl/semver/UPSTREAM.md",
      "native/src/lib.rs",
      "native/vendor/v/README.md",
      "native/vendor/v/lib.rs",
      "packs/p/rules.rerail",
    ]);
    expect(engineInputs(root).slice(-2)).toEqual(["3.0.0 node_modules/@napi-rs/cli", "absent node_modules/@napi-rs/cross-toolchain"]);
  });

  it("leaves this repository's READMEs beside the crates out, and keeps the vendored README tree-sitter embeds", () => {
    const inputs = paths(ROOT);
    expect(inputs).not.toContain("native/README.md");
    expect(inputs).not.toContain("languages/README.md");
    expect(inputs).toContain("native/vendor/tree-sitter-0.27.1/binding_rust/README.md");
    expect(inputs).toContain("native/src/code_rule_dsl/semver/UPSTREAM.md");
    expect(inputs).toContain("native/build.rs");
  });
});
