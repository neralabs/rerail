// scripts/check/ci/rust-selection.mjs: module trees resolved as rustc resolves them, and the selection from a change.
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, sep } from "node:path";
import { afterAll, expect, it } from "vitest";
import { cargoInputs, cliArguments, dataMentions, moduleFiles, outputs, select } from "../../../scripts/check/ci/rust-selection.mjs";
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

it("a crate joining its directory up to the repository root reads every data root; one naming a subdirectory does not", () => {
  const meta = metadata();
  const walk = (up) => `fn corpus() { let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("${up}"); }\n`;
  write({
    "native/crates/low/src/lib.rs": walk("../../.."),
    "native/crates/other/src/lib.rs": 'fn f() { root.join("../../../packs/python"); root.join("../.."); }\n',
  });
  expect([...dataMentions(walk("../../.."))[".."]]).toEqual(["../../.."]);
  expect([...dataMentions('concat!(env!("CARGO_MANIFEST_DIR"), "/../..")')[".."]]).toEqual(["../.."]);
  for (const path of ["packs/complexity/rules/x.rerail", "docs/rule-syntax/examples/x.rerail", "fixtures/php/a.php"]) {
    expect(select(root, meta, [path]).packages, path).toEqual(["low", "mid", "rerail-engine"]);
  }
  expect(select(root, meta, ["packs/python/rules/x.rerail"]).packages).toEqual(["low", "mid", "other", "rerail-engine"]);
  // `../..` from native/crates/low reaches native/, not the root.
  write({ "native/crates/low/src/lib.rs": walk("../..") });
  expect(select(root, meta, ["packs/complexity/rules/x.rerail"]).packages).toEqual([]);
  write({ "native/crates/low/src/lib.rs": "", "native/crates/other/src/lib.rs": "" });
});

it("a `..` compared or matched is a path component, not a walk to the repository root", () => {
  const meta = metadata();
  meta.packages.push({ name: "top", manifest_path: join(root, "top", "Cargo.toml"), targets: [{ src_path: join(root, "top", "src", "lib.rs") }], dependencies: [] });
  const components = [
    "fn safe(path: &str) -> bool { !path.split('/').any(|c| c == \"..\" || c.is_empty()) }",
    'fn arm(c: &str) -> u8 { match c { ".." => 1, _ => 0 } }',
    'fn back(c: &str) -> bool { ".." != c }',
  ].join("\n");
  write({ "top/src/lib.rs": components });
  expect(dataMentions(components)[".."]).toBeUndefined();
  expect(select(root, meta, ["fixtures/php/a.php"]).packages).toEqual([]);
  // The same crate joining its directory to `..` reaches the root, one level up.
  write({ "top/src/lib.rs": 'fn corpus() { PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."); }\n' });
  expect(select(root, meta, ["fixtures/php/a.php"]).packages).toEqual(["top"]);
  write({ "top/src/lib.rs": "" });
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

it("cliArguments: the list comes first as ci.yml passes it, with or without the base's package.json", () => {
  expect(cliArguments(["changed.txt", "--github"])).toEqual({ list: "changed.txt", basePackageJson: undefined, github: true });
  expect(cliArguments(["changed.txt", "--base-package-json", "base.json", "--github"])).toEqual({
    list: "changed.txt",
    basePackageJson: "base.json",
    github: true,
  });
  expect(cliArguments(["--base-package-json", "base.json", "changed.txt"])).toEqual({ list: "changed.txt", basePackageJson: "base.json", github: false });
});
