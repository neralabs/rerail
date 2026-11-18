// scripts/check/ci/rust-selection.mjs: module trees resolved as rustc resolves them, and the selection from a change.
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, sep } from "node:path";
import { afterAll, expect, it } from "vitest";
import { cargoInputs, dataMentions, moduleFiles, outputs, select } from "../../../scripts/check/ci/rust-selection.mjs";
import { removeFolderSync } from "../../support/platform/remove-folder.ts";

const root = mkdtempSync(join(tmpdir(), "rust-selection-"));
afterAll(() => removeFolderSync(root));

function write(files) {
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  }
}

/** A tree's files relative to the root with `/`, as git lists changed files and `select` compares them. */
const tree = (rootFile) => [...moduleFiles(join(root, rootFile))].map((file) => relative(root, file).split(sep).join("/")).sort();

it("a module tree follows mod, #[path], cfg_attr paths, inline modules and includes", () => {
  write({
    "a/src/lib.rs": [
      "// mod commented_out;",
      'const S: &str = "mod in_a_string;";',
      "mod plain;",
      '#[path = "../../shared/src"] pub mod shared { pub mod deep; #[path = "inner"] mod inner { mod leaf; } }',
      '#[cfg_attr(test, path = "alt.rs")] mod either;',
      'static T: &str = include_str!("table.txt");',
    ].join("\n"),
    "a/src/plain.rs": "mod child;\nmod tests { #[path = \"t.rs\"] mod t; }\n",
    "a/src/plain/child.rs": "",
    "a/src/plain/tests/t.rs": "",
    "a/src/either.rs": "",
    "a/src/alt.rs": "",
    "a/src/table.txt": "",
    "shared/src/deep.rs": "",
    "shared/src/inner/leaf.rs": "",
    "shared/src/orphan.rs": "",
  });
  expect(tree("a/src/lib.rs")).toEqual([
    "a/src/alt.rs",
    "a/src/either.rs",
    "a/src/lib.rs",
    "a/src/plain.rs",
    "a/src/plain/child.rs",
    "a/src/plain/tests/t.rs",
    "a/src/table.txt",
    "shared/src/deep.rs",
    "shared/src/inner/leaf.rs",
  ]);
});

it("a non-mod-rs file's #[path] inline module is relative to its directory, not its offset", () => {
  write({
    "b/src/lib.rs": "mod waves;\n",
    "b/src/waves.rs": '#[path = "waves/schedule"] mod schedule { mod batches; }\nmod named { mod x; }\n',
    "b/src/waves/schedule/batches.rs": "",
    "b/src/waves/named/x.rs": "",
  });
  expect(tree("b/src/lib.rs")).toEqual(["b/src/lib.rs", "b/src/waves.rs", "b/src/waves/named/x.rs", "b/src/waves/schedule/batches.rs"]);
});

// Packages: low <- mid <- top (top dev-depends on mid); other stands alone. mid mounts native/src/m.rs.
function metadata() {
  write({
    "native/crates/low/src/lib.rs": "",
    "native/crates/mid/src/lib.rs": '#[path = "../../../src/m.rs"] mod m;\n',
    "native/src/m.rs": "",
    "native/src/lib.rs": "",
    "native/crates/other/src/lib.rs": "",
  });
  const pkg = (name, dir, deps = []) => ({
    name,
    manifest_path: join(root, dir, "Cargo.toml"),
    targets: [{ src_path: join(root, dir, "src", "lib.rs") }],
    dependencies: deps.map(([dep, kind]) => ({ name: dep, kind, path: join(root, `native/crates/${dep}`) })),
  });
  return {
    packages: [
      pkg("low", "native/crates/low"),
      pkg("mid", "native/crates/mid", [["low", null]]),
      pkg("rerail-engine", "native", [["mid", "dev"]]),
      pkg("other", "native/crates/other"),
    ],
  };
}

it("a change selects the crates compiling it and every crate above them", () => {
  const meta = metadata();
  expect(select(root, meta, ["native/crates/low/src/lib.rs"]).packages).toEqual(["low", "mid", "rerail-engine"]);
  expect(select(root, meta, ["native/src/m.rs"]).packages).toEqual(["mid", "rerail-engine"]);
  expect(select(root, meta, ["native/crates/other/README.md"]).packages).toEqual(["other"]);
  expect(select(root, meta, ["src/cli/cli.ts", "docs/x.md"]).packages).toEqual([]);
});

it("the build's inputs and unclaimed engine files select everything", () => {
  const meta = metadata();
  for (const path of ["Cargo.lock", ".cargo/config.toml", "native/vendor/x/src/lib.rs", "package.json", ".github/workflows/ci.yml"]) {
    expect(select(root, meta, [path]).all, path).toBe(true);
  }
  expect(select(root, meta, ["native/src/data.json"]).all).toBe(true);
});

it("data names: path literals under a data root, the fixtures' macros, prose left out", () => {
  const found = dataMentions(
    [
      'let a = root.join("packs/python/rules");',
      'let b = repository().join("packs");',
      'let c = Path::new(fixtures_dir!()).join("php/scope/x.php");',
      'let d = oracle!("rust/calls/lib.rs");',
      'let e = fixtures_dir!();',
      'panic!("check out the repository\'s languages/ directory");',
      'let f = r"..\\conformance\\t1";',
    ].join("\n"),
  );
  expect(Object.fromEntries(Object.entries(found).map(([root, subs]) => [root, [...subs].sort()]))).toEqual({
    packs: ["*", "python"],
    fixtures: ["*", "php", "rust"],
    conformance: ["t1"],
  });
});

it("a data file selects the crates naming it, its directory or its root, and every crate above them", () => {
  const meta = metadata();
  write({
    "native/crates/low/src/lib.rs": 'const P: &str = "packs/python/rules";\n',
    "native/crates/other/src/lib.rs": 'fn f() { Path::new(fixtures_dir!()).join(name); }\n',
  });
  expect(select(root, meta, ["packs/python/rules/x.rerail"]).packages).toEqual(["low", "mid", "rerail-engine"]);
  expect(select(root, meta, ["packs/rust/rules/x.rerail"]).packages).toEqual([]);
  expect(select(root, meta, ["packs/README.md"]).packages).toEqual(["low", "mid", "rerail-engine"]);
  expect(select(root, meta, ["fixtures/php/a.php"]).packages).toEqual(["other"]);
  write({ "native/crates/low/src/lib.rs": "", "native/crates/other/src/lib.rs": "" });
});

it("package.json reaches the crates through napi's configuration and the scripts that run cargo", () => {
  const json = (scripts, extra = {}) => JSON.stringify({ scripts, dependencies: extra, napi: { binaryName: "e" } });
  expect(cargoInputs(json({ "test:native": "cargo test", lint: "eslint" }, { a: "1" }))).toBe(cargoInputs(json({ "test:native": "cargo test", lint: "biome" }, { a: "2" })));
  expect(cargoInputs(json({ "test:native": "cargo test" }))).not.toBe(cargoInputs(json({ "test:native": "cargo test --workspace" })));
});

it("outputs: cargo arguments and a nextest filter, everything as --workspace", () => {
  expect(outputs({ all: false, packages: ["a", "rerail-engine"] })).toEqual({
    "rust-all": "false",
    "rust-packages": '["a","rerail-engine"]',
    "rust-cargo-args": "-p a -p rerail-engine",
    "rust-nextest-filter": "package(a) | package(rerail-engine)",
    "rust-engine": "true",
  });
  expect(outputs({ all: true, packages: ["a"] })["rust-cargo-args"]).toBe("--workspace");
  expect(outputs({ all: false, packages: [] })["rust-nextest-filter"]).toBe("none()");
});
