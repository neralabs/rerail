import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, posix, resolve } from "node:path";
import type { UpstreamRule, UpstreamSource } from "../languages/pack.js";
import type { PythonToolkit } from "./python-toolkit.js";
import { enabledSelections, kebabCase, ruffTestLints, type RuleVariants } from "./ruff-tests.js";
import { closingIndex, lexRust, rustLiterals, rustRegex, withoutTestItems, type RustLiterals, type RustToken } from "./rust-source.js";
import { distinctRegexes, filesUnder, run, sortedSet, unscoredTests, upstreamPath, type UpstreamPin } from "./upstream-source.js";

/**
 * Ruff's source facts (`upstream/ruff/source.json`), read from Ruff's checkout at the pinned commit and from the pinned
 * Ruff itself, run as `ruff check --isolated` with every RUFF_* variable unset: no configuration file, Ruff's default
 * options and target version. A rule's tests are the fixtures that the test functions of crates/ruff_linter's
 * rules/<linter>/mod.rs and linter.rs lint with settings that enable it. Ported from nera-rule-validation's
 * src/validation/adapters/ruff.ts. A test the fixtures recording does not copy is `unscored`: a notebook, a file the
 * tests lint only with other settings than Ruff's defaults (options, preview, a target version), or one they lint with
 * Ruff's defaults for the rules they select, which the recorder could copy.
 */

/** The sparse checkout the facts need. */
export const RUFF_SPARSE_PATHS = [
  "/LICENSE",
  "/pyproject.toml",
  "/crates/ruff_linter/",
  // linter.rs's notebook tests lint ruff_notebook's fixtures.
  "/crates/ruff_notebook/resources/test/fixtures/",
  "/crates/ruff_python_ast/src/",
  "/crates/ruff_python_semantic/src/",
  "/crates/ruff_python_stdlib/src/",
  "/crates/ruff_python_trivia/src/",
];
/** The crates every Ruff rule reads Python through: the AST, the semantic model, and the standard-library tables. */
const SHARED_CRATES = ["ruff_python_ast", "ruff_python_semantic", "ruff_python_stdlib"];
const LINTER = "crates/ruff_linter";
/** The ruff_linter sources whose `mod tests` lint fixture files with rules enabled, besides rules/<linter>/mod.rs. */
const LINTER_TEST_MODULES = ["src/linter.rs"];
/** Ruff's parser-side semantic checks, which detect what some rules report; read from the pinned commit, outside the sparse checkout. */
const PARSER_SEMANTIC_ERRORS = "crates/ruff_python_parser/src/semantic_errors.rs";
const FIXTURES = "resources/test/fixtures";
/**
 * What Ruff lints, as the rule validation ladder's V1 probes it: .py and .pyi files (and notebooks, which no dialect
 * reads, so check 1 does not ask a port to cover them).
 */
const EXTENSIONS = [".py", ".pyi"];

/** Whether a test's settings are Ruff's defaults with only the rules selected: `LinterSettings::for_rule(s)(...)`. */
export function selectsOnly(settings: readonly RustToken[]): boolean {
  const start = settings[0]?.text === "settings" && settings[1]?.text === "::" ? 2 : 0;
  if (settings[start]?.text !== "LinterSettings" || settings[start + 1]?.text !== "::") return false;
  if (!["for_rule", "for_rules"].includes(settings[start + 2]?.text ?? "") || settings[start + 3]?.text !== "(") return false;
  return closingIndex(settings, start + 3) === settings.length - 1;
}

interface RuffRule {
  name: string;
  code: string;
  linter: string | null;
  status: Record<string, { since: string }>;
  explanation: string;
  source_location: { file: string; line: number } | null;
}

/** What `ruff check --show-settings` prints, as `key = value` with multi-line values joined. */
export function parseShowSettings(text: string): Map<string, string> {
  const settings = new Map<string, string>();
  const lines = text.split("\n");
  for (let index = 0; index < lines.length; index += 1) {
    const match = /^([a-z_][\w.]*) = (.*)$/u.exec(lines[index]!);
    if (match === null) continue;
    let value = match[2]!;
    if (value === "[" || value === "{") {
      const close = value === "[" ? "]" : "}";
      const items: string[] = [];
      while (index + 1 < lines.length && lines[index + 1] !== close) {
        index += 1;
        items.push(lines[index]!.trim());
      }
      index += 1;
      value = `${value}${items.join(" ")}${close}`;
    }
    settings.set(match[1]!, value);
  }
  return settings;
}

/** The rule codes of `linter.rules.enabled = [name (CODE), ...]`. */
export function enabledCodes(value: string): string[] {
  return [...value.matchAll(/\(([A-Z]+[0-9]+)\)/gu)].map(match => match[1]!).sort();
}

/** The option names a rule's documentation lists under `## Options` (`lint.flake8-bugbear.extend-immutable-calls`). */
export function documentedOptions(explanation: string): string[] {
  const section = /^## Options\n([\s\S]*?)(?=^## |(?![\s\S]))/mu.exec(explanation)?.[1] ?? "";
  return [...section.matchAll(/^- \[?`([^`]+)`\]?/gmu)].map(match => match[1]!);
}

/** The `--show-settings` key that holds a documented option: `lint.pyflakes.extend-generics` is `linter.pyflakes.extend_generics`. */
export function settingsKey(option: string): string {
  if (option === "target-version") return "linter.unresolved_target_version";
  return `linter.${option.replace(/^lint\./u, "").replaceAll("-", "_")}`;
}

/** Ruff's status for a rule as a reason it is not stable, or null when it is stable. */
export function instability(status: Record<string, { since: string }>): string | null {
  const [kind, detail] = Object.entries(status)[0] ?? ["Unknown", { since: "?" }];
  return kind === "Stable" ? null : `${kind.toLowerCase()} since ${detail.since} (ruff rule --all)`;
}

/** The code cells of a notebook joined, as the words of upstream's notebook test files. */
function notebookCode(text: string): string {
  try {
    const notebook = JSON.parse(text) as { cells?: { cell_type?: string; source?: string | string[] }[] };
    return (notebook.cells ?? [])
      .filter(cell => cell.cell_type === "code")
      .map(cell => (Array.isArray(cell.source) ? cell.source.join("") : (cell.source ?? "")))
      .join("\n");
  } catch {
    return text;
  }
}

/** The string literal values inside a mod.rs `mod tests`: test option values and inline test sources. */
export function testModuleStrings(modRs: string): string[] {
  const tokens = lexRust(modRs);
  const start = tokens.findIndex((token, index) => token.text === "mod" && tokens[index + 1]?.text === "tests" && tokens[index + 2]?.text === "{");
  if (start === -1) return [];
  return tokens
    .slice(start + 3, closingIndex(tokens, start + 2))
    .filter(token => token.kind === "string")
    .map(token => token.value ?? "");
}

/**
 * The Rust source files a `use` path names, trying the longest module prefix first (`a::b::c` is `a/b/c.rs`,
 * `a/b/c/mod.rs`, then `a/b.rs`, ...). `crate`, `self` and `super` resolve from the importing file's module.
 */
export function useTarget(path: string[], importer: string, crateRoots: Map<string, string>, ownCrateRoot: string): string | undefined {
  let base: string;
  let rest: string[];
  const childrenOf = (file: string) => (/(?:^|\/)(?:mod|lib|main)\.rs$/u.test(file) ? dirname(file) : file.replace(/\.rs$/u, ""));
  if (path[0] === "crate") {
    base = ownCrateRoot;
    rest = path.slice(1);
  } else if (path[0] === "super" || path[0] === "self") {
    base = childrenOf(importer);
    rest = [...path];
    while (rest[0] === "super" || rest[0] === "self") {
      if (rest[0] === "super") base = dirname(base);
      rest = rest.slice(1);
    }
  } else if (crateRoots.has(path[0]!)) {
    base = crateRoots.get(path[0]!)!;
    rest = path.slice(1);
  } else {
    return undefined;
  }
  for (let length = rest.length; length >= 1; length -= 1) {
    const module = join(base, ...rest.slice(0, length));
    if (existsSync(`${module}.rs`)) return `${module}.rs`;
    if (existsSync(join(module, "mod.rs"))) return join(module, "mod.rs");
  }
  return existsSync(join(base, "lib.rs")) ? join(base, "lib.rs") : existsSync(join(base, "mod.rs")) ? join(base, "mod.rs") : undefined;
}

/** The violation struct `ruff rule --all` locates at a 1-based line of its source file: the first struct declared from there. */
export function violationStruct(source: string, line: number): string | undefined {
  return /\bstruct\s+([A-Za-z_]\w*)/u.exec(source.split("\n").slice(line - 1).join("\n"))?.[1];
}

/**
 * Where Rust code raises a violation, by token index: it builds the struct (`Struct { .. }`, `Struct::new(..)`, or a
 * unit struct passed on as `Struct,` or `Struct)`). Naming it in a `use` declaration or as `Rule::Struct` (an
 * enablement check) is not raising it.
 */
export function raiseSites(tokens: readonly RustToken[], struct: string): number[] {
  const sites: number[] = [];
  let inUse = false;
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index]!;
    if (token.kind === "ident" && token.text === "use") inUse = true;
    else if (token.kind === "punct" && token.text === ";") inUse = false;
    else if (
      !inUse &&
      token.kind === "ident" &&
      token.text === struct &&
      !(tokens[index - 1]?.text === "::" && tokens[index - 2]?.text === "Rule") &&
      (["{", ",", ")"].includes(tokens[index + 1]?.text ?? "") ||
        (tokens[index + 1]?.text === "::" && tokens[index + 2]?.kind === "ident" && tokens[index + 3]?.text === "("))
    )
      sites.push(index);
  }
  return sites;
}

/**
 * Whether the token at `index` sits in a match arm on `SemanticSyntaxErrorKind`: the checker reports the violation for
 * an error that Ruff's parser detects (crates/ruff_python_parser/src/semantic_errors.rs), which then implements it.
 */
export function inSemanticErrorArm(tokens: readonly RustToken[], index: number): boolean {
  const opens = (token: RustToken) => token.kind === "punct" && "([{".includes(token.text);
  const closes = (token: RustToken) => token.kind === "punct" && ")]}".includes(token.text);
  let depth = 0;
  for (let cursor = index - 1; cursor >= 0; cursor -= 1) {
    const token = tokens[cursor]!;
    if (closes(token)) depth += 1;
    else if (opens(token) && depth > 0) depth -= 1;
    else if (opens(token) && token.text === "{" && tokens[cursor - 1]?.text === "=>") {
      // The arm's pattern runs back from `=>` to the `,`, `{` or `}` that ends the previous arm.
      let start = cursor - 2;
      let nested = 0;
      for (; start >= 0; start -= 1) {
        const part = tokens[start]!;
        if (closes(part)) {
          if (nested === 0 && part.text === "}") break;
          nested += 1;
        } else if (opens(part)) {
          if (nested === 0) break;
          nested -= 1;
        } else if (nested === 0 && part.text === ",") break;
      }
      if (tokens.slice(start + 1, cursor - 1).some(part => part.text === "SemanticSyntaxErrorKind")) return true;
    }
  }
  return false;
}

/** The process environment without the variables Ruff reads its configuration from. */
function scrubbedEnvironment(): NodeJS.ProcessEnv {
  return Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("RUFF_")));
}

/**
 * Ruff's facts for the ported rule codes, from `checkout` (Ruff's repository at `pin.commit`, at least
 * `RUFF_SPARSE_PATHS`) and `ruff`, the pinned executable.
 */
export function ruffSource(
  checkout: string,
  ruff: string,
  toolkit: PythonToolkit,
  codes: string[],
  pin: UpstreamPin & { version: string },
  configurations: string[],
  recorded: readonly string[],
): UpstreamSource {
  if (configurations.join(",") !== "default") {
    throw new Error("Ruff has no named configurations; tools[].recommended must be [default], Ruff's default rule selection");
  }
  // A checkout made with fewer sparse paths would silently lack files the facts read.
  const absent = RUFF_SPARSE_PATHS.filter(path => !existsSync(join(checkout, path)));
  if (absent.length > 0) throw new Error(`the Ruff checkout at ${checkout} lacks ${absent.join(", ")}`);
  const linterDirectory = join(checkout, LINTER);
  const environment = scrubbedEnvironment();
  const ruffRun = (args: string[], cwd: string) => run(ruff, args, { cwd, env: environment });

  const version = ruffRun(["version"], linterDirectory).trim();
  if (!version.startsWith(`ruff ${pin.version} (${pin.commit.slice(0, 9)} `)) {
    throw new Error(`${ruff} reports \`${version}\`; the pack pins Ruff ${pin.version} at ${pin.commit.slice(0, 9)}`);
  }
  const allRules = JSON.parse(ruffRun(["rule", "--all", "--output-format", "json"], linterDirectory)) as RuffRule[];
  const scratch = mkdtempSync(join(tmpdir(), "ruff-settings-"));
  let settings: Map<string, string>;
  try {
    writeFileSync(join(scratch, "settings.py"), "");
    settings = parseShowSettings(ruffRun(["check", "--isolated", "--no-cache", "--show-settings", "settings.py"], scratch));
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
  const defaultSelection = new Set(enabledCodes(settings.get("linter.rules.enabled") ?? ""));
  // Without a configured target version, Ruff lints at PythonVersion::default(), which --show-settings prints as the
  // formatter's resolved version (both are `target_version.unwrap_or_default()`).
  const unresolvedTarget = settings.get("linter.unresolved_target_version") ?? "none";
  const lintTarget = unresolvedTarget === "none" ? (settings.get("formatter.unresolved_target_version") ?? "?") : unresolvedTarget;

  const ruleByCode = new Map(allRules.map(entry => [entry.code, entry]));
  const ruleByName = new Map(allRules.map(entry => [entry.name, entry]));
  const variants: RuleVariants = {
    nameOf: variant => ruleByName.get(kebabCase(variant))?.name,
    codeOf: variant => ruleByName.get(kebabCase(variant))?.code,
  };
  const normalizedLinter = (label: string) => label.toLowerCase().replace(/-specific rules$/u, "").replace(/[^a-z0-9]/gu, "");
  const linterCodes = (variant: string) =>
    allRules.filter(entry => entry.linter !== null && normalizedLinter(entry.linter) === normalizedLinter(variant)).map(entry => entry.code);

  // Every file Ruff's tests lint, with the codes the test's settings enable.
  const rulesDirectory = join(linterDirectory, "src", "rules");
  const modules = readdirSync(rulesDirectory, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && existsSync(join(rulesDirectory, entry.name, "mod.rs")))
    .map(entry => entry.name)
    .sort();
  const testModules = [
    ...modules.map(module => ({ module, file: join(rulesDirectory, module, "mod.rs") })),
    ...LINTER_TEST_MODULES.map(file => ({ module: posix.basename(file, ".rs"), file: join(linterDirectory, file) })),
  ];
  const lints = testModules.flatMap(({ module, file }) => ruffTestLints(module, readFileSync(file, "utf8"), variants));
  const testsOf = new Map<string, Set<string>>();
  const withDefaults = new Map<string, Set<string>>();
  for (const lint of lints) {
    // A file the reader cannot find would silently drop a test case, so it stops the run instead.
    if (lint.fixture === undefined) throw new Error(`${lint.test} lints a file whose path the recorder cannot evaluate`);
    if (!existsSync(join(linterDirectory, lint.fixture))) throw new Error(`${lint.test} lints ${lint.fixture}, which the Ruff checkout lacks`);
    const selection = enabledSelections(lint.settings);
    for (const code of [...selection.rules.flatMap(variant => variants.codeOf(variant) ?? []), ...selection.linters.flatMap(linterCodes)]) {
      testsOf.set(code, (testsOf.get(code) ?? new Set()).add(lint.fixture));
      if (selectsOnly(lint.settings)) withDefaults.set(code, (withDefaults.get(code) ?? new Set()).add(lint.fixture));
    }
  }

  // Upstream's test words are every fixture file, the files the tests lint from elsewhere (ruff_notebook's notebooks),
  // and every string literal of the test modules read above.
  const fixtureFiles = [
    ...new Set([...filesUnder(join(linterDirectory, FIXTURES), path => /\.(?:py|pyi|ipynb)$/u.test(path)), ...lints.map(lint => resolve(linterDirectory, lint.fixture!))]),
  ].sort();
  const testWords = toolkit.words([
    ...fixtureFiles.map(path => (path.endsWith(".ipynb") ? notebookCode(readFileSync(path, "utf8")) : readFileSync(path, "utf8"))),
    ...testModules.flatMap(({ file }) => testModuleStrings(readFileSync(file, "utf8"))),
  ]);

  const literalCache = new Map<string, RustLiterals>();
  const literalsOf = (file: string): RustLiterals => {
    let found = literalCache.get(file);
    if (found === undefined) {
      found = rustLiterals(readFileSync(file, "utf8"));
      literalCache.set(file, found);
    }
    return found;
  };
  const crateRoots = new Map(SHARED_CRATES.map(crate => [crate, join(checkout, "crates", crate, "src")]));
  const sharedCrateFiles = SHARED_CRATES.flatMap(crate => filesUnder(crateRoots.get(crate)!, path => path.endsWith(".rs")));
  const implementationFile = (code: string) => {
    const location = ruleByCode.get(code)?.source_location;
    return location === null || location === undefined ? undefined : join(checkout, location.file);
  };
  // Some rules (most of Pyflakes') are raised by the checker itself rather than by their own module, which then holds
  // only the violation: the checker files that raise a rule's violation are part of its implementation, and so are
  // Ruff's parser-side semantic checks when the checker raises it for a semantic syntax error the parser detected.
  const checkerFiles = filesUnder(join(linterDirectory, "src", "checkers"), path => path.endsWith(".rs"));
  const checkerTokens = new Map(checkerFiles.map(file => [file, withoutTestItems(lexRust(readFileSync(file, "utf8")))]));
  const semanticErrorsFile = join(checkout, PARSER_SEMANTIC_ERRORS);
  literalCache.set(semanticErrorsFile, rustLiterals(run("git", ["-C", checkout, "show", `${pin.commit}:${PARSER_SEMANTIC_ERRORS}`])));
  const implementationFiles = (code: string): string[] => {
    const file = implementationFile(code);
    if (file === undefined) return [];
    const struct = violationStruct(readFileSync(file, "utf8"), ruleByCode.get(code)!.source_location!.line);
    const sites = struct === undefined ? [] : checkerFiles.map(checker => ({ checker, at: raiseSites(checkerTokens.get(checker)!, struct) })).filter(({ at }) => at.length > 0);
    const parserDetected = sites.some(({ checker, at }) => at.some(index => inSemanticErrorArm(checkerTokens.get(checker)!, index)));
    return [file, ...sites.map(({ checker }) => checker), ...(parserDetected ? [semanticErrorsFile] : [])];
  };
  /** The ruff_linter rule modules the implementation imports (its linter's helpers, other rules' modules); plumbing is not delegation. */
  const importedRuleModules = (file: string): string[] => [
    ...new Set(
      literalsOf(file)
        .uses.map(path => useTarget(path, file, crateRoots, join(linterDirectory, "src")))
        .filter((target): target is string => target !== undefined && target !== file && target.startsWith(`${rulesDirectory}/`)),
    ),
  ].sort();
  const regexesOf = (files: string[]) => files.flatMap(file => literalsOf(file).regexes.flatMap(pattern => rustRegex(pattern) ?? []));

  const pragmas = join(checkout, "crates/ruff_python_trivia/src/pragmas.rs");
  const noqa = join(linterDirectory, "src/noqa.rs");
  const directiveWords = [...literalsOf(pragmas).names, ...literalsOf(noqa).names.filter(word => /^[a-z0-9]+$/u.test(word))];
  const sharedNames = sortedSet(sharedCrateFiles.flatMap(file => literalsOf(file).names));
  const everyRuleShares = new Set(sharedNames);

  const recommendedOptions = (entry: RuffRule): Record<string, string>[] =>
    documentedOptions(entry.explanation).map(option => ({
      [option]: option === "target-version" ? lintTarget : (settings.get(settingsKey(option)) ?? "(not printed by --show-settings)"),
    }));

  // The recording's copies, by the upstream path they were copied from.
  const copied = new Set(recorded.map(file => posix.join(LINTER, FIXTURES, file)));
  const rule = (code: string): UpstreamRule => {
    const entry = ruleByCode.get(code)!;
    const implementation = implementationFiles(code);
    const ownFile = implementationFile(code);
    const imported = ownFile === undefined ? [] : importedRuleModules(ownFile);
    const tests = sortedSet([...(testsOf.get(code) ?? [])].map(fixture => posix.join(LINTER, fixture)));
    const defaults = new Set([...(withDefaults.get(code) ?? [])].map(fixture => posix.join(LINTER, fixture)));
    return {
      implementation: sortedSet(implementation.map(file => upstreamPath(checkout, file))),
      tests,
      deprecated: instability(entry.status),
      enabledBy: defaultSelection.has(code) ? { configuration: "default", options: recommendedOptions(entry) } : null,
      names: {
        implementation: sortedSet(implementation.flatMap(file => literalsOf(file).names)),
        shared: sortedSet(imported.flatMap(file => literalsOf(file).names).filter(name => !everyRuleShares.has(name))),
      },
      regexes: distinctRegexes(regexesOf([...implementation, ...imported, ...sharedCrateFiles])),
      unscored: unscoredTests(
        tests
          .filter(test => !copied.has(test))
          .map(test => ({ test, case: null, reason: test.endsWith(".ipynb") ? "notebook" : defaults.has(test) ? "not-copied" : "settings" })),
      ),
    };
  };

  // A ported code Ruff does not have gets no entry, which check 7 reports as provenance:upstream-rule.
  const known = sortedSet(codes.filter(code => ruleByCode.has(code)));
  return {
    tool: "ruff",
    version: pin.version,
    repository: pin.repository,
    commit: pin.commit,
    tag: pin.tag,
    configurations,
    extensions: sortedSet(EXTENSIONS),
    keywords: sortedSet(toolkit.keywords),
    // Ruff reads `# noqa`, `# ruff: ...` and the other pragma comments it recognizes, read from its pinned source.
    commentVocabulary: sortedSet([...toolkit.commentVocabulary, ...directiveWords]),
    testWords: sortedSet(testWords),
    configurationWords: [],
    sharedNames,
    rules: Object.fromEntries(known.map(code => [code, rule(code)])),
  };
}
