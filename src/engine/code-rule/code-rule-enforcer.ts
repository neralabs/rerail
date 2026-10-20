import { type Dirent, type Stats } from "node:fs";
import { lstat, mkdtemp, opendir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, posix, relative, resolve, sep } from "node:path";
import { setImmediate as yieldToHost, setTimeout as pause } from "node:timers/promises";
import { diffLines } from "diff";
import { z } from "zod";
import {
  encodeCodeRuleWorkspaceRequest,
  loadNativeBinding,
  RESULT_PAGE_BYTES,
  type NativeCodeRuleDslWorkspace,
  type NativePackageFacts,
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
  type AuditorCloseOptions,
} from "./code-rule.js";
import {
  AUDIT_BLIND_REASONS,
  AUDIT_FAILURE_CODES,
  AUDIT_COVERAGE_STATUSES,
  BINARY_SOURCE_MESSAGE,
  isCoverageGap,
  isFileWorkStopped,
  isOutOfWork,
  isSourceGap,
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
import { physicalPath, withinCheckout } from "../catalog/checkout-control.js";
import { languageOfPath, SOURCE_LANGUAGES } from "./code-rule-languages.js";

export { SOURCE_LANGUAGES };


const MAX_SOURCE_BYTES = 32_000_000;
const MAX_PENDING_BYTES = 8 * 1024 * 1024;
const AUDIT_BATCH_FILES = 32;
const AUDIT_CHUNK_BYTES = 1024 * 1024;
const AUDIT_MAX_DURATION_MS = 5 * 60_000;
const AUDIT_MAX_FILES = 100_000;
const AUDIT_MAX_FAILURES = 20;
/** The recorded `package` facts of a source a superseded evaluation repackaged: never equal to any facts' JSON, so the source is resent. */
const UNREPORTED_FACTS = "\0unreported";
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
  code: z.enum(AUDIT_FAILURE_CODES),
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
/** A file no clause read (native `blind_file`): every clause on it failed for its work budget or its syntax. */
const blindFileSchema = z.strictObject({
  path: z.string(),
  reason: z.enum(AUDIT_BLIND_REASONS),
  clauses: z.number().int().positive(),
  message: z.string(),
  /** The clauses (`[ruleId, clauseId]`) whose failures on the file this row stands for. */
  folded: z.array(z.tuple([z.string(), z.string()])),
});
const resultPageSchema = z.strictObject({
  revision: z.number().int().nonnegative(),
  cursor: z.number().int().nonnegative(),
  done: z.boolean(),
  violations: z.array(violationSchema),
  failures: z.array(failureSchema),
  coverage: z.array(coverageSchema),
  blindFiles: z.array(blindFileSchema).default([]),
  bytes: z.number().int().nonnegative(),
});
const workspaceResultSchema = z.strictObject({
  revision: z.number().int().nonnegative(),
  replacedPaths: z.array(z.string()).nullable().optional(),
  blindFiles: z.array(blindFileSchema).default([]),
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
type WorkspaceFile = { language: CodeRuleSourceLanguage; path: string; source: string } & SourceFileFacts;
type StoredEdit = {
  edit?: CodeRuleEdit;
  spool?: string;
  /** The edit's language, kept beside a spooled edit so its facts can be compared without reading it back. */
  language: CodeRuleSourceLanguage;
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
/** One audit walk's budget and what it has read; `facts` holds each directory's nearest `package.json` and server graphs. */
type ScopeAuditState = {
  bytes: number;
  deadline: number;
  files: number;
  stopped: boolean;
  nested: NestedCheckoutWalk;
  facts: SourceFactsCache;
};
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
type ScopeAuditSources = { sourcePaths: string[]; stopped: boolean; files: number };
/** What a finished scope audit reports to the daemon log. */
type ScopeAuditSummary = { resultPagesMilliseconds: number; violations: number; failures: number; status: AuditResult["status"] };
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

/** What the engine reads of a source file besides its text: its `module`, and its nearest `package.json` (`package`). */
export type SourceFileFacts = { module?: string | string[]; package?: NativePackageFacts };

/**
 * The manifests one walk read, by language and directory, and the server graphs of their packages, by folder (each
 * requested once, so concurrent readers share one walk; `null` for a package with no graph).
 */
export type SourceFactsCache = {
  packages: Map<string, NativePackageFacts | null>;
  serverGraphs: Map<string, Promise<ReadonlySet<string> | null>>;
};

export function sourceFactsCache(): SourceFactsCache {
  return { packages: new Map(), serverGraphs: new Map() };
}

/** A source whose text the host holds rather than disk: the edit being applied, or a draft's sample (no text: deleted). */
export type SourceOverlay = { path: string; source?: string | undefined };

/**
 * The facts every ingestion path sends with the file at `path` below `root`: its module (`modulePath`), and its nearest
 * `package.json`, never above `root` (none without one, or where the file's folder resolves outside `root`), whose
 * `type` decides an ECMA file's module system and which `file.package` and `file.jsxRuntime` conditions read; the
 * engine sends none for a family that reads no manifest. With `serverGraph` (a code rule reads
 * `file.package.serverGraph`), a file whose manifest names `next` also carries whether it is in its Next app's server
 * module graph, walked on disk off the JS thread once per package and cache, `overlay`'s text standing in for its
 * file's; a package with no graph (an entry unreadable, or past the walk's bounds) gives its files no `serverGraph`, so
 * neither value holds. The engine derives all of them; native repository indexing derives the same values itself.
 * `cache` keeps one walk's manifests and graphs, so one cache serves one overlay. The facts depend on the path, the
 * files on disk and `overlay` only, never on the file's own text.
 */
export async function sourceFileFacts(
  root: string,
  path: string,
  language: CodeRuleSourceLanguage,
  cache?: SourceFactsCache,
  serverGraph = false,
  overlay?: SourceOverlay,
): Promise<SourceFileFacts> {
  const binding = loadNativeBinding();
  const absolute = resolve(root, path);
  const module = modulePath(language, absolute);
  const key = `${language}\0${posix.dirname(path)}`;
  let facts = cache?.packages.get(key);
  if (facts === undefined) {
    facts =
      binding.readsPackageManifests(language) && folderWithinRoot(root, path)
        ? binding.packageFacts(language, absolute, root)
        : null;
    cache?.packages.set(key, facts);
  }
  if (facts !== null && serverGraph && facts.dependencies.includes("next")) {
    let members = cache?.serverGraphs.get(facts.folder);
    if (members === undefined) {
      members = binding
        .serverGraphAsync(root, facts.folder, overlay?.path, overlay?.source)
        .then(paths => (paths === null ? null : new Set(paths)));
      cache?.serverGraphs.set(facts.folder, members);
    }
    const graph = await members;
    const relative = facts.folder === "" ? path : path.slice(facts.folder.length + 1);
    if (graph !== null) facts = { ...facts, serverGraph: graph.has(relative) };
  }
  return { ...(module === undefined ? {} : { module }), ...(facts === null ? {} : { package: facts }) };
}

/**
 * Whether the folder holding `path` (below `root`), as its deepest existing ancestor resolves through symlinks, lies
 * within `root`'s real path: a file reached through a symlinked folder that leads out of the checkout reads no
 * manifest, project configuration or server graph there.
 */
function folderWithinRoot(root: string, path: string): boolean {
  try {
    return withinCheckout(physicalPath(root), physicalPath(dirname(resolve(root, path))));
  } catch {
    return false;
  }
}

/** The folder a `package.json` path holds the manifest of (`""` for the root one); undefined for any other path. */
function manifestDirectory(path: string): string | undefined {
  if (path === "package.json") return "";
  return path.endsWith("/package.json") ? path.slice(0, -"/package.json".length) : undefined;
}

/**
 * Whether `path` names a TypeScript or JavaScript project configuration: a `.json` file whose name starts with
 * `tsconfig` or `jsconfig`, the files the engine reads for `jsxImportSource` and import aliases (`is_module_config`).
 */
function isModuleConfig(path: string): boolean {
  return /^(?:ts|js)config[^/]*\.json$/.test(posix.basename(path));
}

/** Whether `path` lies in a folder named `app`, which a Next package's server module graph reads the presence of. */
function inAppFolder(path: string): boolean {
  return /(?:^|\/)app\//.test(path);
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
/** What a person can change so a rule that ran out of work finishes. A global query's failure names no one file. */
const OUT_OF_WORK_ADVICE =
  "The rule ran out of work: simplify the rule, or exclude generated or vendored files from its scope.";
/** What a person can change so the file's work no longer stops a rule (the file, or the rules that ran before it). */
const FILE_WORK_ADVICE =
  "The file's work ran out before this rule finished, spent by its size or the rules before it: exclude generated or vendored files from the rules' scope, or split the file.";
const BLIND_BUDGET_ADVICE = "If the file is generated or vendored, exclude it from the rules' scope; otherwise split it.";
/** `item` with what would close it, where it is a coverage gap of one rule. */
function advised(item: AuditFailure): AuditFailure {
  if (isOutOfWork(item)) return { ...item, message: `${item.message}. ${OUT_OF_WORK_ADVICE}` };
  if (isCoverageGap(item)) return { ...item, message: `${item.message}. ${FILE_WORK_ADVICE}` };
  return item;
}
/**
 * `failures` with each blind file's work and syntax failures, one per clause, replaced by one failure for the file
 * (`blind` set to why no clause read it), and each other rule's coverage gap told what would close it. Only the
 * failures the engine folded into the row (`folded`) are replaced: a cross-file query's failure on the file stays.
 */
function withBlindFiles(failures: readonly AuditFailure[], blindFiles: readonly z.infer<typeof blindFileSchema>[]): AuditFailure[] {
  const folded = new Set(blindFiles.flatMap((file) =>
    file.folded.map(([ruleId, clauseId]) => JSON.stringify([file.path, ruleId, clauseId]))));
  return [
    ...failures
      .filter((item) => !folded.has(JSON.stringify([item.path, item.ruleId, item.clauseId])))
      .map(advised),
    ...blindFiles.map((file) => ({
      ...failure(
        "file",
        file.reason === "budget" ? "budget-exceeded" : "source-incomplete",
        `Every rule failed on this file, so it was not checked (${file.clauses} clause${file.clauses === 1 ? "" : "s"}): ${file.message}${file.reason === "budget" ? `. ${BLIND_BUDGET_ADVICE}` : ""}`,
        file.path,
        "evaluate",
      ),
      blind: file.reason,
    })),
  ];
}
/**
 * Whether a failure is something the audit reports and still completes: a coverage gap (`isCoverageGap`), or, in a
 * repository audit, a file read only in part (`isSourceGap`); a touched-files scan of such a file fails.
 */
function tolerated(item: AuditFailure, scope: AuditScope): boolean {
  return isCoverageGap(item) || (scope === "repository" && isSourceGap(item));
}
/** Whether `failures` fail an audit of `scope`: any does but those it tolerates (`tolerated`). */
function failsAudit(failures: readonly AuditFailure[], scope: AuditScope): boolean {
  return failures.some((item) => !tolerated(item, scope));
}
/**
 * Whether a failure is always listed: a blind file's, and a scan-level one (the walk stopped at a guardrail, or the
 * scan itself failed), which says what the rest of the list cannot. Other failures are details, listed up to a cap.
 */
function alwaysListed(item: AuditFailure): boolean {
  return item.blind !== undefined || item.stage === "scan";
}
/**
 * A cap on the failure list of one audit of `scope`, over the lists it is given in turn (the failures before
 * evaluation, then each result page): it keeps every failure `alwaysListed` keeps, the first failure of each rule that
 * ran out of work and of each rule the file's work stopped (so the list names every such rule and the summary counts
 * it), at most `limit` failures that fail the audit and at most `limit` it tolerates (`tolerated`), so gaps never crowd
 * out what failed it. Each list is ordered with those that fail the audit first, then the tolerated ones.
 */
function auditFailureCap(scope: AuditScope, limit = AUDIT_MAX_FAILURES): (failures: readonly AuditFailure[]) => AuditFailure[] {
  let failing = limit;
  let gaps = limit;
  const counted = new Set<string>();
  const gap = (item: AuditFailure) => tolerated(item, scope);
  const firstOfRule = (item: AuditFailure) => {
    const kind = isOutOfWork(item) ? "own" : isFileWorkStopped(item) ? "file" : undefined;
    if (kind === undefined || item.ruleId === undefined || counted.has(`${kind}:${item.ruleId}`)) return false;
    counted.add(`${kind}:${item.ruleId}`);
    return true;
  };
  return (failures) => [...failures.filter((item) => !gap(item)), ...failures.filter(gap)]
    .filter((item) => alwaysListed(item) || firstOfRule(item) || (gap(item) ? gaps-- > 0 : failing-- > 0));
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
  readonly #timingLog: ((line: string) => void) | undefined;
  #scope: AuditScope;
  #mode: AuditMode;
  #workspace: PreparedWorkspace | undefined;
  #replacement: PreparedWorkspace | undefined;
  #sessionId: string | undefined;
  #revision = 0;
  readonly #versions = new Map<string, number>();
  readonly #workspaceRevisions = new Map<NativeCodeRuleDslWorkspace, number>();
  readonly #workspaceEvaluations = new Map<NativeCodeRuleDslWorkspace, Evaluation>();
  /** The `package` (as JSON, `null` for none) of each ECMA source `#apply` gave a workspace, by path. */
  readonly #workspacePackages = new Map<NativeCodeRuleDslWorkspace, Map<string, string>>();
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
  /**
   * The findings each source last reported in its own slot, with the revision and mode it reported them at: what a
   * repackaged source's new findings are compared with, and what steers again a source whose queued steer a revision
   * of another source dropped (`#steerAgain`).
   */
  readonly #sourceFindings = new Map<string, { revision: number; mode: AuditMode; violations: CodeRuleViolation[] }>();
  /**
   * The whole-file findings a steer-mode touched-files audit last steered for each source, newer than what the source
   * reported in its own slot: read only by `#steerAgain`, never compared with a source's own-line findings.
   */
  readonly #auditFindings = new Map<string, { revision: number; mode: AuditMode; violations: CodeRuleViolation[] }>();
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
  /** Audits this evaluator started and has not finished: all it may end when the session-wide interrupt is not its to make. */
  readonly #runningAudits = new Set<string>();
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
      /** Receives the storage-attach and repository-audit timing lines (the daemon passes its log); without it they are not written. */
      timingLog?: (line: string) => void;
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
    this.#timingLog = options.timingLog;
    this.#ownedAudits =
      options.audits === undefined
        ? new SqliteAuditStore(defaultRerailDatabasePath())
        : undefined;
    this.#audits = options.audits ?? this.#ownedAudits!;
  }

  /**
   * Where a code rule has an AST clause in a family that reads manifests (ECMA), a `package.json` is accepted whatever
   * the code rules' scopes: it gives the ECMA sources below it their `package`, so an edit of it alone refreshes the
   * sources already applied (`#apply`). So is a `tsconfig.json`-style project configuration, whose `jsxImportSource`
   * and import aliases (`paths`, `baseUrl`) those `package` facts read, and, where a code rule reads Next server module
   * graphs, any source a manifest reads, since its imports move other sources in or out of a graph, and any file in an
   * `app/` folder, which may make a root `app/` (hiding `src/app/`) present. Without such a rule, the scopes alone
   * decide.
   */
  acceptsPath(path: string): boolean {
    const language = languageFor(path);
    const workspace = (this.#replacement ?? this.#workspace)?.workspace;
    const graphInput = language !== undefined && (loadNativeBinding().readsPackageManifests(language) || inAppFolder(path));
    return (
      language !== undefined &&
      (((manifestDirectory(path) !== undefined || isModuleConfig(path)) &&
        (workspace?.readsPackageManifests() ?? true)) ||
        (workspace?.readsServerGraph() === true && graphInput) ||
        (workspace?.acceptsPath(path) ?? true))
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
   * registers its code rules now and attaches storage off the JS thread before its first apply. Before the workspace
   * opens there is nothing to replace: opening prepares from the catalog as it is then.
   */
  refreshCodeRules(): void {
    if (this.#closing || this.#sessionId === undefined) return;
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

  /** Digest of the compiled rule catalog as of now: what a repository audit started now records as its `catalogRevision`. */
  catalogRevision(): string {
    this.refreshCodeRules();
    return this.#catalog.status.version;
  }

  async auditGeneratedEdits(sessionId: string, batch: CodeRuleEditBatch): Promise<CodeRuleResponse> {
    if (this.#sessionId !== sessionId || this.#closing) throw new Error("Executable policy enforcer session mismatch");
    if (batch.revision <= this.#revision) throw new Error("Executable policy revisions must increase");
    this.#revision = batch.revision;
    this.refreshCodeRules();
    const mode = batch.mode ?? this.#mode;
    if (this.#scope === "repository") {
      if (batch.edits.length > 0) {
        for (const edit of batch.edits) this.#versions.set(edit.path, batch.revision);
        this.#trackAudit(this.#runScopeAudit("repository", mode, [], true), true);
      }
      return this.#response(batch.revision);
    }
    const scope = this.#scope;
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
      if (options.audit !== undefined && !options.audit.claim(this.#workspaceRoot, this.catalogRevision())) return [];
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

  interrupt(options?: AuditorCloseOptions): void {
    if (this.#closing) return;
    for (const [path, stored] of this.#pending) {
      // Cancelled before evaluation: preserve the work without claiming a compiled-rule revision.
      stored.auditId ??= this.#startAudit(stored.scope, stored.mode, stored.revision, [path], null);
    }
    this.#closing = true;
    if (this.#sessionId !== undefined) {
      if (options?.interruptAudits === false) this.#interruptOwnAudits();
      else this.#record(() => this.#audits.interrupt(this.#sessionId!));
    }
    this.#workspace?.workspace.close();
    this.#replacement?.workspace.close();
  }

  /** Ends the audits this evaluator started and left running, without touching the session's other audits. */
  #interruptOwnAudits(): void {
    for (const auditId of this.#runningAudits)
      this.#record(() => this.#audits.finish(auditId, { status: "interrupted", violations: [], failures: [] }));
    this.#runningAudits.clear();
  }
  async close(options?: AuditorCloseOptions): Promise<void> {
    if (this.#closed) return;
    this.#closed = true;
    this.interrupt(options);
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
    this.#workspacePackages.clear();
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
      const attachStartedAt = performance.now();
      storage = workspace.enableStorageAsync(this.#workspaceRoot, storePath).then(({ attached, reason, discoveryMilliseconds, publicationMilliseconds }) => {
        this.#timingLog?.(
          `rerail: repository storage ${attached ? "attached" : "not attached"} in ${Math.round(performance.now() - attachStartedAt)} ms (discovery ${Math.round(discoveryMilliseconds)} ms, publication ${Math.round(publicationMilliseconds)} ms)\n`,
        );
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
    this.#workspacePackages.set(workspace, new Map());
    return { workspace, version: snapshot.version, provenance: snapshot.provenance, storage };
  }

  #disposeWorkspace(workspace: NativeCodeRuleDslWorkspace): void {
    workspace.close();
    this.#workspaceRevisions.delete(workspace);
    this.#workspaceEvaluations.delete(workspace);
    this.#workspacePackages.delete(workspace);
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
      // One facts cache for the pass: each package's graph is walked on disk once, not once per applied source.
      const facts = sourceFactsCache();
      for (const stored of this.#applied.values())
        await this.#apply(replacement.workspace, await this.#load(stored), facts);
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
    const { evaluation, repackaged, unreported } = await this.#apply(prepared.workspace, edit);
    this.#replaceAppliedEdit(path, stored, edit);
    await this.#finishPendingEvaluation(path, stored, edit, evaluation, repackaged, unreported);
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

  /**
   * Publishes an evaluation's findings, unless a later revision of its file or a workspace replacement supersedes it.
   * A superseded evaluation reports nothing for the sources it repackaged, so it marks them `unreported`: the next edit
   * that reaches them sends them again, and their findings are reported, whatever facts it gives them.
   */
  async #finishPendingEvaluation(
    path: string,
    stored: StoredEdit,
    edit: CodeRuleEdit,
    result: Evaluation,
    repackaged: readonly CodeRuleEdit[],
    unreported: () => void,
  ): Promise<void> {
    const failures = result.failures.map((item) => ({ ...item, stage: "evaluate" as const }));
    const scopeFailure = this.#scopeFailureForEdit(stored.scope, path, edit);
    if (scopeFailure !== undefined) this.#readFailures.set(path, scopeFailure);
    // Touched-files scope reports every source the edit gave another `package` (`#apply`). Any other scope reports each
    // edit's own generated lines, so such a source is reported again only where the findings in its own edit's
    // generated lines changed; the others keep their earlier findings and steers.
    const reported = new Map<string, CodeRuleViolation[]>();
    for (const source of repackaged) {
      const findings = this.#selectViolations(stored.scope, source, result.violations);
      if (
        stored.scope === "touched-files" ||
        JSON.stringify(findings) !== JSON.stringify(this.#sourceFindings.get(source.path)?.violations ?? [])
      )
        reported.set(source.path, findings);
    }
    const own = scopeFailure === undefined ? this.#selectViolations(stored.scope, edit, result.violations) : [];
    this.#finishAudit(
      stored.auditId,
      scopeFailure === undefined && result.status === "completed" ? "completed" : "failed",
      [...own, ...[...reported.values()].flat()],
      scopeFailure === undefined ? failures : [...failures, scopeFailure],
    );
    if (this.#versions.get(path) !== stored.revision || this.#replacement !== undefined) {
      unreported();
      return;
    }
    this.#evaluation = result;
    let readyViolations = this.#readyViolations.get(stored.revision);
    if (readyViolations === undefined) {
      readyViolations = new Map();
      this.#readyViolations.set(stored.revision, readyViolations);
    }
    // Each source the evaluation reported again owns its findings: a later edit of it in the same batch replaces them,
    // and its later revisions supersede them (`#versions`).
    for (const source of reported.keys())
      if ((this.#versions.get(source) ?? 0) < stored.revision) this.#versions.set(source, stored.revision);
    for (const [slot, violations] of [[path, own] as const, ...reported]) {
      readyViolations.set(slot, violations);
      // What the source reports now supersedes what an audit steered for it before.
      this.#auditFindings.delete(slot);
      if (slot === path && edit.deleted) this.#sourceFindings.delete(slot);
      else this.#sourceFindings.set(slot, { revision: stored.revision, mode: stored.mode, violations });
    }
    this.#readyModes.set(stored.revision, stored.mode);
    // A steer queued for a source this evaluation reported again speaks for its older facts. The runtime drops each
    // such steer whole, so the other sources it spoke for are steered again with the findings they last reported.
    if (reported.size > 0 && this.#sessionId !== undefined) {
      const revised = [...reported.keys()].map(source => this.#presentPath(source));
      await this.#steerAgain(this.#steerDelivery.revised?.(this.#sessionId, stored.revision, revised) ?? []);
    }
    if (this.#pending.size === 0) await this.#publishReady();
  }

  /**
   * Publishes again, at the revision each reported them, the findings of `sources` (paths as the session sees them):
   * what a touched-files audit steered for a source, else what the source last reported in its own slot.
   */
  async #steerAgain(sources: readonly string[]): Promise<void> {
    if (sources.length === 0) return;
    const wanted = new Set(sources);
    const byRevision = new Map<number, { mode: AuditMode; sources: string[]; violations: CodeRuleViolation[] }>();
    for (const source of new Set([...this.#auditFindings.keys(), ...this.#sourceFindings.keys()])) {
      if (!wanted.has(this.#presentPath(source))) continue;
      const findings = this.#auditFindings.get(source) ?? this.#sourceFindings.get(source)!;
      const group = byRevision.get(findings.revision) ?? { mode: findings.mode, sources: [], violations: [] };
      group.sources.push(source);
      group.violations.push(...findings.violations);
      byRevision.set(findings.revision, group);
    }
    for (const [revision, group] of byRevision)
      await this.#publish(revision, group.sources, group.violations, group.mode, true);
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
  /**
   * An edit's violations: in touched-files scope its file's, otherwise those in the lines it generated (none where its
   * file's earlier text was unreadable).
   */
  #selectViolations(
    scope: AuditScope,
    edit: CodeRuleEdit,
    violations: readonly CodeRuleViolation[],
  ): CodeRuleViolation[] {
    const pathViolations = violations.filter((violation) => violation.path === edit.path);
    if (scope === "touched-files" || edit.deleted || edit.generatedText === undefined) return pathViolations;
    if (edit.previousReadError !== undefined) return [];
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
    const startedAt = performance.now();
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
      const indexStartedAt = performance.now();
      const walked = indexed
        ? await this.#indexRepository(workspace, auditId, failures, nested)
        : await this.#walkAuditSources(workspace, scope, auditId, paths, failures, nested);
      const indexMilliseconds = performance.now() - indexStartedAt;
      if (this.#closing) return auditId;
      const summary = await this.#finishScopeAudit({
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
      if (scope === "repository" && summary !== undefined)
        this.#logRepositoryAudit(summary.status, startedAt, indexMilliseconds, summary.resultPagesMilliseconds, walked.files, summary.violations, summary.failures);
      return auditId;
    } catch (error) {
      const all = [...failures, failure("scan", "adapter-error", errorMessage(error), undefined, "scan")];
      if (scope === "repository") this.#logRepositoryAudit("failed", startedAt, 0, 0, 0, 0, all.length);
      const listed = auditFailureCap(scope)(all);
      this.#finishAudit(auditId, "failed", [], listed, all.length - listed.length);
      return auditId;
    } finally {
      if (workspace !== undefined) this.#disposeWorkspace(workspace);
    }
  }

  /** One line per repository audit in the daemon log: where its time went, so a slow audit can be told from a stuck one. */
  #logRepositoryAudit(
    status: AuditResult["status"],
    startedAt: number,
    indexMilliseconds: number,
    resultPagesMilliseconds: number,
    files: number,
    violations: number,
    failures: number,
  ): void {
    const totalMilliseconds = performance.now() - startedAt;
    this.#timingLog?.(
      `rerail: repository audit ${status} in ${Math.round(totalMilliseconds)} ms (index ${Math.round(indexMilliseconds)} ms, result pages ${Math.round(resultPagesMilliseconds)} ms): ${files} files, ${violations} violations, ${failures} failures\n`,
    );
  }

  /**
   * Index the Git checkout natively, one chunk per call. A chunk reads at most one batch (32 files,
   * reading stops at 1 MiB), publishes it and releases its sources and tapes before it returns, so
   * the audit's source residency is one batch: that bound replaces the former index worker's
   * peak-RSS limit. Interactive edits go first between chunks, and the walk's file, byte and
   * five-minute budgets stop the index as they stop the walk; failures to read a file never do.
   */
  async #indexRepository(
    workspace: NativeCodeRuleDslWorkspace,
    auditId: string | undefined,
    failures: AuditFailure[],
    nested: NestedCheckoutWalk,
  ): Promise<ScopeAuditSources> {
    const deadline = Date.now() + AUDIT_MAX_DURATION_MS;
    let files = 0;
    while (!this.#closing) {
      if (Date.now() >= deadline) {
        failures.push(auditBudgetFailure("Repository scan exceeded its five-minute work budget"));
        return { sourcePaths: [], stopped: true, files };
      }
      await this.#waitForEvaluationIdle();
      if (this.#closing) break;
      const chunk = repositoryIndexChunkSchema.parse(JSON.parse(await this.#indexChunk(workspace, nested)));
      this.#workspaceRevisions.set(workspace, chunk.revision);
      files = chunk.discoveredFiles;
      failures.push(...chunk.failures.map((item) => failure("source", item.code, item.message, item.path, "read")));
      const evaluation = chunk.evaluation;
      if (evaluation !== null && auditId !== undefined)
        this.#recordProgress(auditId, "repository", evaluation.violations, [...failures, ...evaluation.failures]);
      if (chunk.overBudget) {
        failures.push(auditBudgetFailure("Repository scan exceeded its 100,000-file or 512 MiB source budget"));
        return { sourcePaths: [], stopped: true, files };
      }

      if (chunk.done) {
        nested.reached.push(...chunk.nestedCheckouts);
        return { sourcePaths: [], stopped: false, files };
      }
      await yieldToHost();
    }
    return { sourcePaths: [], stopped: true, files };
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
    scope: AuditScope,
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
      facts: sourceFactsCache(),
    };
    for await (const file of this.#auditFiles(workspace, paths, failures, budget)) {
      if (this.#closing) return { sourcePaths, stopped: true, files: sourcePaths.length };
      files.push(file);
      sourcePaths.push(file.path);
      bytes += Buffer.byteLength(file.source);
      if (files.length >= AUDIT_BATCH_FILES || bytes >= AUDIT_CHUNK_BYTES) {
        await this.#flushAuditBatch(workspace, scope, auditId, failures, files);
        files = [];
        bytes = 0;
      }
    }
    if (files.length > 0) await this.#flushAuditBatch(workspace, scope, auditId, failures, files);
    return { sourcePaths, stopped: budget.stopped, files: sourcePaths.length };
  }

  async #flushAuditBatch(
    workspace: NativeCodeRuleDslWorkspace,
    scope: AuditScope,
    auditId: string | undefined,
    failures: readonly AuditFailure[],
    files: WorkspaceFile[],
  ): Promise<void> {
    await this.#waitForEvaluationIdle();
    if (this.#closing) return;
    const result = await this.#applyRequest(workspace, { files, removed: [], deferGlobal: true });
    if (auditId !== undefined) this.#recordProgress(auditId, scope, result.violations, [...failures, ...result.failures]);
    await yieldToHost();
  }

  /** Records a running audit's results so far, its failures capped as its finished list is (`auditFailureCap`). */
  #recordProgress(auditId: string, scope: AuditScope, violations: CodeRuleViolation[], failures: readonly AuditFailure[]): void {
    const listed = auditFailureCap(scope)(failures);
    this.#record(() =>
      this.#audits.progress(auditId, { violations, failures: listed, unlistedFailureCount: failures.length - listed.length }),
    );
  }

  async #finishScopeAudit(completion: ScopeAuditCompletion): Promise<ScopeAuditSummary | undefined> {
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
    const resultPagesStartedAt = performance.now();
    const streamed = await this.#resultPages(workspace, scope, auditId, mode === "log" && !generatedEdit, failures);
    const resultPagesMilliseconds = performance.now() - resultPagesStartedAt;
    // Gaps are listed but do not fail the audit; files read only in part are gaps of a repository audit alone.
    const result: Evaluation = {
      status: failsAudit(streamed.failures, scope) ? "failed" : "completed",
      violations: streamed.violations,
      failures: streamed.failures,
      metrics: { files: 0, visitedNodes: 0, witnesses: streamed.violations.length },
    };
    const unboundedFailures = [
      ...failures,
      ...result.failures.map((item) => ({ ...item, stage: "evaluate" as const })),
    ];
    const allFailures = auditFailureCap(scope)(unboundedFailures);
    // The paged list is capped by the same rule over the same failures, so it leaves out as many.
    const unlisted = unboundedFailures.length - allFailures.length;
    const status = failsAudit(unboundedFailures, scope) ? "failed" : "completed";
    if (streamed.paged && auditId !== undefined) {
      this.#record(() => this.#audits.finishResultPages?.(auditId, status, unlisted));
      this.#runningAudits.delete(auditId);
    } else {
      this.#finishAudit(auditId, status, result.violations, allFailures, unlisted);
    }
    if (this.#scope === "repository" && revision === this.#revision) {
      this.#reportableViolations = result.violations;
      this.#responseMode = mode;
      this.#evaluation = { ...result, failures: allFailures };
    }
    // A touched-files steer speaks for each of its sources: a revision of one drops it whole, so each source's findings
    // are recorded for `#steerAgain` to steer the others again. A source a later revision reached keeps what it reported.
    if (scope === "touched-files" && mode === "steer")
      for (const path of sourcePaths)
        if ((this.#versions.get(path) ?? 0) <= revision)
          this.#auditFindings.set(path, {
            revision,
            mode,
            violations: result.violations.filter((violation) => violation.path === path),
          });
    await this.#publish(revision, scope === "repository" ? [] : sourcePaths, result.violations, mode, generatedEdit);
    return { resultPagesMilliseconds, violations: streamed.violationCount, failures: unboundedFailures.length, status };
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
      const file = await this.#auditFileAtPath(workspace, canonicalRoot, absolute, stack, seen, failures, state);
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
    state: ScopeAuditState,
  ): Promise<WorkspaceFile | undefined> {
    const path = relative(canonicalRoot, absolute).split(sep).join("/");
    try {
      const info = await lstat(absolute);
      if (info.isSymbolicLink()) return undefined;
      if (!(await this.#isAuditPathContained(canonicalRoot, absolute, path, failures))) return undefined;
      if (info.isDirectory()) {
        await this.#enqueueAuditDirectory(workspace, canonicalRoot, absolute, stack, state.nested);
        return undefined;
      }
      return await this.#readAuditFile(workspace, absolute, path, info, seen, failures, state.facts);
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
    facts: SourceFactsCache,
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
      failures.push(failure("source", "source-incomplete", BINARY_SOURCE_MESSAGE, path, "read"));
      return undefined;
    }
    return {
      path,
      language,
      source: source.toString("utf8"),
      ...(await sourceFileFacts(this.#workspaceRoot, path, language, facts, workspace.readsServerGraph())),
    };
  }
  /**
   * Streams the results of an audit's private workspace. Nothing else applies to that workspace while
   * it pages; an apply would invalidate the native cursor, which the next page reports as stale.
   */
  async #resultPages(
    workspace: NativeCodeRuleDslWorkspace,
    scope: AuditScope,
    auditId: string | undefined,
    allowPersistentPaging: boolean,
    precedingFailures: readonly AuditFailure[],
  ): Promise<Pick<Evaluation, "violations" | "failures"> & { paged: boolean; pageCount: number; violationCount: number }> {
    const paged =
      allowPersistentPaging &&
      auditId !== undefined &&
      this.#audits.supportsResultPages === true &&
      this.#audits.appendResultPage !== undefined &&
      this.#audits.finishResultPages !== undefined;
    const violations: CodeRuleViolation[] = [];
    const failures: AuditFailure[] = [];
    // A paged audit keeps no violations in memory; the count is all the timing line needs.
    let violationCount = 0;
    // One cap across the preceding failures and every page (`auditFailureCap`).
    const cap = auditFailureCap(scope);
    const preceding = cap(precedingFailures);
    let cursor = 0;
    let pageIndex = 0;
    if (paged && precedingFailures.length > 0) {
      this.#record(() =>
        this.#audits.appendResultPage?.(auditId!, pageIndex, {
          violations: [],
          failures: preceding,
        }),
      );
      pageIndex += 1;
    }
    while (true) {
      const page = resultPageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
      violationCount += page.violations.length;
      const pageFailures = withBlindFiles(page.failures, page.blindFiles).map((item) => ({ ...item, stage: "evaluate" as const }));
      if (paged) {
        const listed = cap(pageFailures);
        this.#record(() =>
          this.#audits.appendResultPage?.(auditId!, pageIndex, {
            violations: page.violations,
            failures: listed,
          }),
        );
      } else {
        violations.push(...page.violations);
      }
      failures.push(...pageFailures);
      pageIndex += 1;
      if (page.done) return { violations: violations, failures, paged, pageCount: pageIndex, violationCount };
      if (page.cursor <= cursor) throw new Error("Native result stream did not advance");
      cursor = page.cursor;
      await yieldToHost();
      if (this.#closing) throw new Error("Policy execution interrupted by session closure");
    }
  }

  /**
   * Applies `edit` with the facts its file has on disk now. As the workspace keeps the `package` a host sent rather
   * than derive one from a `package.json` it holds, an edit of a `package.json` or a `tsconfig.json`-style project
   * configuration (created, changed or deleted) sends again each source applied below it whose family reads manifests
   * and whose `package` that changes, but one whose own edit is still queued; `repackaged` holds their applied edits.
   * Where a code rule reads Next server module graphs, an edit of a source in a package naming `next` does the same for
   * the package's sources, whose membership (`package.serverGraph`) the edit may change. A source's facts depend on its
   * path and disk only (with the edit's text standing in for its file's), so only a source whose facts changed is read
   * back. One facts cache serves the call, so each package's graph is walked once.
   *
   * `pass` is the facts cache a workspace replacement shares while it applies every applied source again: graphs read
   * on disk once for the whole pass, every source gets its facts from it so none needs sending again, and global
   * evaluation waits for the pass's end.
   */
  async #apply(
    workspace: NativeCodeRuleDslWorkspace,
    edit: CodeRuleEdit,
    pass?: SourceFactsCache,
  ): Promise<{ evaluation: Evaluation; repackaged: CodeRuleEdit[]; unreported: () => void }> {
    const packages = this.#workspacePackages.get(workspace);
    if (packages === undefined) throw new Error("Policy workspace was disposed");
    const serverGraph = workspace.readsServerGraph();
    const root = this.#workspaceRoot;
    const cache = pass ?? sourceFactsCache();
    // The graph reads the edit's own text, which disk may not hold yet.
    const overlay: SourceOverlay | undefined =
      pass === undefined ? { path: edit.path, source: edit.deleted ? undefined : edit.generatedText } : undefined;
    const files: WorkspaceFile[] =
      edit.deleted || edit.generatedText === undefined
        ? []
        : [{
            path: edit.path,
            language: edit.language,
            source: edit.generatedText,
            ...(await sourceFileFacts(root, edit.path, edit.language, cache, serverGraph, overlay)),
          }];
    const repackaged: CodeRuleEdit[] = [];
    // A project configuration may be another package's base (`extends`), so its edit rechecks every applied source.
    const manifest = manifestDirectory(edit.path);
    const directory =
      pass !== undefined
        ? undefined
        : manifest !== undefined
          ? this.#manifestEditFolder(manifest, serverGraph)
          : isModuleConfig(edit.path)
            ? ""
            : serverGraph
              ? this.#serverGraphFolder(edit)
              : undefined;
    if (directory !== undefined)
      for (const [path, sent] of packages) {
        const stored = this.#applied.get(path);
        // A source whose own edit is still queued applies with the facts then on disk; resending its older text now would
        // report that text's findings and supersede the edit.
        if (
          stored === undefined ||
          path === edit.path ||
          this.#pending.has(path) ||
          (directory !== "" && !path.startsWith(`${directory}/`))
        )
          continue;
        const facts = await sourceFileFacts(root, path, stored.language, cache, serverGraph, overlay);
        if (JSON.stringify(facts.package ?? null) === sent) continue;
        const applied = await this.#load(stored);
        if (applied.generatedText === undefined) continue;
        files.push({ path, language: applied.language, source: applied.generatedText, ...facts });
        repackaged.push(applied);
      }
    const evaluation = await this.#applyRequest(workspace, {
      files,
      removed: edit.deleted ? [edit.path] : [],
      ...(pass !== undefined ? { deferGlobal: true, quiet: true } : {}),
    });
    if (edit.deleted) packages.delete(edit.path);
    // What the workspace now holds: the facts each file was sent with.
    for (const file of files)
      if (loadNativeBinding().readsPackageManifests(file.language)) packages.set(file.path, JSON.stringify(file.package ?? null));
    if (pass === undefined) workspace.evictInactive();
    // A superseded evaluation's repackaged sources: no facts match the mark, so the next edit reaching them resends them.
    const unreported = () => {
      for (const { path } of repackaged) if (packages.has(path)) packages.set(path, UNREPORTED_FACTS);
    };
    return { evaluation, repackaged, unreported };
  }

  /**
   * The folder of the package naming `next` whose server module graph an edit of a source, or of a file in an `app/`
   * folder, can change; undefined for any other edit, or one whose folder resolves outside the workspace.
   */
  #serverGraphFolder(edit: CodeRuleEdit): string | undefined {
    const binding = loadNativeBinding();
    if (!binding.readsPackageManifests(edit.language) && !inAppFolder(edit.path)) return undefined;
    if (!folderWithinRoot(this.#workspaceRoot, edit.path)) return undefined;
    const facts = binding.nearestPackage(resolve(this.#workspaceRoot, edit.path), this.#workspaceRoot);
    return facts?.dependencies.includes("next") ? facts.folder : undefined;
  }

  /**
   * The folder whose applied sources an edit of the `package.json` in `folder` can change: that folder, or, where a code
   * rule reads Next server module graphs, the enclosing package naming `next`, whose graph a nested package's boundary
   * (created or removed) cuts, unless `folder` resolves outside the workspace.
   */
  #manifestEditFolder(folder: string, serverGraph: boolean): string {
    if (!serverGraph || folder === "" || !folderWithinRoot(this.#workspaceRoot, folder)) return folder;
    const enclosing = loadNativeBinding().nearestPackage(resolve(this.#workspaceRoot, folder), this.#workspaceRoot);
    return enclosing?.dependencies.includes("next") ? enclosing.folder : folder;
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
        const failures = withBlindFiles(parsed.evaluation.failures, parsed.blindFiles);
        // An apply is no repository audit: a file read in part fails it (`#finishScopeAudit` decides a repository audit's).
        const applied: Evaluation = { ...parsed.evaluation, status: failsAudit(failures, "touched-files") ? "failed" : "completed", failures };
        if (request.deferGlobal || parsed.replacedPaths == null) {
          if (!request.deferGlobal) this.#workspaceEvaluations.set(workspace, applied);
          return applied;
        }
        const replaced = new Set(parsed.replacedPaths);
        const previous = this.#workspaceEvaluations.get(workspace) ?? emptyEvaluation();
        const evaluation: Evaluation = {
          ...applied,
          violations: [
            ...previous.violations.filter((violation) => violation.path === undefined || !replaced.has(violation.path)),
            ...applied.violations,
          ],
          failures: [
            ...previous.failures.filter((failure) => failure.path === undefined || !replaced.has(failure.path)),
            ...applied.failures,
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
      else this.#runningAudits.add(auditId);
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
  #finishAudit(
    auditId: string | undefined,
    status: AuditResult["status"],
    violations: CodeRuleViolation[],
    failures: AuditFailure[],
    unlistedFailureCount = 0,
  ): void {
    if (auditId === undefined) return;
    this.#runningAudits.delete(auditId);
    if (!this.#closing) {
      const unlisted = unlistedFailureCount > 0 ? { unlistedFailureCount } : {};
      this.#record(() => this.#audits.finish(auditId, { status, violations: violations, failures, ...unlisted }));
    }
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
    const metadata = { language: edit.language, revision, scope, mode, references: 1, auditId: undefined };
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
