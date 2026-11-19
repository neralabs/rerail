// scripts/check/ci/engine-inputs.mjs: the files the release engine is built from, whose hash names the engine artifact a
// later build reuses (.github/actions/engine-artifact) and whose change selects the tests that load the engine
// (test-selection.mjs). A crate the engine reaches by a cargo `path` is an input wherever it lives, so a change to it
// never reuses an engine built without it.

import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, describe, expect, it } from "vitest";
import { engineInputs, engineSources, isEngineInput } from "../../../scripts/check/ci/engine-inputs.mjs";
import { removeFolderSync } from "../../support/platform/remove-folder.ts";

const ROOT = fileURLToPath(new URL("../../..", import.meta.url));

/** Git with none of an enclosing hook's index or directory. */
const ENV = { ...process.env };
for (const key of ["GIT_DIR", "GIT_INDEX_FILE", "GIT_WORK_TREE"]) delete ENV[key];
const git = (cwd, ...args) => execFileSync("git", args, { cwd, env: ENV, encoding: "utf8" });

const roots = [];
afterEach(() => {
  for (const root of roots.splice(0)) removeFolderSync(root);
});

const NAPI = { binaryName: "rerail_engine", targets: ["x86_64-unknown-linux-gnu"] };

/** A repository whose index holds `files` (path to content), with the package files every engine hash reads. */
function repository(files) {
  const root = mkdtempSync(join(tmpdir(), "rerail-engine-inputs-"));
  roots.push(root);
  const all = {
    "package.json": JSON.stringify({ name: "x", napi: NAPI }),
    "package-lock.json": JSON.stringify({ packages: { "node_modules/@napi-rs/cli": { version: "3.0.0" } } }),
    ...files,
  };
  for (const [path, content] of Object.entries(all)) {
    mkdirSync(join(root, dirname(path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  git(root, "init", "--quiet");
  git(root, "add", "--all");
  return root;
}

/** The engine's crate reaching accel/, which reaches forest/, and a patched crate the workspace manifest names. */
const CRATES = {
  "Cargo.toml": '[workspace]\nmembers = ["native", "accel", "forest", "unused"]\n\n[patch.crates-io]\nx = { path = "patched/x" }\n',
  "native/Cargo.toml": '[package]\nname = "e"\n\n[dependencies]\nrerail-accel = { path = "../accel" }\n',
  "native/src/lib.rs": 'const CORE: &str = include_str!("../../docs/core.rerail");\n',
  "accel/Cargo.toml": '[lib]\npath = "src/lib.rs"\n\n[dependencies]\nrerail-forest = { path = "../forest", default-features = false }\n',
  "accel/src/lib.rs": "",
  "forest/Cargo.toml": '[package]\nname = "forest"\n',
  "forest/src/lib.rs": "",
  "patched/x/Cargo.toml": '[package]\nname = "x"\n',
  "patched/x/src/lib.rs": "",
  "unused/Cargo.toml": '[package]\nname = "unused"\n',
  "unused/src/lib.rs": "",
  "docs/core.rerail": "",
  "docs/README.md": "",
  "languages/python.json": "{}",
  "src/index.ts": "",
};

describe("engineSources", () => {
  it("reaches every crate a manifest names by path, transitively, and no crate nothing reaches", () => {
    const { dirs, paths } = engineSources(repository(CRATES));
    expect(dirs).toEqual(["accel/", "forest/", "languages/", "native/", "patched/x/"]);
    expect([...paths].sort()).toEqual([
      "Cargo.toml",
      "accel/Cargo.toml",
      "accel/src/lib.rs",
      "docs/core.rerail",
      "forest/Cargo.toml",
      "forest/src/lib.rs",
      "languages/python.json",
      "native/Cargo.toml",
      "native/src/lib.rs",
      "patched/x/Cargo.toml",
      "patched/x/src/lib.rs",
    ]);
  });

  it("reads a path in single quotes or with a trailing slash as the same crate", () => {
    const { dirs } = engineSources(
      repository({
        ...CRATES,
        "native/Cargo.toml": "[package]\nname = \"e\"\n\n[dependencies]\nrerail-accel = { path = '../accel' }\nx = { path = \"../unused/\" }\n",
      }),
    );
    expect(dirs).toEqual(["accel/", "forest/", "languages/", "native/", "patched/x/", "unused/"]);
  });

  it("covers this repository's engine crates outside native/", () => {
    const { dirs } = engineSources(ROOT);
    for (const dir of ["accel/", "forest/", "tools/bitlex/", "languages/", "native/"]) expect(dirs).toContain(dir);
  });
});

describe("isEngineInput", () => {
  const sources = engineSources(repository(CRATES));

  it("is a file of a reached crate, tracked or deleted, a single input file, or an embedded file", () => {
    expect(isEngineInput("forest/src/lib.rs", sources)).toBe(true);
    expect(isEngineInput("forest/src/deleted.rs", sources)).toBe(true);
    expect(isEngineInput("rust-toolchain.toml", sources)).toBe(true);
    expect(isEngineInput("docs/core.rerail", sources)).toBe(true);
  });

  it("is not a file the engine is not built from", () => {
    expect(isEngineInput("unused/src/lib.rs", sources)).toBe(false);
    expect(isEngineInput("docs/README.md", sources)).toBe(false);
    expect(isEngineInput("src/index.ts", sources)).toBe(false);
  });
});

describe("engineInputs", () => {
  it("changes with a reached crate's file and package.json's napi block, not with a file nothing reaches", () => {
    const hashOf = (files) => engineInputs(repository({ ...CRATES, ...files })).join("\n");
    const base = hashOf({});
    expect(hashOf({ "forest/src/lib.rs": "pub fn f() {}" })).not.toBe(base);
    expect(hashOf({ "package.json": JSON.stringify({ name: "x", napi: { ...NAPI, binaryName: "other" } }) })).not.toBe(base);
    expect(hashOf({ "unused/src/lib.rs": "pub fn f() {}" })).toBe(base);
    expect(hashOf({ "package.json": JSON.stringify({ name: "y", napi: NAPI }) })).toBe(base);
  });
});
