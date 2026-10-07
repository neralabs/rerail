import { type Dirent, type Stats } from "node:fs";
import { lstat, mkdtemp, opendir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { setImmediate as yieldToHost, setTimeout as pause } from "node:timers/promises";
import { diffLines } from "diff";
import { z } from "zod";
import {
  encodePolicyWorkspaceRequest,
  loadNativeBinding,
  type NativePolicyDslWorkspace,
  type NativeRepositoryIndexOptions,
} from "./native.js";
import type { PolicyDslCatalog } from "./policy-dsl-catalog.js";
import {
  executablePolicySteering,
  type ExecutablePolicyEdit,
  type ExecutablePolicyEditBatch,
  type ExecutablePolicyEvaluator,
  type ExecutablePolicyFinding,
  type ExecutablePolicyResponse,
  type PolicyEvaluationSettings,
  type PolicySourceLanguage,
  type PolicySteeringDelivery,
} from "./executable-policy.js";
import {
  POLICY_COVERAGE_STATUSES,
  type PolicyCheckCoverage,
  type PolicyExecutionFailure,
  type PolicyExecutionRecorder,
  type PolicyMode,
  type PolicyRunResult,
  type RuleEvaluationProvenance,
  type PolicyScope,
} from "./policy-execution.js";
import { defaultRerailDatabasePath } from "./database.js";
import { SqlitePolicyExecutionStore } from "./policy-execution-store.js";
import { compareDeterministicStrings } from "./stable-json.js";
import { assertContentStoreOwner } from "./main-instance.js";
import { contentStorePath } from "./rerail-build.js";
import { withinCheckout } from "./checkout-control.js";
import { languageOfPath, POLICY_DIALECT_SUFFIXES, POLICY_DIALECTS } from "./policy-languages.js";


const MAX_SOURCE_BYTES = 32_000_000;
const MAX_PENDING_BYTES = 8 * 1024 * 1024;
const SCAN_BATCH_FILES = 32;
const SCAN_CHUNK_BYTES = 1024 * 1024;
const SCAN_MAX_DURATION_MS = 5 * 60_000;
const SCAN_MAX_FILES = 100_000;
const SCAN_MAX_FAILURES = 20;
const SCAN_MAX_SOURCE_BYTES = 512 * 1024 * 1024;
const RESULT_PAGE_BYTES = 8 * 1024 * 1024;
export const IGNORED_DIRECTORIES = new Set([
  ".git",
  ".hg",
  ".svn",
  ".rerail",
  ".nera",
  ".venv",
  "__pycache__",
  "node_modules",
  "dist",
  "build",
  ".next",
  ".turbo",
  "coverage",
  "target",
  "vendor",
]);
/** The extensions scans evaluate and the file label each selects; native discovery lists the same. */
export const SOURCE_LANGUAGES: ReadonlyMap<string, PolicySourceLanguage> = new Map<string, PolicySourceLanguage>([
  ...POLICY_DIALECT_SUFFIXES,
  [".rs", "rust"],
  [".json", "json"],
  [".yaml", "yaml"],
  [".yml", "yaml"],
  [".toml", "toml"],
  [".md", "markdown"],
  [".mdx", "markdown"],
  [".rst", "text"],
  [".txt", "text"],
  [".sql", "text"],
  [".sh", "text"],
  [".bash", "text"],
  [".zsh", "text"],
  [".html", "text"],
  [".css", "text"],
  [".scss", "text"],
]);
/** A repository index honours the walk's bounds, ignored directories and source languages. */
const REPOSITORY_INDEX_OPTIONS: NativeRepositoryIndexOptions = {
  maxFiles: SCAN_MAX_FILES,
  maxSourceBytes: SCAN_MAX_SOURCE_BYTES,
  maxFileBytes: MAX_SOURCE_BYTES,
  ignoredDirectories: [...IGNORED_DIRECTORIES],
  sourceLanguages: Object.fromEntries(SOURCE_LANGUAGES),
};
const sourceSpanSchema = z.strictObject({
  startByte: z.number().int().nonnegative(),
  endByte: z.number().int().nonnegative(),
  startLine: z.number().int().nonnegative(),
  startColumn: z.number().int().nonnegative(),
  endLine: z.number().int().nonnegative(),
  endColumn: z.number().int().nonnegative(),
});
const findingSchema = z.strictObject({
  id: z.string().min(1),
  policyId: z.string().min(1),
  checkId: z.string().min(1),
  code: z.string().min(1),
  severity: z.enum(["advisory", "warning", "error"]),
  message: z.string(),
  path: z.string().optional(),
  span: sourceSpanSchema.optional(),
  captures: z.record(z.string(), z.array(sourceSpanSchema)),
  variables: z.record(z.string(), z.union([z.boolean(), z.number(), z.string()])),
  fix: z.strictObject({ span: sourceSpanSchema, text: z.string() }).optional(),
});
const failureSchema = z.strictObject({
  policyId: z.string().optional(),
  checkId: z.string().min(1),
  code: z.enum(["budget-exceeded", "missing-capability", "source-incomplete", "source-partial", "adapter-error"]),
  message: z.string(),
  path: z.string().optional(),
  stage: z.enum(["read", "parse", "evaluate", "scan", "scope", "persistence"]).optional(),
});
const coverageSchema: z.ZodType<PolicyCheckCoverage> = z.strictObject({
  checkId: z.string().min(1),
  policyId: z.string().optional(),
  status: z.enum(POLICY_COVERAGE_STATUSES),
  reasons: z.array(z.strictObject({
    code: failureSchema.shape.code,
    count: z.number().int().positive(),
    paths: z.array(z.string()),
  })),
});
// The enforcer reports findings and failures; it validates the native per-check coverage and does not keep it, since a
// delta result's coverage covers its replaced paths only.
const evaluationSchema = z
  .strictObject({
    status: z.enum(["completed", "failed"]),
    findings: z.array(findingSchema),
    failures: z.array(failureSchema),
    coverage: z.array(coverageSchema),
    metrics: z.strictObject({
      files: z.number().int().nonnegative(),
      visitedNodes: z.number().int().nonnegative(),
      witnesses: z.number().int().nonnegative(),
    }),
  })
  .transform(({ coverage: _coverage, ...evaluation }) => evaluation);
const resultPageSchema = z.strictObject({
  revision: z.number().int().nonnegative(),
  cursor: z.number().int().nonnegative(),
  done: z.boolean(),
  findings: z.array(findingSchema),
  failures: z.array(failureSchema),
  coverage: z.array(coverageSchema),
  bytes: z.number().int().nonnegative(),
});
const workspaceResultSchema = z.strictObject({
  revision: z.number().int().nonnegative(),
  replacedPaths: z.array(z.string()).nullable().optional(),
  evaluation: evaluationSchema,
});
const repositoryIndexChunkSchema = z.strictObject({
  revision: z.number().int().nonnegative(),
  done: z.boolean(),
  overBudget: z.boolean(),
  evaluation: evaluationSchema.nullable(),
  failures: z.array(z.strictObject({
    path: z.string(),
    code: z.enum(["budget-exceeded", "source-incomplete"]),
    message: z.string(),
  })),
  discoveredFiles: z.number().int().nonnegative(),
  restoredFiles: z.number().int().nonnegative(),
  readFiles: z.number().int().nonnegative(),
  readBytes: z.number().int().nonnegative(),
  reusableDirectories: z.number().int().nonnegative(),
});
const storedEditSchema = z.object({
  path: z.string(),
  language: z.enum([...new Set(SOURCE_LANGUAGES.values())] as [PolicySourceLanguage, ...PolicySourceLanguage[]]),
  generatedText: z.string().optional(),
  previousText: z.string().optional(),
  previousReadError: z.string().optional(),
  deleted: z.boolean().optional(),
  readError: z.string().optional(),
});
type Evaluation = z.infer<typeof evaluationSchema>;
type WorkspaceFile = { language: PolicySourceLanguage; path: string; source: string; module?: string | string[] };
type StoredEdit = {
  edit?: ExecutablePolicyEdit;
  spool?: string;
  bytes: number;
  references: number;
  revision: number;
  scope: PolicyScope;
  mode: PolicyMode;
  runId: string | undefined;
};
type PreparedWorkspace = {
  workspace: NativePolicyDslWorkspace;
  version: string;
  provenance: readonly RuleEvaluationProvenance[];
  /** The storage attach, settled before the workspace's first apply; true when a Git store attached (repository scans then index natively). */
  storage: Promise<boolean>;
};
type ByteRange = { start: number; end: number };
type DiagnosticScanState = { bytes: number; deadline: number; files: number; stopped: boolean };
type DiagnosticScanCompletion = {
  workspace: NativePolicyDslWorkspace;
  runId: string | undefined;
  revision: number;
  scope: "repository" | "touched-files";
  mode: PolicyMode;
  sourcePaths: readonly string[];
  failures: readonly PolicyExecutionFailure[];
  stopped: boolean;
  generatedEdit: boolean;
  /** The native repository index produced the results; a complete one is recorded for reuse. */
  indexed: boolean;
};
type DiagnosticScanSources = { sourcePaths: string[]; stopped: boolean };

function generatedRanges(previousText: string, generatedText: string): ByteRange[] {
  const ranges: ByteRange[] = [];
  let offset = 0;
  for (const change of diffLines(previousText, generatedText)) {
    const bytes = Buffer.byteLength(change.value);
    if (change.added) ranges.push({ start: offset, end: offset + bytes });
    if (!change.removed) offset += bytes;
  }
  return ranges;
}

function intersectsGeneratedRange(finding: ExecutablePolicyFinding, ranges: readonly ByteRange[]): boolean {
  if (finding.span === undefined) return false;
  const { startByte, endByte } = finding.span;
  return ranges.some(({ start, end }) =>
    startByte === endByte ? start <= startByte && startByte <= end : startByte < end && start < endByte,
  );
}

export function languageFor(path: string): PolicySourceLanguage | undefined {
  return languageOfPath(SOURCE_LANGUAGES, path);
}

/**
 * Ruff's module path for a Python file (`detect_package_root`), as the kernel derives it for every ingestion path:
 * dotted, a list when a name keeps a dot (`["json.d", "__init__"]`), or undefined outside a package. Native repository
 * indexing derives the same value itself, so buffer, inline and repository requests agree.
 */
export function pythonModulePath(absolute: string): string | string[] | undefined {
  return loadNativeBinding().pythonModulePath(absolute) ?? undefined;
}

function emptyEvaluation(): Evaluation {
  return {
    status: "completed",
    findings: [],
    failures: [],
    metrics: { files: 0, visitedNodes: 0, witnesses: 0 },
  };
}
function failure(
  checkId: string,
  code: PolicyExecutionFailure["code"],
  message: string,
  path?: string,
  stage?: PolicyExecutionFailure["stage"],
): PolicyExecutionFailure {
  return { checkId, code, message, ...(path === undefined ? {} : { path }), ...(stage === undefined ? {} : { stage }) };
}
function scanBudgetFailure(message: string): PolicyExecutionFailure {
  return failure("scan", "budget-exceeded", message, undefined, "scan");
}
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Generated snapshots and repository diagnostics have independent native state. */
export class NativeExecutablePolicyEnforcer implements ExecutablePolicyEvaluator {
  readonly #workspaceRoot: string;
  readonly #catalog: PolicyDslCatalog;
  readonly #delivery: PolicySteeringDelivery;
  readonly #executions: PolicyExecutionRecorder;
  readonly #ownedExecutions: SqlitePolicyExecutionStore | undefined;
  #scope: PolicyScope;
  #mode: PolicyMode;
  #workspace: PreparedWorkspace | undefined;
  #replacement: PreparedWorkspace | undefined;
  #sessionId: string | undefined;
  #revision = 0;
  readonly #versions = new Map<string, number>();
  readonly #workspaceRevisions = new Map<NativePolicyDslWorkspace, number>();
  readonly #workspaceEvaluations = new Map<NativePolicyDslWorkspace, Evaluation>();
  readonly #workspaceStorage = new Map<NativePolicyDslWorkspace, Promise<boolean>>();
  readonly #pending = new Map<string, StoredEdit>();
  readonly #applied = new Map<string, StoredEdit>();
  readonly #readFailures = new Map<string, PolicyExecutionFailure>();
  /** The quarantined set last reported, so each catalog load reports a skipped rule once rather than on every edit. */
  #quarantineReport: string | undefined;
  /** Why repository reuse was last reported disabled, so a persistent cause is reported once rather than on every scan. */
  #reuseDisabledReport: string | undefined;
  #pendingBytes = 0;
  readonly #readyModes = new Map<number, PolicyMode>();
  readonly #readyFindings = new Map<number, Map<string, ExecutablePolicyFinding[]>>();
  #spoolDirectory: string | undefined;
  #spoolSequence = 0;
  readonly #spoolRemovals = new Set<Promise<void>>();
  readonly #storing = new Set<Promise<StoredEdit>>();
  #pump: Promise<void> | undefined;
  #busy = false;
  #closing = false;
  #closed = false;
  readonly #diagnostics = new Set<Promise<unknown>>();
  readonly #generatedEditDiagnostics = new Set<Promise<unknown>>();
  #reportableFindings: ExecutablePolicyFinding[] = [];
  #responseMode: PolicyMode;
  #evaluation = emptyEvaluation();

  constructor(
    catalog: PolicyDslCatalog,
    options: {
      workspaceRoot: string;
      delivery: PolicySteeringDelivery;
      executions?: PolicyExecutionRecorder;
      scope?: PolicyScope;
      mode?: PolicyMode;
    },
  ) {
    this.#catalog = catalog;
    this.#workspaceRoot = resolve(options.workspaceRoot);
    this.#delivery = options.delivery;
    this.#scope = options.scope ?? "touched-files";
    this.#mode = options.mode ?? "resteer";
    this.#responseMode = this.#mode;
    this.#ownedExecutions =
      options.executions === undefined
        ? new SqlitePolicyExecutionStore(defaultRerailDatabasePath())
        : undefined;
    this.#executions = options.executions ?? this.#ownedExecutions!;
  }

  acceptsPath(path: string): boolean {
    return (
      languageFor(path) !== undefined && ((this.#replacement ?? this.#workspace)?.workspace.acceptsPath(path) ?? true)
    );
  }

  snapshotStatus(): ExecutablePolicyResponse & { pending: boolean } {
    return {
      ...this.#response(this.#revision),
      pending: this.#busy || this.#pending.size > 0 || this.#pump !== undefined || this.#diagnostics.size > 0,
    };
  }
  async settleGeneratedEdits(): Promise<void> {
    while (
      this.#busy ||
      this.#pending.size > 0 ||
      this.#pump !== undefined ||
      this.#generatedEditDiagnostics.size > 0
    ) {
      const work = [...(this.#pump === undefined ? [] : [this.#pump]), ...this.#generatedEditDiagnostics];
      if (work.length === 0) {
        await yieldToHost();
        continue;
      }
      await Promise.allSettled(work);
    }
  }
  async settle(): Promise<void> {
    while (this.snapshotStatus().pending) {
      const work = [...(this.#pump === undefined ? [] : [this.#pump]), ...this.#diagnostics];
      if (work.length === 0) {
        await yieldToHost();
        continue;
      }
      await Promise.allSettled(work);
    }
  }

  async openWorkspace(sessionId: string): Promise<void> {
    if (this.#sessionId !== undefined || this.#closing)
      throw new Error("Executable policy enforcer cannot be reopened");
    this.#sessionId = sessionId;
    this.#record(() => this.#executions.interrupt(sessionId));
    this.#reportQuarantined();
    let prepared: PreparedWorkspace | undefined;
    try {
      prepared = this.#prepareWorkspace();
      this.#workspace = prepared;
      await prepared.storage;
    } catch (error) {
      if (prepared !== undefined && this.#workspace === prepared) {
        this.#disposeWorkspace(prepared.workspace);
        this.#workspace = undefined;
      }
      this.#recordImmediate(
        this.#scope,
        this.#mode,
        0,
        [],
        [failure("catalog", "adapter-error", errorMessage(error), undefined, "evaluate")],
      );
    }
  }

  updateSettings(settings: Required<PolicyEvaluationSettings>): void {
    if (this.#closing) return;
    this.#scope = settings.scope;
    this.#mode = settings.mode;
  }
  /**
   * Synchronous, so the bridge's next `acceptsPath` already answers with the refreshed scopes: the replacement
   * registers its policies now and attaches storage off the JS thread before its first apply.
   */
  refreshPolicies(): void {
    if (this.#closing) return;
    try {
      this.#catalog.refresh();
      this.#reportQuarantined();
      if ((this.#replacement ?? this.#workspace)?.version === this.#catalog.status.version) return;
      const replacement = this.#prepareWorkspace();
      if (this.#replacement !== undefined) this.#disposeWorkspace(this.#replacement.workspace);
      this.#replacement = replacement;
    } catch (error) {
      this.#readFailures.set(
        "catalog",
        failure("catalog", "adapter-error", errorMessage(error), undefined, "evaluate"),
      );
    }
  }

  async evaluateGeneratedEdits(sessionId: string, batch: ExecutablePolicyEditBatch): Promise<ExecutablePolicyResponse> {
    if (this.#sessionId !== sessionId || this.#closing) throw new Error("Executable policy enforcer session mismatch");
    if (batch.revision <= this.#revision) throw new Error("Executable policy revisions must increase");
    this.#revision = batch.revision;
    this.refreshPolicies();
    if (this.#scope === "repository") {
      if (batch.edits.length > 0) {
        for (const edit of batch.edits) this.#versions.set(edit.path, batch.revision);
        this.#trackDiagnostic(this.#diagnosticScan("repository", this.#mode, [], true), true);
      }
      return this.#response(batch.revision);
    }
    const scope = this.#scope;
    const mode = this.#mode;
    const accepted = batch.edits.filter(
      (edit) =>
        this.acceptsPath(edit.path) && withinCheckout(this.#workspaceRoot, resolve(this.#workspaceRoot, edit.path)),
    );
    for (const edit of accepted) {
      this.#versions.set(edit.path, batch.revision);
      await this.#queueGeneratedEdit(edit, batch.revision, scope, mode);
    }
    this.#schedule();
    return this.#response(batch.revision);
  }

  async #queueGeneratedEdit(
    edit: ExecutablePolicyEdit,
    revision: number,
    scope: PolicyScope,
    mode: PolicyMode,
  ): Promise<void> {
    if (this.#closing) {
      this.#startRun(scope, mode, revision, [edit.path], null);
      return;
    }
    try {
      const storing = this.#store(edit, revision, scope, mode);
      this.#storing.add(storing);
      let stored: StoredEdit;
      try {
        stored = await storing;
      } finally {
        this.#storing.delete(storing);
      }
      if (this.#closing) {
        this.#startRun(scope, mode, revision, [edit.path], null);
        this.#release(stored);
        return;
      }
      const previous = this.#pending.get(edit.path);
      if (previous !== undefined) {
        previous.runId ??= this.#startRun(previous.scope, previous.mode, previous.revision, [edit.path], null);
        this.#finishRun(previous.runId, "superseded", [], []);
        this.#release(previous);
      }
      this.#pending.set(edit.path, stored);
    } catch (error) {
      if (this.#closing) {
        this.#startRun(scope, mode, revision, [edit.path], null);
        return;
      }
      this.#recordImmediate(
        scope,
        mode,
        revision,
        [edit.path],
        [failure("spool", "adapter-error", errorMessage(error), edit.path, "persistence")],
      );
    }
  }

  async scan(
    sessionId: string,
    options: { scope: "repository" | "touched-files"; mode: PolicyMode; paths?: string[] },
  ): Promise<readonly string[]> {
    if (sessionId !== this.#sessionId || this.#closing) throw new Error("Executable policy enforcer session mismatch");
    const paths = options.paths ?? [];
    if (options.scope === "repository" && paths.length > 0)
      throw new Error("Repository scope always covers the full repository");
    if (options.scope === "touched-files" && paths.length === 0)
      throw new Error("Touched-files policy scan requires paths");
    const scope = paths.map((path) => {
      const absolute = resolve(this.#workspaceRoot, path);
      if (!withinCheckout(this.#workspaceRoot, absolute)) throw new Error("Policy scan path escapes workspace");
      return relative(this.#workspaceRoot, absolute).split(sep).join("/");
    });
    const task = this.#diagnosticScan(options.scope, options.mode, scope);
    this.#trackDiagnostic(task);
    const runId = await task;
    return runId === undefined ? [] : [runId];
  }

  interrupt(): void {
    if (this.#closing) return;
    for (const [path, stored] of this.#pending) {
      // Cancelled before evaluation: preserve the work without claiming a compiled-rule revision.
      stored.runId ??= this.#startRun(stored.scope, stored.mode, stored.revision, [path], null);
    }
    this.#closing = true;
    if (this.#sessionId !== undefined) this.#record(() => this.#executions.interrupt(this.#sessionId!));
    this.#workspace?.workspace.close();
    this.#replacement?.workspace.close();
  }
  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    this.interrupt();
    await this.#pump;
    await Promise.allSettled(this.#diagnostics);
    await Promise.allSettled(this.#storing);
    for (const stored of this.#pending.values()) this.#release(stored);
    for (const stored of this.#applied.values()) this.#release(stored);
    this.#pending.clear();
    this.#applied.clear();
    await Promise.all(this.#spoolRemovals);
    if (this.#spoolDirectory !== undefined) await rm(this.#spoolDirectory, { recursive: true, force: true });
    this.#workspaceRevisions.clear();
    this.#workspaceStorage.clear();
    this.#workspaceEvaluations.clear();
    this.#ownedExecutions?.close();
  }

  /**
   * Rules the catalog skipped because their rows fail validation are adapter errors: kept in every response while
   * they last, and recorded as one failed run each time the catalog loads a different set of them.
   */
  #reportQuarantined(): void {
    const quarantined = this.#catalog.quarantined();
    const report = JSON.stringify(quarantined.map(({ ruleId, owner, reason }) => [ruleId, owner, reason]));
    if (report === this.#quarantineReport) return;
    this.#quarantineReport = report;
    for (const key of [...this.#readFailures.keys()]) if (key.startsWith("quarantined:")) this.#readFailures.delete(key);
    const failures: PolicyExecutionFailure[] = [];
    for (const { ruleId, owner, reason } of quarantined) {
      const message = `policy rule ${ruleId} (${owner}) is skipped: ${reason}`;
      const problem = { ...failure("catalog", "adapter-error", message, undefined, "parse"), policyId: ruleId };
      this.#readFailures.set(`quarantined:${owner}:${ruleId}`, problem);
      failures.push(problem);
    }
    if (failures.length > 0) this.#recordImmediate(this.#scope, this.#mode, this.#revision, [], failures);
  }

  /**
   * A workspace for the catalog snapshot, labelled with that snapshot's version and provenance. Its scopes are live on
   * return; `#applyRequest` waits for the storage attach before the first apply.
   */
  #prepareWorkspace(): PreparedWorkspace {
    const snapshot = this.#catalog.snapshot();
    // This build's own store; only the main daemon may open the machine-wide one.
    const storePath = contentStorePath();
    assertContentStoreOwner(storePath);
    const workspace = new (loadNativeBinding().PolicyDslWorkspace)();
    let storage: Promise<boolean>;
    try {
      for (const policy of snapshot.policies) workspace.addPolicy(policy.executable);
      storage = workspace.enableStorageAsync(this.#workspaceRoot, storePath).then(({ attached, reason }) => {
        // Reuse is a cache: losing it costs time, never results, so a cause Git reports is logged, not raised.
        if (!attached && reason && reason !== this.#reuseDisabledReport) {
          this.#reuseDisabledReport = reason;
          process.stderr.write(`rerail: repository reuse is disabled: ${reason}\n`);
        }
        return attached;
      });
    } catch (error) {
      workspace.close();
      throw error;
    }
    // Observed by every apply and by the owner of the workspace; never an unhandled rejection meanwhile.
    storage.catch(() => undefined);
    this.#workspaceStorage.set(workspace, storage);
    this.#workspaceRevisions.set(workspace, 0);
    this.#workspaceEvaluations.set(workspace, emptyEvaluation());
    return { workspace, version: snapshot.version, provenance: snapshot.provenance, storage };
  }

  #disposeWorkspace(workspace: NativePolicyDslWorkspace): void {
    workspace.close();
    this.#workspaceRevisions.delete(workspace);
    this.#workspaceEvaluations.delete(workspace);
    this.#workspaceStorage.delete(workspace);
  }

  /** Installs `replacement` unless a newer catalog superseded (and closed) it meanwhile. */
  async #replaceWorkspace(replacement: PreparedWorkspace): Promise<void> {
    try {
      await replacement.storage;
    } catch (error) {
      // Keep serving the current catalog; the next refresh prepares the replacement again.
      if (this.#replacement !== replacement) return;
      this.#replacement = undefined;
      this.#disposeWorkspace(replacement.workspace);
      this.#readFailures.set("catalog", failure("catalog", "adapter-error", errorMessage(error), undefined, "evaluate"));
      return;
    }
    try {
      for (const stored of this.#applied.values())
        await this.#apply(replacement.workspace, await this.#load(stored), true);
      const result = await this.#applyRequest(replacement.workspace, { files: [], removed: [] });
      if (this.#replacement !== replacement) return;
      this.#replacement = undefined;
      if (this.#workspace !== undefined) this.#disposeWorkspace(this.#workspace.workspace);
      this.#workspace = replacement;
      this.#readFailures.delete("catalog");
      this.#evaluation = result;
    } catch (error) {
      if (this.#replacement !== replacement) return;
      this.#replacement = undefined;
      this.#disposeWorkspace(replacement.workspace);
      throw error;
    }
  }

  #schedule(): void {
    if (this.#pump !== undefined || this.#closing) return;
    this.#pump = this.#run()
      .catch((error) => {
        if (this.#closing) return;
        const problem = failure("worker", "adapter-error", errorMessage(error), undefined, "evaluate");
        this.#readFailures.set("worker", problem);
        for (const [path, stored] of this.#pending) {
          this.#finishRun(
            this.#ensureStoredRun(stored, path, (this.#replacement ?? this.#workspace)?.provenance ?? []),
            "failed",
            [],
            [problem],
          );
          this.#release(stored);
        }
        this.#pending.clear();
        this.#recordImmediate(this.#scope, this.#mode, this.#revision, [], [problem]);
      })
      .finally(() => {
        this.#pump = undefined;
        if (!this.#closing && this.#pending.size > 0) this.#schedule();
      });
  }

  async #run(): Promise<void> {
    await yieldToHost();
    while (!this.#closing) {
      while (this.#replacement !== undefined && !this.#closing) await this.#replaceWorkspace(this.#replacement);
      const entry = this.#pending.entries().next().value;
      if (entry === undefined) return;
      const [path, stored] = entry;
      this.#pending.delete(path);
      this.#ensureStoredRun(stored, path, this.#workspace?.provenance ?? []);
      this.#busy = true;
      try {
        await this.#evaluatePendingEdit(path, stored);
      } catch (error) {
        await this.#recordPendingEvaluationError(path, stored, error);
      } finally {
        this.#busy = false;
        this.#release(stored);
      }
    }
  }

  async #evaluatePendingEdit(path: string, stored: StoredEdit): Promise<void> {
    if (this.#closing) return;
    const prepared = this.#workspace;
    stored.runId ??= this.#startRun(stored.scope, stored.mode, stored.revision, [path], prepared);
    const edit = await this.#load(stored);
    if (this.#closing) return;
    const readFailure = this.#readFailureForEdit(path, edit);
    if (readFailure !== undefined) {
      this.#readFailures.set(path, readFailure);
      this.#finishRun(stored.runId, "failed", [], [readFailure]);
      if (this.#pending.size === 0) await this.#publishReady();
      return;
    }
    if (prepared === undefined) throw new Error("Policy workspace could not be initialized");
    this.#readFailures.delete(path);
    this.#readFailures.delete("worker");
    const result = await this.#apply(prepared.workspace, edit);
    this.#replaceAppliedEdit(path, stored, edit);
    await this.#finishPendingEvaluation(path, stored, edit, result);
  }

  #readFailureForEdit(path: string, edit: ExecutablePolicyEdit): PolicyExecutionFailure | undefined {
    if (edit.readError !== undefined) return failure("source", "source-incomplete", edit.readError, path, "read");
    if (edit.generatedText !== undefined && Buffer.byteLength(edit.generatedText) > MAX_SOURCE_BYTES)
      return failure("source", "budget-exceeded", "Generated source exceeds the per-file byte budget", path, "read");
    return undefined;
  }

  #replaceAppliedEdit(path: string, stored: StoredEdit, edit: ExecutablePolicyEdit): void {
    const previous = this.#applied.get(path);
    if (previous !== undefined) this.#release(previous);
    if (edit.deleted) this.#applied.delete(path);
    else {
      stored.references += 1;
      this.#applied.set(path, stored);
    }
  }

  async #finishPendingEvaluation(
    path: string,
    stored: StoredEdit,
    edit: ExecutablePolicyEdit,
    result: Evaluation,
  ): Promise<void> {
    const failures = result.failures.map((item) => ({ ...item, stage: "evaluate" as const }));
    const scopeFailure = this.#scopeFailureForEdit(stored.scope, path, edit);
    if (scopeFailure !== undefined) this.#readFailures.set(path, scopeFailure);
    const findings = scopeFailure === undefined ? this.#selectFindings(stored.scope, edit, result.findings) : [];
    this.#finishRun(
      stored.runId,
      scopeFailure === undefined && result.status === "completed" ? "completed" : "failed",
      findings,
      scopeFailure === undefined ? failures : [...failures, scopeFailure],
    );
    if (this.#versions.get(path) !== stored.revision || this.#replacement !== undefined) return;
    this.#evaluation = result;
    let readyFindings = this.#readyFindings.get(stored.revision);
    if (readyFindings === undefined) {
      readyFindings = new Map();
      this.#readyFindings.set(stored.revision, readyFindings);
    }
    readyFindings.set(path, findings);
    this.#readyModes.set(stored.revision, stored.mode);
    if (this.#pending.size === 0) await this.#publishReady();
  }

  #scopeFailureForEdit(
    scope: PolicyScope,
    path: string,
    edit: ExecutablePolicyEdit,
  ): PolicyExecutionFailure | undefined {
    return scope === "ai-output" && edit.previousReadError !== undefined
      ? failure("preimage", "source-incomplete", edit.previousReadError, path, "scope")
      : undefined;
  }

  async #recordPendingEvaluationError(path: string, stored: StoredEdit, error: unknown): Promise<void> {
    if (this.#closing) return;
    const problem = failure("worker", "adapter-error", errorMessage(error), path, "evaluate");
    if (this.#pending.size === 0) await this.#publishReady();
    this.#readFailures.set(path, problem);
    this.#finishRun(stored.runId, "failed", [], [problem]);
  }
  #selectFindings(
    scope: PolicyScope,
    edit: ExecutablePolicyEdit,
    findings: readonly ExecutablePolicyFinding[],
  ): ExecutablePolicyFinding[] {
    const pathFindings = findings.filter((finding) => finding.path === edit.path);
    if (scope === "touched-files" || edit.deleted || edit.generatedText === undefined) return pathFindings;
    const ranges = generatedRanges(edit.previousText ?? "", edit.generatedText);
    return pathFindings.filter((finding) => intersectsGeneratedRange(finding, ranges));
  }
  /**
   * A repository-scope scan of a Git checkout runs the native repository index on the scan
   * workspace (`#indexRepository`); any other scan walks its paths here (`#walkDiagnosticSources`).
   * Both evaluate the same files: Git and Rerail ignore rules decide only what the index restores.
   */
  async #diagnosticScan(
    scope: "repository" | "touched-files",
    mode: PolicyMode,
    paths: string[] = [],
    generatedEdit = false,
  ): Promise<string | undefined> {
    this.refreshPolicies();
    const revision = this.#revision;
    let runId: string | undefined;
    const failures: PolicyExecutionFailure[] = [];
    let workspace: NativePolicyDslWorkspace | undefined;
    try {
      await yieldToHost();
      if (this.#closing) return runId;
      const prepared = this.#prepareWorkspace();
      workspace = prepared.workspace;
      runId = this.#startRun(scope, mode, revision, paths, prepared);
      const indexed = scope === "repository" && (await prepared.storage);
      const scanned = indexed
        ? await this.#indexRepository(workspace, runId, failures)
        : await this.#walkDiagnosticSources(workspace, runId, paths, failures);
      if (this.#closing) return runId;
      await this.#finishDiagnosticScan({
        workspace,
        runId,
        revision,
        scope,
        mode,
        sourcePaths: scanned.sourcePaths,
        failures,
        stopped: scanned.stopped,
        generatedEdit,
        indexed,
      });
      return runId;
    } catch (error) {
      this.#finishRun(
        runId,
        "failed",
        [],
        [...failures, failure("scan", "adapter-error", errorMessage(error), undefined, "scan")],
      );
      return runId;
    } finally {
      if (workspace !== undefined) this.#disposeWorkspace(workspace);
    }
  }

  /**
   * Index the Git checkout natively, one chunk per call. A chunk reads at most one batch (32 files,
   * reading stops at 1 MiB), publishes it and releases its sources and tapes before it returns, so
   * the scan's source residency is one batch: that bound replaces the former index worker's
   * peak-RSS limit. Interactive edits go first between chunks, and the walk's file, byte, failure
   * and five-minute budgets stop the index as they stop the walk.
   */
  async #indexRepository(
    workspace: NativePolicyDslWorkspace,
    runId: string | undefined,
    failures: PolicyExecutionFailure[],
  ): Promise<DiagnosticScanSources> {
    const deadline = Date.now() + SCAN_MAX_DURATION_MS;
    while (!this.#closing) {
      if (Date.now() >= deadline) {
        failures.push(scanBudgetFailure("Repository scan exceeded its five-minute work budget"));
        return { sourcePaths: [], stopped: true };
      }
      await this.#waitForEvaluationIdle();
      if (this.#closing) break;
      const chunk = repositoryIndexChunkSchema.parse(JSON.parse(await this.#indexChunk(workspace)));
      this.#workspaceRevisions.set(workspace, chunk.revision);
      failures.push(...chunk.failures.map((item) => failure("source", item.code, item.message, item.path, "read")));
      const evaluation = chunk.evaluation;
      if (evaluation !== null && runId !== undefined)
        this.#record(() =>
          this.#executions.progress(runId, {
            findings: evaluation.findings,
            failures: [...failures, ...evaluation.failures],
          }),
        );
      if (chunk.overBudget) {
        failures.push(scanBudgetFailure("Repository scan exceeded its 100,000-file or 512 MiB source budget"));
        return { sourcePaths: [], stopped: true };
      }
      if (failures.length >= SCAN_MAX_FAILURES) {
        failures.push(scanBudgetFailure(`Repository scan stopped after reaching ${SCAN_MAX_FAILURES} failures`));
        return { sourcePaths: [], stopped: true };
      }
      if (chunk.done) return { sourcePaths: [], stopped: false };
      await yieldToHost();
    }
    return { sourcePaths: [], stopped: true };
  }

  /** One index chunk, retried while an interactive apply holds the evaluation slot. */
  async #indexChunk(workspace: NativePolicyDslWorkspace): Promise<string> {
    let backoff = 2;
    while (!this.#closing) {
      try {
        return await workspace.indexRepositoryAsync(REPOSITORY_INDEX_OPTIONS);
      } catch (error) {
        if (!(error instanceof Error) || error.message !== "RERAIL_POLICY_BUSY") throw error;
        await pause(backoff);
        backoff = Math.min(backoff * 2, 25);
      }
    }
    throw new Error("Policy execution interrupted by session closure");
  }

  async #walkDiagnosticSources(
    workspace: NativePolicyDslWorkspace,
    runId: string | undefined,
    paths: readonly string[],
    failures: PolicyExecutionFailure[],
  ): Promise<DiagnosticScanSources> {
    let files: WorkspaceFile[] = [];
    let bytes = 0;
    const sourcePaths: string[] = [];
    const scanState = {
      bytes: 0,
      deadline: Date.now() + SCAN_MAX_DURATION_MS,
      files: 0,
      stopped: false,
    };
    for await (const file of this.#diagnosticFiles(workspace, paths, failures, scanState)) {
      if (this.#closing) return { sourcePaths, stopped: true };
      files.push(file);
      sourcePaths.push(file.path);
      bytes += Buffer.byteLength(file.source);
      if (files.length >= SCAN_BATCH_FILES || bytes >= SCAN_CHUNK_BYTES) {
        await this.#flushDiagnosticBatch(workspace, runId, failures, files);
        files = [];
        bytes = 0;
      }
    }
    if (files.length > 0) await this.#flushDiagnosticBatch(workspace, runId, failures, files);
    return { sourcePaths, stopped: scanState.stopped };
  }

  async #flushDiagnosticBatch(
    workspace: NativePolicyDslWorkspace,
    runId: string | undefined,
    failures: readonly PolicyExecutionFailure[],
    files: WorkspaceFile[],
  ): Promise<void> {
    await this.#waitForEvaluationIdle();
    if (this.#closing) return;
    const result = await this.#applyRequest(workspace, { files, removed: [], deferGlobal: true });
    if (runId !== undefined)
      this.#record(() =>
        this.#executions.progress(runId, {
          findings: result.findings,
          failures: [...failures, ...result.failures],
        }),
      );
    await yieldToHost();
  }

  async #finishDiagnosticScan(completion: DiagnosticScanCompletion): Promise<void> {
    const { workspace, runId, revision, scope, mode, sourcePaths, failures, stopped, generatedEdit, indexed } =
      completion;
    await this.#waitForEvaluationIdle();
    if (this.#closing) return;
    await this.#applyRequest(workspace, {
      files: [],
      removed: [],
      quiet: true,
      ...(stopped ? { deferGlobal: true } : {}),
    });
    if (indexed && !stopped)
      // Reuse is a cache: a failed write (another process holding the store) costs reuse, never these results.
      await workspace.completeStorageRevisionAsync().catch((error: unknown) => {
        process.stderr.write(`rerail: repository reuse was not recorded: ${errorMessage(error)}\n`);
      });
    if (this.#closing) return;
    const streamed = await this.#resultPages(workspace, runId, mode === "log" && !generatedEdit, failures);
    const result: Evaluation = {
      status: streamed.failures.length === 0 ? "completed" : "failed",
      findings: streamed.findings,
      failures: streamed.failures,
      metrics: { files: 0, visitedNodes: 0, witnesses: streamed.findings.length },
    };
    const unboundedFailures = [
      ...failures,
      ...result.failures.map((item) => ({ ...item, stage: "evaluate" as const })),
    ];
    const allFailures =
      unboundedFailures.length <= SCAN_MAX_FAILURES
        ? unboundedFailures
        : [
            ...unboundedFailures.slice(0, SCAN_MAX_FAILURES),
            scanBudgetFailure(`Repository scan stopped after reaching ${SCAN_MAX_FAILURES} failures`),
          ];
    const status = allFailures.length > 0 ? "failed" : "completed";
    if (streamed.paged && runId !== undefined) {
      if (unboundedFailures.length > SCAN_MAX_FAILURES)
        this.#record(() =>
          this.#executions.appendResultPage?.(runId, streamed.pageCount, {
            findings: [],
            failures: [scanBudgetFailure(`Repository scan stopped after reaching ${SCAN_MAX_FAILURES} failures`)],
          }),
        );
      this.#record(() => this.#executions.finishResultPages?.(runId, status));
    } else {
      this.#finishRun(runId, status, result.findings, allFailures);
    }
    if (this.#scope === "repository" && revision === this.#revision) {
      this.#reportableFindings = result.findings;
      this.#responseMode = mode;
      this.#evaluation = { ...result, failures: allFailures };
    }
    await this.#publish(revision, scope === "repository" ? [] : sourcePaths, result.findings, mode, generatedEdit);
  }

  async #waitForEvaluationIdle(): Promise<void> {
    while (!this.#closing && (this.#busy || this.#pending.size > 0)) await pause(2);
  }
  async *#diagnosticFiles(
    workspace: NativePolicyDslWorkspace,
    paths: readonly string[],
    failures: PolicyExecutionFailure[],
    state: DiagnosticScanState,
  ): AsyncGenerator<WorkspaceFile> {
    // The walk starts at the resolved root, as native Git discovery does: a root reached through a
    // symlink is walked as its target, never skipped as a symlink.
    const canonicalRoot = await realpath(this.#workspaceRoot);
    const stack = paths.length === 0 ? [canonicalRoot] : paths.map((path) => resolve(canonicalRoot, path));
    const seen = new Set<string>();
    while (stack.length > 0 && !this.#closing) {
      if (Date.now() >= state.deadline) {
        failures.push(scanBudgetFailure("Repository scan exceeded its five-minute work budget"));
        state.stopped = true;
        return;
      }
      const absolute = stack.pop()!;
      const file = await this.#diagnosticFileAtPath(workspace, canonicalRoot, absolute, stack, seen, failures);
      if (failures.length >= SCAN_MAX_FAILURES) {
        failures.push(scanBudgetFailure(`Repository scan stopped after reaching ${SCAN_MAX_FAILURES} failures`));
        state.stopped = true;
        return;
      }
      if (this.#closing) return;
      if (file === undefined) continue;
      const bytes = Buffer.byteLength(file.source);
      if (state.files >= SCAN_MAX_FILES || state.bytes + bytes > SCAN_MAX_SOURCE_BYTES) {
        failures.push(scanBudgetFailure("Repository scan exceeded its 100,000-file or 512 MiB source budget"));
        state.stopped = true;
        return;
      }
      state.files += 1;
      state.bytes += bytes;
      yield file;
    }
  }

  async #diagnosticFileAtPath(
    workspace: NativePolicyDslWorkspace,
    canonicalRoot: string,
    absolute: string,
    stack: string[],
    seen: Set<string>,
    failures: PolicyExecutionFailure[],
  ): Promise<WorkspaceFile | undefined> {
    const path = relative(canonicalRoot, absolute).split(sep).join("/");
    try {
      const info = await lstat(absolute);
      if (info.isSymbolicLink()) return undefined;
      if (!(await this.#isDiagnosticPathContained(canonicalRoot, absolute, path, failures))) return undefined;
      if (info.isDirectory()) {
        await this.#enqueueDiagnosticDirectory(workspace, canonicalRoot, absolute, stack);
        return undefined;
      }
      return await this.#readDiagnosticFile(workspace, absolute, path, info, seen, failures);
    } catch (error) {
      failures.push(failure("source", "source-incomplete", errorMessage(error), path, "read"));
      return undefined;
    }
  }

  async #isDiagnosticPathContained(
    canonicalRoot: string,
    absolute: string,
    path: string,
    failures: PolicyExecutionFailure[],
  ): Promise<boolean> {
    if (withinCheckout(canonicalRoot, await realpath(absolute))) return true;
    failures.push(failure("scope", "source-incomplete", "Scan path resolves outside the workspace", path, "read"));
    return false;
  }

  async #enqueueDiagnosticDirectory(
    workspace: NativePolicyDslWorkspace,
    canonicalRoot: string,
    absolute: string,
    stack: string[],
  ): Promise<void> {
    for await (const entry of await opendir(absolute)) {
      if (this.#closing) return;
      if (!this.#shouldScanDirectoryEntry(workspace, canonicalRoot, absolute, entry)) continue;
      stack.push(join(absolute, entry.name));
    }
  }

  #shouldScanDirectoryEntry(
    workspace: NativePolicyDslWorkspace,
    canonicalRoot: string,
    directory: string,
    entry: Dirent,
  ): boolean {
    if (entry.isSymbolicLink() || (entry.isDirectory() && IGNORED_DIRECTORIES.has(entry.name))) return false;
    if (entry.isDirectory()) return true;
    const path = relative(canonicalRoot, join(directory, entry.name)).split(sep).join("/");
    return entry.isFile() && languageFor(path) !== undefined && workspace.acceptsPath(path);
  }

  async #readDiagnosticFile(
    workspace: NativePolicyDslWorkspace,
    absolute: string,
    path: string,
    info: Stats,
    seen: Set<string>,
    failures: PolicyExecutionFailure[],
  ): Promise<WorkspaceFile | undefined> {
    const language = languageFor(path);
    if (!info.isFile() || language === undefined || !workspace.acceptsPath(path) || seen.has(path)) return undefined;
    seen.add(path);
    if (info.size > MAX_SOURCE_BYTES) {
      failures.push(failure("source", "budget-exceeded", "Source exceeds the per-file byte budget", path, "read"));
      return undefined;
    }
    const source = await readFile(absolute);
    if (this.#closing) return undefined;
    if (source.includes(0)) {
      failures.push(failure("source", "source-incomplete", "Binary source cannot be evaluated", path, "read"));
      return undefined;
    }
    return { path, language, source: source.toString("utf8"), ...this.#module(language, path) };
  }

  /** The module path the kernel qualifies a Python file's definitions and relative imports by, and file conditions read. */
  #module(language: PolicySourceLanguage, path: string): { module?: string | string[] } {
    if (language !== POLICY_DIALECTS.python.label) return {};
    const module = pythonModulePath(resolve(this.#workspaceRoot, path));
    return module === undefined ? {} : { module };
  }
  /**
   * Streams the results of a scan's private workspace. Nothing else applies to that workspace while
   * it pages; an apply would invalidate the native cursor, which the next page reports as stale.
   */
  async #resultPages(
    workspace: NativePolicyDslWorkspace,
    runId: string | undefined,
    allowPersistentPaging: boolean,
    precedingFailures: readonly PolicyExecutionFailure[],
  ): Promise<Pick<Evaluation, "findings" | "failures"> & { paged: boolean; pageCount: number }> {
    const paged =
      allowPersistentPaging &&
      runId !== undefined &&
      this.#executions.supportsResultPages === true &&
      this.#executions.appendResultPage !== undefined &&
      this.#executions.finishResultPages !== undefined;
    const findings: ExecutablePolicyFinding[] = [];
    const failures: PolicyExecutionFailure[] = [];
    let remainingFailures = Math.max(0, SCAN_MAX_FAILURES - precedingFailures.length);
    let cursor = 0;
    let pageIndex = 0;
    if (paged && precedingFailures.length > 0) {
      this.#record(() =>
        this.#executions.appendResultPage?.(runId!, pageIndex, {
          findings: [],
          failures: precedingFailures.slice(0, SCAN_MAX_FAILURES),
        }),
      );
      pageIndex += 1;
    }
    while (true) {
      const page = resultPageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
      if (paged) {
        const pageFailures = page.failures
          .slice(0, remainingFailures)
          .map((item) => ({ ...item, stage: "evaluate" as const }));
        remainingFailures -= pageFailures.length;
        this.#record(() =>
          this.#executions.appendResultPage?.(runId!, pageIndex, {
            findings: page.findings,
            failures: pageFailures,
          }),
        );
      } else {
        findings.push(...page.findings);
      }
      failures.push(...page.failures);
      pageIndex += 1;
      if (page.done) return { findings, failures, paged, pageCount: pageIndex };
      if (page.cursor <= cursor) throw new Error("Native result stream did not advance");
      cursor = page.cursor;
      await yieldToHost();
      if (this.#closing) throw new Error("Policy execution interrupted by session closure");
    }
  }

  async #apply(
    workspace: NativePolicyDslWorkspace,
    edit: ExecutablePolicyEdit,
    deferGlobal = false,
  ): Promise<Evaluation> {
    const evaluation = await this.#applyRequest(workspace, {
      files:
        edit.deleted || edit.generatedText === undefined
          ? []
          : [{ path: edit.path, language: edit.language, source: edit.generatedText, ...this.#module(edit.language, edit.path) }],
      removed: edit.deleted ? [edit.path] : [],
      ...(deferGlobal ? { deferGlobal: true, quiet: true } : {}),
    });
    if (!deferGlobal) workspace.evictInactive();
    return evaluation;
  }

  async #applyRequest(
    workspace: NativePolicyDslWorkspace,
    request: { files: WorkspaceFile[]; removed: string[]; deferGlobal?: boolean; quiet?: boolean },
  ): Promise<Evaluation> {
    await this.#workspaceStorage.get(workspace);
    const current = this.#workspaceRevisions.get(workspace);
    // A workspace disposed meanwhile (a superseded replacement) must not be revived in the revision map.
    if (current === undefined) throw new Error("Policy workspace was disposed");
    const revision = current + 1;
    this.#workspaceRevisions.set(workspace, revision);
    let backoff = 2;
    while (!this.#closing) {
      try {
        const inline =
          workspace === this.#workspace?.workspace &&
          !request.deferGlobal &&
          request.files.length <= 1 &&
          (request.files[0]?.source.length ?? 0) < 4_096
            ? workspace.tryApply(JSON.stringify({ revision, ...request }))
            : undefined;
        const result =
          inline ?? (await workspace.applyBufferAsync(encodePolicyWorkspaceRequest({ revision, ...request })));
        const parsed = workspaceResultSchema.parse(JSON.parse(result));
        if (request.deferGlobal || parsed.replacedPaths == null) {
          if (!request.deferGlobal) this.#workspaceEvaluations.set(workspace, parsed.evaluation);
          return parsed.evaluation;
        }
        const replaced = new Set(parsed.replacedPaths);
        const previous = this.#workspaceEvaluations.get(workspace) ?? emptyEvaluation();
        const evaluation: Evaluation = {
          ...parsed.evaluation,
          findings: [
            ...previous.findings.filter((finding) => finding.path === undefined || !replaced.has(finding.path)),
            ...parsed.evaluation.findings,
          ],
          failures: [
            ...previous.failures.filter((failure) => failure.path === undefined || !replaced.has(failure.path)),
            ...parsed.evaluation.failures,
          ],
        };
        this.#workspaceEvaluations.set(workspace, evaluation);
        return evaluation;
      } catch (error) {
        if (!(error instanceof Error) || error.message !== "RERAIL_POLICY_BUSY") throw error;
        await pause(backoff);
        backoff = Math.min(backoff * 2, 25);
      }
    }
    throw new Error("Policy execution interrupted by session closure");
  }
  async #publishReady(): Promise<void> {
    const ready = [...this.#readyFindings.entries()];
    this.#readyFindings.clear();
    for (const [revision, byPath] of ready) {
      const current = [...byPath].filter(([path]) => this.#versions.get(path) === revision);
      if (current.length === 0) continue;
      const findings = current.flatMap(([, pathFindings]) => pathFindings);
      const mode = this.#readyModes.get(revision) ?? this.#mode;
      this.#readyModes.delete(revision);
      this.#reportableFindings = findings;
      this.#responseMode = mode;
      await this.#publish(
        revision,
        current.map(([path]) => path),
        findings,
        mode,
        true,
      );
    }
  }

  async #publish(
    revision: number,
    sourceScope: readonly string[],
    findings: readonly ExecutablePolicyFinding[],
    mode: PolicyMode,
    generatedEdit = false,
  ): Promise<void> {
    if (this.#sessionId === undefined || this.#closing || mode !== "resteer") return;
    if (
      sourceScope.length === 0
        ? revision < this.#revision
        : sourceScope.some((path) => (this.#versions.get(path) ?? 0) > revision)
    )
      return;
    const scope = [...sourceScope].sort(compareDeterministicStrings);
    const steerable = findings.filter((finding) => finding.severity !== "advisory");
    if (steerable.length === 0) return;
    const steering = executablePolicySteering(steerable);
    await this.#delivery.requestDelivery(this.#sessionId, "steer", steering.content, {
      revision,
      scope,
      ruleIds: steering.ruleIds,
      background: true,
      ...(generatedEdit ? { generatedEdit: true } : {}),
    });
  }

  #response(revision: number): ExecutablePolicyResponse {
    const findings = this.#reportableFindings;
    return {
      revision,
      findings,
      failures: [...this.#evaluation.failures, ...this.#readFailures.values()],
      disposition:
        this.#responseMode === "resteer" && findings.some((finding) => finding.severity !== "advisory")
          ? "resteer"
          : "continue",
    };
  }

  #record(operation: () => void): void {
    try {
      operation();
    } catch (error) {
      process.stderr.write(`rerail: policy execution recording failed: ${errorMessage(error)}\n`);
    }
  }

  #startRun(
    scope: PolicyScope,
    mode: PolicyMode,
    revision: number,
    paths: string[],
    prepared: PreparedWorkspace | null | undefined = this.#replacement ?? this.#workspace,
    provenance: readonly RuleEvaluationProvenance[] = prepared?.provenance ?? this.#catalog.provenance(),
  ): string | undefined {
    if (this.#closing && prepared !== null) return undefined;
    let runId: string | undefined;
    this.#record(() => {
      runId = this.#executions.start({
        sessionId: this.#sessionId!,
        workspaceRoot: this.#workspaceRoot,
        scope,
        mode,
        catalogRevision: prepared === null ? null : (prepared?.version ?? this.#catalog.status.version),
        ruleRevisions: prepared === null ? [] : provenance.map(({ ruleId, revision }) => ({ ruleId, revision })),
        revision,
        paths,
        ruleProvenance: prepared === null ? [] : [...provenance],
      });
      if (this.#closing) this.#executions.finish(runId, { status: "interrupted", findings: [], failures: [] });
    });
    return runId;
  }
  #ensureStoredRun(
    stored: StoredEdit,
    path: string,
    provenance: readonly RuleEvaluationProvenance[],
  ): string | undefined {
    if (stored.runId === undefined) {
      const runId = this.#startRun(stored.scope, stored.mode, stored.revision, [path], undefined, provenance);
      if (runId !== undefined) stored.runId = runId;
    }
    return stored.runId;
  }
  #finishRun(
    runId: string | undefined,
    status: PolicyRunResult["status"],
    findings: ExecutablePolicyFinding[],
    failures: PolicyExecutionFailure[],
  ): void {
    if (runId !== undefined && !this.#closing)
      this.#record(() => this.#executions.finish(runId, { status, findings, failures }));
  }
  #recordImmediate(
    scope: PolicyScope,
    mode: PolicyMode,
    revision: number,
    paths: string[],
    failures: PolicyExecutionFailure[],
  ): void {
    this.#finishRun(this.#startRun(scope, mode, revision, paths), "failed", [], failures);
  }
  #trackDiagnostic(task: Promise<unknown>, generatedEdit = false): void {
    this.#diagnostics.add(task);
    if (generatedEdit) this.#generatedEditDiagnostics.add(task);
    void task.then(
      () => {
        this.#diagnostics.delete(task);
        this.#generatedEditDiagnostics.delete(task);
      },
      (error) => {
        this.#diagnostics.delete(task);
        this.#generatedEditDiagnostics.delete(task);
        process.stderr.write(`rerail: policy diagnostic failed: ${errorMessage(error)}\n`);
      },
    );
  }
  async #store(
    edit: ExecutablePolicyEdit,
    revision: number,
    scope: PolicyScope,
    mode: PolicyMode,
  ): Promise<StoredEdit> {
    const bytes = Buffer.byteLength(edit.generatedText ?? "") + Buffer.byteLength(edit.previousText ?? "");
    const metadata = { revision, scope, mode, references: 1, runId: undefined };
    if (this.#pendingBytes + bytes <= MAX_PENDING_BYTES) {
      this.#pendingBytes += bytes;
      return { edit, bytes, ...metadata };
    }
    this.#spoolDirectory ??= await mkdtemp(join(tmpdir(), "rerail-policy-"));
    const spool = join(this.#spoolDirectory, `${++this.#spoolSequence}.json`);
    await writeFile(spool, JSON.stringify(edit), { mode: 0o600 });
    return { spool, bytes: 0, ...metadata };
  }
  async #load(stored: StoredEdit): Promise<ExecutablePolicyEdit> {
    if (stored.edit !== undefined) return stored.edit;
    const { path, language, ...fields } = storedEditSchema.parse(JSON.parse(await readFile(stored.spool!, "utf8")));
    return { path, language, ...Object.fromEntries(Object.entries(fields).filter(([, value]) => value !== undefined)) };
  }
  #release(stored: StoredEdit): void {
    if (--stored.references !== 0) return;
    this.#pendingBytes -= stored.bytes;
    if (stored.spool === undefined) return;
    const removal: Promise<void> = rm(stored.spool, { force: true })
      .catch((error) => {
        if (!this.#closing)
          this.#recordImmediate(
            this.#scope,
            this.#mode,
            stored.revision,
            [],
            [failure("spool", "adapter-error", errorMessage(error), undefined, "persistence")],
          );
      })
      .finally(() => this.#spoolRemovals.delete(removal));
    this.#spoolRemovals.add(removal);
  }
}
