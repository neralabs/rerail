import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import {
  documentedOptions,
  enabledCodes,
  instability,
  guardStrings,
  inSemanticErrorArm,
  parseShowSettings,
  pathCallSites,
  publicFunctions,
  raiseSites,
  ruffVersionMatches,
  runtimeVersionCheck,
  settingsKey,
  testModuleStrings,
  useTarget,
  violationStruct,
} from "../../../conformance/recorders/port-readiness/adapters/ruff.js";
import { codePointPosition, diagnosticSpan } from "../../../conformance/port-readiness/span-units/ruff.js";
import { enabledSelections, evaluatePath, kebabCase, ruffTestLints, tokenText, type RuleVariants } from "../../../conformance/recorders/port-readiness/adapters/ruff-tests.js";
import { lexRust, literalWords, rustLiterals, useTreePaths, withoutTestItems } from "../../../conformance/recorders/port-readiness/languages/rust.js";

const strings = (source: string) => lexRust(source).filter((token) => token.kind === "string").map((token) => token.value);

test("comments of every kind are not tokens, and block comments nest", () => {
  const tokens = lexRust(`/// doc "a"\n//! inner "b"\n/* outer /* inner "c" */ still "d" */ let x = "e"; // "f"\n/** block doc "g" */`);
  assert.deepEqual(tokens.map((token) => token.text), ["let", "x", "=", '"e"', ";"]);
});

test("raw, byte and C strings decode as Rust reads them", () => {
  assert.deepEqual(strings(String.raw`r"a\nb" r#"say "hi""# r##"x"#y"## b"bytes" br"raw\x" c"cee"`), [String.raw`a\nb`, 'say "hi"', 'x"#y', "bytes", String.raw`raw\x`, "cee"]);
});

test("escapes in quoted strings decode, and a backslash before a line break continues the string", () => {
  assert.deepEqual(strings(String.raw`"a\"b\\c\n\t\x41\u{1F600}" "one \
      two"`), ['a"b\\c\n\tA\u{1F600}', "one two"]);
});

test("char and byte literals are not lifetimes, and lifetimes are not chars", () => {
  const tokens = lexRust(String.raw`fn f<'a>(x: &'a str) -> &'static str { let c = 'x'; let n = '\n'; let q = '\''; let e = '\u{1F600}'; let b = b'y'; let s = '😀'; }`);
  assert.deepEqual(tokens.filter((token) => token.kind === "lifetime").map((token) => token.text), ["'a", "'a", "'static"]);
  assert.deepEqual(tokens.filter((token) => token.kind === "char").map((token) => token.value), ["x", "\n", "'", "\u{1F600}", "y", "😀"]);
});

test("a char literal of a quote does not open a string", () => {
  assert.deepEqual(strings(`let q = '"'; let s = "after";`), ["after"]);
});

test("items marked #[cfg(test)] are dropped with their bodies", () => {
  const tokens = withoutTestItems(lexRust(`
    fn keep() { "kept" }
    #[cfg(test)]
    mod tests { fn t() { "test-only" } }
    #[cfg(test)]
    #[allow(unused)]
    fn helper(x: [u8; 2]) -> u8 { "helper-only" }
    #[cfg(test)]
    use crate::testing::Fixture;
    fn also() { "also-kept" }
  `));
  assert.deepEqual(tokens.filter((token) => token.kind === "string").map((token) => token.value), ["kept", "also-kept"]);
  assert.equal(tokens.some((token) => token.text === "Fixture"), false);
});

test("a literal names itself when identifier-shaped, its parts when a qualified name or path, and the words it quotes", () => {
  assert.deepEqual(literalWords("deque"), ["deque"]);
  assert.deepEqual(literalWords("collections.abc.Sequence"), ["collections", "abc", "Sequence"]);
  assert.deepEqual(literalWords("/var/tmp"), ["var", "tmp"]);
  assert.deepEqual(literalWords("Use `contextlib.suppress` or 'pass' instead"), ["pass"]);
  assert.deepEqual(literalWords("Do not use bare `except`"), ["except"]);
  assert.deepEqual(literalWords("two words"), []);
});

test("rustLiterals reads names, regex patterns and imports outside comments and tests", () => {
  const found = rustLiterals(`
    use ruff_python_semantic::analyze::typing::{is_mutable_expr, self as typing};
    use crate::rules::flake8_bugbear::helpers::*;
    /// Documentation mentions "Documented".
    static PASSWORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(^|_)(?i)(pas+wo?r?d|token)($|_)").unwrap());
    static TODO: LazyLock<RegexSet> = LazyLock::new(|| RegexSet::new([r"^#\\s*(?i)fixme", "^#\\\\s*(?i)xxx"]).unwrap());
    fn check() { matches!(segments, ["collections", "deque"]) && name == "typing.List" }
    #[cfg(test)]
    mod tests { fn t() { "FixtureOnly" } }
  `);
  // Regex words are read as the ECMAScript and Python readers read them, single letters included.
  assert.deepEqual(found.names, ["List", "_", "collections", "d", "deque", "fixme", "pas", "r", "token", "typing", "wo", "xxx"]);
  assert.deepEqual(found.regexes, [String.raw`(^|_)(?i)(pas+wo?r?d|token)($|_)`, String.raw`^#\s*(?i)fixme`, String.raw`^#\s*(?i)xxx`]);
  assert.deepEqual(found.uses, [
    ["ruff_python_semantic", "analyze", "typing", "is_mutable_expr"],
    ["ruff_python_semantic", "analyze", "typing"],
    ["crate", "rules", "flake8_bugbear", "helpers"],
  ]);
});

test("rustLiterals reads the words of a substring search's needle", () => {
  const found = rustLiterals(`static FINDER: LazyLock<Finder> = LazyLock::new(|| Finder::new(b"# /// script"));\nlet s = "# /// other";`);
  assert.deepEqual(found.names, ["script"]);
  assert.deepEqual(found.regexes, []);
});

test("a use tree expands into every path it imports, aliases dropped", () => {
  const tokens = lexRust("a::{b, c::{d as e, f}, self, g::*}").filter((token) => token.text !== "::");
  assert.deepEqual(useTreePaths(tokens), [["a", "b"], ["a", "c", "d"], ["a", "c", "f"], ["a"], ["a", "g"]]);
});

test("useTarget resolves crate, super and external paths to the longest module file", () => {
  const root = mkdtempSync(join(tmpdir(), "ruff-use-"));
  try {
    const file = (path: string) => {
      mkdirSync(join(root, path, ".."), { recursive: true });
      writeFileSync(join(root, path), "");
      return join(root, path);
    };
    const rule = file("linter/src/rules/bugbear/rules/mutable.rs");
    const helpers = file("linter/src/rules/bugbear/helpers.rs");
    const typing = file("semantic/src/analyze/typing.rs");
    const lib = file("semantic/src/lib.rs");
    const crates = new Map([["ruff_python_semantic", join(root, "semantic/src")]]);
    const own = join(root, "linter/src");
    assert.equal(useTarget(["crate", "rules", "bugbear", "helpers", "is_x"], rule, crates, own), helpers);
    assert.equal(useTarget(["super", "super", "helpers", "is_x"], rule, crates, own), helpers);
    assert.equal(useTarget(["ruff_python_semantic", "analyze", "typing", "is_mutable_expr"], rule, crates, own), typing);
    assert.equal(useTarget(["ruff_python_semantic", "SemanticModel"], rule, crates, own), lib);
    assert.equal(useTarget(["std", "fmt"], rule, crates, own), undefined);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

const variants: RuleVariants = {
  nameOf: (variant) => ({ NoneComparison: "none-comparison", UnusedImport: "unused-import" })[variant],
  codeOf: (variant) => ({ NoneComparison: "E711", UnusedImport: "F401" })[variant],
};

test("evaluatePath folds the path expressions Ruff's tests build", () => {
  const path = (text: string) => evaluatePath(lexRust(text), variants);
  assert.equal(path(`Path::new("pycodestyle").join(path).as_path()`.replace("path)", `Path::new("E711.py"))`)), "pycodestyle/E711.py");
  assert.equal(path(`&Path::new("pyflakes").join(format!("{}_{}.py", Rule::UnusedImport.noqa_code(), "0"))`), "pyflakes/F401_0.py");
  assert.equal(path(`PathBuf::from_iter(["pep8_naming", "ignore_names", "N801.py"]).as_path()`), "pep8_naming/ignore_names/N801.py");
  assert.equal(path(`test_resource_path("fixtures").join("pycodestyle/W391.ipynb")`), "resources/test/fixtures/pycodestyle/W391.ipynb");
  assert.equal(path(`Path::new(&format!("{name}.py"))`), undefined);
  assert.equal(path(`Path::new(dir).join("x.py")`), undefined);
});

test("ruffTestLints finds each fixture a test lints, with test-case arguments and let bindings substituted, once", () => {
  const modRs = `
    mod helpers {}
    #[cfg(test)]
    mod tests {
      #[test_case(Rule::NoneComparison, Path::new("E711.py"))]
      #[test_case(Rule::UnusedImport, Path::new("F401_0.py"); "named")]
      fn rules(rule_code: Rule, path: &Path) -> Result<()> {
        let snapshot = format!("{}_{}", rule_code.name(), path.to_string_lossy());
        let diagnostics = test_path(Path::new("pycodestyle").join(path).as_path(), &LinterSettings::for_rule(rule_code))?;
        assert_diagnostics!(snapshot, diagnostics);
        Ok(())
      }
      #[test]
      fn with_settings() -> Result<()> {
        let settings = LinterSettings { preview: PreviewMode::Enabled, ..LinterSettings::for_rules(vec![Rule::UnusedImport]) };
        let diagnostics = test_path(Path::new("pyflakes/F401_1.py"), &settings)?;
        assert_diagnostics!(diagnostics);
        Ok(())
      }
      #[test]
      fn diff() -> Result<()> {
        assert_diagnostics_diff!("snap", Path::new("pyflakes/F401_2.py"), &LinterSettings::for_rule(Rule::UnusedImport), &LinterSettings::for_rule(Rule::UnusedImport).with_preview_mode());
        Ok(())
      }
      #[test]
      fn notebook() -> Result<()> {
        let actual = test_resource_path("fixtures").join("pycodestyle/W391.ipynb");
        assert_notebook_path(&actual, &actual, &LinterSettings::for_rules(Linter::Pyflakes.rules()))?;
        Ok(())
      }
      #[test]
      fn snippet() { let diagnostics = test_snippet("import os", &LinterSettings::for_rule(Rule::UnusedImport)); }
    }`;
  const lints = ruffTestLints("pycodestyle", modRs, variants).map((lint) => ({ test: lint.test, fixture: lint.fixture, settings: tokenText(lint.settings) }));
  assert.deepEqual(lints, [
    { test: "pycodestyle::rules", fixture: "resources/test/fixtures/pycodestyle/E711.py", settings: "LinterSettings::for_rule((Rule::NoneComparison))" },
    { test: "pycodestyle::rules", fixture: "resources/test/fixtures/pycodestyle/F401_0.py", settings: "LinterSettings::for_rule((Rule::UnusedImport))" },
    { test: "pycodestyle::with_settings", fixture: "resources/test/fixtures/pyflakes/F401_1.py", settings: "LinterSettings { preview : PreviewMode::Enabled, ..LinterSettings::for_rules(vec![Rule::UnusedImport])}" },
    { test: "pycodestyle::diff", fixture: "resources/test/fixtures/pyflakes/F401_2.py", settings: "LinterSettings::for_rule(Rule::UnusedImport)" },
    { test: "pycodestyle::diff", fixture: "resources/test/fixtures/pyflakes/F401_2.py", settings: "LinterSettings::for_rule(Rule::UnusedImport).with_preview_mode()" },
    { test: "pycodestyle::notebook", fixture: "resources/test/fixtures/pycodestyle/W391.ipynb", settings: "LinterSettings::for_rules(Linter::Pyflakes.rules())" },
  ]);
});

test("ruffTestLints reads test_contents of a fixture file read from disk, and skips inline sources", () => {
  const modRs = `
    #[cfg(test)]
    mod tests {
      #[test]
      fn from_file() -> Result<()> {
        let path = test_resource_path("fixtures").join("ruff/RUF050_F401.py");
        let source_type = SourceType::Python(PySourceType::from(&path));
        let source_kind = SourceKind::from_path(&path, source_type)?.expect("valid source");
        let settings = settings::LinterSettings::for_rules(vec![Rule::UnusedImport, Rule::NoneComparison]);
        let (diagnostics, transformed) = test_contents(&source_kind, &path, &settings);
        assert_diagnostics!(diagnostics);
        Ok(())
      }
      #[test_case("import os", "inline")]
      fn inline(contents: &str, snapshot: &str) {
        let diagnostics = test_contents(&SourceKind::Python { code: dedent(contents).to_string(), is_stub: false }, Path::new("f401_inline.py"), &LinterSettings::for_rule(Rule::UnusedImport)).0;
      }
    }`;
  const lints = ruffTestLints("ruff", modRs, variants).map((lint) => ({ fixture: lint.fixture, rules: enabledSelections(lint.settings).rules }));
  assert.deepEqual(lints, [{ fixture: "resources/test/fixtures/ruff/RUF050_F401.py", rules: ["NoneComparison", "UnusedImport"] }]);
});

test("ruffTestLints reads linter.rs's shapes: its path helper inlined, files read for test_contents_syntax_errors and notebooks", () => {
  const linterRs = `
    #[cfg(test)]
    mod tests {
      fn notebook_path(path: impl AsRef<Path>) -> std::path::PathBuf {
        Path::new("../ruff_notebook/resources/test/fixtures/jupyter").join(path)
      }
      fn test_contents_syntax_errors(source_kind: &SourceKind, path: &Path, settings: &LinterSettings) -> Vec<Diagnostic> {
        let parsed = parse_unchecked_source(source_kind, source_type, target_version.parser_version());
        check_path(path, None, &locator, settings)
      }
      #[test]
      fn test_undefined_name() -> Result<(), NotebookError> {
        let actual = notebook_path("undefined_name.ipynb");
        let expected = notebook_path("undefined_name.ipynb");
        let TestedNotebook { diagnostics, source_notebook, .. } = assert_notebook_path(&actual, expected, &LinterSettings::for_rule(Rule::UndefinedName))?;
        Ok(())
      }
      #[test_case(Path::new("no_cell_id.ipynb"), false; "no_cell_id")]
      fn test_cell_id(path: &Path, has_id: bool) -> Result<()> {
        let source_notebook = Notebook::from_path(&notebook_path(path))?;
        let source_kind = SourceKind::ipy_notebook(source_notebook);
        let (_, transformed) = test_contents(&source_kind, path, &LinterSettings::for_rule(Rule::UnusedImport));
        Ok(())
      }
      #[test_case(Rule::YieldOutsideFunction, Path::new("yield_scope.py"))]
      fn test_syntax_errors(rule: Rule, path: &Path) -> Result<()> {
        let snapshot = path.to_string_lossy().to_string();
        let path = Path::new("resources/test/fixtures/syntax_errors").join(path);
        let diagnostics = test_contents_syntax_errors(&SourceKind::Python { code: std::fs::read_to_string(&path)?, is_stub: false }, &path, &LinterSettings::for_rule(rule));
        Ok(())
      }
      #[test]
      fn test_await_scope_notebook() -> Result<()> {
        let path = Path::new("resources/test/fixtures/syntax_errors/await_scope.ipynb");
        let TestedNotebook { diagnostics, .. } = assert_notebook_path(path, path, &LinterSettings::for_rule(Rule::YieldOutsideFunction))?;
        Ok(())
      }
    }`;
  const lints = ruffTestLints("linter", linterRs, variants).map((lint) => ({ test: lint.test, fixture: lint.fixture, rules: enabledSelections(lint.settings).rules }));
  assert.deepEqual(lints, [
    { test: "linter::test_undefined_name", fixture: "../ruff_notebook/resources/test/fixtures/jupyter/undefined_name.ipynb", rules: ["UndefinedName"] },
    { test: "linter::test_cell_id", fixture: "../ruff_notebook/resources/test/fixtures/jupyter/no_cell_id.ipynb", rules: ["UnusedImport"] },
    { test: "linter::test_syntax_errors", fixture: "resources/test/fixtures/syntax_errors/yield_scope.py", rules: ["YieldOutsideFunction"] },
    { test: "linter::test_await_scope_notebook", fixture: "resources/test/fixtures/syntax_errors/await_scope.ipynb", rules: ["YieldOutsideFunction"] },
  ]);
});

test("enabledSelections reads the rules and whole linters a settings expression turns on", () => {
  assert.deepEqual(enabledSelections(lexRust("LinterSettings { x: 1, ..LinterSettings::for_rules(vec![Rule::A, Rule::B]) }")), { rules: ["A", "B"], linters: [] });
  assert.deepEqual(enabledSelections(lexRust("LinterSettings::for_rules(Linter::Pyflakes.rules())")), { rules: [], linters: ["Pyflakes"] });
});

test("kebabCase gives Ruff's rule names", () => {
  assert.equal(kebabCase("NoneComparison"), "none-comparison");
  assert.equal(kebabCase("NonPEP585Annotation"), "non-pep585-annotation");
  assert.equal(kebabCase("UnusedImport"), "unused-import");
});

test("parseShowSettings reads single and multi-line values, and enabledCodes the default selection", () => {
  const settings = parseShowSettings([
    "# Linter Settings",
    "linter.rules.enabled = [",
    "\tunused-import (F401),",
    "\tnone-comparison (E711),",
    "]",
    "linter.unresolved_target_version = none",
    "linter.flake8_bugbear.extend_immutable_calls = []",
    "formatter.unresolved_target_version = 3.10",
  ].join("\n"));
  assert.equal(settings.get("linter.unresolved_target_version"), "none");
  assert.equal(settings.get("formatter.unresolved_target_version"), "3.10");
  assert.equal(settings.get("linter.flake8_bugbear.extend_immutable_calls"), "[]");
  assert.deepEqual(enabledCodes(settings.get("linter.rules.enabled")!), ["E711", "F401"]);
});

test("documentedOptions and settingsKey map a rule's documented options to --show-settings keys", () => {
  const explanation = "## What it does\nx\n\n## Options\n- `lint.flake8-bugbear.extend-immutable-calls`\n- [`target-version`]\n\n## References\n- `not.an.option`\n";
  assert.deepEqual(documentedOptions(explanation), ["lint.flake8-bugbear.extend-immutable-calls", "target-version"]);
  assert.deepEqual(documentedOptions("## Options\n- `lint.pyflakes.extend-generics`"), ["lint.pyflakes.extend-generics"]);
  assert.equal(settingsKey("lint.flake8-bugbear.extend-immutable-calls"), "linter.flake8_bugbear.extend_immutable_calls");
  assert.equal(settingsKey("lint.dummy-variable-rgx"), "linter.dummy_variable_rgx");
  assert.equal(settingsKey("target-version"), "linter.unresolved_target_version");
});

test("instability is null only for a stable rule", () => {
  assert.equal(instability({ Stable: { since: "v0.0.92" } }), null);
  assert.equal(instability({ Preview: { since: "0.5.0" } }), "preview since 0.5.0 (ruff rule --all)");
  assert.equal(instability({ Removed: { since: "0.8.0" } }), "removed since 0.8.0 (ruff rule --all)");
});

test("ruffVersionMatches accepts the pinned version alone or with the pinned commit, and nothing else", () => {
  const commit = "0be08a206f9c3180afd3e93bcc792ed5cb1f4db1";
  const other = { ok: false, form: "other" };
  // CI's Linux wheel of 0.16.9 prints the version alone; the macOS wheel prints the commit it was built from.
  assert.deepEqual(ruffVersionMatches("ruff 0.16.9", "0.16.9", commit), { ok: true, form: "bare" });
  assert.deepEqual(ruffVersionMatches("ruff 0.16.9 (0be08a206 2026-09-24)", "0.16.9", commit), { ok: true, form: "with-commit" });
  // A printed commit that is not the pinned one, or a version that only starts with the pinned one, still fails.
  assert.deepEqual(ruffVersionMatches("ruff 0.16.9 (1234abcde 2026-09-24)", "0.16.9", commit), other);
  // A longer hash whose first 9 characters are the pinned commit's names another commit (the pin goes on 0be08a206f).
  assert.deepEqual(ruffVersionMatches("ruff 0.16.9 (0be08a2069 2026-09-24)", "0.16.9", commit), other);
  assert.deepEqual(ruffVersionMatches("ruff 0.16.8", "0.16.9", commit), other);
  assert.deepEqual(ruffVersionMatches("ruff 0.16.8 (0be08a206 2026-09-24)", "0.16.9", commit), other);
  assert.deepEqual(ruffVersionMatches("ruff 0.16.90", "0.16.9", commit), other);
  assert.deepEqual(ruffVersionMatches("ruff 0.16.90 (0be08a206 2026-09-24)", "0.16.9", commit), other);
  // Ruff's form for a build past the release tag.
  assert.deepEqual(ruffVersionMatches("ruff 0.16.9+3 (0be08a206 2026-09-24)", "0.16.9", commit), other);
  assert.deepEqual(ruffVersionMatches("", "0.16.9", commit), other);
});

test("runtimeVersionCheck passes on the pinned Ruff and the pinned Python, and its detail names the form printed", () => {
  const upstream = { version: "0.16.9", commit: "0be08a206f9c3180afd3e93bcc792ed5cb1f4db1" };
  const python = { version: "Python 3.13.12", pinned: "3.13.12" };
  const pins = "the pack pins 0.16.9 at 0be08a206 on Python 3.13.12";
  assert.deepEqual(runtimeVersionCheck("ruff 0.16.9", python, upstream), {
    id: "runtime-version",
    ok: true,
    detail: `the installed wheel reports \`ruff 0.16.9\` (the version alone, no build commit printed) on Python 3.13.12; ${pins}`,
  });
  assert.deepEqual(runtimeVersionCheck("ruff 0.16.9 (0be08a206 2026-09-24)", python, upstream), {
    id: "runtime-version",
    ok: true,
    detail: `the installed wheel reports \`ruff 0.16.9 (0be08a206 2026-09-24)\` (the version and the pinned commit) on Python 3.13.12; ${pins}`,
  });
  // The pinned Ruff on another interpreter than .python-version pins still fails.
  assert.deepEqual(runtimeVersionCheck("ruff 0.16.9", { version: "Python 3.13.11", pinned: "3.13.12" }, upstream), {
    id: "runtime-version",
    ok: false,
    detail: `the installed wheel reports \`ruff 0.16.9\` (the version alone, no build commit printed) on Python 3.13.11; ${pins}`,
  });
  assert.deepEqual(runtimeVersionCheck("ruff 0.16.8", python, upstream), {
    id: "runtime-version",
    ok: false,
    detail: `the installed wheel reports \`ruff 0.16.8\` (neither the pinned version alone nor with the pinned commit) on Python 3.13.12; ${pins}`,
  });
});

test("codePointPosition counts code points, breaks lines as Ruff does, and skips a byte-order mark", () => {
  const at = (source: string, text: string) => codePointPosition(source, Buffer.byteLength(source.slice(0, source.indexOf(text)), "utf8"));
  // Ruff reports `None` in this line at 1-based column 21: a code-point count, not UTF-16 (22) or bytes (26).
  assert.deepEqual(at('x = "𝐁é"; y = (x == None)', "None"), { line: 0, column: 20 });
  assert.deepEqual(at("\ty = (x == None)", "None"), { line: 0, column: 11 });
  assert.deepEqual(at("﻿y = (x == None)", "None"), { line: 0, column: 10 });
  assert.deepEqual(at("a = 1\ry = 2\r\nz = None", "None"), { line: 2, column: 4 });
  assert.deepEqual(at("﻿a\nb = None", "None"), { line: 1, column: 4 });
});

test("diagnosticSpan makes Ruff's 1-based rows and columns 0-based, keeping the exclusive end", () => {
  assert.deepEqual(diagnosticSpan({ location: { row: 2, column: 11 }, end_location: { row: 2, column: 15 } }), { startLine: 1, startCol: 10, endLine: 1, endCol: 14 });
});

test("testModuleStrings reads the string literals of a mod.rs test module only", () => {
  const modRs = `const OUTSIDE: &str = "outside";\nmod tests {\n  fn t() { settings(vec!["fastapi.Depends".to_string()]); test_snippet(r#"import os"#); }\n}`;
  assert.deepEqual(testModuleStrings(modRs), ["fastapi.Depends", "import os"]);
});

test("violationStruct reads the struct `ruff rule --all` points at", () => {
  const source = "use x;\n\n/// ## What it does\n#[derive(ViolationMetadata)]\n#[violation_metadata(stable_since = \"v0.0.1\")]\npub(crate) struct BreakOutsideLoop;\n\npub struct Other;\n";
  assert.equal(violationStruct(source, 4), "BreakOutsideLoop");
  assert.equal(violationStruct(source, 7), "Other");
});

test("raiseSites finds where code builds a violation, not where it imports it or checks it is enabled", () => {
  const raises = (code: string, struct = "ForwardAnnotationSyntaxError") => raiseSites(lexRust(code), struct).length > 0;
  assert.equal(raises("self.report_diagnostic(pyflakes::rules::ForwardAnnotationSyntaxError { parse_error }, range);"), true);
  assert.equal(raises("checker.report_diagnostic(pyflakes::rules::BreakOutsideLoop, stmt.range());", "BreakOutsideLoop"), true);
  assert.equal(raises("self.report_diagnostic(YieldOutsideFunction::new(kind), error.range);", "YieldOutsideFunction"), true);
  assert.equal(raises("checker.report_diagnostic(LoggingExtraAttrClash(key.to_string()), range);", "LoggingExtraAttrClash"), true);
  assert.equal(raises("pub(crate) struct LoggingExtraAttrClash(pub String);", "LoggingExtraAttrClash"), false);
  assert.equal(raises("if self.is_rule_enabled(Rule::ForwardAnnotationSyntaxError) { visit(); }"), false);
  assert.equal(raises("use crate::rules::pyflakes::rules::{ForwardAnnotationSyntaxError, Other};"), false);
  assert.equal(raises("// ForwardAnnotationSyntaxError { }\nlet s = \"ForwardAnnotationSyntaxError {\";"), false);
});

test("inSemanticErrorArm tells a violation raised for a parser-detected semantic error from one the checker detects", () => {
  const code = `fn report_semantic_error(&self, error: SemanticSyntaxError) {
    if self.semantic.in_string_type_definition() { self.report(ForwardAnnotationSyntaxError { parse_error: error.to_string() }, error.range); return; }
    match error.kind {
      SemanticSyntaxErrorKind::LateFutureImport => { if self.is_rule_enabled(Rule::LateFutureImport) { self.report(LateFutureImport, error.range); } }
      SemanticSyntaxErrorKind::FutureFeatureNotDefined(name) => { if self.on() { self.report(pyflakes::rules::FutureFeatureNotDefined { name }, error.range); } }
      SemanticSyntaxErrorKind::A | SemanticSyntaxErrorKind::B => {}
    }
  }`;
  const tokens = lexRust(code);
  const arm = (struct: string) => raiseSites(tokens, struct).some((index) => inSemanticErrorArm(tokens, index));
  assert.equal(arm("LateFutureImport"), true);
  assert.equal(arm("FutureFeatureNotDefined"), true);
  assert.equal(arm("ForwardAnnotationSyntaxError"), false);
});

test("guardStrings reads the strings of the conditions around a call, out to its function, and no others", () => {
  const code = `pub(crate) fn expression(expr: &Expr, checker: &Checker) {
    let label = "not a guard";
    match expr {
      Expr::Call(call) if call.name() != "skip" => {
        if let Expr::StringLiteral(ast::ExprStringLiteral { value, .. }) = value.as_ref() {
          if attr == "join" {
            if checker.is_rule_enabled(Rule::StaticJoinToFString) { flynt::rules::static_join_to_fstring(checker, expr, value); }
          } else if attr == "split" { flynt::rules::other(checker); }
        }
        if checker.semantic.match_typing_expr(annotation, "TypeAlias") { other_rule(checker); }
      }
      _ => {}
    }
  }`;
  const tokens = lexRust(code);
  const guards = (fn: string) => pathCallSites(tokens, new Set([fn])).flatMap((index) => guardStrings(tokens, index)).sort();
  assert.deepEqual(guards("static_join_to_fstring"), ["join", "skip"]);
  assert.deepEqual(guards("other"), ["skip", "split"]);
  const enabled = tokens.findIndex((token, index) => token.text === "StaticJoinToFString" && tokens[index - 2]?.text === "Rule");
  assert.deepEqual(guardStrings(tokens, enabled).sort(), ["join", "skip"]);
});

test("publicFunctions reads the functions a rule file exposes to the checker", () => {
  const tokens = lexRust("pub(crate) fn logging_call(c: &Checker) {}\npub fn find() {}\nfn helper() {}\npub(super) fn near() {}\nimpl X { pub(crate) fn method(&self) {} }");
  assert.deepEqual([...publicFunctions(tokens)].sort(), ["find", "logging_call", "method", "near"]);
});
