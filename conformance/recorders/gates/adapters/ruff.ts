import { existsSync, readFileSync, readdirSync, realpathSync } from "node:fs";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { dirname, join, posix, relative, resolve, sep } from "node:path";

import { PACKAGE_ROOT } from "../../../../src/engine/index.js";
import type { CorpusFile } from "../../../gates/corpus.js";
import { runChecked, type CommandRunner } from "../../../gates/process.js";
import { notebookCodeCells } from "../../../gates/recorded-adapter.js";
import { diagnosticSpan, RUFF_SPAN_PARTS, ruffSpan } from "../../../gates/span-units/ruff.js";
import type { FixtureTextSource, PackAdapter, PackManifest, Span, UpstreamCase, UpstreamFile, UpstreamRule } from "../../../gates/types.js";
import { createPythonToolkit } from "../languages/python.js";
import { closingIndex, lexRust, rustLiterals, rustRegex, withoutTestItems, type RustLiterals, type RustToken } from "../languages/rust.js";
import { checkoutUpstream, filesUnder, uvRuntime } from "../runtimes.js";
import { enabledSelections, kebabCase, ruffTestLints, type RuleVariants } from "./ruff-tests.js";

/**
 * The Ruff adapter. Expected answers come from running the pinned Ruff wheel itself, as `ruff check --isolated
 * --no-cache --select <CODE>` on explicit file paths with every RUFF_* variable unset: no configuration file, Ruff's
 * default options and target version, `# noqa` honoured. A rule's test files are the fixtures that the test functions
 * of crates/ruff_linter's rules/<linter>/mod.rs and linter.rs lint with settings that enable it; each is linted with the rule
 * alone and the recommended (default) options, whatever options its own test sets.
 */

const SPARSE_PATHS = [
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

interface RuffRule {
  name: string;
  code: string;
  linter: string | null;
  preview: boolean;
  status: Record<string, { since: string }>;
  explanation: string;
  source_location: { file: string; line: number } | null;
}

interface RuffDiagnostic {
  code: string | null;
  filename: string;
  cell: number | null;
  message: string;
  location: { row: number; column: number } | null;
  end_location: { row: number; column: number } | null;
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
  return [...value.matchAll(/\(([A-Z]+[0-9]+)\)/gu)].map((match) => match[1]!).sort();
}

/** The option names a rule's documentation lists under `## Options` (`lint.flake8-bugbear.extend-immutable-calls`). */
export function documentedOptions(explanation: string): string[] {
  const section = /^## Options\n([\s\S]*?)(?=^## |(?![\s\S]))/mu.exec(explanation)?.[1] ?? "";
  return [...section.matchAll(/^- \[?`([^`]+)`\]?/gmu)].map((match) => match[1]!);
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

function compareSpans(a: Span, b: Span): number {
  return a.startLine - b.startLine || (a.startCol ?? 0) - (b.startCol ?? 0) || (a.endLine ?? 0) - (b.endLine ?? 0) || (a.endCol ?? 0) - (b.endCol ?? 0);
}

/** The string literal values inside a mod.rs `mod tests`: test option values and inline test sources. */
export function testModuleStrings(modRs: string): string[] {
  const tokens = lexRust(modRs);
  const start = tokens.findIndex((token, index) => token.text === "mod" && tokens[index + 1]?.text === "tests" && tokens[index + 2]?.text === "{");
  if (start === -1) return [];
  return tokens.slice(start + 3, closingIndex(tokens, start + 2)).filter((token) => token.kind === "string").map((token) => token.value ?? "");
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
    else if (!inUse && token.kind === "ident" && token.text === struct
      && !(tokens[index - 1]?.text === "::" && tokens[index - 2]?.text === "Rule")
      && (["{", ",", ")"].includes(tokens[index + 1]?.text ?? "")
        || (tokens[index + 1]?.text === "::" && tokens[index + 2]?.kind === "ident" && tokens[index + 3]?.text === "("))) sites.push(index);
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
      if (tokens.slice(start + 1, cursor - 1).some((part) => part.text === "SemanticSyntaxErrorKind")) return true;
    }
  }
  return false;
}

/** The process environment without the variables Ruff reads its configuration from. */
function scrubbedEnvironment(): NodeJS.ProcessEnv {
  return Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith("RUFF_")));
}

export async function createRuffAdapter(pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner): Promise<PackAdapter> {
  if (pack.upstream.recommended_configs.join(",") !== "default") {
    throw new Error(`${pack.id}: Ruff has no named configurations; recommended_configs must be [default], Ruff's default rule selection`);
  }
  // Ruff tags releases with the bare version and publishes its wheels from the crates at that tag.
  const upstream = await checkoutUpstream(cacheRoot, "ruff", pack.upstream, pack.upstream.version, "crates", SPARSE_PATHS, runner);
  // A cached checkout made with fewer sparse paths would silently lack files the gates read.
  const absent = SPARSE_PATHS.filter((path) => !existsSync(join(upstream.directory, path)));
  if (absent.length > 0) throw new Error(`the Ruff checkout at ${upstream.directory} lacks ${absent.join(", ")}: it was made with other sparse paths; delete it to fetch it again`);
  const runtime = await uvRuntime(pack.upstream.runtime, cacheRoot, runner);
  const python = await createPythonToolkit(cacheRoot, runner);
  const ruffBinary = join(runtime.directory, ".venv", "bin", "ruff");
  const linterDirectory = join(upstream.directory, LINTER);
  const environment = scrubbedEnvironment();
  const ruff = async (args: string[], cwd: string, accept: number[] = [0]): Promise<string> => {
    const result = await runner.run([ruffBinary, ...args], { cwd, env: environment });
    if (!accept.includes(result.exitCode)) throw new Error(`ruff ${args.slice(0, 6).join(" ")} exited ${result.exitCode}: ${result.stderr.trim().split("\n").slice(-5).join("\n")}`);
    return result.stdout;
  };

  await mkdir(resolve(cacheRoot, "tmp"), { recursive: true });
  const scratch = realpathSync(await mkdtemp(resolve(cacheRoot, "tmp", "ruff-")));
  try {
    const version = (await ruff(["version"], linterDirectory)).trim();
    const allRules = JSON.parse(await ruff(["rule", "--all", "--output-format", "json"], linterDirectory)) as RuffRule[];
    await writeFile(join(scratch, "settings.py"), "");
    const settings = parseShowSettings(await ruff(["check", "--isolated", "--no-cache", "--show-settings", "settings.py"], scratch));
    const defaultSelection = new Set(enabledCodes(settings.get("linter.rules.enabled") ?? ""));
    // Without a configured target version, Ruff lints at PythonVersion::default(), which --show-settings prints as the
    // formatter's resolved version (both are `target_version.unwrap_or_default()`), and parses at PythonVersion::latest().
    const unresolvedTarget = settings.get("linter.unresolved_target_version") ?? "none";
    const lintTarget = unresolvedTarget === "none" ? settings.get("formatter.unresolved_target_version") ?? "?" : unresolvedTarget;
    const pythonVersionSource = readFileSync(join(upstream.directory, "crates/ruff_python_ast/src/python_version.rs"), "utf8");
    const parseTarget = /pub const fn latest\(\) -> Self \{\s*Self::PY3(\d+)/u.exec(pythonVersionSource)?.[1];
    const engineOptions = join(PACKAGE_ROOT, "native/src/code_rule_dsl/scope/options.rs");
    const engineMinor = /DEFAULT_PYTHON_MINOR: u8 = (\d+);/u.exec(readFileSync(engineOptions, "utf8"))?.[1];
    if (engineMinor === undefined) throw new Error(`${engineOptions} declares no DEFAULT_PYTHON_MINOR`);

    const ruleByCode = new Map(allRules.map((entry) => [entry.code, entry]));
    const ruleByName = new Map(allRules.map((entry) => [entry.name, entry]));
    const variants: RuleVariants = {
      nameOf: (variant) => ruleByName.get(kebabCase(variant))?.name,
      codeOf: (variant) => ruleByName.get(kebabCase(variant))?.code,
    };
    const normalizedLinter = (label: string) => label.toLowerCase().replace(/-specific rules$/u, "").replace(/[^a-z0-9]/gu, "");
    const linterCodes = (variant: string) => allRules.filter((entry) => entry.linter !== null && normalizedLinter(entry.linter) === normalizedLinter(variant)).map((entry) => entry.code);

    // Every file Ruff's tests lint, with the codes the test's settings enable.
    const rulesDirectory = join(linterDirectory, "src", "rules");
    const modules = readdirSync(rulesDirectory, { withFileTypes: true })
      .filter((entry) => entry.isDirectory() && existsSync(join(rulesDirectory, entry.name, "mod.rs")))
      .map((entry) => entry.name)
      .sort();
    const testModules = [
      ...modules.map((module) => ({ module, file: join(rulesDirectory, module, "mod.rs") })),
      ...LINTER_TEST_MODULES.map((file) => ({ module: posix.basename(file, ".rs"), file: join(linterDirectory, file) })),
    ];
    const lints = testModules.flatMap(({ module, file }) => ruffTestLints(module, readFileSync(file, "utf8"), variants));
    const testsOf = new Map<string, Map<string, Set<string>>>();
    for (const lint of lints) {
      // A file the reader cannot find would silently drop a test case, so it stops the run instead.
      if (lint.fixture === undefined) throw new Error(`${lint.test} lints a file whose path the adapter cannot evaluate`);
      if (!existsSync(join(linterDirectory, lint.fixture))) throw new Error(`${lint.test} lints ${lint.fixture}, which the Ruff checkout lacks`);
      const selection = enabledSelections(lint.settings);
      const codes = [...selection.rules.flatMap((variant) => variants.codeOf(variant) ?? []), ...selection.linters.flatMap(linterCodes)];
      for (const code of codes) {
        const fixtures = testsOf.get(code) ?? new Map<string, Set<string>>();
        fixtures.set(lint.fixture, (fixtures.get(lint.fixture) ?? new Set()).add(lint.test));
        testsOf.set(code, fixtures);
      }
    }

    const codes = [...new Set(names)].filter((code) => ruleByCode.has(code)).sort();
    const fixturesOf = (code: string) => [...(testsOf.get(code)?.keys() ?? [])].sort();
    const caseFixtures = [...new Set(codes.flatMap(fixturesOf))].sort();

    /** Files Ruff's own parser rejects, read from `ruff format --check` (which reports parse errors only), with the first error. */
    const parseFailures = async (paths: string[], cwd: string): Promise<Map<string, string>> => {
      const failures = new Map<string, string>();
      if (paths.length === 0) return failures;
      const diagnostics = JSON.parse(await ruff(["format", "--isolated", "--no-cache", "--check", "--output-format", "json", ...paths], cwd, [0, 1, 2])) as RuffDiagnostic[];
      for (const diagnostic of diagnostics) {
        if (diagnostic.code !== "invalid-syntax" || failures.has(diagnostic.filename)) continue;
        failures.set(diagnostic.filename, `${diagnostic.message} (line ${diagnostic.location?.row ?? "?"})`);
      }
      return failures;
    };

    /** Ruff's findings with the given codes selected, by code and then by the file's real path. */
    const lint = async (selected: string[], paths: string[], cwd: string): Promise<Map<string, Map<string, Span[]>>> => {
      const findings = new Map<string, Map<string, Span[]>>(selected.map((code) => [code, new Map()]));
      if (paths.length === 0 || selected.length === 0) return findings;
      const diagnostics = JSON.parse(await ruff(["check", "--isolated", "--no-cache", "--exit-zero", "--output-format", "json", "--select", selected.join(","), ...paths], cwd)) as RuffDiagnostic[];
      for (const diagnostic of diagnostics) {
        const byFile = diagnostic.code === null ? undefined : findings.get(diagnostic.code);
        if (byFile === undefined) continue;
        byFile.set(diagnostic.filename, [...(byFile.get(diagnostic.filename) ?? []), diagnosticSpan(diagnostic)]);
      }
      for (const byFile of findings.values()) for (const spans of byFile.values()) spans.sort(compareSpans);
      return findings;
    };

    // matches-upstream-tests: every ported rule's fixtures linted with that rule alone, from the directory cargo runs
    // Ruff's tests in.
    const realFixture = (fixture: string) => realpathSync(join(linterDirectory, fixture));
    const caseFindings = new Map<string, Map<string, Span[]>>();
    for (const code of codes) caseFindings.set(code, (await lint([code], fixturesOf(code), linterDirectory)).get(code)!);
    const fixtureText = new Map(caseFixtures.map((fixture) => [fixture, readFileSync(join(linterDirectory, fixture), "utf8")]));

    // not-overfit: upstream's test words are every fixture file, the files the tests lint from elsewhere
    // (ruff_notebook's notebooks), and every string literal of the test modules read above.
    const fixtureFiles = [...new Set([
      ...filesUnder(join(linterDirectory, FIXTURES), (path) => /\.(?:py|pyi|ipynb)$/u.test(path)),
      ...lints.map((lint) => resolve(linterDirectory, lint.fixture!)),
    ])].sort();
    // Each text's origin, so the gates can rebuild the list: a file by its path in the checkout and the digest of the text
    // as read here; a test-module string literal inline (Ruff's own Rust sources, never a GPL-derived fixture).
    const fixtureTextSources: FixtureTextSource[] = [];
    const fixtureTexts: string[] = [];
    for (const path of fixtureFiles) {
      const raw = readFileSync(path, "utf8");
      const text = path.endsWith(".ipynb") ? notebookCodeCells(raw) : raw;
      const upstreamRelative = relative(upstream.directory, path).split(sep).join("/");
      if (upstreamRelative.startsWith("../")) throw new Error(`${path} is outside the Ruff checkout`);
      fixtureTexts.push(text);
      fixtureTextSources.push({ path: upstreamRelative, sha256: createHash("sha256").update(text).digest("hex"), ...(text === raw ? {} : { transform: "notebook-code-cells" as const }) });
    }
    for (const { file } of testModules) {
      for (const text of testModuleStrings(readFileSync(file, "utf8"))) {
        fixtureTexts.push(text);
        fixtureTextSources.push({ inline: text });
      }
    }
    const fixtureWords = new Set((await python.words(fixtureTexts)).flat());

    const literalCache = new Map<string, RustLiterals>();
    const literalsOf = (file: string): RustLiterals => {
      let found = literalCache.get(file);
      if (found === undefined) {
        found = rustLiterals(readFileSync(file, "utf8"));
        literalCache.set(file, found);
      }
      return found;
    };
    const crateRoots = new Map(SHARED_CRATES.map((crate) => [crate, join(upstream.directory, "crates", crate, "src")]));
    const sharedCrateFiles = SHARED_CRATES.flatMap((crate) => filesUnder(crateRoots.get(crate)!, (path) => path.endsWith(".rs")));
    const implementationFile = (code: string) => {
      const location = ruleByCode.get(code)?.source_location;
      return location === null || location === undefined ? undefined : join(upstream.directory, location.file);
    };
    // Some rules (most of Pyflakes') are raised by the checker itself rather than by their own module, which then holds
    // only the violation: the checker files that raise a rule's violation are part of its implementation, and so are
    // Ruff's parser-side semantic checks when the checker raises it for a semantic syntax error the parser detected.
    const checkerFiles = filesUnder(join(linterDirectory, "src", "checkers"), (path) => path.endsWith(".rs"));
    const checkerTokens = new Map(checkerFiles.map((file) => [file, withoutTestItems(lexRust(readFileSync(file, "utf8")))]));
    const semanticErrorsFile = join(upstream.directory, PARSER_SEMANTIC_ERRORS);
    literalCache.set(semanticErrorsFile, rustLiterals(await runChecked(runner, ["git", "-C", upstream.directory, "show", `${pack.upstream.commit}:${PARSER_SEMANTIC_ERRORS}`])));
    const implementationCache = new Map<string, string[]>();
    const implementationFiles = (code: string): string[] => {
      let files = implementationCache.get(code);
      if (files === undefined) {
        const file = implementationFile(code);
        const struct = file === undefined ? undefined : violationStruct(readFileSync(file, "utf8"), ruleByCode.get(code)!.source_location!.line);
        const sites = struct === undefined ? [] : checkerFiles.map((checker) => ({ checker, at: raiseSites(checkerTokens.get(checker)!, struct) })).filter(({ at }) => at.length > 0);
        const parserDetected = sites.some(({ checker, at }) => at.some((index) => inSemanticErrorArm(checkerTokens.get(checker)!, index)));
        files = file === undefined ? [] : [file, ...sites.map(({ checker }) => checker), ...(parserDetected ? [semanticErrorsFile] : [])];
        implementationCache.set(code, files);
      }
      return files;
    };
    /** The ruff_linter rule modules the implementation imports (its linter's helpers, other rules' modules); plumbing is not delegation. */
    const importedRuleModules = (file: string): string[] => [...new Set(literalsOf(file).uses
      .map((path) => useTarget(path, file, crateRoots, join(linterDirectory, "src")))
      .filter((target): target is string => target !== undefined && target !== file && target.startsWith(`${rulesDirectory}/`)))].sort();
    const sharedFiles = (code: string): string[] => {
      const file = implementationFile(code);
      return file === undefined ? [] : [...importedRuleModules(file), ...sharedCrateFiles];
    };
    const regexesOf = (files: string[]) => files.flatMap((file) => literalsOf(file).regexes.flatMap((pattern) => rustRegex(pattern) ?? []));

    const pragmas = join(upstream.directory, "crates/ruff_python_trivia/src/pragmas.rs");
    const noqa = join(linterDirectory, "src/noqa.rs");
    const directiveWords = [...literalsOf(pragmas).names, ...literalsOf(noqa).names.filter((word) => /^[a-z0-9]+$/u.test(word))];

    const upstreamPyproject = readFileSync(join(upstream.directory, "pyproject.toml"), "utf8");
    const declaredLicense = /^license = "([^"]+)"/mu.exec(upstreamPyproject)?.[1];
    const sitePackages = join(runtime.directory, ".venv", "lib");
    const metadata = filesUnder(sitePackages, (path) => /ruff-[^/]+\.dist-info\/METADATA$/u.test(path))[0];
    const wheelLicense = metadata === undefined ? undefined : /^License-Expression: (.+)$/mu.exec(readFileSync(metadata, "utf8"))?.[1];

    const recommendedOptions = (entry: RuffRule): Record<string, string>[] => documentedOptions(entry.explanation)
      .map((option) => ({ [option]: option === "target-version" ? lintTarget : settings.get(settingsKey(option)) ?? "(not printed by --show-settings)" }));

    const upstreamPath = (fixture: string) => posix.join(LINTER, fixture);

    const rule = (name: string): UpstreamRule | undefined => {
      const entry = ruleByCode.get(name);
      if (entry === undefined) return undefined;
      return {
        name,
        implementation: implementationFiles(name).map((file) => relative(upstream.directory, file)),
        tests: fixturesOf(name).map(upstreamPath),
        deprecation: instability(entry.status),
        recommended: defaultSelection.has(name) ? { config: "default", options: recommendedOptions(entry) } : null,
        notOnRealCode: null,
      };
    };

    const corpusDirectory = resolve(cacheRoot, "tmp");
    return {
      identity: {
        repository: pack.upstream.repository,
        commit: pack.upstream.commit,
        version: pack.upstream.version,
        runtime: `${version}, PyPI wheel on CPython ${runtime.version.replace(/^Python /u, "")}`,
        invocation: "ruff check --isolated --no-cache --exit-zero --output-format json --select <codes> <files>, every RUFF_* variable unset",
        // Five ported documents gate on pythonVersion; they agree with Ruff only where both evaluate at one version.
        "target version": `Ruff lints at Python ${lintTarget} under --isolated (linter.unresolved_target_version = ${unresolvedTarget}, read from --show-settings) and parses at Python 3.${parseTarget ?? "?"} (PythonVersion::latest()); the engine evaluates a Python document without languageOptions.pythonVersion at 3.${engineMinor} (DEFAULT_PYTHON_MINOR, native/src/code_rule_dsl/scope/options.rs)`,
        parsers: `CPython ${python.version.replace(/^Python /u, "")} and libcst`,
      },
      packChecks: [
        ...upstream.checks,
        {
          id: "runtime-version",
          ok: version.startsWith(`ruff ${pack.upstream.version} (${pack.upstream.commit.slice(0, 9)} `) && runtime.version === `Python ${runtime.pinned}`,
          detail: `the installed wheel reports \`${version}\` on ${runtime.version}; the pack pins ${pack.upstream.version} at ${pack.upstream.commit.slice(0, 9)} on Python ${runtime.pinned}`,
        },
        {
          id: "declared-license",
          ok: declaredLicense === pack.upstream.license && wheelLicense === pack.upstream.license,
          detail: `pyproject.toml at the pinned commit declares ${declaredLicense}, the installed wheel's metadata ${wheelLicense}; the pack records ${pack.upstream.license}`,
        },
      ],
      licenseText: readFileSync(join(upstream.directory, "LICENSE"), "utf8"),
      rule,

      implementationNames(name: string): Set<string> {
        return new Set(implementationFiles(name).flatMap((file) => literalsOf(file).names));
      },
      sharedImplementationNames(name: string): Set<string> {
        return new Set(sharedFiles(name).flatMap((file) => literalsOf(file).names));
      },
      implementationRegexes(name: string): RegExp[] {
        return regexesOf([...implementationFiles(name), ...sharedFiles(name)]);
      },
      fixtureWords: () => fixtureWords,
      fixtureTexts: () => fixtureTexts,
      fixtureTextSources: () => fixtureTextSources,
      keywords: python.keywords,
      // Ruff reads `# noqa`, `# ruff: ...` and the other pragma comments it recognizes, read from its pinned source.
      commentVocabulary: new Set([...python.commentVocabulary, ...directiveWords]),
      // Ruff lints .py and .pyi files (and notebooks, which Rerail has no dialect for), at the same path as the ESLint
      // and Bandit packs use; Ruff treats `src` as a source root, which path-dependent rules (INP001, A005) read.
      inScopeSamples: ["src/example.py", "src/example.pyi"],

      cases(name: string): UpstreamCase[] {
        return fixturesOf(name).map((fixture): UpstreamCase => ({
          fixture: upstreamPath(fixture),
          variant: [...testsOf.get(name)!.get(fixture)!].sort().join(", "),
          // The engine reads the file at the path Ruff's project root (the directory cargo runs the tests from) gives it.
          path: fixture,
          source: fixtureText.get(fixture)!,
          // Ruff lints every file it is given. On a file its parser rejects it runs no syntax-tree rule
          // (crates/ruff_linter/src/linter.rs), so its answer there is what it reports, possibly nothing.
          skipped: null,
          expected: caseFindings.get(name)?.get(realFixture(fixture)) ?? [],
          // Ruff's snapshots record its tests' own settings (Python 3.14 and per-test options), never the recommended configuration.
          markup: null,
        }));
      },

      toSpan(_name: string, source: string, startByte: number, endByte: number): Span {
        return ruffSpan(source, startByte, endByte, RUFF_SPAN_PARTS);
      },

      // Ruff lints .py and .pyi files, and notebooks, which Rerail has no dialect for.
      inCorpus: (path: string) => /\.pyi?$/u.test(path),

      async lintCorpus(files, ruleNames): Promise<UpstreamFile[]> {
        const directory = realpathSync(await mkdtemp(join(corpusDirectory, "ruff-corpus-")));
        try {
          // Each task's files sit at their repository paths under one directory per task, and Ruff runs from it, so the
          // project root Ruff resolves module paths against is the task's repository root, as the engine reads the path.
          const location = (file: CorpusFile) => join(directory, file.task, file.path);
          await Promise.all(files.map(async (file) => {
            await mkdir(dirname(location(file)), { recursive: true });
            await writeFile(location(file), file.source);
          }));
          const selected = [...new Set(ruleNames)].filter((code) => ruleByCode.has(code)).sort();
          const rejected = new Map<string, string>();
          const findings = new Map<string, Map<string, Span[]>>(selected.map((code) => [code, new Map()]));
          for (const task of [...new Set(files.map((file) => file.task))].sort()) {
            const taskDirectory = join(directory, task);
            const paths = files.filter((file) => file.task === task).map((file) => file.path);
            for (const [path, failure] of await parseFailures(paths, taskDirectory)) rejected.set(path, failure);
            for (const [code, byFile] of await lint(selected, paths, taskDirectory)) {
              for (const [path, spans] of byFile) findings.get(code)!.set(path, spans);
            }
          }
          return files.map((file) => {
            const failure = rejected.get(location(file));
            if (failure !== undefined) return { file, excluded: `upstream's parser rejects it: ${failure}`, findings: new Map() };
            return { file, excluded: null, findings: new Map(ruleNames.map((name) => [name, findings.get(name)?.get(location(file)) ?? []])) };
          });
        } finally {
          await rm(directory, { recursive: true, force: true });
        }
      },

      parserVerdicts: (files) => python.verdicts(files),
    };
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}
