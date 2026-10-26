import { createHash } from "node:crypto";
import { existsSync, readFileSync, realpathSync } from "node:fs";
import { mkdir, mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, join, posix, relative, resolve, sep } from "node:path";

import { runChecked, type CommandRunner } from "../../../port-readiness/process.js";
import { upstreamLicenseText } from "../../../port-readiness/recorded-adapter.js";
import { clippySpan, rustcSpan, type RustcSpan } from "../../../port-readiness/span-units/clippy.js";
import type { FixtureTextSource, PackAdapter, PackManifest, ParserVerdict, Span, SpanParts, UpstreamCase, UpstreamRule } from "../../../port-readiness/types.js";
import { closingIndex, lexRust, rustLiterals, rustRegex, withoutTestItems, type RustLiterals } from "../languages/rust.js";
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
 * configuration: a tests/ui-toml test whose clippy.toml is not tests/clippy.toml is skipped, with the reason. So is a
 * test rustc rejects (`compileError`), for every lint when it stops before Clippy's lint passes run, and for the lints
 * of its late passes when it stops in analysis, after the early passes (`earlyPassLints`) have reported.
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
  filenames?: string[];
  target?: { name: string; crate_types: string[] };
  linked_paths?: string[];
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

/**
 * The first compile error in rustc's diagnostics, with its code when it has one: an `E` code, or none (a syntax error),
 * not a lint raised to `error` (`deny`, or a Clippy lint upstream's test sets to `deny`), which the lint passes report
 * beside the code they read, nor rustc's closing "aborting due to" summary. Undefined when rustc rejects nothing.
 */
export function compileError(found: readonly Pick<RustcDiagnostic, "level" | "code" | "message">[]): string | undefined {
  const error = found.find((diagnostic) => diagnostic.level === "error"
    && !diagnostic.message.startsWith("aborting due to")
    && (diagnostic.code === null || /^E\d{4}$/u.test(diagnostic.code.code)));
  return error === undefined ? undefined : `${error.code === null ? "" : `${error.code.code}: `}${error.message.split("\n")[0]}`;
}

/**
 * The lints only Clippy's early lint passes check, from clippy_lints' sources: every lint a `declare_lint_pass!` or
 * `impl_lint_pass!` lists for a pass type, unless a pass that lists it implements `LateLintPass`. rustc runs the early
 * passes before analysis, so they have reported everything when a test is rejected in type checking; the late passes
 * run only on a crate that type-checks.
 */
export function earlyPassLints(sources: readonly string[]): Set<string> {
  const listed: [pass: string, lints: string[]][] = [];
  const late = new Set<string>();
  for (const source of sources) {
    const tokens = lexRust(source);
    tokens.forEach((token, index) => {
      if ((token.text === "declare_lint_pass" || token.text === "impl_lint_pass") && tokens[index + 1]?.text === "!" && tokens[index + 2]?.text === "(") {
        const body = tokens.slice(index + 3, closingIndex(tokens, index + 2));
        const pass = body.find((inner) => inner.kind === "ident")?.text;
        const open = body.findIndex((inner) => inner.text === "[");
        if (pass !== undefined && open >= 0) listed.push([pass, body.slice(open + 1, closingIndex(body, open)).filter((inner) => inner.kind === "ident").map((inner) => inner.text.toLowerCase())]);
      }
      if (token.text === "impl") {
        const end = tokens.findIndex((inner, position) => position > index && (inner.text === "{" || inner.text === ";"));
        const header = tokens.slice(index + 1, end < 0 ? tokens.length : end);
        const at = header.findIndex((inner) => inner.text === "for");
        const pass = header.slice(at + 1).find((inner) => inner.kind === "ident")?.text;
        if (at >= 0 && pass !== undefined && header.slice(0, at).some((inner) => inner.text === "LateLintPass")) late.add(pass);
      }
    });
  }
  const lateLints = new Set(listed.filter(([pass]) => late.has(pass)).flatMap(([, lints]) => lints));
  return new Set(listed.flatMap(([, lints]) => lints).filter((lint) => !lateLints.has(lint)));
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

  // Every lint Clippy declares, and its group; and the lints only its early lint passes check.
  const lints = new Map<string, DeclaredLint>();
  const lintSources: string[] = [];
  for (const file of filesUnder(join(checkout, LINTS), (path) => path.endsWith(".rs"))) {
    const path = relative(checkout, file).split(sep).join("/");
    const source = readFileSync(file, "utf8");
    lintSources.push(source);
    for (const lint of declaredLints(source)) lints.set(lint.name, { ...lint, file: path });
  }
  const earlyLints = earlyPassLints(lintSources);
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
    // A test rustc rejects, and whether its early lint passes ran: rejected only in analysis, after them.
    const rejected = new Map<string, { reason: string; earlyRan: boolean }>();
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
      const error = compileError(found);
      if (error !== undefined) {
        // rustc stopped by -Zno-analysis has parsed, loaded crates and resolved names, and run no lint pass: an error
        // there is one before the early passes, which never reported. Otherwise the early passes ran in full.
        const beforeAnalysis = await runner.run([runtime.driver, ...args, "-Zno-analysis"], { cwd: checkout, env: driverEnv(uiToml ? posix.dirname(test) : "tests") });
        const early = compileError(diagnostics(beforeAnalysis.stderr));
        rejected.set(test, early === undefined
          ? { reason: `rustc rejects it in analysis (${error}), before Clippy's late lint passes run, so this lint has no answer for it`, earlyRan: true }
          : { reason: `rustc rejects it before Clippy's lint passes run (${early}), so Clippy has no answer for it`, earlyRan: false });
        if (early !== undefined) return;
      }
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

    const parserName = `rustc ${runtime.pin.channel} -Zparse-crate-root-only`;

    return {
      identity: {
        repository: pack.upstream.repository,
        commit: pack.upstream.commit,
        version: pack.upstream.version,
        runtime: `${runtime.clippy}, from the rustup toolchain ${runtime.pin.channel} (\`${runtime.rustc}\`)`,
        invocation: "clippy-driver on each UI test as tests/compile-test.rs (ui_test 0.30.7) runs it: edition 2024 unless //@edition, //@compile-flags, //@aux-build crates and clippy_test_deps passed with --extern, ui_test's crate type, --emit=metadata -Aunused -Ainternal_features -Zui-testing -Zdeduplicate-diagnostics=no, RUSTC_BOOTSTRAP=1, without -Dwarnings",
        configuration: `Clippy's default lint levels, with -W clippy::<lint> for every ported lint; CLIPPY_CONF_DIR=tests (${DEFAULT_CONF}); a tests/ui-toml test with another clippy.toml is skipped, and so is a test rustc rejects with a compile error, for every lint when -Zno-analysis rejects it too and for the lints of late passes otherwise`,
        positions: "the primary span of each diagnostic: rustc's 1-based lines and character columns, start and end",
        parsers: `${parserName} (rustc's parser alone: syntax errors, no grammar checks), in the test's edition`,
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
          const rejection = rejected.get(fixture);
          const skipped = variantReason(fixture) ?? (rejection === undefined || (rejection.earlyRan && earlyLints.has(name)) ? null : rejection.reason);
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

      async parserVerdicts(files): Promise<ParserVerdict[][]> {
        return mapLimit(files, 8, async (file) => {
          // The parser reads a file in the edition its `//@edition` directive names, else the default.
          const edition = directives(file.source).find(([key]) => key === "edition")?.[1] ?? DEFAULT_EDITION;
          const run = await runner.run(["rustc", toolchain, "-Zparse-crate-root-only", "--edition", edition, "--error-format=json", "--crate-name", "parsed", "-"], {
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
