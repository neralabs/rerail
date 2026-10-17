// The native engine as the data-layer benchmark drives it: one addon, the ported rules compiled against the bundled
// capabilities, and one incremental workspace per evaluation.
import { createHash } from "node:crypto";
import { existsSync, readFileSync, readdirSync, realpathSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import {
  loadNativeBinding,
  resolveNativeBindingPath,
  type NativeBinding,
  type NativeCodeRuleDslWorkspace,
  type NativeCompiledCodeRuleDsl,
} from "../../src/engine/index.js";

export const rerailRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const bundledCapabilitiesPath = resolve(rerailRoot, "native/src/code_rule_dsl/capabilities.json");

/** What the `unbounded-work` benchmark build reports from `health()` (native/src/lib.rs). */
export const UNBOUNDED_WORK_SUFFIX = "+unbounded-work";

export function sha256(value: string | Buffer): string {
  return `sha256:${createHash("sha256").update(value).digest("hex")}`;
}

export function compareStrings(left: string, right: string): number {
  if (left < right) return -1;
  if (left > right) return 1;
  return 0;
}

export interface SourceFile {
  path: string;
  language: "python";
  source: string;
  /** The Python module path from the package markers on disk, as the enforcer sends it. */
  module?: string | string[];
}

/** A finding as the data-layer linters report it: the line is 1-based; 0 means the file itself. */
export interface FindingKey {
  code: string;
  path: string;
  line: number;
}

const rangeSchema = z.object({
  startByte: z.number().int(), endByte: z.number().int(),
  startLine: z.number().int().nonnegative(), startColumn: z.number().int(),
  endLine: z.number().int(), endColumn: z.number().int(),
});
const violationSchema = z.object({
  ruleId: z.string(),
  clauseId: z.string(),
  code: z.string(),
  message: z.string(),
  path: z.string().optional(),
  span: rangeSchema.optional(),
});
const failureSchema = z.object({
  ruleId: z.string().optional(),
  clauseId: z.string(),
  code: z.string(),
  message: z.string(),
  path: z.string().optional(),
});
const blindFileSchema = z.object({ path: z.string() }).passthrough();
const responseSchema = z.object({
  revision: z.number().int(),
  blindFiles: z.array(blindFileSchema).default([]),
  evaluation: z.object({
    status: z.string(),
    violations: z.array(violationSchema),
    failures: z.array(failureSchema),
    metrics: z.object({ files: z.number(), visitedNodes: z.number(), witnesses: z.number() }).passthrough(),
  }),
});
export type EngineViolation = z.infer<typeof violationSchema>;
export type EngineFailure = z.infer<typeof failureSchema>;
export type BlindFile = z.infer<typeof blindFileSchema>;
export type EngineResult = z.infer<typeof responseSchema>["evaluation"] & { blindFiles: BlindFile[] };

/** Spans are 0-based; the linters report 1-based lines. A finding without a span is about its file: line 0. */
export function findingKey(finding: EngineViolation): FindingKey {
  return { code: finding.code, path: finding.path ?? "", line: finding.span === undefined ? 0 : finding.span.startLine + 1 };
}

const field = (value: string | undefined) => (value ?? "").replace(/[\t\n\\]/g, character => (character === "\t" ? "\\t" : character === "\n" ? "\\n" : "\\\\"));

/** One tab-separated line per violation: rule, clause, code, path, 0-based span, message. */
export function violationLine(violation: EngineViolation): string {
  const span = violation.span === undefined ? "-" : `${violation.span.startLine}:${violation.span.startColumn}-${violation.span.endLine}:${violation.span.endColumn}`;
  return ["violation", violation.ruleId, violation.clauseId, violation.code, violation.path, span, violation.message].map(field).join("\t");
}

/** One tab-separated line per failure: rule, clause, code, path, message. */
export function failureLine(failure: EngineFailure): string {
  return ["failure", failure.ruleId, failure.clauseId, failure.code, failure.path, failure.message].map(field).join("\t");
}

export function blindFileLine(file: BlindFile): string {
  const { path, ...rest } = file;
  return ["blind", path, JSON.stringify(rest)].map(field).join("\t");
}

export interface Engine {
  binding: NativeBinding;
  addon: { path: string; sha256: string; kernelVersion: string; work: "unbounded" | "bounded" };
  /** Rayon worker threads the addon evaluates on (`nativeMemoryProfile().workerPool.threads`). */
  workerThreads: number;
  capabilities: { path: string; sha256: string; source: string };
}

/**
 * Loads the addon at `addonPath` (else `RERAIL_ENGINE_PATH`, else the package's own `native/` build). The path is
 * pinned in `RERAIL_ENGINE_PATH` so every later `loadNativeBinding()` in this process loads the same file. Rules
 * compile against `capabilitiesPath` (default: the bundled capabilities, whose `limits` production enforces).
 */
export function loadEngine(addonPath?: string, capabilitiesPath = bundledCapabilitiesPath): Engine {
  if (addonPath !== undefined) process.env.RERAIL_ENGINE_PATH = resolve(addonPath);
  const path = realpathSync(resolveNativeBindingPath());
  process.env.RERAIL_ENGINE_PATH = path;
  const binding = loadNativeBinding();
  const kernelVersion = binding.health().kernelVersion;
  const memory = z.object({ workerPool: z.object({ threads: z.number() }) }).parse(JSON.parse(binding.nativeMemoryProfile()));
  const capabilities = readFileSync(capabilitiesPath, "utf8");
  return {
    binding,
    addon: {
      path,
      sha256: sha256(readFileSync(path)),
      kernelVersion,
      work: kernelVersion.endsWith(UNBOUNDED_WORK_SUFFIX) ? "unbounded" : "bounded",
    },
    workerThreads: memory.workerPool.threads,
    capabilities: { path: capabilitiesPath, sha256: sha256(capabilities), source: capabilities },
  };
}

export interface PortedRule {
  /** The data-layer linter the rule ports: its directory under the policies. */
  linter: string;
  path: string;
  sha256: string;
  clauses: number;
  document: unknown;
  compiled: NativeCompiledCodeRuleDsl;
}

const ruleDocumentSchema = z.object({ id: z.string().min(1), clauses: z.array(z.unknown()) });

/**
 * Every `<linter>/policy.json` under `directory`, sorted by linter; `only` (when non-empty) selects linters and must all
 * exist. A rule that does not compile throws: the conversion script (`convert.ts`) is what reports those.
 */
export function loadRules(engine: Engine, directory: string, only: readonly string[] = []): PortedRule[] {
  const linters = readdirSync(directory, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && existsSync(resolve(directory, entry.name, "policy.json")))
    .map(entry => entry.name)
    .sort(compareStrings);
  const missing = only.filter(linter => !linters.includes(linter));
  if (missing.length > 0) throw new Error(`No policy for ${missing.join(", ")} in ${directory}`);
  return linters
    .filter(linter => only.length === 0 || only.includes(linter))
    .map(linter => {
      const path = resolve(directory, linter, "policy.json");
      const source = readFileSync(path, "utf8");
      const document = ruleDocumentSchema.parse(JSON.parse(source));
      let compiled: NativeCompiledCodeRuleDsl;
      try {
        compiled = engine.binding.CodeRuleDslRuntime.compile(source, "json", engine.capabilities.source);
      } catch (error) {
        throw new Error(`${path} (${document.id}) does not compile: ${error instanceof Error ? error.message : String(error)}`);
      }
      return { linter, path, sha256: sha256(source), clauses: document.clauses.length, document, compiled };
    });
}

/** The shared per-file work a rule's own cost is measured above: a parse for AST rules, reading the text for regex-only ones. */
export type BaselineKind = "parse" | "text";

/** Whether any clause of the rule document runs an AST query (anywhere: `forbid`, `select_when`, defines). */
export function baselineKind(document: unknown): BaselineKind {
  const pending: unknown[] = [document];
  while (pending.length > 0) {
    const value = pending.pop();
    if (Array.isArray(value)) pending.push(...value);
    else if (typeof value === "object" && value !== null) {
      if ("ast" in value || "define" in value) return "parse";
      pending.push(...Object.values(value));
    }
  }
  return "text";
}

/**
 * A rule every Python file is in scope of and that never finds anything: what a file costs with no rule's own work.
 * `parse`: one AST clause that forces the shared per-file AST work and then matches at most the root node. `text`: one
 * regex clause over the file's text that never matches.
 */
export function compileBaselineRule(engine: Engine, kind: BaselineKind): PortedRule {
  const never = "[^\\s\\S]";
  const document = {
    format: "rerail.policy/v1",
    id: `rerail-benchmark:data-layer-${kind}-baseline`,
    revision: 1,
    mode: "steer",
    description: `${kind} baseline for benchmark/data-layer: one clause that never matches.`,
    scope: { include: ["**/*.py"], exclude: [] },
    clauses: [{
      id: "never",
      forbid: kind === "parse"
        ? { ast: { language: "python", rule: { kind: "module", regex: never } } }
        : { regex: { source: "file.content", pattern: never } },
      violation: { code: "RERAIL_BENCHMARK_BASELINE", message: "never reported", anchor: "witness.span" },
    }],
  };
  const source = JSON.stringify(document);
  return {
    linter: `(${kind} baseline)`, path: "(built in)", sha256: sha256(source), clauses: 1, document,
    compiled: engine.binding.CodeRuleDslRuntime.compile(source, "json", engine.capabilities.source),
  };
}

export interface TimedApply {
  /** The native `applyAsync` call alone (request encoding and result decoding excluded). */
  wallMs: number;
  /** Process CPU (user + system, every thread) over the same call. */
  cpuMs: number;
  result: EngineResult;
}

/** Which engine matches the rules' AST clauses (`--engine`), and the files per SQL batch (`--sql-batch`). */
export interface Matcher {
  engine: "native" | "sql";
  sqlBatchSize?: number;
}

export const nativeMatcher: Matcher = { engine: "native" };

const refusalSchema = z.object({ ruleId: z.string(), clauseId: z.string(), language: z.string(), reason: z.string() });
export const engineReportSchema = z.union([
  z.object({ engine: z.literal("native") }),
  z.object({
    engine: z.literal("sql"),
    batchSize: z.number().int(),
    clauses: z.number().int(),
    compiled: z.number().int(),
    ran: z.number().int(),
    refused: z.array(refusalSchema),
    statements: z.object({ count: z.number(), rows: z.number() }).passthrough(),
  }),
]);
export type EngineReport = z.infer<typeof engineReportSchema>;

/** One native code-rule workspace without a content store: files are applied in revisions, every result covers every resident file. */
export class RuleWorkspace {
  readonly #workspace: NativeCodeRuleDslWorkspace;
  #revision = 0;

  constructor(engine: Engine, rules: readonly PortedRule[], matcher: Matcher = nativeMatcher) {
    this.#workspace = new engine.binding.CodeRuleDslWorkspace(matcher);
    for (const rule of rules) this.#workspace.addCodeRule(rule.compiled);
  }

  accepts(path: string): boolean {
    return this.#workspace.acceptsPath(path);
  }

  /** The engine's report: on SQL, the clauses it refuses and why, and how many it compiled and ran. */
  engineReport(): EngineReport {
    return engineReportSchema.parse(JSON.parse(this.#workspace.engineReport()));
  }

  async apply(files: readonly SourceFile[], removed: readonly string[] = []): Promise<TimedApply> {
    this.#revision += 1;
    const request = JSON.stringify({ revision: this.#revision, files, removed });
    const cpu = process.cpuUsage();
    const started = performance.now();
    const serialized = await this.#workspace.applyAsync(request);
    const wallMs = performance.now() - started;
    const used = process.cpuUsage(cpu);
    const response = responseSchema.parse(JSON.parse(serialized));
    if (response.revision !== this.#revision) throw new Error(`Native workspace answered revision ${response.revision}, not ${this.#revision}`);
    return { wallMs, cpuMs: (used.user + used.system) / 1000, result: { ...response.evaluation, blindFiles: response.blindFiles } };
  }

  close(): void {
    this.#workspace.close();
  }
}
