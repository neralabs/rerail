import { type Dirent, type Stats } from "node:fs";
import { lstat, mkdtemp, opendir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { setImmediate as yieldToHost, setTimeout as pause } from "node:timers/promises";
import { diffLines } from "diff";
import { z } from "zod";
import {
  encodeCodeRuleWorkspaceRequest,
  loadNativeBinding,
  RESULT_PAGE_BYTES,
  type NativeCodeRuleDslWorkspace,
  type NativeRepositoryIndexOptions,
} from "../native/native.js";
import type { CodeRuleDslCatalog } from "./code-rule-dsl-catalog.js";
import {
  buildSteer,
  type CodeRuleEdit,
  type CodeRuleEditBatch,
  type CodeRuleAuditor,
  type CodeRuleViolation,
  type CodeRuleResponse,
  type AuditSettings,
  type CodeRuleSourceLanguage,
  type RuleSteerDelivery,
  type RepositoryAuditHooks,
} from "./code-rule.js";
import {
  AUDIT_COVERAGE_STATUSES,
  type ClauseCoverage,
  type AuditFailure,
  type AuditRecorder,
  type AuditMode,
  type AuditResult,
  type AuditRuleProvenance,
  type AuditScope,
} from "../audit/audit.js";
import { defaultRerailDatabasePath } from "../storage/database.js";
import { SqliteAuditStore } from "../audit/audit-store.js";
import { RULE_MODES, steers } from "../catalog/rule-mode.js";
import { compareDeterministicStrings } from "../stable-json.js";
import { assertContentStoreOwner } from "../native/main-instance.js";
import { contentStorePath } from "../native/rerail-build.js";
import { withinCheckout } from "../catalog/checkout-control.js";
import { languageOfPath, SOURCE_LANGUAGES } from "./code-rule-languages.js";

export { SOURCE_LANGUAGES };


const MAX_SOURCE_BYTES = 32_000_000;
const MAX_PENDING_BYTES = 8 * 1024 * 1024;
const AUDIT_BATCH_FILES = 32;
const AUDIT_CHUNK_BYTES = 1024 * 1024;
const AUDIT_MAX_DURATION_MS = 5 * 60_000;
const AUDIT_MAX_FILES = 100_000;
const AUDIT_MAX_FAILURES = 20;
const AUDIT_MAX_SOURCE_BYTES = 512 * 1024 * 1024;
export const IGNORED_DIRECTORIES = new Set([
  ".git",
  ".hg",
  ".svn",
  ".rerail",
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
/** A repository index honours the walk's bounds, ignored directories and source languages. */
const REPOSITORY_INDEX_OPTIONS: Omit<NativeRepositoryIndexOptions, "nestedCheckouts"> = {
  maxFiles: AUDIT_MAX_FILES,
  maxSourceBytes: AUDIT_MAX_SOURCE_BYTES,
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
const violationSchema = z.strictObject({
  id: z.string().min(1),
  ruleId: z.string().min(1),
  clauseId: z.string().min(1),
  code: z.string().min(1),
  mode: z.enum(RULE_MODES),
  message: z.string(),
  path: z.string().optional(),
  span: sourceSpanSchema.optional(),
  captures: z.record(z.string(), z.array(sourceSpanSchema)),
  variables: z.record(z.string(), z.union([z.boolean(), z.number(), z.string()])),
  fix: z.strictObject({ span: sourceSpanSchema, text: z.string() }).optional(),
});
const failureSchema = z.strictObject({
  ruleId: z.string().exactOptional(),
  clauseId: z.string().min(1),
  code: z.enum(["budget-exceeded", "missing-capability", "source-incomplete", "source-partial", "adapter-error"]),
  message: z.string(),
  path: z.string().optional(),
  stage: z.enum(["read", "parse", "evaluate", "scan", "scope", "persistence"]).optional(),
});
const coverageSchema: z.ZodType<ClauseCoverage> = z.strictObject({
  clauseId: z.string().min(1),
  ruleId: z.string().exactOptional(),
  status: z.enum(AUDIT_COVERAGE_STATUSES),
  reasons: z.array(z.strictObject({
    code: failureSchema.shape.code,
    count: z.number().int().positive(),
    paths: z.array(z.string()),
  })),
});
// The enforcer reports violations and failures; it validates the native per-clause coverage and does not keep it, since
// a delta result's coverage covers its replaced paths only.
const evaluationSchema = z
  .strictObject({
    status: z.enum(["completed", "failed"]),
    violations: z.array(violationSchema),
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
  violations: z.array(violationSchema),
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
  /** Folders the walk skipped because each is a checkout of its own, relative to the workspace root. */
  nestedCheckouts: z.array(z.string()),
});
const storedEditSchema = z.object({
  path: z.string(),
  language: z.enum([...new Set(SOURCE_LANGUAGES.values())] as [CodeRuleSourceLanguage, ...CodeRuleSourceLanguage[]]),
  generatedText: z.string().optional(),
  previousText: z.string().optional(),
  previousReadError: z.string().optional(),
  deleted: z.boolean().optional(),
  readError: z.string().optional(),
});
type Evaluation = z.infer<typeof evaluationSchema>;
type WorkspaceFile = { language: CodeRuleSourceLanguage; path: string; source: string; module?: string | string[] };
type StoredEdit = {
  edit?: CodeRuleEdit;
  spool?: string;
  bytes: number;
  references: number;
  revision: number;
  scope: AuditScope;
  mode: AuditMode;
  auditId: string | undefined;
};
type PreparedWorkspace = {
  workspace: NativeCodeRuleDslWorkspace;
  version: string;
  provenance: readonly AuditRuleProvenance[];
  /** The storage attach, settled before the workspace's first apply; true when a Git store attached (repository audits then index natively). */
  storage: Promise<boolean>;
};
type ByteRange = { start: number; end: number };
/** The nested checkouts a walk skips (judged by their own rules), and the ones it reached. */
type NestedCheckoutWalk = { readonly pruned: ReadonlySet<string>; readonly reached: string[] };
type ScopeAuditState = { bytes: number; deadline: number; files: number; stopped: boolean; nested: NestedCheckoutWalk };
type ScopeAuditCompletion = {
  workspace: NativeCodeRuleDslWorkspace;
  auditId: string | undefined;
  revision: number;
  scope: "repository" | "touched-files";
  mode: AuditMode;
  sourcePaths: readonly string[];
  failures: readonly AuditFailure[];
  stopped: boolean;
  generatedEdit: boolean;
  /** The native repository index produced the results; a complete one is recorded for reuse. */
  indexed: boolean;
};
type ScopeAuditSources = { sourcePaths: string[]; stopped: boolean };
/** A repository audit's run of this checkout, and the checkouts nested in it that it left to their own rules. */
export interface RepositoryAuditReport {
  readonly auditId: string | undefined;
  readonly nestedCheckouts: readonly string[];
}

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

function intersectsGeneratedRange(violation: CodeRuleViolation, ranges: readonly ByteRange[]): boolean {
  if (violation.span === undefined) return false;
  const { startByte, endByte } = violation.span;
  return ranges.some(({ start, end }) =>
    startByte === endByte ? start <= startByte && startByte <= end : startByte < end && start < endByte,
  );
}

export function languageFor(path: string): CodeRuleSourceLanguage | undefined {
  return languageOfPath(SOURCE_LANGUAGES, path);
}

/**
 * A file's module as the engine derives it for every ingestion path: for Python, Ruff's module path
 * (`detect_package_root`), dotted, a list when a name keeps a dot (`["json.d", "__init__"]`), or undefined outside a
 * package; undefined for a family without modules. Native repository indexing derives the same value itself, so
 * buffer, inline and repository requests agree.
 */
export function modulePath(label: CodeRuleSourceLanguage, absolute: string): string | string[] | undefined {
  return loadNativeBinding().modulePath(label, absolute) ?? undefined;
}

function emptyEvaluation(): Evaluation {
  return {
    status: "completed",
    violations: [],
    failures: [],
    metrics: { files: 0, visitedNodes: 0, witnesses: 0 },
  };
}
function failure(
  clauseId: string,
  code: AuditFailure["code"],
  message: string,
  path?: string,
  stage?: AuditFailure["stage"],
): AuditFailure {
  return { clauseId: clauseId, code, message, ...(path === undefined ? {} : { path }), ...(stage === undefined ? {} : { stage }) };
}
function auditBudgetFailure(message: string): AuditFailure {
  return failure("scan", "budget-exceeded", message, undefined, "scan");
}
const AUDIT_FAILURES_STOPPED = `Repository scan stopped after reaching ${AUDIT_MAX_FAILURES} failures`;
/** At most `AUDIT_MAX_FAILURES` failures, then one budget failure saying the scan stopped there. */
function cappedAuditFailures(failures: AuditFailure[]): AuditFailure[] {
  if (failures.length <= AUDIT_MAX_FAILURES) return failures;
  return [...failures.slice(0, AUDIT_MAX_FAILURES), auditBudgetFailure(AUDIT_FAILURES_STOPPED)];
}
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** Generated snapshots and repository audits have independent native state. */
export class NativeCodeRuleEnforcer implements CodeRuleAuditor {
  readonly #workspaceRoot: string;
  readonly #catalog: CodeRuleDslCatalog;
  readonly #steerDelivery: RuleSteerDelivery;
  readonly #audits: AuditRecorder;
  readonly #ownedAudits: SqliteAuditStore | undefined;
  /** How a path relative to the workspace root is shown to the session (steering text and delivery scope). */
  readonly #presentPath: (path: string) => string;
  /** Folders below the workspace root that are checkouts of their own, as the owner of this lane decides. */
  readonly #nestedCheckouts: () => readonly string[];
  #scope: AuditScope;
  #mode: AuditMode;
  #workspace: PreparedWorkspace | undefined;
  #replacement: PreparedWorkspace | undefined;
  #sessionId: string | undefined;
  #revision = 0;
  readonly #versions = new Map<string, number>();
  readonly #workspaceRevisions = new Map<NativeCodeRuleDslWorkspace, number>();
  readonly #workspaceEvaluations = new Map<NativeCodeRuleDslWorkspace, Evaluation>();
  readonly #workspaceStorage = new Map<NativeCodeRuleDslWorkspace, Promise<boolean>>();
  readonly #pending = new Map<string, StoredEdit>();
  readonly #applied = new Map<string, StoredEdit>();
  readonly #readFailures = new Map<string, AuditFailure>();
  /** The quarantined set last reported, so each catalog load reports a skipped rule once rather than on every edit. */
  #quarantineReport: string | undefined;
  /** Why repository reuse was last reported disabled, so a persistent cause is reported once rather than on every audit. */
  #reuseDisabledReport: string | undefined;
  #pendingBytes = 0;
  readonly #readyModes = new Map<number, AuditMode>();
  readonly #readyViolations = new Map<number, Map<string, CodeRuleViolation[]>>();
  #spoolDirectory: string | undefined;
  #spoolSequence = 0;
  readonly #spoolRemovals = new Set<Promise<void>>();
  readonly #storing = new Set<Promise<StoredEdit>>();
  #pump: Promise<void> | undefined;
  #busy = false;
  #closing = false;
  #closed = false;
  readonly #scopeAudits = new Set<Promise<unknown>>();
  readonly #generatedEditAudits = new Set<Promise<unknown>>();
  #reportableViolations: CodeRuleViolation[] = [];
  #responseMode: AuditMode;
  #evaluation = emptyEvaluation();

  constructor(
    catalog: CodeRuleDslCatalog,
    options: {
      workspaceRoot: string;
      steerDelivery: RuleSteerDelivery;
      audits?: AuditRecorder;
      scope?: AuditScope;
      mode?: AuditMode;
      presentPath?: (path: string) => string;
      /**
       * Folders below the workspace root that are separate checkouts judged by their own rules; repository walks
       * skip them. Any other folder is walked, whether or not it holds a `.git` entry. None by default.
       */
      nestedCheckouts?: () => readonly string[];
    },
  ) {
    this.#catalog = catalog;
    this.#workspaceRoot = resolve(options.workspaceRoot);
    this.#steerDelivery = options.steerDelivery;
    this.#scope = options.scope ?? "touched-files";
    this.#mode = options.mode ?? "steer";
    this.#responseMode = this.#mode;
    this.#presentPath = options.presentPath ?? (path => path);
    this.#nestedCheckouts = options.nestedCheckouts ?? (() => []);
    this.#ownedAudits =
      options.audits === undefined
        ? new SqliteAuditStore(defaultRerailDatabasePath())
        : undefined;
    this.#audits = options.audits ?? this.#ownedAudits!;
  }

  acceptsPath(path: string): boolean {
    return (
      languageFor(path) !== undefined && ((this.#replacement ?? this.#workspace)?.workspace.acceptsPath(path) ?? true)
    );
  }

  snapshotStatus(): CodeRuleResponse & { pending: boolean } {
    return {
      ...this.#response(this.#revision),
      pending: this.#busy || this.#pending.size > 0 || this.#pump !== undefined || this.#scopeAudits.size > 0,
    };
  }
  async settleGeneratedEdits(): Promise<void> {
    while (
      this.#busy ||
      this.#pending.size > 0 ||
      this.#pump !== undefined ||
      this.#generatedEditAudits.size > 0
    ) {
      const work = [...(this.#pump === undefined ? [] : [this.#pump]), ...this.#generatedEditAudits];
      if (work.length === 0) {
        await yieldToHost();
        continue;
      }
      await Promise.allSettled(work);
    }
  }
  async settle(): Promise<void> {
    while (this.snapshotStatus().pending) {
      const work = [...(this.#pump === undefined ? [] : [this.#pump]), ...this.#scopeAudits];
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

  updateSettings(settings: Required<AuditSettings>): void {
    if (this.#closing) return;
    this.#scope = settings.scope;
    this.#mode = settings.mode;
  }
  /**
   * Synchronous, so the bridge's next `acceptsPath` already answers with the refreshed scopes: the replacement
   * registers its code rules now and attaches storage off the JS thread before its first apply.
   */
  refreshCodeRules(): void {
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

  async auditGeneratedEdits(sessionId: string, batch: CodeRuleEditBatch): Promise<CodeRuleResponse> {
    if (this.#sessionId !== sessionId || this.#closing) throw new Error("Executable policy enforcer session mismatch");
    if (batch.revision <= this.#revision) throw new Error("Executable policy revisions must increase");
    this.#revision = batch.revision;
    this.refreshCodeRules();
    if (this.#scope === "repository") {
      if (batch.edits.length > 0) {
        for (const edit of batch.edits) this.#versions.set(edit.path, batch.revision);
        this.#trackAudit(this.#runScopeAudit("repository", this.#mode, [], true), true);
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
    edit: CodeRuleEdit,
    revision: number,
    scope: AuditScope,
    mode: AuditMode,
  ): Promise<void> {
    if (this.#closing) {
      this.#startAudit(scope, mode, revision, [edit.path], null);
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
        this.#startAudit(scope, mode, revision, [edit.path], null);
        this.#release(stored);
        return;
      }
      const previous = this.#pending.get(edit.path);
      if (previous !== undefined) {
        previous.auditId ??= this.#startAudit(previous.scope, previous.mode, previous.revision, [edit.path], null);
        this.#finishAudit(previous.auditId, "superseded", [], []);
        this.#release(previous);
      }
      this.#pending.set(edit.path, stored);
    } catch (error) {
      if (this.#closing) {
        this.#startAudit(scope, mode, revision, [edit.path], null);
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

  async audit(
    sessionId: string,
    options: { scope: "repository" | "touched-files"; mode: AuditMode; paths?: string[]; audit?: RepositoryAuditHooks },
  ): Promise<readonly string[]> {
    if (sessionId !== this.#sessionId || this.#closing) throw new Error("Executable policy enforcer session mismatch");
    const paths = options.paths ?? [];
    if (options.scope === "repository" && paths.length > 0)
      throw new Error("Repository scope always covers the full repository");
    if (options.scope === "touched-files" && paths.length === 0)
      throw new Error("Touched-files policy scan requires paths");
    if (options.scope === "repository") {
      if (options.audit !== undefined && !options.audit.claim(this.#workspaceRoot)) return [];
      const { auditId } = await this.auditRepository(sessionId, options.mode);
      options.audit?.finish(this.#workspaceRoot, auditId);
      return auditId === undefined ? [] : [auditId];
    }
    const scope = paths.map((path) => {
      const absolute = resolve(this.#workspaceRoot, path);
      if (!withinCheckout(this.#workspaceRoot, absolute)) throw new Error("Policy scan path escapes workspace");
      return relative(this.#workspaceRoot, absolute).split(sep).join("/");
    });
    const task = this.#runScopeAudit(options.scope, options.mode, scope);
    this.#trackAudit(task);
    const auditId = await task;
    return auditId === undefined ? [] : [auditId];
  }

  /** Audit this whole checkout; the checkouts nested in it are skipped and reported, for their own rules to judge. */
  async auditRepository(sessionId: string, mode: AuditMode): Promise<RepositoryAuditReport> {
    if (sessionId !== this.#sessionId || this.#closing) throw new Error("Executable policy enforcer session mismatch");
    const nestedCheckouts: string[] = [];
    const task = this.#runScopeAudit("repository", mode, [], false, nestedCheckouts);
    this.#trackAudit(task);
    const auditId = await task;
    return { auditId, nestedCheckouts: [...new Set(nestedCheckouts)].sort(compareDeterministicStrings) };
  }

  interrupt(): void {
    if (this.#closing) return;
    for (const [path, stored] of this.#pending) {
      // Cancelled before evaluation: preserve the work without claiming a compiled-rule revision.
      stored.auditId ??= this.#startAudit(stored.scope, stored.mode, stored.revision, [path], null);
    }
    this.#closing = true;
    if (this.#sessionId !== undefined) this.#record(() => this.#audits.interrupt(this.#sessionId!));
    this.#workspace?.workspace.close();
    this.#replacement?.workspace.close();
  }
  async close(): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    this.interrupt();
    await this.#pump;
    await Promise.allSettled(this.#scopeAudits);
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
    this.#ownedAudits?.close();
  }

  /**
   * Rules the catalog skipped because their rows fail validation are adapter errors: kept in every response while
   * they last, and recorded as one failed run each time the catalog loads a different set of them. A rule kept
   * inactive as `language-not-supported` is no failure: it evaluates nothing until the engine reads its language.
   */
  #reportQuarantined(): void {
    const quarantined = this.#catalog.quarantined().filter(entry => entry.code === undefined);
    const report = JSON.stringify(quarantined.map(({ ruleId, owner, reason }) => [ruleId, owner, reason]));
    if (report === this.#quarantineReport) return;
    this.#quarantineReport = report;
    for (const key of [...this.#readFailures.keys()]) if (key.startsWith("quarantined:")) this.#readFailures.delete(key);
    const failures: AuditFailure[] = [];
    for (const { ruleId, owner, reason } of quarantined) {
      const message = `policy rule ${ruleId} (${owner}) is skipped: ${reason}`;
      const problem = { ...failure("catalog", "adapter-error", message, undefined, "parse"), ruleId };
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
    const workspace = new (loadNativeBinding().CodeRuleDslWorkspace)();
    let storage: Promise<boolean>;
    try {
      for (const codeRule of snapshot.codeRules) workspace.addCodeRule(codeRule.executable, JSON.stringify(codeRule.placement));
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

  #disposeWorkspace(workspace: NativeCodeRuleDslWorkspace): void {
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
          this.#finishAudit(
            this.#ensureStoredAudit(stored, path, (this.#replacement ?? this.#workspace)?.provenance ?? []),
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
      this.#ensureStoredAudit(stored, path, this.#workspace?.provenance ?? []);
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
    stored.auditId ??= this.#startAudit(stored.scope, stored.mode, stored.revision, [path], prepared);
    const edit = await this.#load(stored);
    if (this.#closing) return;
    const readFailure = this.#readFailureForEdit(path, edit);
    if (readFailure !== undefined) {
      this.#readFailures.set(path, readFailure);
      this.#finishAudit(stored.auditId, "failed", [], [readFailure]);
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

  #readFailureForEdit(path: string, edit: CodeRuleEdit): AuditFailure | undefined {
    if (edit.readError !== undefined) return failure("source", "source-incomplete", edit.readError, path, "read");
    if (edit.generatedText !== undefined && Buffer.byteLength(edit.generatedText) > MAX_SOURCE_BYTES)
      return failure("source", "budget-exceeded", "Generated source exceeds the per-file byte budget", path, "read");
    return undefined;
  }

  #replaceAppliedEdit(path: string, stored: StoredEdit, edit: CodeRuleEdit): void {
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
    edit: CodeRuleEdit,
    result: Evaluation,
  ): Promise<void> {
    const failures = result.failures.map((item) => ({ ...item, stage: "evaluate" as const }));
    const scopeFailure = this.#scopeFailureForEdit(stored.scope, path, edit);
    if (scopeFailure !== undefined) this.#readFailures.set(path, scopeFailure);
    const violations = scopeFailure === undefined ? this.#selectViolations(stored.scope, edit, result.violations) : [];
    this.#finishAudit(
      stored.auditId,
      scopeFailure === undefined && result.status === "completed" ? "completed" : "failed",
      violations,
      scopeFailure === undefined ? failures : [...failures, scopeFailure],
    );
    if (this.#versions.get(path) !== stored.revision || this.#replacement !== undefined) return;
    this.#evaluation = result;
    let readyViolations = this.#readyViolations.get(stored.revision);
    if (readyViolations === undefined) {
      readyViolations = new Map();
      this.#readyViolations.set(stored.revision, readyViolations);
    }
    readyViolations.set(path, violations);
    this.#readyModes.set(stored.revision, stored.mode);
    if (this.#pending.size === 0) await this.#publishReady();
  }

  #scopeFailureForEdit(
    scope: AuditScope,
    path: string,
    edit: CodeRuleEdit,
  ): AuditFailure | undefined {
    return scope === "ai-output" && edit.previousReadError !== undefined
      ? failure("preimage", "source-incomplete", edit.previousReadError, path, "scope")
      : undefined;
  }

  async #recordPendingEvaluationError(path: string, stored: StoredEdit, error: unknown): Promise<void> {
    if (this.#closing) return;
    const problem = failure("worker", "adapter-error", errorMessage(error), path, "evaluate");
    if (this.#pending.size === 0) await this.#publishReady();
    this.#readFailures.set(path, problem);
    this.#finishAudit(stored.auditId, "failed", [], [problem]);
  }
  #selectViolations(
    scope: AuditScope,
    edit: CodeRuleEdit,
    violations: readonly CodeRuleViolation[],
  ): CodeRuleViolation[] {
    const pathViolations = violations.filter((violation) => violation.path === edit.path);
    if (scope === "touched-files" || edit.deleted || edit.generatedText === undefined) return pathViolations;
    const ranges = generatedRanges(edit.previousText ?? "", edit.generatedText);
    return pathViolations.filter((violation) => intersectsGeneratedRange(violation, ranges));
  }
  /**
   * A repository-scope audit of a Git checkout runs the native repository index on the audit
   * workspace (`#indexRepository`); any other audit walks its paths here (`#walkAuditSources`).
   * Both evaluate the same files: Git and Rerail ignore rules decide only what the index restores.
   */
  async #runScopeAudit(
    scope: "repository" | "touched-files",
    mode: AuditMode,
    paths: string[] = [],
    generatedEdit = false,
    /** Receives the nested checkouts the walk reached and skipped. */
    reached: string[] = [],
  ): Promise<string | undefined> {
    this.refreshCodeRules();
    const revision = this.#revision;
    let auditId: string | undefined;
    const failures: AuditFailure[] = [];
    let workspace: NativeCodeRuleDslWorkspace | undefined;
    const nested: NestedCheckoutWalk = { pruned: new Set(this.#nestedCheckouts()), reached };
    try {
      await yieldToHost();
      if (this.#closing) return auditId;
      const prepared = this.#prepareWorkspace();
      workspace = prepared.workspace;
      auditId = this.#startAudit(scope, mode, revision, paths, prepared);
      const indexed = scope === "repository" && (await prepared.storage);
      const walked = indexed
        ? await this.#indexRepository(workspace, auditId, failures, nested)
        : await this.#walkAuditSources(workspace, auditId, paths, failures, nested);
      if (this.#closing) return auditId;
      await this.#finishScopeAudit({
        workspace,
        auditId,
        revision,
        scope,
        mode,
        sourcePaths: walked.sourcePaths,
        failures,
        stopped: walked.stopped,
        generatedEdit,
        indexed,
      });
      return auditId;
    } catch (error) {
      this.#finishAudit(
        auditId,
        "failed",
        [],
        [...failures, failure("scan", "adapter-error", errorMessage(error), undefined, "scan")],
      );
      return auditId;
    } finally {
      if (workspace !== undefined) this.#disposeWorkspace(workspace);
    }
  }

  /**
   * Index the Git checkout natively, one chunk per call. A chunk reads at most one batch (32 files,
   * reading stops at 1 MiB), publishes it and releases its sources and tapes before it returns, so
   * the audit's source residency is one batch: that bound replaces the former index worker's
   * peak-RSS limit. Interactive edits go first between chunks, and the walk's file, byte, failure
   * and five-minute budgets stop the index as they stop the walk.
   */
  async #indexRepository(
    workspace: NativeCodeRuleDslWorkspace,
    auditId: string | undefined,
    failures: AuditFailure[],
    nested: NestedCheckoutWalk,
  ): Promise<ScopeAuditSources> {
    const deadline = Date.now() + AUDIT_MAX_DURATION_MS;
    while (!this.#closing) {
      if (Date.now() >= deadline) {
        failures.push(auditBudgetFailure("Repository scan exceeded its five-minute work budget"));
        return { sourcePaths: [], stopped: true };
      }
      await this.#waitForEvaluationIdle();
      if (this.#closing) break;
      const chunk = repositoryIndexChunkSchema.parse(JSON.parse(await this.#indexChunk(workspace, nested)));
      this.#workspaceRevisions.set(workspace, chunk.revision);
      failures.push(...chunk.failures.map((item) => failure("source", item.code, item.message, item.path, "read")));
      const evaluation = chunk.evaluation;
      if (evaluation !== null && auditId !== undefined)
        this.#record(() =>
          this.#audits.progress(auditId, {
            violations: evaluation.violations,
            failures: [...failures, ...evaluation.failures],
          }),
        );
      if (chunk.overBudget) {
        failures.push(auditBudgetFailure("Repository scan exceeded its 100,000-file or 512 MiB source budget"));
        return { sourcePaths: [], stopped: true };
      }
      if (failures.length >= AUDIT_MAX_FAILURES) {
        failures.push(auditBudgetFailure(`Repository scan stopped after reaching ${AUDIT_MAX_FAILURES} failures`));
        return { sourcePaths: [], stopped: true };
      }
      if (chunk.done) {
        nested.reached.push(...chunk.nestedCheckouts);
        return { sourcePaths: [], stopped: false };
      }
      await yieldToHost();
    }
    return { sourcePaths: [], stopped: true };
  }

  /** One index chunk, retried while an interactive apply holds the evaluation slot. */
  async #indexChunk(workspace: NativeCodeRuleDslWorkspace, nested: NestedCheckoutWalk): Promise<string> {
    let backoff = 2;
    while (!this.#closing) {
      try {
        return await workspace.indexRepositoryAsync({ ...REPOSITORY_INDEX_OPTIONS, nestedCheckouts: [...nested.pruned] });
      } catch (error) {
        if (!(error instanceof Error) || error.message !== "WORKSPACE_BUSY") throw error;
        await pause(backoff);
        backoff = Math.min(backoff * 2, 25);
      }
    }
    throw new Error("Policy execution interrupted by session closure");
  }

  async #walkAuditSources(
    workspace: NativeCodeRuleDslWorkspace,
    auditId: string | undefined,
    paths: readonly string[],
    failures: AuditFailure[],
    nested: NestedCheckoutWalk,
  ): Promise<ScopeAuditSources> {
    let files: WorkspaceFile[] = [];
    let bytes = 0;
    const sourcePaths: string[] = [];
    const budget: ScopeAuditState = {
      bytes: 0,
      deadline: Date.now() + AUDIT_MAX_DURATION_MS,
      files: 0,
      stopped: false,
      nested,
    };
    for await (const file of this.#auditFiles(workspace, paths, failures, budget)) {
      if (this.#closing) return { sourcePaths, stopped: true };
      files.push(file);
      sourcePaths.push(file.path);
      bytes += Buffer.byteLength(file.source);
      if (files.length >= AUDIT_BATCH_FILES || bytes >= AUDIT_CHUNK_BYTES) {
        await this.#flushAuditBatch(workspace, auditId, failures, files);
        files = [];
        bytes = 0;
      }
    }
    if (files.length > 0) await this.#flushAuditBatch(workspace, auditId, failures, files);
    return { sourcePaths, stopped: budget.stopped };
  }

  async #flushAuditBatch(
    workspace: NativeCodeRuleDslWorkspace,
    auditId: string | undefined,
    failures: readonly AuditFailure[],
    files: WorkspaceFile[],
  ): Promise<void> {
    await this.#waitForEvaluationIdle();
    if (this.#closing) return;
    const result = await this.#applyRequest(workspace, { files, removed: [], deferGlobal: true });
    if (auditId !== undefined)
      this.#record(() =>
        this.#audits.progress(auditId, {
          violations: result.violations,
          failures: [...failures, ...result.failures],
        }),
      );
    await yieldToHost();
  }

  async #finishScopeAudit(completion: ScopeAuditCompletion): Promise<void> {
    const { workspace, auditId, revision, scope, mode, sourcePaths, failures, stopped, generatedEdit, indexed } =
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
    const streamed = await this.#resultPages(workspace, auditId, mode === "log" && !generatedEdit, failures);
    const result: Evaluation = {
      status: streamed.failures.length === 0 ? "completed" : "failed",
      violations: streamed.violations,
      failures: streamed.failures,
      metrics: { files: 0, visitedNodes: 0, witnesses: streamed.violations.length },
    };
    const unboundedFailures = [
      ...failures,
      ...result.failures.map((item) => ({ ...item, stage: "evaluate" as const })),
    ];
    const allFailures = cappedAuditFailures(unboundedFailures);
    const status = allFailures.length > 0 ? "failed" : "completed";
    if (streamed.paged && auditId !== undefined) {
      this.#finishResultPages(auditId, streamed.pageCount, status, unboundedFailures.length > AUDIT_MAX_FAILURES);
    } else {
      this.#finishAudit(auditId, status, result.violations, allFailures);
    }
    if (this.#scope === "repository" && revision === this.#revision) {
      this.#reportableViolations = result.violations;
      this.#responseMode = mode;
      this.#evaluation = { ...result, failures: allFailures };
    }
    await this.#publish(revision, scope === "repository" ? [] : sourcePaths, result.violations, mode, generatedEdit);
  }

  async #waitForEvaluationIdle(): Promise<void> {
    while (!this.#closing && (this.#busy || this.#pending.size > 0)) await pause(2);
  }
  async *#auditFiles(
    workspace: NativeCodeRuleDslWorkspace,
    paths: readonly string[],
    failures: AuditFailure[],
    state: ScopeAuditState,
  ): AsyncGenerator<WorkspaceFile> {
    // The walk starts at the resolved root, as native Git discovery does: a root reached through a
    // symlink is walked as its target, never skipped as a symlink.
    const canonicalRoot = await realpath(this.#workspaceRoot);
    const stack = paths.length === 0 ? [canonicalRoot] : paths.map((path) => resolve(canonicalRoot, path));
    const seen = new Set<string>();
    while (stack.length > 0 && !this.#closing) {
      if (Date.now() >= state.deadline) {
        failures.push(auditBudgetFailure("Repository scan exceeded its five-minute work budget"));
        state.stopped = true;
        return;
      }
      const absolute = stack.pop()!;
      const file = await this.#auditFileAtPath(workspace, canonicalRoot, absolute, stack, seen, failures, state.nested);
      if (failures.length >= AUDIT_MAX_FAILURES) {
        failures.push(auditBudgetFailure(`Repository scan stopped after reaching ${AUDIT_MAX_FAILURES} failures`));
        state.stopped = true;
        return;
      }
      if (this.#closing) return;
      if (file === undefined) continue;
      const bytes = Buffer.byteLength(file.source);
      if (state.files >= AUDIT_MAX_FILES || state.bytes + bytes > AUDIT_MAX_SOURCE_BYTES) {
        failures.push(auditBudgetFailure("Repository scan exceeded its 100,000-file or 512 MiB source budget"));
        state.stopped = true;
        return;
      }
      state.files += 1;
      state.bytes += bytes;
      yield file;
    }
  }

  async #auditFileAtPath(
    workspace: NativeCodeRuleDslWorkspace,
    canonicalRoot: string,
    absolute: string,
    stack: string[],
    seen: Set<string>,
    failures: AuditFailure[],
    nested: NestedCheckoutWalk,
  ): Promise<WorkspaceFile | undefined> {
    const path = relative(canonicalRoot, absolute).split(sep).join("/");
    try {
      const info = await lstat(absolute);
      if (info.isSymbolicLink()) return undefined;
      if (!(await this.#isAuditPathContained(canonicalRoot, absolute, path, failures))) return undefined;
      if (info.isDirectory()) {
        await this.#enqueueAuditDirectory(workspace, canonicalRoot, absolute, stack, nested);
        return undefined;
      }
      return await this.#readAuditFile(workspace, absolute, path, info, seen, failures);
    } catch (error) {
      failures.push(failure("source", "source-incomplete", errorMessage(error), path, "read"));
      return undefined;
    }
  }

  async #isAuditPathContained(
    canonicalRoot: string,
    absolute: string,
    path: string,
    failures: AuditFailure[],
  ): Promise<boolean> {
    if (withinCheckout(canonicalRoot, await realpath(absolute))) return true;
    failures.push(failure("scope", "source-incomplete", "Scan path resolves outside the workspace", path, "read"));
    return false;
  }

  async #enqueueAuditDirectory(
    workspace: NativeCodeRuleDslWorkspace,
    canonicalRoot: string,
    absolute: string,
    stack: string[],
    nested: NestedCheckoutWalk,
  ): Promise<void> {
    for await (const entry of await opendir(absolute)) {
      if (this.#closing) return;
      if (!this.#shouldWalkDirectoryEntry(workspace, canonicalRoot, absolute, entry, nested)) continue;
      stack.push(join(absolute, entry.name));
    }
  }

  /** A folder the lane's owner names as a checkout of its own is skipped and reported, so its own rules judge it. */
  #shouldWalkDirectoryEntry(
    workspace: NativeCodeRuleDslWorkspace,
    canonicalRoot: string,
    directory: string,
    entry: Dirent,
    nested: NestedCheckoutWalk,
  ): boolean {
    if (entry.isSymbolicLink() || (entry.isDirectory() && IGNORED_DIRECTORIES.has(entry.name))) return false;
    const absolute = join(directory, entry.name);
    const path = relative(canonicalRoot, absolute).split(sep).join("/");
    if (entry.isDirectory()) {
      if (!nested.pruned.has(path)) return true;
      nested.reached.push(path);
      return false;
    }
    return entry.isFile() && languageFor(path) !== undefined && workspace.acceptsPath(path);
  }

  async #readAuditFile(
    workspace: NativeCodeRuleDslWorkspace,
    absolute: string,
    path: string,
    info: Stats,
    seen: Set<string>,
    failures: AuditFailure[],
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

  /** The module path the engine qualifies a file's definitions and relative imports by, and file conditions read. */
  #module(language: CodeRuleSourceLanguage, path: string): { module?: string | string[] } {
    const module = modulePath(language, resolve(this.#workspaceRoot, path));
    return module === undefined ? {} : { module };
  }
  /**
   * Streams the results of an audit's private workspace. Nothing else applies to that workspace while
   * it pages; an apply would invalidate the native cursor, which the next page reports as stale.
   */
  async #resultPages(
    workspace: NativeCodeRuleDslWorkspace,
    auditId: string | undefined,
    allowPersistentPaging: boolean,
    precedingFailures: readonly AuditFailure[],
  ): Promise<Pick<Evaluation, "violations" | "failures"> & { paged: boolean; pageCount: number }> {
    const paged =
      allowPersistentPaging &&
      auditId !== undefined &&
      this.#audits.supportsResultPages === true &&
      this.#audits.appendResultPage !== undefined &&
      this.#audits.finishResultPages !== undefined;
    const violations: CodeRuleViolation[] = [];
    const failures: AuditFailure[] = [];
    let remainingFailures = Math.max(0, AUDIT_MAX_FAILURES - precedingFailures.length);
    let cursor = 0;
    let pageIndex = 0;
    if (paged && precedingFailures.length > 0) {
      this.#record(() =>
        this.#audits.appendResultPage?.(auditId!, pageIndex, {
          violations: [],
          failures: precedingFailures.slice(0, AUDIT_MAX_FAILURES),
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
          this.#audits.appendResultPage?.(auditId!, pageIndex, {
            violations: page.violations,
            failures: pageFailures,
          }),
        );
      } else {
        violations.push(...page.violations);
      }
      failures.push(...page.failures);
      pageIndex += 1;
      if (page.done) return { violations: violations, failures, paged, pageCount: pageIndex };
      if (page.cursor <= cursor) throw new Error("Native result stream did not advance");
      cursor = page.cursor;
      await yieldToHost();
      if (this.#closing) throw new Error("Policy execution interrupted by session closure");
    }
  }

  async #apply(
    workspace: NativeCodeRuleDslWorkspace,
    edit: CodeRuleEdit,
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
    workspace: NativeCodeRuleDslWorkspace,
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
          inline ?? (await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision, ...request })));
        const parsed = workspaceResultSchema.parse(JSON.parse(result));
        if (request.deferGlobal || parsed.replacedPaths == null) {
          if (!request.deferGlobal) this.#workspaceEvaluations.set(workspace, parsed.evaluation);
          return parsed.evaluation;
        }
        const replaced = new Set(parsed.replacedPaths);
        const previous = this.#workspaceEvaluations.get(workspace) ?? emptyEvaluation();
        const evaluation: Evaluation = {
          ...parsed.evaluation,
          violations: [
            ...previous.violations.filter((violation) => violation.path === undefined || !replaced.has(violation.path)),
            ...parsed.evaluation.violations,
          ],
          failures: [
            ...previous.failures.filter((failure) => failure.path === undefined || !replaced.has(failure.path)),
            ...parsed.evaluation.failures,
          ],
        };
        this.#workspaceEvaluations.set(workspace, evaluation);
        return evaluation;
      } catch (error) {
        if (!(error instanceof Error) || error.message !== "WORKSPACE_BUSY") throw error;
        await pause(backoff);
        backoff = Math.min(backoff * 2, 25);
      }
    }
    throw new Error("Policy execution interrupted by session closure");
  }
  async #publishReady(): Promise<void> {
    const ready = [...this.#readyViolations.entries()];
    this.#readyViolations.clear();
    for (const [revision, byPath] of ready) {
      const current = [...byPath].filter(([path]) => this.#versions.get(path) === revision);
      if (current.length === 0) continue;
      const violations = current.flatMap(([, pathViolations]) => pathViolations);
      const mode = this.#readyModes.get(revision) ?? this.#mode;
      this.#readyModes.delete(revision);
      this.#reportableViolations = violations;
      this.#responseMode = mode;
      await this.#publish(
        revision,
        current.map(([path]) => path),
        violations,
        mode,
        true,
      );
    }
  }

  async #publish(
    revision: number,
    sourceScope: readonly string[],
    violations: readonly CodeRuleViolation[],
    mode: AuditMode,
    generatedEdit = false,
  ): Promise<void> {
    if (this.#sessionId === undefined || this.#closing || mode !== "steer") return;
    if (
      sourceScope.length === 0
        ? revision < this.#revision
        : sourceScope.some((path) => (this.#versions.get(path) ?? 0) > revision)
    )
      return;
    const scope = sourceScope.map(path => this.#presentPath(path)).sort(compareDeterministicStrings);
    const steerable = violations.filter((violation) => steers(violation.mode)).map(violation => this.#presented(violation));
    if (steerable.length === 0) return;
    const provenance = this.#workspace?.provenance ?? this.#catalog.provenance();
    const steer = buildSteer(steerable, ruleId => {
      const recorded = provenance.find(entry => entry.ruleId === ruleId);
      return recorded !== undefined && "source" in recorded ? recorded.source : undefined;
    });
    await this.#steerDelivery.requestSteer(this.#sessionId, "steer", steer.content, {
      revision,
      scope,
      ruleIds: steer.ruleIds,
      background: true,
      ...(generatedEdit ? { generatedEdit: true } : {}),
    });
  }

  #response(revision: number): CodeRuleResponse {
    const violations = this.#reportableViolations;
    return {
      revision,
      violations: violations.map(violation => this.#presented(violation)),
      failures: [...this.#evaluation.failures, ...this.#readFailures.values()].map(item => this.#presented(item)),
      disposition: this.#responseMode === "steer" && violations.some((violation) => steers(violation.mode))
        ? "steer"
        : "continue",
    };
  }

  /** A violation or failure with its path as the session sees it; audit records keep the workspace-relative path. */
  #presented<T extends { path?: string | undefined }>(item: T): T {
    return item.path === undefined ? item : { ...item, path: this.#presentPath(item.path) };
  }

  #record(operation: () => void): void {
    try {
      operation();
    } catch (error) {
      process.stderr.write(`rerail: policy execution recording failed: ${errorMessage(error)}\n`);
    }
  }

  #startAudit(
    scope: AuditScope,
    mode: AuditMode,
    revision: number,
    paths: string[],
    prepared: PreparedWorkspace | null | undefined = this.#replacement ?? this.#workspace,
    provenance: readonly AuditRuleProvenance[] = prepared?.provenance ?? this.#catalog.provenance(),
  ): string | undefined {
    if (this.#closing && prepared !== null) return undefined;
    let auditId: string | undefined;
    this.#record(() => {
      auditId = this.#audits.start({
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
      if (this.#closing) this.#audits.finish(auditId, { status: "interrupted", violations: [], failures: [] });
    });
    return auditId;
  }
  #ensureStoredAudit(
    stored: StoredEdit,
    path: string,
    provenance: readonly AuditRuleProvenance[],
  ): string | undefined {
    if (stored.auditId === undefined) {
      const auditId = this.#startAudit(stored.scope, stored.mode, stored.revision, [path], undefined, provenance);
      if (auditId !== undefined) stored.auditId = auditId;
    }
    return stored.auditId;
  }
  /** Closes a paged audit, first adding the page that says the scan stopped at the failure limit when it did. */
  #finishResultPages(auditId: string, pageCount: number, status: AuditResult["status"], stoppedAtLimit: boolean): void {
    if (stoppedAtLimit)
      this.#record(() =>
        this.#audits.appendResultPage?.(auditId, pageCount, {
          violations: [],
          failures: [auditBudgetFailure(AUDIT_FAILURES_STOPPED)],
        }),
      );
    this.#record(() => this.#audits.finishResultPages?.(auditId, status));
  }
  #finishAudit(
    auditId: string | undefined,
    status: AuditResult["status"],
    violations: CodeRuleViolation[],
    failures: AuditFailure[],
  ): void {
    if (auditId !== undefined && !this.#closing)
      this.#record(() => this.#audits.finish(auditId, { status, violations: violations, failures }));
  }
  #recordImmediate(
    scope: AuditScope,
    mode: AuditMode,
    revision: number,
    paths: string[],
    failures: AuditFailure[],
  ): void {
    this.#finishAudit(this.#startAudit(scope, mode, revision, paths), "failed", [], failures);
  }
  #trackAudit(task: Promise<unknown>, generatedEdit = false): void {
    this.#scopeAudits.add(task);
    if (generatedEdit) this.#generatedEditAudits.add(task);
    void task.then(
      () => {
        this.#scopeAudits.delete(task);
        this.#generatedEditAudits.delete(task);
      },
      (error) => {
        this.#scopeAudits.delete(task);
        this.#generatedEditAudits.delete(task);
        process.stderr.write(`rerail: policy diagnostic failed: ${errorMessage(error)}\n`);
      },
    );
  }
  async #store(
    edit: CodeRuleEdit,
    revision: number,
    scope: AuditScope,
    mode: AuditMode,
  ): Promise<StoredEdit> {
    const bytes = Buffer.byteLength(edit.generatedText ?? "") + Buffer.byteLength(edit.previousText ?? "");
    const metadata = { revision, scope, mode, references: 1, auditId: undefined };
    if (this.#pendingBytes + bytes <= MAX_PENDING_BYTES) {
      this.#pendingBytes += bytes;
      return { edit, bytes, ...metadata };
    }
    this.#spoolDirectory ??= await mkdtemp(join(tmpdir(), "rerail-policy-"));
    const spool = join(this.#spoolDirectory, `${++this.#spoolSequence}.json`);
    await writeFile(spool, JSON.stringify(edit), { mode: 0o600 });
    return { spool, bytes: 0, ...metadata };
  }
  async #load(stored: StoredEdit): Promise<CodeRuleEdit> {
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
