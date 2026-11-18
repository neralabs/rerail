// scripts/check/rust-selection.mjs: module trees resolved as rustc resolves them, and the selection from a change.
import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { after, test } from "node:test";

import { moduleFiles, outputs, select } from "../../scripts/check/rust-selection.mjs";

const root = mkdtempSync(join(tmpdir(), "rust-selection-"));
after(() => rmSync(root, { recursive: true, force: true }));

function write(files) {
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  }
}

const tree = (rootFile) => [...moduleFiles(join(root, rootFile))].map((file) => relative(root, file)).sort();

test("a module tree follows mod, #[path], cfg_attr paths, inline modules and includes", () => {
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
  assert.deepEqual(tree("a/src/lib.rs"), [
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

test("a non-mod-rs file's #[path] inline module is relative to its directory, not its offset", () => {
  write({
    "b/src/lib.rs": "mod waves;\n",
    "b/src/waves.rs": '#[path = "waves/schedule"] mod schedule { mod batches; }\nmod named { mod x; }\n',
    "b/src/waves/schedule/batches.rs": "",
    "b/src/waves/named/x.rs": "",
  });
  assert.deepEqual(tree("b/src/lib.rs"), ["b/src/lib.rs", "b/src/waves.rs", "b/src/waves/named/x.rs", "b/src/waves/schedule/batches.rs"]);
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

test("a change selects the crates compiling it and every crate above them", () => {
  const meta = metadata();
  assert.deepEqual(select(root, meta, ["native/crates/low/src/lib.rs"]).packages, ["low", "mid", "rerail-engine"]);
  assert.deepEqual(select(root, meta, ["native/src/m.rs"]).packages, ["mid", "rerail-engine"]);
  assert.deepEqual(select(root, meta, ["native/crates/other/README.md"]).packages, ["other"]);
  assert.deepEqual(select(root, meta, ["src/cli/cli.ts", "docs/x.md"]).packages, []);
});

test("the build's inputs and unclaimed engine files select everything", () => {
  const meta = metadata();
  for (const path of ["Cargo.lock", ".cargo/config.toml", "native/vendor/x/src/lib.rs", "languages/python.json", "fixtures/a/b.json", ".github/workflows/ci.yml"]) {
    assert.equal(select(root, meta, [path]).all, true, path);
  }
  assert.equal(select(root, meta, ["native/src/data.json"]).all, true);
});

test("outputs: cargo arguments and a nextest filter, everything as --workspace", () => {
  assert.deepEqual(outputs({ all: false, packages: ["a", "rerail-engine"] }), {
    "rust-all": "false",
    "rust-packages": '["a","rerail-engine"]',
    "rust-cargo-args": "-p a -p rerail-engine",
    "rust-nextest-filter": "package(a) | package(rerail-engine)",
    "rust-engine": "true",
  });
  assert.equal(outputs({ all: true, packages: ["a"] })["rust-cargo-args"], "--workspace");
  assert.equal(outputs({ all: false, packages: [] })["rust-nextest-filter"], "none()");
});
