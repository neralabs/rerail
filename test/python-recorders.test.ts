/**
 * The Python pack's source-facts recorders (conformance/recorders/{rust-source,ruff-tests,ruff-source,python-toolkit}.ts):
 * the Rust reader, the reader of Ruff's test modules and the Ruff facts helpers, ported with their tests from
 * nera-rule-validation's test/validation-ruff.test.ts, and the regex translations. The recorders run offline; the
 * last tests read only the source.json files they wrote.
 */
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { fixturesRecordingPath, loadPack, loadRecording, loadUpstreamSource, packDirectory, upstreamSourcePath } from "../conformance/languages/pack.js";
import { pythonRegex } from "../conformance/recorders/python-toolkit.js";
import {
  documentedOptions,
  enabledCodes,
  inSemanticErrorArm,
  instability,
  parseShowSettings,
  raiseSites,
  settingsKey,
  testModuleStrings,
  useTarget,
  violationStruct,
} from "../conformance/recorders/ruff-source.js";
import { enabledSelections, evaluatePath, kebabCase, ruffTestLints, tokenText, type RuleVariants } from "../conformance/recorders/ruff-tests.js";
import { lexRust, literalWords, rustLiterals, rustRegex, useTreePaths, withoutTestItems } from "../conformance/recorders/rust-source.js";
import { parseOrigin } from "../conformance/recorders/upstream-source.js";

const strings = (source: string) => lexRust(source).filter(token => token.kind === "string").map(token => token.value);

describe("the Rust reader", () => {
  it("drops comments of every kind, and block comments nest", () => {
    const tokens = lexRust(`/// doc "a"\n//! inner "b"\n/* outer /* inner "c" */ still "d" */ let x = "e"; // "f"\n/** block doc "g" */`);
    expect(tokens.map(token => token.text)).toEqual(["let", "x", "=", '"e"', ";"]);
  });

  it("decodes raw, byte and C strings as Rust reads them", () => {
    expect(strings(String.raw`r"a\nb" r#"say "hi""# r##"x"#y"## b"bytes" br"raw\x" c"cee"`)).toEqual([String.raw`a\nb`, 'say "hi"', 'x"#y', "bytes", String.raw`raw\x`, "cee"]);
  });

  it("decodes escapes in quoted strings, and a backslash before a line break continues the string", () => {
    expect(
      strings(String.raw`"a\"b\\c\n\t\x41\u{1F600}" "one \
      two"`),
    ).toEqual(['a"b\\c\n\tA\u{1F600}', "one two"]);
  });

  it("lexes char and byte literals apart from lifetimes", () => {
    const tokens = lexRust(String.raw`fn f<'a>(x: &'a str) -> &'static str { let c = 'x'; let n = '\n'; let q = '\''; let e = '\u{1F600}'; let b = b'y'; let s = '😀'; }`);
    expect(tokens.filter(token => token.kind === "lifetime").map(token => token.text)).toEqual(["'a", "'a", "'static"]);
    expect(tokens.filter(token => token.kind === "char").map(token => token.value)).toEqual(["x", "\n", "'", "\u{1F600}", "y", "😀"]);
  });

  it("does not open a string at a char literal of a quote", () => {
    expect(strings(`let q = '"'; let s = "after";`)).toEqual(["after"]);
  });

  it("drops items marked #[cfg(test)] with their bodies", () => {
    const tokens = withoutTestItems(
      lexRust(`
    fn keep() { "kept" }
    #[cfg(test)]
    mod tests { fn t() { "test-only" } }
    #[cfg(test)]
    #[allow(unused)]
    fn helper(x: [u8; 2]) -> u8 { "helper-only" }
    #[cfg(test)]
    use crate::testing::Fixture;
    fn also() { "also-kept" }
  `),
    );
    expect(tokens.filter(token => token.kind === "string").map(token => token.value)).toEqual(["kept", "also-kept"]);
    expect(tokens.some(token => token.text === "Fixture")).toBe(false);
  });

  it("reads a literal as itself when identifier-shaped, its parts when a qualified name or path, and the words it quotes", () => {
    expect(literalWords("deque")).toEqual(["deque"]);
    expect(literalWords("collections.abc.Sequence")).toEqual(["collections", "abc", "Sequence"]);
    expect(literalWords("/var/tmp")).toEqual(["var", "tmp"]);
    expect(literalWords("Use `contextlib.suppress` or 'pass' instead")).toEqual(["pass"]);
    expect(literalWords("Do not use bare `except`")).toEqual(["except"]);
    expect(literalWords("two words")).toEqual([]);
  });

  it("reads names, regex patterns and imports outside comments and tests", () => {
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
    // Regex words are read as the check 8 gate reads them, single letters included.
    expect(found.names).toEqual(["List", "_", "collections", "d", "deque", "fixme", "pas", "r", "token", "typing", "wo", "xxx"]);
    expect(found.regexes).toEqual([String.raw`(^|_)(?i)(pas+wo?r?d|token)($|_)`, String.raw`^#\s*(?i)fixme`, String.raw`^#\s*(?i)xxx`]);
    expect(found.uses).toEqual([
      ["ruff_python_semantic", "analyze", "typing", "is_mutable_expr"],
      ["ruff_python_semantic", "analyze", "typing"],
      ["crate", "rules", "flake8_bugbear", "helpers"],
    ]);
  });

  it("reads the words of a substring search's needle", () => {
    const found = rustLiterals(`static FINDER: LazyLock<Finder> = LazyLock::new(|| Finder::new(b"# /// script"));\nlet s = "# /// other";`);
    expect(found.names).toEqual(["script"]);
    expect(found.regexes).toEqual([]);
  });

  it("expands a use tree into every path it imports, aliases dropped", () => {
    const tokens = lexRust("a::{b, c::{d as e, f}, self, g::*}").filter(token => token.text !== "::");
    expect(useTreePaths(tokens)).toEqual([["a", "b"], ["a", "c", "d"], ["a", "c", "f"], ["a"], ["a", "g"]]);
  });

  it("translates a regex crate pattern to JavaScript only where the translation is faithful", () => {
    expect(rustRegex(String.raw`(?i)^(?P<name>\pL+)\.py$`)).toEqual(/^(?<name>\p{L}+)\.py$/iu);
    expect(rustRegex(String.raw`\x{1F600}[]a-]`)?.source).toBe(String.raw`\u{1F600}[\]a-]`);
    expect(rustRegex(String.raw`\Afoo`)).toBeUndefined();
    expect(rustRegex("(?x)a b")).toBeUndefined();
    expect(rustRegex("a(?i)b")).toBeUndefined();
  });
});

describe("the reader of Ruff's test modules", () => {
  const variants: RuleVariants = {
    nameOf: variant => ({ NoneComparison: "none-comparison", UnusedImport: "unused-import" })[variant],
    codeOf: variant => ({ NoneComparison: "E711", UnusedImport: "F401" })[variant],
  };

  it("folds the path expressions Ruff's tests build", () => {
    const path = (text: string) => evaluatePath(lexRust(text), variants);
    expect(path(`Path::new("pycodestyle").join(Path::new("E711.py")).as_path()`)).toBe("pycodestyle/E711.py");
    expect(path(`&Path::new("pyflakes").join(format!("{}_{}.py", Rule::UnusedImport.noqa_code(), "0"))`)).toBe("pyflakes/F401_0.py");
    expect(path(`PathBuf::from_iter(["pep8_naming", "ignore_names", "N801.py"]).as_path()`)).toBe("pep8_naming/ignore_names/N801.py");
    expect(path(`test_resource_path("fixtures").join("pycodestyle/W391.ipynb")`)).toBe("resources/test/fixtures/pycodestyle/W391.ipynb");
    expect(path(`Path::new(&format!("{name}.py"))`)).toBeUndefined();
    expect(path(`Path::new(dir).join("x.py")`)).toBeUndefined();
  });

  it("finds each fixture a test lints once, with test-case arguments and let bindings substituted", () => {
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
    const lints = ruffTestLints("pycodestyle", modRs, variants).map(lint => ({ test: lint.test, fixture: lint.fixture, settings: tokenText(lint.settings) }));
    expect(lints).toEqual([
      { test: "pycodestyle::rules", fixture: "resources/test/fixtures/pycodestyle/E711.py", settings: "LinterSettings::for_rule((Rule::NoneComparison))" },
      { test: "pycodestyle::rules", fixture: "resources/test/fixtures/pycodestyle/F401_0.py", settings: "LinterSettings::for_rule((Rule::UnusedImport))" },
      {
        test: "pycodestyle::with_settings",
        fixture: "resources/test/fixtures/pyflakes/F401_1.py",
        settings: "LinterSettings { preview : PreviewMode::Enabled, ..LinterSettings::for_rules(vec![Rule::UnusedImport])}",
      },
      { test: "pycodestyle::diff", fixture: "resources/test/fixtures/pyflakes/F401_2.py", settings: "LinterSettings::for_rule(Rule::UnusedImport)" },
      { test: "pycodestyle::diff", fixture: "resources/test/fixtures/pyflakes/F401_2.py", settings: "LinterSettings::for_rule(Rule::UnusedImport).with_preview_mode()" },
      { test: "pycodestyle::notebook", fixture: "resources/test/fixtures/pycodestyle/W391.ipynb", settings: "LinterSettings::for_rules(Linter::Pyflakes.rules())" },
    ]);
  });

  it("reads test_contents of a fixture file read from disk, and skips inline sources", () => {
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
    const lints = ruffTestLints("ruff", modRs, variants).map(lint => ({ fixture: lint.fixture, rules: enabledSelections(lint.settings).rules }));
    expect(lints).toEqual([{ fixture: "resources/test/fixtures/ruff/RUF050_F401.py", rules: ["NoneComparison", "UnusedImport"] }]);
  });

  it("reads linter.rs's shapes: its path helper inlined, files read for test_contents_syntax_errors and notebooks", () => {
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
    const lints = ruffTestLints("linter", linterRs, variants).map(lint => ({ test: lint.test, fixture: lint.fixture, rules: enabledSelections(lint.settings).rules }));
    expect(lints).toEqual([
      { test: "linter::test_undefined_name", fixture: "../ruff_notebook/resources/test/fixtures/jupyter/undefined_name.ipynb", rules: ["UndefinedName"] },
      { test: "linter::test_cell_id", fixture: "../ruff_notebook/resources/test/fixtures/jupyter/no_cell_id.ipynb", rules: ["UnusedImport"] },
      { test: "linter::test_syntax_errors", fixture: "resources/test/fixtures/syntax_errors/yield_scope.py", rules: ["YieldOutsideFunction"] },
      { test: "linter::test_await_scope_notebook", fixture: "resources/test/fixtures/syntax_errors/await_scope.ipynb", rules: ["YieldOutsideFunction"] },
    ]);
  });

  it("reads the rules and whole linters a settings expression turns on", () => {
    expect(enabledSelections(lexRust("LinterSettings { x: 1, ..LinterSettings::for_rules(vec![Rule::A, Rule::B]) }"))).toEqual({ rules: ["A", "B"], linters: [] });
    expect(enabledSelections(lexRust("LinterSettings::for_rules(Linter::Pyflakes.rules())"))).toEqual({ rules: [], linters: ["Pyflakes"] });
  });

  it("gives Ruff's rule names", () => {
    expect(kebabCase("NoneComparison")).toBe("none-comparison");
    expect(kebabCase("NonPEP585Annotation")).toBe("non-pep585-annotation");
    expect(kebabCase("UnusedImport")).toBe("unused-import");
  });

  it("reads the string literals of a mod.rs test module only", () => {
    const modRs = `const OUTSIDE: &str = "outside";\nmod tests {\n  fn t() { settings(vec!["fastapi.Depends".to_string()]); test_snippet(r#"import os"#); }\n}`;
    expect(testModuleStrings(modRs)).toEqual(["fastapi.Depends", "import os"]);
  });
});

describe("Ruff's facts", () => {
  it("resolves crate, super and external use paths to the longest module file", () => {
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
      expect(useTarget(["crate", "rules", "bugbear", "helpers", "is_x"], rule, crates, own)).toBe(helpers);
      expect(useTarget(["super", "super", "helpers", "is_x"], rule, crates, own)).toBe(helpers);
      expect(useTarget(["ruff_python_semantic", "analyze", "typing", "is_mutable_expr"], rule, crates, own)).toBe(typing);
      expect(useTarget(["ruff_python_semantic", "SemanticModel"], rule, crates, own)).toBe(lib);
      expect(useTarget(["std", "fmt"], rule, crates, own)).toBeUndefined();
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  });

  it("reads single and multi-line --show-settings values, and the default selection's codes", () => {
    const settings = parseShowSettings(
      [
        "# Linter Settings",
        "linter.rules.enabled = [",
        "\tunused-import (F401),",
        "\tnone-comparison (E711),",
        "]",
        "linter.unresolved_target_version = none",
        "linter.flake8_bugbear.extend_immutable_calls = []",
        "formatter.unresolved_target_version = 3.10",
      ].join("\n"),
    );
    expect(settings.get("linter.unresolved_target_version")).toBe("none");
    expect(settings.get("formatter.unresolved_target_version")).toBe("3.10");
    expect(settings.get("linter.flake8_bugbear.extend_immutable_calls")).toBe("[]");
    expect(enabledCodes(settings.get("linter.rules.enabled")!)).toEqual(["E711", "F401"]);
  });

  it("maps a rule's documented options to --show-settings keys", () => {
    const explanation = "## What it does\nx\n\n## Options\n- `lint.flake8-bugbear.extend-immutable-calls`\n- [`target-version`]\n\n## References\n- `not.an.option`\n";
    expect(documentedOptions(explanation)).toEqual(["lint.flake8-bugbear.extend-immutable-calls", "target-version"]);
    expect(documentedOptions("## Options\n- `lint.pyflakes.extend-generics`")).toEqual(["lint.pyflakes.extend-generics"]);
    expect(settingsKey("lint.flake8-bugbear.extend-immutable-calls")).toBe("linter.flake8_bugbear.extend_immutable_calls");
    expect(settingsKey("lint.dummy-variable-rgx")).toBe("linter.dummy_variable_rgx");
    expect(settingsKey("target-version")).toBe("linter.unresolved_target_version");
  });

  it("reports instability only for a rule that is not stable", () => {
    expect(instability({ Stable: { since: "v0.0.92" } })).toBeNull();
    expect(instability({ Preview: { since: "0.5.0" } })).toBe("preview since 0.5.0 (ruff rule --all)");
    expect(instability({ Removed: { since: "0.8.0" } })).toBe("removed since 0.8.0 (ruff rule --all)");
  });

  it("reads the struct `ruff rule --all` points at", () => {
    const source = 'use x;\n\n/// ## What it does\n#[derive(ViolationMetadata)]\n#[violation_metadata(stable_since = "v0.0.1")]\npub(crate) struct BreakOutsideLoop;\n\npub struct Other;\n';
    expect(violationStruct(source, 4)).toBe("BreakOutsideLoop");
    expect(violationStruct(source, 7)).toBe("Other");
  });

  it("finds where code builds a violation, not where it imports it or checks it is enabled", () => {
    const raises = (code: string, struct = "ForwardAnnotationSyntaxError") => raiseSites(lexRust(code), struct).length > 0;
    expect(raises("self.report_diagnostic(pyflakes::rules::ForwardAnnotationSyntaxError { parse_error }, range);")).toBe(true);
    expect(raises("checker.report_diagnostic(pyflakes::rules::BreakOutsideLoop, stmt.range());", "BreakOutsideLoop")).toBe(true);
    expect(raises("self.report_diagnostic(YieldOutsideFunction::new(kind), error.range);", "YieldOutsideFunction")).toBe(true);
    expect(raises("if self.is_rule_enabled(Rule::ForwardAnnotationSyntaxError) { visit(); }")).toBe(false);
    expect(raises("use crate::rules::pyflakes::rules::{ForwardAnnotationSyntaxError, Other};")).toBe(false);
    expect(raises('// ForwardAnnotationSyntaxError { }\nlet s = "ForwardAnnotationSyntaxError {";')).toBe(false);
  });

  it("tells a violation raised for a parser-detected semantic error from one the checker detects", () => {
    const code = `fn report_semantic_error(&self, error: SemanticSyntaxError) {
    if self.semantic.in_string_type_definition() { self.report(ForwardAnnotationSyntaxError { parse_error: error.to_string() }, error.range); return; }
    match error.kind {
      SemanticSyntaxErrorKind::LateFutureImport => { if self.is_rule_enabled(Rule::LateFutureImport) { self.report(LateFutureImport, error.range); } }
      SemanticSyntaxErrorKind::FutureFeatureNotDefined(name) => { if self.on() { self.report(pyflakes::rules::FutureFeatureNotDefined { name }, error.range); } }
      SemanticSyntaxErrorKind::A | SemanticSyntaxErrorKind::B => {}
    }
  }`;
    const tokens = lexRust(code);
    const arm = (struct: string) => raiseSites(tokens, struct).some(index => inSemanticErrorArm(tokens, index));
    expect(arm("LateFutureImport")).toBe(true);
    expect(arm("FutureFeatureNotDefined")).toBe(true);
    expect(arm("ForwardAnnotationSyntaxError")).toBe(false);
  });
});

describe("Bandit's facts", () => {
  it("translates a compiled Python regex to JavaScript only where the translation is faithful", () => {
    // re.IGNORECASE | re.UNICODE
    expect(pythonRegex("(?P<word>pas+wo?r?d)(?P=word)", 2 | 32)).toEqual(/(?<word>pas+wo?r?d)\k<word>/i);
    // re.VERBOSE has no JavaScript flag; \Z and inline flag groups have no equivalent.
    expect(pythonRegex("a b", 64 | 32)).toBeUndefined();
    expect(pythonRegex(String.raw`foo\Z`, 32)).toBeUndefined();
    expect(pythonRegex("(?i:foo)", 32)).toBeUndefined();
  });
});

describe("the recorded source facts", () => {
  const loaded = loadPack(packDirectory("python"));

  it.each(loaded.pack.tools.map(tool => [tool.name, tool] as const))("%s: source.json agrees with the pack's pins and covers every port", (name, tool) => {
    const source = loadUpstreamSource(upstreamSourcePath(loaded, name));
    const recording = loadRecording(fixturesRecordingPath(loaded, name));
    expect(recording.source.kind).toBe("fixtures");
    expect(source.tool).toBe(name);
    expect(source.version).toBe(tool.version);
    expect(recording.version).toBe(tool.version);
    expect(source.configurations).toEqual(tool.recommended);
    if (recording.source.kind === "fixtures") expect(parseOrigin(recording.source.origin)).toEqual({ repository: source.repository, commit: source.commit });
    const ported = loaded.pack.rules.filter(rule => rule.upstream.tool === name).map(rule => rule.upstream.rule);
    expect(Object.keys(source.rules).sort()).toEqual([...new Set(ported)].sort());
  });
});
