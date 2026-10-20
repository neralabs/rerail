import { createHash } from "node:crypto";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { mkdir, mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, posix, relative, resolve, sep } from "node:path";

import { PACKAGE_ROOT } from "../../../../src/engine/index.js";
import type { CorpusFile } from "../../../port-readiness/corpus.js";
import { checkoutPinned, readRerailCorpora, rerailCorpusId, type RerailCorpus } from "../../../port-readiness/corpus.js";
import { runChecked, type CommandRunner } from "../../../port-readiness/process.js";
import { upstreamLicenseText } from "../../../port-readiness/recorded-adapter.js";
import { clippySpan, rustcSpan, type RustcSpan } from "../../../port-readiness/span-units/clippy.js";
import type { FixtureTextSource, PackAdapter, PackManifest, ParserVerdict, Span, SpanParts, UpstreamCase, UpstreamFile, UpstreamRule } from "../../../port-readiness/types.js";
import { closingIndex, lexRust, rustLiterals, rustRegex, splitTopLevel, withoutTestItems, type RustLiterals, type RustToken } from "../languages/rust.js";
import { checkoutUpstream, filesUnder, rustupRuntime } from "../runtimes.js";
import { useTarget } from "./ruff.js";

/**
 * The Clippy adapter. Expected answers come from running the pinned toolchain's clippy-driver itself on Clippy's own
 * UI tests (tests/ui/**) as rust-clippy's tests/compile-test.rs (ui_test 0.30.7) runs them, which is how
 * packs/rust/upstream/clippy/generate.py records them: edition 2024 unless `//@edition`, `//@compile-flags` appended,
 * `//@aux-build` crates built first and passed with `--extern`, clippy_test_deps built with cargo and its direct
 * dependencies passed with `--extern`, ui_test's crate type from the file's contents, `--emit=metadata -Aunused
 * -Ainternal_features -Zui-testing -Zdeduplicate-diagnostics=no --error-format=json`, `CLIPPY_CONF_DIR=tests` (Clippy's
 * test default configuration, tests/clippy.toml), and `RUSTC_BOOTSTRAP=1` for the tests' `#![feature]`s and the `-Z`
 * flags; `-Dwarnings` changes only the level and is not passed. Every ported lint is passed with `-W clippy::<lint>`,
 * so one that Clippy's default levels leave off is recorded too (in-source `allow`/`expect` still apply).
 *
 * The gates judge every case of an upstream lint for each document, so cases are recorded under that one default
 * configuration: a tests/ui-toml test whose clippy.toml is not tests/clippy.toml is skipped, with the reason.
 */

const SPARSE_PATHS = [
  "/COPYRIGHT",
  "/Cargo.toml",
  "/clippy_lints/src/",
  "/clippy_test_deps/",
  "/clippy_utils/src/",
  "/tests/clippy.toml",
  "/tests/ui-toml/",
  "/tests/ui/",
];
const LINTS = "clippy_lints/src";
const UTILS = "clippy_utils/src";
const UI = "tests/ui/";
const UI_TOML = "tests/ui-toml/";
/** The configuration of every tests/ui test (`CLIPPY_CONF_DIR=tests`): the default the recording is made under. */
const DEFAULT_CONF = "tests/clippy.toml";
/** Clippy's lint groups whose lints its default levels turn on (`warn`, or `deny` for correctness). */
const DEFAULT_GROUPS = new Set(["correctness", "suspicious", "style", "complexity", "perf"]);
const BASE_ARGUMENTS = ["--error-format=json", "-Aunused", "-Ainternal_features", "-Zui-testing", "-Zdeduplicate-diagnostics=no"];
const DIRECTIVE = /^\/\/@\s*([\w-]+)\s*(?::\s*(.*?))?\s*$/u;
const ANNOTATION = /\/\/~(\^+|\||v+)?\s*([A-Za-z_]\w*(?:::\w+)*)?/gu;
/** The default edition of ui_test 0.30.7 as rust-clippy configures it (`defaults.set_custom("edition", "2024")`). */
const DEFAULT_EDITION = "2024";

/**
 * Rust's keywords at the pinned release (the Rust Reference, "Keywords"): strict, reserved, and the weak keywords that
 * are keywords where they appear (`macro_rules`, `raw`, `safe`, `union`, and `static` of the `'static` lifetime).
 */
const RUST_KEYWORDS = [
  "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for",
  "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
  "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
  "abstract", "become", "box", "do", "final", "gen", "macro", "override", "priv", "try", "typeof", "unsized", "virtual", "yield",
  "macro_rules", "raw", "safe", "union",
];

/**
 * Names the Rust language itself defines besides its keywords, which not-overfit takes as the language's as it takes
 * Python's soft keywords (python_toolkit.py): a rule that spells `usize` or `cfg_attr` names Rust, not upstream's test
 * text. They are the pinned release's Rust Reference (https://doc.rust-lang.org/1.99.0/reference/), as the toolchain
 * ships no list of them (rustc's `sym` table is in rustc-dev, not in the clippy component the runtime pins):
 * - the primitive types: `bool` (types/boolean.html), the integer and floating-point types (types/numeric.html), `char`
 *   and `str` (types/textual.html), and the unstable `f16` and `f128` primitives of the release's standard library
 *   (https://doc.rust-lang.org/1.99.0/std/primitive.f16.html, primitive.f128.html);
 * - the built-in attributes of conditional compilation (`cfg`, `cfg_attr`) and of lint checks (`allow`, `expect`,
 *   `warn`, `deny`, `forbid`), from the built-in attributes index (attributes.html#built-in-attributes-index);
 * - the configuration predicate operators `all`, `any` and `not` (conditional-compilation.html).
 * The rest of the attribute index stays out: `test`, `path`, `inline` or `doc` are as often a program's own names, so
 * taking them as Rust's would explain test words the language does not.
 */
const RUST_VOCABULARY = [
  "bool", "char", "str",
  "i8", "i16", "i32", "i64", "i128", "isize", "u8", "u16", "u32", "u64", "u128", "usize", "f16", "f32", "f64", "f128",
  "cfg", "cfg_attr", "allow", "expect", "warn", "deny", "forbid",
  "all", "any", "not",
];

/** Clippy reports a primary span with both ends; the recording keeps them. */
const CLIPPY_SPAN_PARTS: SpanParts = { endLine: true, endCol: true };

interface RustcDiagnostic {
  $message_type?: string;
  message: string;
  level: string;
  code: { code: string } | null;
  spans: (RustcSpan & { file_name: string; is_primary: boolean })[];
}

interface CargoMessage {
  reason: string;
  package_id?: string;
  manifest_path?: string;
  filenames?: string[];
  message?: RustcDiagnostic;
  target?: { name: string; kind: string[]; crate_types: string[]; src_path: string };
  linked_paths?: string[];
  /** compiler-artifact: the features the unit was compiled with, and its profile. */
  features?: string[];
  profile?: { test: boolean; debug_assertions: boolean };
  /** build-script-executed: the `--cfg` options the package's build script sets on its units. */
  cfgs?: string[];
}

interface DeclaredLint {
  /** The lint's name, lower case (`option_env_unwrap`). */
  name: string;
  group: string;
  /** The file that declares it, relative to the checkout. */
  file: string;
}

const sha256 = (value: string | Buffer): string => createHash("sha256").update(value).digest("hex");
const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** Run `work` on every item, at most `limit` at a time, keeping the items' order. */
async function mapLimit<T, R>(items: readonly T[], limit: number, work: (item: T) => Promise<R>): Promise<R[]> {
  const results: R[] = new Array(items.length);
  let next = 0;
  await Promise.all(Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (next < items.length) {
      const index = next;
      next += 1;
      results[index] = await work(items[index]!);
    }
  }));
  return results;
}

/** ui_test `//@` directives, as (name, value). */
export function directives(text: string): [string, string][] {
  return text.split("\n").flatMap((line) => {
    const match = DIRECTIVE.exec(line.replace(/\r$/u, ""));
    return match === null ? [] : [[match[1]!, match[2] ?? ""] as [string, string]];
  });
}

/** ui_test 0.30.7's `default_per_file_config`: the crate type a test's contents call for (no flag for a binary). */
export function crateType(text: string): string[] {
  if (/#\[proc_macro(?:_derive|_attribute)?[\](]/u.test(text)) return ["--crate-type=proc-macro"];
  if (text.includes("#[test]")) return ["--test"];
  if (/fn main\(\)|#\[start\]/u.test(text)) return [];
  return ["--crate-type=lib"];
}

/** The edition a test is compiled in: its `//@edition`, else ui_test's default. */
export function testEdition(text: string): string {
  return directives(text).find(([key]) => key === "edition")?.[1] ?? DEFAULT_EDITION;
}

/** The lints a test's ui_test `//~` annotations name, without the `clippy::` prefix (`ERROR`-style annotations aside). */
export function annotatedLints(text: string): Set<string> {
  const found = new Set<string>();
  for (const match of text.matchAll(ANNOTATION)) {
    const code = match[2];
    if (code !== undefined && !/^(?:ERROR|WARN|HELP|NOTE|SUGGESTION)$/u.test(code)) found.add(code.replace(/^clippy::/u, ""));
  }
  return found;
}

/**
 * The lint a test is named after: the longest declared lint name its file stem starts with (a tests/ui-toml test: its
 * directory's name), whole or followed by `_` — so `precedence_bits.rs` is precedence_bits', not precedence's.
 */
export function namedLint(test: string, lints: ReadonlySet<string>): string | undefined {
  const stem = test.startsWith(UI_TOML) ? test.slice(UI_TOML.length).split("/")[0]! : basename(test, ".rs");
  let best: string | undefined;
  for (let end = stem.length; end > 0; end = stem.lastIndexOf("_", end - 1)) {
    const prefix = stem.slice(0, end);
    if (lints.has(prefix)) {
      best = prefix;
      break;
    }
  }
  return best;
}

/** Every lint `declare_clippy_lint!` declares in one source file: its name and group. */
export function declaredLints(source: string): { name: string; group: string }[] {
  const tokens = lexRust(source);
  const found: { name: string; group: string }[] = [];
  tokens.forEach((token, index) => {
    if (token.text !== "declare_clippy_lint" || tokens[index + 1]?.text !== "!" || tokens[index + 2]?.text !== "{") return;
    const body = tokens.slice(index + 3, closingIndex(tokens, index + 2));
    const at = body.findIndex((inner, position) => inner.text === "pub" && body[position + 1]?.kind === "ident" && body[position + 2]?.text === "," && body[position + 3]?.kind === "ident");
    if (at >= 0) found.push({ name: body[at + 1]!.text.toLowerCase(), group: body[at + 3]!.text });
  });
  return found;
}

/** clippy_lints/src/deprecated_lints.rs's DEPRECATED table: each deprecated lint (without `clippy::`) and its reason. */
export function deprecatedLints(source: string): Map<string, string> {
  const tokens = lexRust(source);
  const found = new Map<string, string>();
  const start = tokens.findIndex((token, index) => token.text === "DEPRECATED" && tokens[index - 1]?.text === "{");
  if (start < 0) return found;
  const open = tokens.findIndex((token, index) => index > start && token.text === "[");
  const body = tokens.slice(open + 1, closingIndex(tokens, open));
  body.forEach((token, index) => {
    if (token.text === "(" && body[index + 1]?.kind === "string" && body[index + 2]?.text === "," && body[index + 3]?.kind === "string" && body[index + 4]?.text === ")") {
      found.set(body[index + 1]!.value!.replace(/^clippy::/u, ""), body[index + 3]!.value!);
    }
  });
  return found;
}

/** Top-level rustc diagnostics with a primary span, from JSON lines (rustc's stderr, or cargo's `message`s). */
function diagnostics(lines: string): RustcDiagnostic[] {
  return lines.split("\n").filter((line) => line.startsWith("{")).map((line) => JSON.parse(line) as RustcDiagnostic)
    .filter((diagnostic) => (diagnostic.$message_type ?? "diagnostic") === "diagnostic");
}

/** A diagnostic's primary span (the first, as generate.py reads it). */
function primarySpan(diagnostic: RustcDiagnostic): RustcDiagnostic["spans"][number] | undefined {
  return diagnostic.spans.find((span) => span.is_primary);
}

/** A rustc parse error, without the `aborting due to` summary. */
function parseErrors(stderr: string): string[] {
  return diagnostics(stderr).filter((diagnostic) => diagnostic.level === "error" && !diagnostic.message.startsWith("aborting due to")).map((diagnostic) => {
    const span = primarySpan(diagnostic);
    return span === undefined ? diagnostic.message : `${diagnostic.message} (line ${span.line_start})`;
  });
}

/**
 * The corpus-relative `.rs` files the dep-info beside a cargo artifact lists (common.py's dep_info_sources):
 * `deps/lib<crate>-<hash>.rmeta` reads `deps/<crate>-<hash>.d`; a build script `build/<pkg>-<hash>/build-script-build`
 * reads `build/<pkg>-<hash>/build_script_build-<hash>.d`.
 */
function depInfoSources(artifact: string, root: string): string[] {
  const name = basename(artifact);
  const directory = dirname(artifact);
  const depInfo = name.startsWith("build-script-")
    ? join(directory, `build_script_${name.slice("build-script-".length).replaceAll("-", "_")}-${basename(directory).split("-").pop()!}.d`)
    : join(directory, `${(name.includes(".") && name.startsWith("lib") ? name.slice(3) : name).split(".")[0]!}.d`);
  if (!existsSync(depInfo)) throw new Error(`no dep-info ${depInfo} for ${artifact}`);
  const sources: string[] = [];
  for (const line of readFileSync(depInfo, "utf8").split("\n")) {
    const separator = line.indexOf(": ");
    if (line === "" || line.startsWith("#") || separator < 0 || !line.slice(0, separator).endsWith(".d")) continue;
    for (const file of line.slice(separator + 2).trim().split(/(?<!\\) /u)) {
      const path = file.endsWith(".rs") ? relativeTo(file.replaceAll("\\ ", " "), root) : undefined;
      if (path !== undefined) sources.push(path);
    }
  }
  return sources;
}

/** A path rustc or cargo names (relative to the workspace root, or absolute) relative to `root`, or undefined outside it. */
function relativeTo(file: string, root: string): string | undefined {
  const path = relative(root, resolve(root, file)).split(sep).join("/");
  return path === "" || path.startsWith("../") || posix.isAbsolute(path) ? undefined : path;
}

/** A unit's configuration options, each `name` or `name="value"` as `--print cfg` writes them. */
export type CfgOptions = ReadonlySet<string>;

/**
 * The predicates of a file's leading inner `#![cfg(…)]` attributes (the inner attributes before its first item), each
 * as its tokens: when one is false, rustc drops everything the file holds before any lint runs.
 */
export function innerCfgPredicates(source: string): RustToken[][] {
  const tokens = lexRust(source);
  const predicates: RustToken[][] = [];
  let index = 0;
  while (tokens[index]?.text === "#" && tokens[index + 1]?.text === "!" && tokens[index + 2]?.text === "[") {
    const close = closingIndex(tokens, index + 2);
    const inner = tokens.slice(index + 3, close);
    if (inner[0]?.kind === "ident" && inner[0].text === "cfg" && inner[1]?.text === "(" && closingIndex(inner, 1) === inner.length - 1) predicates.push(inner.slice(2, -1));
    index = close + 1;
  }
  return predicates;
}

/**
 * Whether a configuration predicate holds under `options`, as the pinned release's Rust Reference defines one
 * (https://doc.rust-lang.org/1.99.0/reference/conditional-compilation.html): an option `name` or `name = "value"`,
 * `all(…)`, `any(…)`, `not(…)`, `true` or `false`; undefined for tokens that are none of them.
 */
export function cfgHolds(tokens: readonly RustToken[], options: CfgOptions): boolean | undefined {
  const [head, ...rest] = tokens;
  if (head?.kind !== "ident") return undefined;
  if (rest.length === 0) return head.text === "true" ? true : head.text === "false" ? false : options.has(head.text);
  if (rest.length === 2 && rest[0]!.text === "=" && rest[1]!.kind === "string") return options.has(`${head.text}="${rest[1]!.value}"`);
  if (!["all", "any", "not"].includes(head.text) || rest[0]?.text !== "(" || closingIndex(rest, 0) !== rest.length - 1) return undefined;
  const operands = splitTopLevel(rest.slice(1, -1), ",").map((operand) => cfgHolds(operand, options));
  if (operands.some((value) => value === undefined)) return undefined;
  if (head.text === "not") return operands.length === 1 ? !operands[0] : undefined;
  return head.text === "all" ? operands.every(Boolean) : operands.some(Boolean);
}

export async function createClippyAdapter(pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner): Promise<PackAdapter> {
  if (pack.upstream.recommended_configs.join(",") !== "default") {
    throw new Error(`${pack.id}: recommended_configs must be [default], Clippy's default lint levels`);
  }
  const runtime = await rustupRuntime(pack.upstream.runtime, runner);
  const toolchain = `+${runtime.pin.channel}`;
  // rust-clippy tags the commit each Rust release ships as rust-<release>; the toolchain's release names the tag.
  const upstream = await checkoutUpstream(cacheRoot, pack.upstream, `rust-${runtime.pin.channel}`, "clippy_lints", SPARSE_PATHS, runner);
  const checkout = upstream.directory;
  const absent = SPARSE_PATHS.filter((path) => !existsSync(join(checkout, path)));
  if (absent.length > 0) throw new Error(`the rust-clippy checkout at ${checkout} lacks ${absent.join(", ")}: it was made with other sparse paths; delete it to fetch it again`);
  const read = (path: string) => readFileSync(join(checkout, path), "utf8");
  const driverEnv = (confDir: string): NodeJS.ProcessEnv => {
    const env = Object.fromEntries(Object.entries(runtime.env).filter(([key]) => !key.startsWith("CLIPPY_") && !key.startsWith("CARGO_")));
    return { ...env, RUSTC_BOOTSTRAP: "1", RUSTC_ICE: "0", CLIPPY_CONF_DIR: confDir };
  };

  // Every lint Clippy declares, and its group.
  const lints = new Map<string, DeclaredLint>();
  for (const file of filesUnder(join(checkout, LINTS), (path) => path.endsWith(".rs"))) {
    const path = relative(checkout, file).split(sep).join("/");
    for (const lint of declaredLints(readFileSync(file, "utf8"))) lints.set(lint.name, { ...lint, file: path });
  }
  const deprecated = deprecatedLints(read(`${LINTS}/deprecated_lints.rs`));
  const lintNames = new Set(lints.keys());
  const ported = [...new Set(names)].filter((name) => lints.has(name)).sort(byText);

  // Every UI test, and the lints each one is a test of: those its `//~` annotations name, and the one it is named after.
  const isTest = (path: string) => path.endsWith(".rs") && !path.split("/").includes("auxiliary");
  const testFiles = [...filesUnder(join(checkout, UI), isTest), ...filesUnder(join(checkout, UI_TOML), isTest)]
    .map((file) => relative(checkout, file).split(sep).join("/")).sort(byText);
  const testsOf = new Map<string, string[]>();
  for (const test of testFiles) {
    const owners = new Set(annotatedLints(read(test)));
    const named = namedLint(test, lintNames);
    if (named !== undefined) owners.add(named);
    for (const lint of owners) testsOf.set(lint, [...(testsOf.get(lint) ?? []), test]);
  }
  const caseFixtures = [...new Set(ported.flatMap((name) => testsOf.get(name) ?? []))].sort(byText);
  const defaultConf = read(DEFAULT_CONF);
  const variantReason = (test: string): string | null => {
    if (!test.startsWith(UI_TOML)) return null;
    const conf = `${posix.dirname(test)}/clippy.toml`;
    if (existsSync(join(checkout, conf)) && read(conf) === defaultConf) return null;
    return `Clippy's own test runs it with ${conf}, not the default configuration (${DEFAULT_CONF}) the recording is made under`;
  };

  await mkdir(resolve(cacheRoot, "tmp"), { recursive: true });
  const scratch = realpathSync(await mkdtemp(resolve(cacheRoot, "tmp", "clippy-")));
  try {
    // ui_test 0.30.7's DependencyBuilder: clippy_test_deps for the host, each direct dependency passed with --extern.
    // Cargo reads its configuration from the directory it runs in and every parent, so it runs from a directory
    // outside both upstream's checkout (whose .cargo/config.toml sets nightly flags) and this repository.
    const manifest = join(checkout, "clippy_test_deps", "Cargo.toml");
    const depsEnv: NodeJS.ProcessEnv = { ...runtime.env, CARGO_TARGET_DIR: resolve(cacheRoot, "clippy", "test-deps", pack.upstream.commit), CARGO_TERM_COLOR: "never" };
    const outside = realpathSync(await mkdtemp(join(tmpdir(), "rerail-clippy-")));
    let built: string;
    let metadataText: string;
    try {
      built = await runChecked(runner, ["cargo", toolchain, "build", "--manifest-path", manifest, "--locked", `--target=${runtime.host}`, "--message-format=json"], { cwd: outside, env: depsEnv });
      metadataText = await runChecked(runner, ["cargo", toolchain, "metadata", "--manifest-path", manifest, "--locked", "--format-version=1", "--no-deps"], { cwd: outside, env: depsEnv });
    } finally {
      await rm(outside, { recursive: true, force: true });
    }
    const metadata = JSON.parse(metadataText) as {
      packages: { dependencies: { name: string; rename: string | null; kind: string | null }[] }[];
    };
    const direct = new Set(metadata.packages[0]!.dependencies.filter((dependency) => dependency.kind === null).map((dependency) => (dependency.rename ?? dependency.name).replaceAll("-", "_")));
    const externs = new Map<string, string[]>();
    const linkPaths = new Set<string>();
    for (const line of built.split("\n").filter((text) => text.startsWith("{"))) {
      const message = JSON.parse(line) as CargoMessage;
      if (message.reason === "build-script-executed") for (const path of message.linked_paths ?? []) linkPaths.add(path);
      const types = message.target?.crate_types ?? [];
      if (message.reason !== "compiler-artifact" || !types.some((type) => ["lib", "rlib", "proc-macro"].includes(type))) continue;
      const files = message.filenames ?? [];
      // A host build of a dependency (for a build script or a proc macro) is not what the tests link.
      if (!types.includes("proc-macro") && basename(dirname(dirname(dirname(files[0]!)))) !== runtime.host) continue;
      for (const file of files) linkPaths.add(dirname(file));
      const name = message.target!.name.replaceAll("-", "_");
      if (!direct.has(name)) continue;
      if (externs.has(name)) throw new Error(`clippy_test_deps built ${name} twice for ${runtime.host}`);
      externs.set(name, files);
    }
    if ([...externs.keys()].sort(byText).join(",") !== [...direct].sort(byText).join(",")) {
      throw new Error(`clippy_test_deps built ${[...externs.keys()].sort(byText).join(", ")}; it depends on ${[...direct].sort(byText).join(", ")}`);
    }
    const deps = [
      ...[...externs.keys()].sort(byText).flatMap((name) => externs.get(name)!.flatMap((file) => ["--extern", `${name}=${file}`])),
      ...[...linkPaths].sort(byText).flatMap((path) => ["-L", path]),
    ];

    // Aux crates, built once each as ui_test builds them (their own directives, crate type from their contents, --emit=link).
    const auxFiles = (test: string, text: string) => directives(text).filter(([key]) => key === "aux-build").map(([, value]) => posix.join(posix.dirname(test), "auxiliary", value));
    const auxBuilt = new Map<string, Promise<string[]>>();
    const aux = (path: string): Promise<string[]> => {
      let pending = auxBuilt.get(path);
      if (pending === undefined) {
        pending = (async () => {
          const text = read(path);
          const out = join(scratch, "aux");
          await mkdir(out, { recursive: true });
          const extra = (await Promise.all(auxFiles(path, text).map(aux))).flat();
          const args = [path, "--edition", testEdition(text), ...BASE_ARGUMENTS, ...crateType(text), "--emit=link", "--out-dir", out, ...deps, ...extra];
          const run = await runner.run([runtime.driver, ...args], { cwd: checkout, env: driverEnv("tests") });
          if (run.exitCode !== 0) throw new Error(`aux build ${path} failed:\n${run.stderr.split("\n").slice(-20).join("\n")}`);
          const produced = (await runChecked(runner, [runtime.driver, ...args, "--print", "file-names"], { cwd: checkout, env: driverEnv("tests") })).split(/\s+/u).filter((file) => file !== "");
          const crate = posix.basename(path, ".rs").replaceAll("-", "_");
          return produced.flatMap((file) => ["--extern", `${crate}=${join(out, file)}`, "-L", out]);
        })();
        auxBuilt.set(path, pending);
      }
      return pending;
    };

    // Each test run once, every ported lint at `warn` at least; findings whose primary span is in the test itself.
    const recordFlags = ported.flatMap((lint) => ["-W", `clippy::${lint}`]);
    const fixtureText = new Map(caseFixtures.map((test) => [test, read(test)]));
    const runnable = caseFixtures.filter((test) => variantReason(test) === null);
    for (const test of runnable) for (const path of auxFiles(test, fixtureText.get(test)!)) await aux(path);
    const caseFindings = new Map<string, Map<string, Span[]>>(ported.map((name) => [name, new Map()]));
    await mapLimit(runnable, 8, async (test) => {
      const text = fixtureText.get(test)!;
      const flags = directives(text).filter(([key]) => key === "compile-flags").map(([, value]) => value).join(" ").split(/\s+/u).filter((flag) => flag !== "");
      const externsOf = (await Promise.all(auxFiles(test, text).map(aux))).flat();
      const uiToml = test.startsWith(UI_TOML);
      const out = join(scratch, "out", test);
      await mkdir(out, { recursive: true });
      const args = [test, "--edition", testEdition(text), "--emit=metadata", ...BASE_ARGUMENTS, ...(uiToml ? [] : crateType(text)), ...flags, ...deps, ...externsOf, "--out-dir", out, ...recordFlags];
      const run = await runner.run([runtime.driver, ...args], { cwd: checkout, env: driverEnv(uiToml ? posix.dirname(test) : "tests") });
      const found = diagnostics(run.stderr);
      if (run.exitCode !== 0 && !found.some((diagnostic) => diagnostic.level === "error")) throw new Error(`clippy-driver failed on ${test}:\n${run.stderr.split("\n").slice(-20).join("\n")}`);
      const seen = new Set<string>();
      for (const diagnostic of found) {
        if (diagnostic.level !== "error" && diagnostic.level !== "warning") continue;
        const lint = diagnostic.code?.code.replace(/^clippy::/u, "");
        const span = primarySpan(diagnostic);
        const byFile = lint === undefined || diagnostic.code?.code === lint ? undefined : caseFindings.get(lint);
        if (byFile === undefined || span === undefined || span.file_name !== test) continue;
        const key = `${lint}\0${span.line_start}:${span.column_start}-${span.line_end}:${span.column_end}`;
        if (seen.has(key)) continue;
        seen.add(key);
        byFile.set(test, [...(byFile.get(test) ?? []), rustcSpan(span, CLIPPY_SPAN_PARTS)]);
      }
    });

    // not-overfit: upstream's test words are every UI test, aux crate and clippy.toml under tests/.
    const fixtureFiles = [
      ...filesUnder(join(checkout, UI), (path) => path.endsWith(".rs")),
      ...filesUnder(join(checkout, UI_TOML), (path) => path.endsWith(".rs") || path.endsWith("clippy.toml")),
      join(checkout, DEFAULT_CONF),
    ].map((file) => relative(checkout, file).split(sep).join("/")).sort(byText);
    const fixtureTexts = fixtureFiles.map(read);
    const fixtureTextSources: FixtureTextSource[] = fixtureFiles.map((path, index) => ({ path, sha256: sha256(fixtureTexts[index]!) }));
    // Every word of every text but number literals (a suffix such as `u8` or a hex digit run is not a name).
    const fixtureWords = new Set(fixtureTexts.flatMap((text) => text.replace(/\b\d[\w]*/gu, " ").match(/[A-Za-z_]\w*/gu) ?? []));

    // The lint's implementation: the file that declares it and, for a lint pass split into modules, the module named
    // after it; its helpers are the clippy_utils modules those files import.
    const implementationFiles = (name: string): string[] => {
      const lint = lints.get(name);
      if (lint === undefined) return [];
      const own = posix.join(posix.dirname(lint.file), `${name}.rs`);
      return own !== lint.file && existsSync(join(checkout, own)) ? [lint.file, own] : [lint.file];
    };
    const literalCache = new Map<string, RustLiterals & { symbols: string[] }>();
    const literalsOf = (path: string) => {
      let found = literalCache.get(path);
      if (found === undefined) {
        const text = read(path);
        const tokens = withoutTestItems(lexRust(text));
        // rustc and Clippy name what they match as interned symbols: `sym::unwrap`, `kw::Static`.
        const symbols = tokens.filter((token, index) => token.kind === "ident" && tokens[index - 1]?.text === "::" && ["sym", "kw"].includes(tokens[index - 2]?.text ?? "")).map((token) => token.text);
        found = { ...rustLiterals(text), symbols };
        literalCache.set(path, found);
      }
      return found;
    };
    const namesOf = (path: string) => [...literalsOf(path).names, ...literalsOf(path).symbols];
    const crateRoots = new Map([["clippy_utils", join(checkout, UTILS)]]);
    const helperFiles = (name: string): string[] => [...new Set(implementationFiles(name).flatMap((file) => literalsOf(file).uses
      .map((path) => useTarget(path, join(checkout, file), crateRoots, join(checkout, LINTS)))
      .filter((target): target is string => target !== undefined && target.startsWith(`${join(checkout, UTILS)}/`))
      .map((target) => relative(checkout, target).split(sep).join("/"))))].sort(byText);
    const regexesOf = (files: string[]) => files.flatMap((file) => literalsOf(file).regexes.flatMap((pattern) => rustRegex(pattern) ?? []));

    const declaredLicense = /^license = "([^"]+)"/mu.exec(read("Cargo.toml"))?.[1];

    const rule = (name: string): UpstreamRule | undefined => {
      const lint = lints.get(name);
      if (lint === undefined) return undefined;
      return {
        name,
        implementation: implementationFiles(name),
        tests: testsOf.get(name) ?? [],
        deprecation: deprecated.get(name) ?? null,
        recommended: DEFAULT_GROUPS.has(lint.group) ? { config: "default", options: [] } : null,
        notOnRealCode: null,
      };
    };

    // Each corpus crate's edition, by file, from the corpus runs; the parser reads a corpus file in its crate's edition.
    const corpusEditions = new Map<string, string>();
    const editionOf = (file: { path: string; source: string }) => corpusEditions.get(`${file.path}\0${sha256(file.source)}`)
      ?? directives(file.source).find(([key]) => key === "edition")?.[1] ?? DEFAULT_EDITION;
    const corpora = await readRerailCorpora(PACKAGE_ROOT, pack.rerail.pack);
    const parserName = `rustc ${runtime.pin.channel} -Zparse-crate-root-only`;

    // The configuration options every corpus unit compiles under before its own (features, test profile, debug
    // assertions, its build script's cfgs): the host's, as the driver prints them, and `clippy`, which clippy-driver
    // sets while it compiles but does not print, so a probe that compiles only with it set must compile.
    let hostOptions: Promise<Set<string>> | undefined;
    const unitBaseOptions = (): Promise<Set<string>> => (hostOptions ??= (async () => {
      const printed = await runChecked(runner, [runtime.driver, "--print", "cfg"], { cwd: tmpdir(), env: runtime.env });
      const options = new Set(printed.split("\n").map((line) => line.trim()).filter((line) => line !== "" && line !== "debug_assertions"));
      const out = realpathSync(await mkdtemp(join(tmpdir(), "rerail-clippy-cfg-")));
      try {
        const probe = await runner.run([runtime.driver, "-", "--crate-type=lib", "--crate-name=cfg_probe", "--emit=metadata", "--out-dir", out], {
          cwd: out, env: runtime.env, input: "#[cfg(not(clippy))]\ncompile_error!(\"no cfg clippy\");\n",
        });
        if (probe.exitCode !== 0) throw new Error(`clippy-driver does not set cfg clippy:\n${probe.stderr}`);
      } finally {
        await rm(out, { recursive: true, force: true });
      }
      return options.add("clippy");
    })());

    /**
     * `cargo clippy` over one pinned corpus at its commit, as packs/rust/upstream/common.py's run_corpus runs it: the
     * files some unit read (its dep-info), each with why Clippy compiled none of it when it did not, and the findings.
     */
    const lintCorpusCrate = async (corpus: RerailCorpus, root: string): Promise<{ read: Map<string, string | null>; findings: Map<string, Map<string, Span[]>>; failures: string[] }> => {
      const env: NodeJS.ProcessEnv = { ...runtime.env, CARGO_TARGET_DIR: resolve(cacheRoot, "clippy", "corpus", corpus.name, corpus.commit), CARGO_TERM_COLOR: "never" };
      const tracked = (await runChecked(runner, ["git", "-C", root, "ls-files", "--", "Cargo.lock"])).trim() !== "";
      if (!tracked) {
        // A corpus that commits no Cargo.lock (tokio) is resolved as crates.io stood at its commit, so `--locked` holds.
        if ((await runner.run(["git", "-C", root, "check-ignore", "-q", "Cargo.lock"])).exitCode !== 0) {
          throw new Error(`${corpus.name} commits no Cargo.lock and does not ignore one: resolving it would modify the pinned checkout`);
        }
        const committed = (await runChecked(runner, ["git", "-C", root, "log", "-1", "--format=%ct", corpus.commit])).trim();
        const publishTime = new Date(Number(committed) * 1000).toISOString().replace(/\.\d{3}Z$/u, "Z");
        await rm(join(root, "Cargo.lock"), { force: true });
        await runChecked(runner, ["cargo", toolchain, "generate-lockfile", "-Zunstable-options", "--publish-time", publishTime], { cwd: root, env: { ...env, RUSTC_BOOTSTRAP: "1" } });
      }
      const metadata = JSON.parse(await runChecked(runner, ["cargo", toolchain, "metadata", "--locked", "--format-version=1", "--no-deps"], { cwd: root, env })) as {
        packages: { manifest_path: string; edition: string; targets: { src_path: string; edition: string }[] }[];
      };
      const packageDirectories = metadata.packages.map((entry) => ({ directory: relativeTo(dirname(entry.manifest_path), root) ?? "", edition: entry.edition })).sort((a, b) => b.directory.length - a.directory.length);
      const run = await runner.run(["cargo", toolchain, "clippy", "--keep-going", "--locked", "--workspace", "--all-targets", "--all-features", "--message-format=json", "--", ...recordFlags], { cwd: root, env });
      // Each file some unit read, with the configuration options of every unit that read it, and each unit's root.
      const readers = new Map<string, CfgOptions[]>();
      const crateRoots = new Set<string>();
      const scriptOptions = new Map<string, string[]>();
      const baseOptions = await unitBaseOptions();
      const findings = new Map<string, Map<string, Span[]>>(ported.map((name) => [name, new Map()]));
      const seen = new Set<string>();
      const failures = new Set<string>();
      for (const line of run.stdout.split("\n").filter((text) => text.startsWith("{"))) {
        const message = JSON.parse(line) as CargoMessage;
        // A build script runs before the units of its package compile, and its cfgs are set on each of them.
        if (message.reason === "build-script-executed" && message.package_id !== undefined) scriptOptions.set(message.package_id, message.cfgs ?? []);
        if (message.manifest_path === undefined || relativeTo(message.manifest_path, root) === undefined) continue;
        if (message.reason === "compiler-artifact") {
          const options = new Set([
            ...baseOptions,
            ...(message.features ?? []).map((feature) => `feature="${feature}"`),
            ...(message.profile?.test === true ? ["test"] : []),
            ...(message.profile?.debug_assertions === true ? ["debug_assertions"] : []),
            ...(message.package_id === undefined ? [] : scriptOptions.get(message.package_id) ?? []),
          ]);
          for (const artifact of message.filenames ?? []) for (const path of depInfoSources(artifact, root)) readers.set(path, [...(readers.get(path) ?? []), options]);
          const crateRoot = message.target === undefined ? undefined : relativeTo(message.target.src_path, root);
          if (crateRoot !== undefined) crateRoots.add(crateRoot);
          continue;
        }
        if (message.reason !== "compiler-message" || message.message === undefined) continue;
        const diagnostic = message.message;
        const code = diagnostic.code?.code ?? "";
        if (diagnostic.level === "error" && !code.startsWith("clippy::") && !diagnostic.message.startsWith("aborting due to")) {
          failures.add(`${relativeTo(message.manifest_path, root)} ${message.target?.kind.join("/")} ${message.target?.name}: ${diagnostic.message}`);
        }
        const byFile = code.startsWith("clippy::") ? findings.get(code.slice("clippy::".length)) : undefined;
        const span = primarySpan(diagnostic);
        const path = span === undefined ? undefined : relativeTo(span.file_name, root);
        if (byFile === undefined || span === undefined || path === undefined) continue;
        const key = `${code}\0${path}\0${span.line_start}:${span.column_start}-${span.line_end}:${span.column_end}`;
        if (seen.has(key)) continue;
        seen.add(key);
        byFile.set(path, [...(byFile.get(path) ?? []), rustcSpan(span, CLIPPY_SPAN_PARTS)]);
      }
      if (run.exitCode !== 0 && failures.size === 0) throw new Error(`cargo clippy failed on ${corpus.name} with status ${run.exitCode} and no compile error:\n${run.stderr.split("\n").slice(-20).join("\n")}`);
      const read = new Map<string, string | null>();
      for (const [path, units] of readers) {
        const file = join(root, path);
        if (!existsSync(file)) continue;
        const source = await readFile(file, "utf8");
        const owner = packageDirectories.find(({ directory }) => directory === "" || path.startsWith(`${directory}/`));
        if (owner !== undefined) corpusEditions.set(`${path}\0${sha256(source)}`, owner.edition);
        // A module file is read and dropped whole in a unit where one of its leading `#![cfg(…)]` is false: rustc
        // strips the module, its inner attributes with it, before any lint runs. When that holds in every unit that
        // read it, Clippy's silence on it is no answer. A crate root keeps its crate attributes, which Clippy still
        // lints (`#![allow]` before a false `#![cfg]`), so it is not dropped; nor is a file whose predicate this cannot
        // evaluate.
        const predicates = innerCfgPredicates(source);
        const dropped = !crateRoots.has(path) && predicates.length > 0 && units.every((options) => predicates.some((predicate) => cfgHolds(predicate, options) === false));
        const attributes = predicates.map((predicate) => `\`#![cfg(${predicate.map((token) => token.text).join("")})]\``).join(", ");
        read.set(path, dropped
          ? `Clippy compiled none of it: of its ${attributes}, one is false in each of the ${units.length} unit(s) \`cargo clippy --workspace --all-targets --all-features\` read it in, so rustc dropped its contents before any lint ran`
          : null);
        if (dropped && ported.some((name) => findings.get(name)?.has(path))) throw new Error(`${corpus.name}: Clippy reports on ${path}, which every unit that read it dropped by its #![cfg]`);
      }
      return { read, findings, failures: [...failures].sort(byText) };
    };

    return {
      identity: {
        repository: pack.upstream.repository,
        commit: pack.upstream.commit,
        version: pack.upstream.version,
        runtime: `${runtime.clippy}, from the rustup toolchain ${runtime.pin.channel} (\`${runtime.rustc}\`)`,
        invocation: "clippy-driver on each UI test as tests/compile-test.rs (ui_test 0.30.7) runs it: edition 2024 unless //@edition, //@compile-flags, //@aux-build crates and clippy_test_deps passed with --extern, ui_test's crate type, --emit=metadata -Aunused -Ainternal_features -Zui-testing -Zdeduplicate-diagnostics=no, RUSTC_BOOTSTRAP=1, without -Dwarnings",
        configuration: `Clippy's default lint levels, with -W clippy::<lint> for every ported lint; CLIPPY_CONF_DIR=tests (${DEFAULT_CONF}); a tests/ui-toml test with another clippy.toml is skipped`,
        corpus: `cargo ${toolchain} clippy --keep-going --locked --workspace --all-targets --all-features --message-format=json -- -W clippy::<lint>… in each pinned corpus (RUSTFLAGS unset); a corpus that commits no Cargo.lock is resolved with cargo generate-lockfile --publish-time <its commit's time>; a file no compiled unit read (dep-info) is excluded, and so is one whose leading #![cfg(…)] is false in every unit that read it (the unit's host, feature, test, debug_assertions, build-script and clippy options)`,
        positions: "the primary span of each diagnostic: rustc's 1-based lines and character columns, start and end",
        parsers: `${parserName} (rustc's parser alone: syntax errors, no grammar checks), in the test's or the corpus crate's edition`,
      },
      packChecks: [
        ...upstream.checks,
        {
          id: "runtime-version",
          ok: runtime.clippyVersion === pack.upstream.version,
          detail: `the rustup toolchain ${runtime.pin.channel} is \`${runtime.rustc}\` with \`${runtime.clippy}\`; the pack pins clippy ${pack.upstream.version}`,
        },
        {
          id: "declared-license",
          ok: declaredLicense === pack.upstream.license,
          detail: `Cargo.toml at the pinned commit declares ${declaredLicense}; the pack records ${pack.upstream.license}`,
        },
      ],
      licenseText: upstreamLicenseText(checkout),
      rule,

      implementationNames: (name) => new Set(implementationFiles(name).flatMap(namesOf)),
      sharedImplementationNames: (name) => new Set(helperFiles(name).flatMap(namesOf)),
      implementationRegexes: (name) => regexesOf([...implementationFiles(name), ...helperFiles(name)]),
      fixtureWords: () => fixtureWords,
      fixtureTexts: () => fixtureTexts,
      fixtureTextSources: () => fixtureTextSources,
      keywords: new Set([...RUST_KEYWORDS, ...RUST_VOCABULARY]),
      // rustc and Clippy give no word inside a comment a meaning (lints are configured by attributes and clippy.toml).
      commentVocabulary: new Set(),
      inScopeSamples: ["src/example.rs"],

      cases(name: string): UpstreamCase[] {
        if (!lints.has(name)) return [];
        return caseFixtures.map((fixture): UpstreamCase => {
          const skipped = variantReason(fixture);
          return {
            fixture,
            variant: fixture.startsWith(UI_TOML) ? posix.dirname(fixture).slice(UI_TOML.length) : ".",
            path: fixture,
            source: fixtureText.get(fixture)!,
            skipped,
            expected: skipped === null ? caseFindings.get(name)?.get(fixture) ?? [] : [],
            // Each .stderr records the test's own levels, not every ported lint at `warn`; generate.py compares them
            // (fixtures.upstream-check.json).
            markup: null,
          };
        });
      },

      toSpan: (_name, source, startByte, endByte) => clippySpan(source, startByte, endByte, CLIPPY_SPAN_PARTS),

      inCorpus: (path: string) => path.endsWith(".rs"),

      async lintCorpus(files: CorpusFile[], ruleNames: string[]): Promise<UpstreamFile[]> {
        const answers = new Map<string, UpstreamFile>();
        for (const corpus of corpora) {
          const task = rerailCorpusId(corpus);
          const mine = files.filter((file) => file.task === task);
          if (mine.length === 0) continue;
          // The gates fetch only the files goldens name; cargo builds the whole workspace, so it gets the corpus whole.
          const root = await checkoutPinned(corpus.repository, corpus.commit, resolve(cacheRoot, "clippy", "checkout", corpus.name, corpus.commit), ["/*"], runner);
          const result = await lintCorpusCrate(corpus, root);
          for (const file of mine) {
            const dropped = result.read.get(file.path);
            answers.set(`${task}\0${file.path}`, dropped === undefined
              ? { file, excluded: "no unit `cargo clippy --workspace --all-targets --all-features` compiled reads it (its dep-info): Clippy lints a file only as part of a crate", findings: new Map() }
              : dropped !== null
                ? { file, excluded: dropped, findings: new Map() }
                : { file, excluded: null, findings: new Map(ruleNames.map((name) => [name, result.findings.get(name)?.get(file.path) ?? []])) });
          }
        }
        return files.map((file) => answers.get(`${file.task}\0${file.path}`)
          ?? { file, excluded: "Clippy lints crates, not single files: this file is not built as part of a pinned crate", findings: new Map() });
      },

      async parserVerdicts(files): Promise<ParserVerdict[][]> {
        return mapLimit(files, 8, async (file) => {
          const run = await runner.run(["rustc", toolchain, "-Zparse-crate-root-only", "--edition", editionOf(file), "--error-format=json", "--crate-name", "parsed", "-"], {
            cwd: tmpdir(), env: { ...runtime.env, RUSTC_BOOTSTRAP: "1" }, input: file.source,
          });
          return [{ parser: parserName, syntax: run.exitCode === 0 ? [] : parseErrors(run.stderr), grammar: null }];
        });
      },
    };
  } finally {
    await rm(scratch, { recursive: true, force: true });
  }
}
