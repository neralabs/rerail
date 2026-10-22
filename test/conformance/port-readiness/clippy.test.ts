import assert from "node:assert/strict";
import { test } from "vitest";

import { annotatedLints, cfgHolds, compileError, crateType, declaredLints, deprecatedLints, earlyPassLints, innerCfgPredicates, namedLint, testEdition } from "../../../conformance/recorders/port-readiness/adapters/clippy.js";
import { parseRustToolchainPin } from "../../../conformance/recorders/port-readiness/runtimes.js";
import { clippySpan, rustcPosition, rustcSpan } from "../../../conformance/port-readiness/span-units/clippy.js";

test("an engine byte offset lands where rustc counts it: characters, a dropped BOM, and only \\n breaking lines", () => {
  // `é` is 2 bytes and the emoji 4, each one column; CRLF ends line 0, a lone CR is a column.
  const source = "\uFEFFfn é() {}\r\nlet x\r= \"\u{1F600}\";\n";
  const bytes = (prefix: string) => Buffer.byteLength(prefix, "utf8");
  assert.deepEqual(rustcPosition(source, bytes("\uFEFFfn é")), { line: 0, column: 4 });
  assert.deepEqual(rustcPosition(source, bytes("\uFEFFfn é() {}\r\nlet x\r= \"\u{1F600}")), { line: 1, column: 10 });
  assert.deepEqual(clippySpan(source, bytes("\uFEFFfn "), bytes("\uFEFFfn é"), { endLine: true, endCol: true }), { startLine: 0, startCol: 3, endLine: 0, endCol: 4 });
  assert.deepEqual(clippySpan(source, 0, 3, { endLine: false, endCol: false }), { startLine: 0, startCol: 0, endLine: null, endCol: null });
});

test("a rustc JSON span is 1-based and becomes the 0-based span the engine's is compared with", () => {
  assert.deepEqual(rustcSpan({ line_start: 3, column_start: 5, line_end: 4, column_end: 2 }, { endLine: true, endCol: true }), { startLine: 2, startCol: 4, endLine: 3, endCol: 1 });
});

test("a UI test belongs to the lints its //~ annotations name and to the longest lint its name starts with", () => {
  const text = "//@aux-build:proc_macros.rs\nlet _ = (1); //~ double_parens\n//~^ clippy::precedence\n//~| ERROR: message\n";
  assert.deepEqual([...annotatedLints(text)].sort(), ["double_parens", "precedence"]);
  const lints = new Set(["precedence", "precedence_bits", "allow_attributes", "allow_attributes_without_reason", "needless_raw_string_hashes"]);
  assert.equal(namedLint("tests/ui/precedence_bits.rs", lints), "precedence_bits");
  assert.equal(namedLint("tests/ui/precedence.rs", lints), "precedence");
  assert.equal(namedLint("tests/ui/allow_attributes_without_reason.rs", lints), "allow_attributes_without_reason");
  assert.equal(namedLint("tests/ui-toml/needless_raw_string_hashes_one_allowed/needless_raw_string_hashes.rs", lints), "needless_raw_string_hashes");
  assert.equal(namedLint("tests/ui/literals.rs", lints), undefined);
});

test("a test compiles as ui_test 0.30.7 compiles it: edition 2024 unless //@edition, crate type from its contents", () => {
  assert.equal(testEdition("fn main() {}\n"), "2024");
  assert.equal(testEdition("//@edition:2015\nfn main() {}\n"), "2015");
  assert.deepEqual(crateType("#[proc_macro_derive(X)]"), ["--crate-type=proc-macro"]);
  assert.deepEqual(crateType("#[test]\nfn t() {}"), ["--test"]);
  assert.deepEqual(crateType("fn main() {}"), []);
  assert.deepEqual(crateType("pub fn f() {}"), ["--crate-type=lib"]);
});

test("a test rustc rejects is a compile error, not a lint raised to deny or rustc's closing summary", () => {
  const aborting = { level: "error", code: null, message: "aborting due to 2 previous errors" };
  const denied = { level: "error", code: { code: "clippy::double_parens" }, message: "unnecessary parentheses" };
  const deniedRustc = { level: "error", code: { code: "dead_code" }, message: "function `f` is never used" };
  assert.equal(compileError([denied, deniedRustc, aborting]), undefined);
  assert.equal(compileError([denied, { level: "error", code: { code: "E0432" }, message: "unresolved import `aux`\nmore" }, aborting]), "E0432: unresolved import `aux`");
  assert.equal(compileError([{ level: "error", code: null, message: "expected one of `;` or `}`" }, aborting]), "expected one of `;` or `}`");
  assert.equal(compileError([{ level: "warning", code: { code: "E0133" }, message: "not an error" }]), undefined);
});

test("a lint is an early pass's only when no pass that lists it implements LateLintPass", () => {
  const misc = "use rustc_lint::{EarlyContext, EarlyLintPass, LateLintPass};\ndeclare_lint_pass!(MiscEarlyLints => [\n    BUILTIN_TYPE_SHADOW,\n    ZERO_PREFIXED_LITERAL,\n]);\nimpl EarlyLintPass for MiscEarlyLints {}\n";
  const attrs = "impl_lint_pass!(Attributes => [INLINE_ALWAYS]);\nimpl_lint_pass!(PostExpansionEarlyAttributes => [ALLOW_ATTRIBUTES, INLINE_ALWAYS]);\nimpl<'tcx> LateLintPass<'tcx> for Attributes {}\nimpl EarlyLintPass for PostExpansionEarlyAttributes {}\n";
  assert.deepEqual([...earlyPassLints([misc, attrs])].sort(), ["allow_attributes", "builtin_type_shadow", "zero_prefixed_literal"]);
});

test("declare_clippy_lint! gives each lint's name and group, and deprecated_lints.rs the deprecated ones", () => {
  const source = `declare_clippy_lint! {\n    /// ### What it does\n    #[clippy::version = "1.43.0"]\n    pub OPTION_ENV_UNWRAP,\n    correctness,\n    "using \`option_env!(...).unwrap()\`"\n}\n`;
  assert.deepEqual(declaredLints(source), [{ name: "option_env_unwrap", group: "correctness" }]);
  const deprecated = `declare_with_version! { DEPRECATED(DEPRECATED_VERSION) = [\n    #[clippy::version = "1.30.0"]\n    ("clippy::assign_ops", "compound operators are harmless"),\n]}\ndeclare_with_version! { RENAMED(RENAMED_VERSION) = [\n    #[clippy::version = ""]\n    ("clippy::old", "clippy::new"),\n]}\n`;
  assert.deepEqual([...deprecatedLints(deprecated)], [["assign_ops", "compound operators are harmless"]]);
});

test("a rust-toolchain.toml pin names an exact release, its components and profile, and nothing else", () => {
  assert.deepEqual(parseRustToolchainPin('[toolchain]\nchannel = "1.99.0"\ncomponents = ["clippy"]\nprofile = "minimal"\n', "pin"), { channel: "1.99.0", components: ["clippy"], profile: "minimal" });
  assert.throws(() => parseRustToolchainPin('[toolchain]\nchannel = "stable"\ncomponents = ["clippy"]\nprofile = "minimal"\n', "pin"), /exact release/u);
  assert.throws(() => parseRustToolchainPin('[toolchain]\nchannel = "1.99.0"\ncomponents = ["clippy"]\nprofile = "minimal"\ntargets = ["wasm32-unknown-unknown"]\n', "pin"), /not targets/u);
});

test("a file's leading #![cfg] predicates decide whether rustc kept anything of it, under the options a unit sets", () => {
  const source = "//! Tests.\n#![warn(rust_2018_idioms)]\n#![cfg(all(\n    feature = \"full\",\n    tokio_unstable,\n    not(target_os = \"wasi\"),\n))]\n#![cfg(any())]\n\nuse std::sync::Arc;\n#![cfg(windows)]\n";
  const predicates = innerCfgPredicates(source);
  // Only the inner attributes before the first item; `#![cfg_attr]` and other attributes are not predicates.
  assert.equal(predicates.length, 2);
  const macos = new Set(['target_os="macos"', "unix", 'feature="full"', "clippy"]);
  assert.equal(cfgHolds(predicates[0]!, macos), false);
  assert.equal(cfgHolds(predicates[0]!, new Set([...macos, "tokio_unstable"])), true);
  assert.equal(cfgHolds(predicates[1]!, macos), false, "any() of nothing is false");
  assert.deepEqual(innerCfgPredicates("#![cfg_attr(docsrs, feature(doc_cfg))]\nfn main() {}\n"), []);
  const [notClippy] = innerCfgPredicates("#![cfg(not(clippy))]\n");
  assert.equal(cfgHolds(notClippy!, macos), false);
  // What is not a configuration predicate cannot be evaluated, so it drops nothing.
  const [unknown] = innerCfgPredicates("#![cfg(feature(x))]\n");
  assert.equal(cfgHolds(unknown!, macos), undefined);
});
