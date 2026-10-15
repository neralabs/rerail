/**
 * The engine the gates judge rules on: this checkout's own, its TypeScript modules imported from src/engine/index.ts
 * and its native binding loaded the way the rest of conformance/ loads it. `loads` therefore means "compiles and
 * loads on this commit's engine"; nothing is pinned, fetched or built here.
 */
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import {
  bundledCodeRuleDslDirectory,
  CODE_RULE_AST_LANGUAGES,
  CODE_RULE_DIALECTS,
  codeRuleLanguageFamily,
  codeRuleSchema,
  loadBundledRules,
  loadNativeBinding,
  PACKAGE_ROOT,
  parseJsonWithoutDuplicateKeys,
  resolveNativeBindingPath,
  stableJsonText,
} from "../../src/engine/index.js";

/**
 * What identifies the engine a run used, from this checkout: every tracked file the native engine, its language
 * registry and the TypeScript engine are built from, split into the files every language reads and the files only one
 * language family does, so a rule keys on the engine it can actually be judged by.
 *
 * The native binary is not part of it. It is not bit-reproducible, so hashing it made a build-cache miss on unchanged
 * sources re-judge every rule; the sources it was built from say the same thing, and say it reproducibly. That holds
 * only while the binary is their build: one older than a source (`staleNativeBinary`) saves no result.
 */
export interface EngineIdentity {
  /** sha256 over each shared engine source file's path and the sha256 of its bytes in the working tree. */
  shared: string;
  /** Each family of FAMILY_SOURCES, its own files hashed the same way, by family. */
  families: Readonly<Record<string, string>>;
  /** The whole engine: the engine key of a rule that is no one family's (`engineKey`). */
  sha256: string;
}

export interface EngineSpan {
  startByte: number;
  endByte: number;
}

/** A violation the engine reports: the clause that matched and where. */
export interface EngineViolation {
  clauseId: string;
  code: string;
  path: string;
  span: EngineSpan;
}

export interface EngineFailure {
  clauseId?: string;
  code: string;
  message: string;
  path?: string;
}

export interface EngineEvaluation {
  status: "completed" | "failed";
  violations: EngineViolation[];
  failures: EngineFailure[];
}

interface ZodLike {
  safeParse(value: unknown): { success: true; data: unknown } | { success: false; error: { issues: readonly { path: readonly PropertyKey[]; message: string }[] } };
}

/** The rerail modules the gates call, from src/engine/index.ts. */
export interface RerailModules {
  findDuplicateJsonKeys(text: string): { pointer: string; key: string }[];
  codeRuleSchema: ZodLike;
  stableJsonText(value: unknown): string | undefined;
  /** Each pack directory's `rules/*.json`, read as rerail reads the packs it ships. */
  loadBundledRules(packs: readonly string[]): { kind: string; rule: { id: string } }[];
  /** The engine dialect for a file path (by its longest known suffix), if any. */
  dialectForPath(path: string): string | undefined;
}

export interface CompiledPolicy {
  canonicalJson: string;
  digest: string;
  inScope(path: string): boolean;
  /** Stateless report-mode evaluation of one file: no wall-clock budget and no early exit after the first violation. */
  evaluateFile(path: string, language: string, source: string): EngineEvaluation;
}

export interface Engine {
  identity: EngineIdentity;
  /** The capabilities JSON every policy is compiled against: native/src/code_rule_dsl/capabilities.json. */
  capabilities: string;
  rerail: RerailModules;
  compile(text: string): CompiledPolicy;
  /** The source text of the first node of kind `selector` the engine finds in `code`, parsed as `language`. */
  selectedText(code: string, selector: string, language: string): string | undefined;
}

/**
 * The character ast-grep writes a metavariable with where `$` is not valid in the language (ast-grep-language 0.45's
 * `impl_lang_expando!`), as rerail's patterns are read.
 */
const EXPANDO: Record<string, string> = { python: "µ" };

/** ast-grep's `pre_process_pattern`: `$A`, `$$A`, `$$$A` and a bare `$$$` take the expando character. */
export function withExpando(code: string, expando: string): string {
  return code.replace(/\$+(?=[A-Z_])|\${3}/gu, (dollars) => expando.repeat(dollars.length));
}

/** The file extension that gives each policy language its dialect. */
const LANGUAGE_EXTENSIONS: Record<string, string> = { typescript: ".ts", javascript: ".js", python: ".py" };

/**
 * The tracked paths the engine is built from, as `git ls-files` pathspecs: the native crate (with its vendored
 * grammars and the tables it embeds), the language registry it generates from, the TypeScript engine, and the Rust
 * build's lockfile, manifest and toolchain.
 */
export const ENGINE_SOURCES = ["native", "languages", "src/engine", "Cargo.lock", "Cargo.toml", "rust-toolchain.toml"] as const;

/**
 * Engine source files only one language family can reach, by family (`languages/*.json`'s `family`: `ecma` for
 * JavaScript and TypeScript, `python` for Python). A rule of one family keys on these and the shared files alone, so a
 * change to Python support leaves every ECMAScript rule's cached verdict standing. An entry is a tracked file, or a
 * directory with a trailing `/`.
 *
 * The list is a ledger of what the engine's own dispatch already decides, never a second opinion on it: the language
 * registry says which families honour which suppressions (`noqa` and `nosec`, read by `suppress/noqa.rs` and
 * `suppress/nosec.rs`, are listed by `languages/python.json` alone), `literal/mod.rs` hosts `eslintDirective` on ECMA
 * comments only, and the rest are each family's vendored grammars, its registry entries and the scope and value
 * modules named for it. `suppress/eslint.rs` is the one entry a family outside its list reads: `languages/vue.json`
 * honours `eslint` too, and a Vue rule belongs to no listed family, so it keys on the whole engine regardless.
 *
 * Only the families the gates judge earn a list; every other engine file, a `matcher/`, `compile/`, `workspace/` or
 * `language/adapter/` one included, is shared and so part of every rule's key. That is the direction this list fails
 * in: a file nobody listed costs one cold re-judge of everything, never a stale verdict. Adding an entry is therefore
 * the edit to justify, and `test/conformance/gates/engine.test.ts` fails when one matches no engine source at all.
 */
export const FAMILY_SOURCES: Readonly<Record<string, readonly string[]>> = {
  ecma: [
    "languages/javascript.json",
    "languages/typescript.json",
    "native/grammars/tree-sitter-javascript/",
    "native/grammars/tree-sitter-typescript/",
    "native/src/code_rule_dsl/literal/eslint_directive.rs",
    "native/src/code_rule_dsl/scope/data/es_globals.generated.json",
    "native/src/code_rule_dsl/scope/data/ts_lib_globals.generated.json",
    "native/src/code_rule_dsl/scope/ecma.rs",
    "native/src/code_rule_dsl/scope/usage/ecma.rs",
    "native/src/code_rule_dsl/suppress/biome.rs",
    "native/src/code_rule_dsl/suppress/biome_registry.generated.rs",
    "native/src/code_rule_dsl/suppress/eslint.rs",
    "native/src/code_rule_dsl/suppress/oxlint.rs",
    "native/src/code_rule_dsl/value/ecma.rs",
    "native/src/code_rule_dsl/value/ecma/",
  ],
  python: [
    "languages/python.json",
    "native/grammars/tree-sitter-python/",
    "native/src/code_rule_dsl/scope/data/python_builtins.generated.json",
    "native/src/code_rule_dsl/scope/python.rs",
    "native/src/code_rule_dsl/scope/python/",
    "native/src/code_rule_dsl/suppress/noqa.rs",
    "native/src/code_rule_dsl/suppress/nosec.rs",
    "native/src/code_rule_dsl/value/python.rs",
    "native/src/code_rule_dsl/value/python/",
  ],
} as const;

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** Every tracked file `git ls-files` lists under `pathspec` at `root`, sorted and without repeats. */
function tracked(root: string, pathspec: readonly string[]): string[] {
  const listed = execFileSync("git", ["ls-files", "-z", "--", ...pathspec], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 })
    .split("\0")
    .filter((path) => path !== "");
  return [...new Set(listed)].sort(byText);
}

/**
 * `#[cfg(test)]`, or an `all(test, ..)` of one level, immediately before a `mod <name>;` that names another file. A
 * `cfg` shape this does not read is a declaration `testOnlySources` misses, which leaves a test file in the key: the
 * cheap mistake, and why this stays narrow rather than looking for `test` anywhere inside the attribute.
 */
const CFG_TEST_MODULE = /#\[cfg\((?:test|all\(test[^)]*\)[^)]*)\)\]\s*(?:pub(?:\(crate\))?\s+)?mod\s+([a-z0-9_]+)\s*;/gu;

/** A plain `mod <name>;` at the start of a line: how a test module's own submodules are declared. */
const MODULE = /(?:^|\n)\s*(?:pub(?:\(crate\))?\s+)?mod\s+([a-z0-9_]+)\s*;/gu;

/** `#[path = "<file>"]`: the file a module is read from instead of the one its name would give. */
const MODULE_PATH = /#\[path\s*=\s*"([^"]+)"\]/gu;

/** The directory a Rust file's submodules live in: its own for `mod.rs` and `lib.rs`, the one it names otherwise. */
function moduleDirectory(path: string): string {
  const base = path.slice(path.lastIndexOf("/") + 1);
  return base === "mod.rs" || base === "lib.rs" ? path.slice(0, path.lastIndexOf("/")) : path.slice(0, -3);
}

/**
 * The engine sources only `cargo test` compiles: every file a `#[cfg(test)] mod` declaration reaches, and everything
 * under its directory, which is how the engine keeps its oracle tests (`suppress/ruff.rs`, `value/eslint_rules.rs`)
 * beside the code they check. None of them decides a verdict the gates read, so an otherwise unchanged rule keeps its
 * cached result when one changes.
 *
 * A declaration this misses leaves a test file in the key and costs time; a source file it wrongly claimed would cost
 * a verdict, so a file any `#[path]` attribute names is kept whatever else declares it. `src/build_identity.rs` and
 * `src/build_languages.rs` are why: `lib.rs` declares both under `#[cfg(test)]` so `cargo test` checks them, but
 * `build.rs` reads them through `#[path]` to generate the tables the engine embeds.
 */
function testOnlySources(root: string, listed: readonly string[]): ReadonlySet<string> {
  const rust = listed.filter((path) => path.startsWith("native/") && path.endsWith(".rs"));
  const text = new Map(rust.map((path) => [path, readFileSync(join(root, path), "utf8")] as const));
  const tested = new Set<string>();
  const declare = (declaring: string, name: string): boolean => {
    const directory = moduleDirectory(declaring);
    let added = false;
    for (const path of listed) {
      if ((path === `${directory}/${name}.rs` || path.startsWith(`${directory}/${name}/`)) && !tested.has(path)) {
        tested.add(path);
        added = true;
      }
    }
    return added;
  };
  const declarations = (pattern: RegExp, from: readonly string[]): boolean => {
    let added = false;
    for (const path of from) for (const match of (text.get(path) ?? "").matchAll(pattern)) added = declare(path, match[1]!) || added;
    return added;
  };
  declarations(CFG_TEST_MODULE, rust);
  // A test module's own submodules carry no cfg of their own; follow them until nothing new is reached.
  while (declarations(MODULE, [...tested].filter((path) => path.endsWith(".rs")))) continue;
  // `#[path]` outside an inline module is relative to the directory of the file that declares it.
  for (const path of rust) {
    for (const match of (text.get(path) ?? "").matchAll(MODULE_PATH)) tested.delete(join(path.slice(0, path.lastIndexOf("/")), match[1]!).replaceAll("\\", "/"));
  }
  return tested;
}

/**
 * Where each engine source file belongs in a rule's key: one family of FAMILY_SOURCES when an entry of that family
 * names it or holds it, shared otherwise. Throws when two families claim one file, which would make a rule's key
 * depend on the order the lists are read in.
 */
function splitEngineSources(paths: readonly string[]): { shared: string[]; families: Record<string, string[]> } {
  const owner = new Map<string, string>();
  for (const [family, entries] of Object.entries(FAMILY_SOURCES)) {
    for (const entry of entries) {
      for (const path of paths) {
        if (path !== entry && !(entry.endsWith("/") && path.startsWith(entry))) continue;
        const claimed = owner.get(path);
        if (claimed !== undefined && claimed !== family) throw new Error(`FAMILY_SOURCES: both ${claimed} and ${family} claim ${path}`);
        owner.set(path, family);
      }
    }
  }
  const families = Object.fromEntries(Object.keys(FAMILY_SOURCES).map((family) => [family, [] as string[]]));
  const shared: string[] = [];
  for (const path of paths) {
    const family = owner.get(path);
    if (family === undefined) shared.push(path);
    else families[family]!.push(path);
  }
  return { shared, families };
}

/**
 * The engine source files a rule's key covers, in the checkout at `root`: every file `git ls-files` lists under
 * ENGINE_SOURCES except the ones only `cargo test` compiles, each on the shared side or one family's.
 */
export function engineSources(root: string = PACKAGE_ROOT): { shared: string[]; families: Record<string, string[]> } {
  const listed = tracked(root, ENGINE_SOURCES);
  const testOnly = testOnlySources(root, listed);
  return splitEngineSources(listed.filter((path) => !testOnly.has(path)));
}

/**
 * An engine source the native binary at `binary` is older than, by modification time (a deleted one by its
 * directory's), or undefined when there is none. Only the sources the binary is built from count: `src/engine` is
 * imported as TypeScript, so its edits take effect without a build.
 *
 * The binary is no part of a rule's key, so one left over from before a source edit would judge rules under the edited
 * sources' key and its verdicts would be read back as theirs; a run on one saves no result and commits no goldens,
 * nor does a merge --write of shard reports (cli.ts). Shards run on a stale binary and merged after a rebuild are not
 * caught: a shard report does not say which binary made it. `npm run build:native` writes the binary after cargo
 * rebuilds what a newer source changed, and CI unpacks it from the build job's artifact after the checkout, so a
 * binary built from these sources is newer than all of them.
 * A binary misjudged stale costs a run its cache writes and `--write`, never a verdict.
 */
export function staleNativeBinary(root: string = PACKAGE_ROOT, binary: string = resolveNativeBindingPath()): string | undefined {
  const built = statSync(binary).mtimeMs;
  const modified = (path: string): number => {
    let absolute = join(root, path);
    while (!existsSync(absolute)) absolute = dirname(absolute);
    return statSync(absolute).mtimeMs;
  };
  const { shared, families } = engineSources(root);
  return [...shared, ...Object.values(families).flat()].find((path) => !path.startsWith("src/engine/") && modified(path) > built);
}

/**
 * The engine's identity in the checkout at `root`: its `engineSources`, hashed from the working tree (a tracked file
 * deleted there counts as deleted) by the side of the key each one is on.
 */
export function engineIdentity(root: string = PACKAGE_ROOT): EngineIdentity {
  const { shared, families } = engineSources(root);
  const digest = (paths: readonly string[]) => sha256(paths.map((path) => {
    const absolute = join(root, path);
    return `${path}\0${existsSync(absolute) ? sha256(readFileSync(absolute)) : "deleted"}`;
  }).join("\n"));
  const halves = {
    shared: digest(shared),
    families: Object.fromEntries(Object.entries(families).map(([family, paths]) => [family, digest(paths)])),
  };
  return { ...halves, sha256: engineKey({ ...halves, sha256: "" }, []) };
}

/**
 * The engine's part of a rule's cache key: the shared engine files, plus the files of each family in `families`.
 *
 * A rule of no family, of more than the registry knows, or of a family FAMILY_SOURCES does not list keys on every
 * family's files, which is the whole engine: when the gates cannot say which language support a verdict rests on, the
 * answer is all of it, and a change anywhere judges the rule again.
 */
export function engineKey(identity: EngineIdentity, families: readonly string[]): string {
  const own = [...new Set(families)].sort(byText);
  const all = own.length === 0 || own.some((family) => identity.families[family] === undefined);
  const keyed = all ? Object.keys(identity.families).sort(byText) : own;
  return sha256([identity.shared, ...keyed.map((family) => `${family}\0${identity.families[family]!}`)].join("\n"));
}

/**
 * The language families a rule document can be judged in: the family of every `language` its clauses name. Empty when
 * the document names none, names one the language registry does not know, or is not JSON at all; `engineKey` then
 * keys the rule on the whole engine, which is the only safe answer when the document does not say.
 *
 * Every `language` anywhere in the document counts, not only the ones under a clause's `ast`: a key that covers a
 * family the rule never reaches costs a re-judge, and one that misses a family it does reach would keep a stale
 * verdict.
 */
export function policyFamilies(text: string): string[] {
  let document: unknown;
  try {
    document = JSON.parse(text);
  } catch {
    return [];
  }
  const languages = new Set<string>();
  const walk = (value: unknown): void => {
    if (Array.isArray(value)) return void value.forEach(walk);
    if (value === null || typeof value !== "object") return;
    for (const [key, child] of Object.entries(value)) {
      if (key === "language" && typeof child === "string") languages.add(child);
      walk(child);
    }
  };
  walk(document);
  const known = new Set<string>(CODE_RULE_AST_LANGUAGES);
  if (languages.size === 0 || [...languages].some((language) => !known.has(language))) return [];
  return [...new Set([...languages].map((language) => codeRuleLanguageFamily(language as typeof CODE_RULE_AST_LANGUAGES[number])))].sort(byText);
}

/**
 * Every repeated object key in a JSON text, as src/engine's `findDuplicateJsonKeys` reports it: read back from the
 * error `parseJsonWithoutDuplicateKeys` (exported by src/engine/index.ts) names them in, until index.ts exports
 * `findDuplicateJsonKeys` itself. Only the first repeat is read back, which is all `loads` reports; text that is not
 * JSON throws, as there.
 */
function findDuplicateJsonKeys(text: string): { pointer: string; key: string }[] {
  const label = "document";
  try {
    parseJsonWithoutDuplicateKeys(text, label);
    return [];
  } catch (error) {
    if (!(error instanceof Error) || error instanceof SyntaxError || !error.message.startsWith(`${label}: duplicate key \``)) throw error;
    const first = error.message.slice(`${label}: `.length).split("; duplicate key `")[0]!;
    const match = /^duplicate key `([\s\S]*)` at ([\s\S]*)$/u.exec(first);
    return [match === null ? { pointer: "", key: first } : { key: match[1]!, pointer: match[2] === "/" ? "" : match[2]! }];
  }
}

/** Each supported dialect suffix and the dialect it selects (src/engine's CODE_RULE_DIALECT_SUFFIXES). */
const DIALECT_SUFFIXES: ReadonlyMap<string, string> = new Map(
  Object.values(CODE_RULE_DIALECTS).flatMap((dialect) => dialect.suffixes.map((suffix) => [suffix, dialect.label] as const)),
);

/**
 * The dialect a path routes to, as src/engine's `codeRuleDialectForPath` reads it (its longest listed suffix, the
 * basename lowercased in ASCII and longer than the suffix), built from the exported CODE_RULE_DIALECTS until
 * src/engine/index.ts exports `codeRuleDialectForPath` itself.
 */
function dialectForPath(path: string): string | undefined {
  const name = path.slice(path.lastIndexOf("/") + 1).replace(/[A-Z]+/g, (upper) => upper.toLowerCase());
  let found: { suffix: string; dialect: string } | undefined;
  for (const [suffix, dialect] of DIALECT_SUFFIXES) {
    if (name.length > suffix.length && name.endsWith(suffix) && (found === undefined || suffix.length > found.suffix.length)) found = { suffix, dialect };
  }
  return found?.dialect;
}

/** The rerail modules the gates call, as this checkout's src/engine/index.ts provides them. */
export const RERAIL_MODULES: RerailModules = {
  findDuplicateJsonKeys,
  codeRuleSchema,
  stableJsonText,
  loadBundledRules,
  dialectForPath,
};

/** This checkout's engine, the one every gate in a run uses. */
export function loadEngine(root: string = PACKAGE_ROOT): Engine {
  // The engine reads this from the environment; a stray value adds timings to every evaluation.
  if (process.env.RERAIL_CODE_RULE_PROFILE !== undefined) throw new Error("unset RERAIL_CODE_RULE_PROFILE: it adds timings to every evaluation");
  const runtime = loadNativeBinding().CodeRuleDslRuntime;
  const capabilities = readFileSync(join(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");

  const compile = (text: string): CompiledPolicy => {
    const compiled = runtime.compile(text, "json", capabilities);
    return {
      canonicalJson: compiled.canonicalJson(),
      digest: compiled.digest(),
      inScope: (path) => compiled.inScope(path),
      evaluateFile(path, language, source) {
        const snapshot = { format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: [{ path, language, source }] };
        return JSON.parse(compiled.evaluate(JSON.stringify(snapshot), "report")) as EngineEvaluation;
      },
    };
  };

  return {
    identity: engineIdentity(root),
    capabilities,
    rerail: RERAIL_MODULES,
    compile,
    selectedText(code, selector, language) {
      const extension = LANGUAGE_EXTENSIONS[language];
      const dialect = extension === undefined ? undefined : dialectForPath(`probe${extension}`);
      if (extension === undefined || dialect === undefined) return undefined;
      let probe: CompiledPolicy;
      try {
        probe = compile(JSON.stringify({
          format: "rerail.policy/v1",
          id: "validation:selector-probe",
          revision: 1,
          mode: "log",
          scope: { include: ["**/*"], exclude: [] },
          clauses: [{
            id: "selector",
            forbid: { ast: { language, rule: { kind: selector } } },
            violation: { anchor: "witness.span", code: "SELECTOR", message: "selector" },
          }],
        }));
      } catch (error) {
        // A selector the grammar has no node kind for selects nothing. Any other compile error means the probe no
        // longer fits the engine's policy contract, which must stop the run rather than widen what not-overfit judges.
        if (error instanceof Error && error.message.includes("/forbid/ast/rule/kind:")) return undefined;
        throw error;
      }
      // A context is pattern text: where `$` is not valid in the language, ast-grep parses metavariables with the expando.
      const expando = EXPANDO[language];
      const parsed = expando === undefined ? code : withExpando(code, expando);
      const result = probe.evaluateFile(`context${extension}`, dialect, parsed);
      if (result.failures.length > 0) return undefined;
      const first = [...result.violations].sort((a, b) => a.span.startByte - b.span.startByte || b.span.endByte - a.span.endByte)[0];
      if (first === undefined) return undefined;
      const selected = Buffer.from(parsed, "utf8").subarray(first.span.startByte, first.span.endByte).toString("utf8");
      return expando === undefined ? selected : selected.replaceAll(expando, "$");
    },
  };
}
