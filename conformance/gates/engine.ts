/**
 * The engine the gates judge rules on: this checkout's own, its TypeScript modules imported from src/engine/index.ts
 * and its native binding loaded the way the rest of conformance/ loads it. `loads` therefore means "compiles and
 * loads on this commit's engine"; nothing is pinned, fetched or built here.
 */
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import {
  bundledCodeRuleDslDirectory,
  CODE_RULE_DIALECTS,
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
 * registry and the TypeScript engine are built from, and the native binary itself, which is not bit-reproducible.
 */
export interface EngineIdentity {
  /** sha256 over each tracked engine source file's path and the sha256 of its bytes in the working tree. */
  sources: string;
  /** sha256 of the native binary the run loaded. */
  nativeBinarySha256: string;
  /** sha256 of both: the engine's part of a rule's cache key. */
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

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

/**
 * The engine's identity in the checkout at `root`: every file `git ls-files` lists under ENGINE_SOURCES, hashed from
 * the working tree (a tracked file deleted there counts as deleted), and the native binary at `binaryPath`.
 */
export function engineIdentity(root: string = PACKAGE_ROOT, binaryPath: string = resolveNativeBindingPath()): EngineIdentity {
  const listed = execFileSync("git", ["ls-files", "-z", "--", ...ENGINE_SOURCES], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 })
    .split("\0")
    .filter((path) => path !== "")
    .sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
  const lines = [...new Set(listed)].map((path) => {
    const absolute = join(root, path);
    return `${path}\0${existsSync(absolute) ? sha256(readFileSync(absolute)) : "deleted"}`;
  });
  const sources = sha256(lines.join("\n"));
  const nativeBinarySha256 = sha256(readFileSync(binaryPath));
  return { sources, nativeBinarySha256, sha256: sha256(`${sources}\n${nativeBinarySha256}`) };
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
  const binaryPath = resolveNativeBindingPath();
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
    identity: engineIdentity(root, binaryPath),
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
