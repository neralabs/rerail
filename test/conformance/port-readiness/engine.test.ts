import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, unlinkSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterAll, describe, test } from "vitest";

import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { engineIdentity, engineKey, engineSources, FAMILY_SOURCES, policyFamilies, staleNativeBinary, withExpando } from "../../../conformance/port-readiness/engine.js";

test("pattern text takes ast-grep's expando exactly where ast-grep's pre_process_pattern puts it", () => {
  assert.equal(withExpando("def f($NAME=$VALUE):", "µ"), "def f(µNAME=µVALUE):");
  assert.equal(withExpando("f($$$ARGS)", "µ"), "f(µµµARGS)");
  assert.equal(withExpando("x = $$$", "µ"), "x = µµµ");
  // A `$` before anything but a capital or an underscore is not a metavariable.
  assert.equal(withExpando("cost = '$5' + $a", "µ"), "cost = '$5' + $a");
  assert.equal(withExpando("$_ + $$B", "µ"), "µ_ + µµB");
});

const tracked = (pathspec: readonly string[]): string[] =>
  execFileSync("git", ["ls-files", "-z", "--", ...pathspec], { cwd: PACKAGE_ROOT, encoding: "utf8", maxBuffer: 1 << 28 }).split("\0").filter((path) => path !== "");

describe("the engine sources a rule's key covers", () => {
  const split = engineSources(PACKAGE_ROOT);
  const keyed = new Set([...split.shared, ...Object.values(split.families).flat()]);

  // Without this, an entry whose path moved or was deleted would quietly claim nothing and the family's rules would
  // key on the shared engine alone, keeping a stale verdict until the nightly judged them again.
  test("every FAMILY_SOURCES entry names engine sources of this checkout", () => {
    for (const [family, entries] of Object.entries(FAMILY_SOURCES)) {
      for (const entry of entries) {
        const matched = split.families[family]!.filter((path) => path === entry || (entry.endsWith("/") && path.startsWith(entry)));
        assert.ok(matched.length > 0, `FAMILY_SOURCES.${family} lists ${entry}, which no engine source matches`);
      }
    }
  });

  test("a family's own grammars, registry entries and language modules are its own", () => {
    assert.deepEqual(split.families.python, [
      "languages/python.json",
      "native/grammars/tree-sitter-python/LICENSE",
      "native/grammars/tree-sitter-python/PATCHES.md",
      "native/grammars/tree-sitter-python/src/grammar.json",
      "native/grammars/tree-sitter-python/src/scanner.c",
      "native/grammars/tree-sitter-python/src/tree_sitter/alloc.h",
      "native/grammars/tree-sitter-python/src/tree_sitter/array.h",
      "native/grammars/tree-sitter-python/src/tree_sitter/parser.h",
      "native/src/code_rule_dsl/scope/data/python_builtins.generated.json",
      "native/src/code_rule_dsl/scope/python.rs",
      "native/src/code_rule_dsl/scope/python/typedefs.rs",
      "native/src/code_rule_dsl/suppress/noqa.rs",
      "native/src/code_rule_dsl/suppress/nosec.rs",
      "native/src/code_rule_dsl/value/python.rs",
      "native/src/code_rule_dsl/value/python/ops.rs",
    ]);
    assert.deepEqual(split.families.rust, [
      "languages/rust.json",
      "native/src/code_rule_dsl/calls/rust.rs",
      "native/src/code_rule_dsl/language/adapter/rust.rs",
      "native/src/code_rule_dsl/literal/rust_format.rs",
      "native/src/code_rule_dsl/matcher/arguments/rust.rs",
      "native/src/code_rule_dsl/matcher/format_args/rust.rs",
      "native/src/code_rule_dsl/scope/rust.rs",
      "native/src/code_rule_dsl/scope/usage/rust.rs",
      "native/src/code_rule_dsl/suppress/clippy.rs",
      "native/src/code_rule_dsl/value/rust.rs",
    ]);
    assert.deepEqual(split.families.sql, [
      "languages/sql.json",
      "native/grammars/tree-sitter-sql/LICENSE",
      "native/grammars/tree-sitter-sql/PATCHES.md",
      "native/grammars/tree-sitter-sql/rerail.patch",
      "native/grammars/tree-sitter-sql/src/grammar.json",
      "native/grammars/tree-sitter-sql/src/scanner.c",
      "native/grammars/tree-sitter-sql/src/tree_sitter/alloc.h",
      "native/grammars/tree-sitter-sql/src/tree_sitter/array.h",
      "native/grammars/tree-sitter-sql/src/tree_sitter/parser.h",
      "native/src/code_rule_dsl/language/adapter/sql.rs",
      "native/src/code_rule_dsl/matcher/arguments_sql.rs",
      "native/src/code_rule_dsl/scope/sql.rs",
      "native/src/code_rule_dsl/suppress/sqlfluff.rs",
      "native/src/code_rule_dsl/value/sql.rs",
    ]);
    for (const path of ["languages/javascript.json", "languages/typescript.json", "native/src/code_rule_dsl/scope/ecma.rs", "native/src/code_rule_dsl/value/ecma/ops.rs"]) {
      assert.ok(split.families.ecma!.includes(path), `${path} is not ecma's`);
    }
    for (const path of ["languages/php.json", "native/src/code_rule_dsl/scope/php.rs", "native/src/code_rule_dsl/scope/php/blade.rs", "native/src/code_rule_dsl/suppress/phpcs.rs", "native/src/code_rule_dsl/php_view.rs", "native/src/code_rule_dsl/blade/forte/scan.rs"]) {
      assert.ok(split.families.php!.includes(path), `${path} is not php's`);
    }
    for (const path of ["languages/blade.json", "native/src/code_rule_dsl/blade.rs", "native/src/code_rule_dsl/blade/view.rs", "native/src/code_rule_dsl/blade/forte/parser/mod.rs", "native/src/code_rule_dsl/embed/compiled_view.rs"]) {
      assert.ok(split.families.blade!.includes(path), `${path} is not blade's`);
    }
  });

  /**
   * The only family files under the directories every language reads: Rust's adapter and two matchers only it calls,
   * and SQL's adapter and the call binding only it reads.
   */
  const FAMILY_UNDER_SHARED: Record<string, string> = {
    "native/src/code_rule_dsl/language/adapter/rust.rs": "rust",
    "native/src/code_rule_dsl/matcher/arguments/rust.rs": "rust",
    "native/src/code_rule_dsl/matcher/format_args/rust.rs": "rust",
    "native/src/code_rule_dsl/language/adapter/sql.rs": "sql",
    "native/src/code_rule_dsl/matcher/arguments_sql.rs": "sql",
  };

  // The engine code every language reads: a change to it can move any verdict, so it must re-judge every rule.
  test("the engine every language reads is shared", () => {
    const shared = new Set(split.shared);
    for (const [path, family] of Object.entries(FAMILY_UNDER_SHARED)) assert.ok(split.families[family]!.includes(path), `${path} is not ${family}'s`);
    for (const prefix of ["native/src/code_rule_dsl/matcher/", "native/src/code_rule_dsl/compile/", "native/src/code_rule_dsl/workspace/", "native/src/code_rule_dsl/language/adapter/"]) {
      const listed = [...keyed].filter((path) => path.startsWith(prefix) && FAMILY_UNDER_SHARED[path] === undefined);
      assert.ok(listed.length > 0, `no engine source under ${prefix}`);
      for (const path of listed) assert.ok(shared.has(path), `${path} is not shared`);
    }
    // The adapter seam, the lockfile, the manifests, the host's TypeScript engine and the language registry's generator
    // are shared too.
    for (const path of ["native/src/code_rule_dsl/language/adapter.rs", "Cargo.lock", "Cargo.toml", "rust-toolchain.toml", "native/build.rs", "native/src/build_languages.rs", "src/engine/index.ts"]) {
      assert.ok(shared.has(path), `${path} is not shared`);
    }
  });

  test("the sources only cargo test compiles are in nobody's key", () => {
    for (const path of [
      "native/src/code_rule_dsl/scope/tests.rs",
      "native/src/code_rule_dsl/value/python/tests.rs",
      "native/src/code_rule_dsl/matcher/tests/mod.rs",
      "native/src/code_rule_dsl/suppress/ruff.rs",
      "native/src/code_rule_dsl/suppress/eslint_tests.rs",
      "native/src/code_rule_dsl/value/eslint_rules.rs",
      "native/src/code_rule_dsl/lines/eslint.rs",
      "native/src/code_rule_dsl/test_support.rs",
    ]) {
      assert.ok(tracked([path]).length === 1, `${path} is not tracked`);
      assert.ok(!keyed.has(path), `${path} is in a rule's key`);
    }
    // The generated tables these two build are the engine's, however `lib.rs` declares them (engine.ts).
    for (const path of ["native/src/build_identity.rs", "native/src/build_languages.rs"]) assert.ok(keyed.has(path), `${path} is in nobody's key`);
  });
});

describe("a rule's engine key", () => {
  const roots: string[] = [];
  afterAll(() => {
    for (const root of roots) rmSync(root, { recursive: true, force: true });
  });

  /** A checkout holding one engine source per family and one shared one, each with the text `files` gives it. */
  function checkout(files: Record<string, string>): string {
    const root = mkdtempSync(join(tmpdir(), "gates-engine-"));
    roots.push(root);
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: root });
    for (const [path, text] of Object.entries(files)) {
      mkdirSync(join(root, dirname(path)), { recursive: true });
      writeFileSync(join(root, path), text);
    }
    execFileSync("git", ["add", "-A", "--", ...Object.keys(files)], { cwd: root });
    return root;
  }

  const sources = {
    "native/src/code_rule_dsl/scope/python.rs": "// python scope\n",
    "native/src/code_rule_dsl/scope/ecma.rs": "// ecma scope\n",
    "native/src/code_rule_dsl/scope/rust.rs": "// rust scope\n",
    "native/src/code_rule_dsl/matcher/mod.rs": "// every language's matcher\n",
  };
  const before = engineIdentity(checkout(sources));

  test("a change to one family's sources moves that family's key and no other", () => {
    const after = engineIdentity(checkout({ ...sources, "native/src/code_rule_dsl/scope/python.rs": "// python scope, fixed\n" }));
    assert.notEqual(after.families.python, before.families.python);
    assert.equal(after.families.ecma, before.families.ecma);
    assert.equal(after.shared, before.shared);
    assert.notEqual(engineKey(after, ["python"]), engineKey(before, ["python"]));
    assert.equal(engineKey(after, ["ecma"]), engineKey(before, ["ecma"]));
    // The whole engine moved, so a rule the gates cannot place in a family is judged again.
    assert.notEqual(after.sha256, before.sha256);
  });

  // No Rust rule is gated, so a change to Rust support alone must leave every gated family's results standing.
  test("a Rust-only change moves no gated family's key", () => {
    const after = engineIdentity(checkout({ ...sources, "native/src/code_rule_dsl/scope/rust.rs": "// rust scope, fixed\n" }));
    assert.equal(after.shared, before.shared);
    assert.equal(after.families.ecma, before.families.ecma);
    assert.equal(after.families.python, before.families.python);
    assert.equal(engineKey(after, ["ecma"]), engineKey(before, ["ecma"]));
    assert.equal(engineKey(after, ["python"]), engineKey(before, ["python"]));
    assert.notEqual(after.families.rust, before.families.rust);
    // A rule of no family keys on every family's files, Rust's included.
    assert.notEqual(after.sha256, before.sha256);
    assert.notEqual(engineKey(after, []), engineKey(before, []));
  });

  test("a change to the shared engine moves every key", () => {
    const after = engineIdentity(checkout({ ...sources, "native/src/code_rule_dsl/matcher/mod.rs": "// every language's matcher, fixed\n" }));
    assert.notEqual(after.shared, before.shared);
    for (const families of [["python"], ["ecma"], ["ecma", "python"], []]) assert.notEqual(engineKey(after, families), engineKey(before, families));
  });

  // The native binary is not bit-reproducible, so a build-cache miss on unchanged sources used to re-judge every rule.
  test("a rebuilt native binary with unchanged sources keeps every key", () => {
    const root = checkout(sources);
    writeFileSync(join(root, "native", "rerail_engine.darwin-arm64.node"), "the binary of some other build\n");
    assert.deepEqual(engineIdentity(root), before);
  });

  // The key cannot tell a binary left over from before a source edit from the edited sources' build; its age can.
  test("a native binary older than an engine source it is built from is stale", () => {
    const root = checkout({ ...sources, "languages/python.json": "{}\n", "src/engine/index.ts": "export {};\n" });
    const binary = join(root, "native", "rerail_engine.darwin-arm64.node");
    writeFileSync(binary, "a build\n");
    const at = (path: string, seconds: number) => utimesSync(join(root, path), seconds, seconds);
    for (const path of [...Object.keys(sources), "languages/python.json", "src/engine/index.ts"]) at(path, 1_000);
    at("native/rerail_engine.darwin-arm64.node", 2_000);
    assert.equal(staleNativeBinary(root, binary), undefined);
    // The TypeScript engine is imported from source, so an edit to it needs no build.
    at("src/engine/index.ts", 3_000);
    assert.equal(staleNativeBinary(root, binary), undefined);
    at("native/src/code_rule_dsl/scope/python.rs", 3_000);
    assert.equal(staleNativeBinary(root, binary), "native/src/code_rule_dsl/scope/python.rs");
    at("native/src/code_rule_dsl/scope/python.rs", 1_000);
    // A deleted source is as new as the directory it was deleted from.
    unlinkSync(join(root, "languages/python.json"));
    at("languages", 3_000);
    assert.equal(staleNativeBinary(root, binary), "languages/python.json");
    at("native/rerail_engine.darwin-arm64.node", 4_000);
    assert.equal(staleNativeBinary(root, binary), undefined);
  });

  test("a rule of no family, or of one with no sources of its own, keys on the whole engine", () => {
    assert.equal(engineKey(before, []), before.sha256);
    assert.equal(engineKey(before, ["vue"]), before.sha256);
    assert.equal(engineKey(before, Object.keys(FAMILY_SOURCES)), before.sha256);
    assert.notEqual(engineKey(before, ["ecma", "python"]), before.sha256);
    assert.equal(engineKey(before, ["python", "ecma"]), engineKey(before, ["ecma", "python"]));
    assert.notEqual(engineKey(before, ["python"]), before.sha256);
    assert.notEqual(engineKey(before, ["python"]), engineKey(before, ["ecma"]));
  });
});

describe("the language families a rule document names", () => {
  const document = (clauses: unknown[]) => JSON.stringify({ format: "rerail.policy/v1", id: "r", clauses });
  const clause = (language: string) => ({ id: "c", forbid: { ast: { language, rule: { kind: "call" } } } });

  test("the family of every language its clauses name", () => {
    assert.deepEqual(policyFamilies(document([clause("python")])), ["python"]);
    assert.deepEqual(policyFamilies(document([clause("typescript")])), ["ecma"]);
    assert.deepEqual(policyFamilies(document([clause("javascript"), clause("typescript")])), ["ecma"]);
    assert.deepEqual(policyFamilies(document([clause("python"), clause("typescript")])), ["ecma", "python"]);
    // Nested utils and sub-rules name languages too, and all of them count.
    assert.deepEqual(policyFamilies(JSON.stringify({ clauses: [{ forbid: { any: [{ ast: { language: "php" } }] } }] })), ["php"]);
    // A Blade rule reads the PHP of a template's compiled view, so it is judged in PHP's family too; PHP embeds nothing.
    assert.deepEqual(policyFamilies(document([clause("blade")])), ["blade", "php"]);
    assert.deepEqual(policyFamilies(document([clause("php")])), ["php"]);
  });

  // Empty means "the gates cannot tell", which `engineKey` answers with the whole engine.
  test("a document that does not say which language it is judged in names no family", () => {
    assert.deepEqual(policyFamilies(document([{ id: "c", forbid: { lines: { max: 10 } } }])), []);
    assert.deepEqual(policyFamilies(document([clause("python"), clause("cobol")])), []);
    assert.deepEqual(policyFamilies("not json"), []);
  });
});
