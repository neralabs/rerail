import { createRequire } from "node:module";
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { PACKAGE_ROOT } from "../package-root.js";
import type { MatchOperator, SampleBoundary, SampleSource } from "../catalog/rule-schema.js";
import type { EngineGitExit, EngineGitRun, EngineStartedGit } from "../git-start.js";
import type { CodeRuleSourceSpan } from "../code-rule/code-rule.js";

export interface NativeHealth {
  ok: boolean;
  kernelVersion: string;
}

export interface ModelFingerprint {
  modelId: string;
  revision: string;
  dimensions: number;
  weightsSha256: string;
  tokenizerSha256: string;
  kernelFormatVersion: number;
}

/** What `verifyModelFiles` found: `problem` and `file` are absent where every file matches `manifest.json`. */
export interface NativeModelCheck {
  problem?: "missing" | "unreadable" | "invalid-manifest" | "checksum-mismatch" | null;
  file?: string | null;
}

export interface NativeRuleInput {
  id: string;
  /** Similarity rules only: the text samples are compared against. */
  reference?: string | undefined;
  operator?: MatchOperator | undefined;
  threshold?: number | undefined;
  /** Structural rules only: the rule's `match` condition as JSON. */
  conditions?: string | undefined;
  source: SampleSource | "tool:*";
  boundary: SampleBoundary;
  minimumTokens: number;
}

export interface NativeSample {
  source: SampleSource;
  boundary: SampleBoundary;
  text: string;
  /** A tool call's arguments as JSON; absent for every other sample, tool results included. */
  input?: string | undefined;
}

/** Similarity facts are present only on a `similarity` match; a structural match computes none. */
export interface NativeMatch {
  ruleId: string;
  kind: "similarity" | "structural";
  similarity?: number;
  operator?: MatchOperator;
  threshold?: number;
  tokenCount?: number;
}

export interface CompileSummary {
  count: number;
  dimensions: number;
}
export interface NativeMemoryProfile {
  allocator: {
    allocatedBytes: number;
    activeBytes: number;
    residentBytes: number;
    mappedBytes: number;
    retainedBytes: number;
  };
  workerPool: { threads: number };
}

export interface NativeConversationEngine {
  fingerprint(): ModelFingerprint;
  embedTexts(texts: string[]): Float32Array[];
  tokenCounts(texts: string[]): number[];
  fitTexts(texts: string[], maximumTokens: number, overflow: "truncate-start" | "truncate-end" | "reject"): string[];
  replaceRules(rules: NativeRuleInput[]): CompileSummary;
  upsertRules(rules: NativeRuleInput[]): CompileSummary;
  removeRules(ids: string[]): number;
  matchText(sample: NativeSample): NativeMatch[];
}

export interface NativeConversationEngineConstructor {
  load(modelPath: string, maxInputTokens: number): NativeConversationEngine;
}
export type CodeRuleDslSourceFormat = "json" | "yaml";
export type CodeRuleDslEvaluationMode = "decision" | "report";

export interface NativeCompiledCodeRuleDsl {
  /** Diagnostic availability depends on the installed addon version. Uses the normal compiler. */
  candidateFrontierAudit?(): string;
  canonicalJson(): string;
  /** Whether a repository-relative path is inside the code rule's scope; the matcher evaluation uses. */
  inScope(path: string): boolean;
  digest(): string;
  requiredCapabilitiesJson(): string;
  evaluate(snapshotJson: string, mode: CodeRuleDslEvaluationMode): string;
  openEditor(snapshotJson: string): NativeCodeRuleDslEditor;
  /** Whether the code rule reads `file.package.serverGraph`, so hosts send `package.serverGraph`. */
  readsServerGraph(): boolean;
  /** Whether the code rule reads text: some clause, however nested, is a `regex` or `file` query, so files no language lists are read for it. */
  readsText(): boolean;
  /**
   * Whether a finding of the code rule in one file can depend on another file's text: a clause the engine evaluates over
   * many files (any but a `regex`, `ast` or `file` forbid, and an `llm` clause, which reads no file), a `calls` or
   * `importTarget` term, or a `report_when` gate.
   * The engine routes each clause by the same predicates, so the answer cannot drift from what it evaluates.
   */
  readsOtherFiles(): boolean;
}

export interface NativeCodeRuleDslEditor {
  evaluate(mode: CodeRuleDslEvaluationMode): string;
  applyEdit(editJson: string, mode: CodeRuleDslEvaluationMode): string;
}

/**
 * What a storage attach did. No store attaches where Git cannot describe the workspace; `reason` then says why, unless
 * the root is simply outside any Git repository.
 */
export interface NativeStorageAttachment {
  attached: boolean;
  reason?: string;
  /** Wall time the attach spent discovering the Git checkout (0 if it did not get that far). */
  discoveryMilliseconds: number;
  /** Wall time the attach spent opening the store and publishing the checkout's snapshot (0 if that phase did not run). */
  publicationMilliseconds: number;
}

/**
 * The names `enableStorageAsync` keeps out of the content store (`SENSITIVE_FILE_NAMES` and `SENSITIVE_FOLDER_NAMES`
 * in path-boundary.ts): `files` match a path's last segment and `folders` any segment on the way, as `sensitiveByName`
 * reads them.
 */
export interface NativeSecretNames {
  files: readonly string[];
  folders: readonly string[];
}

/**
 * The bytes of violations a host reads a whole result in per `resultPageAsync` page. Small, since a page is held three
 * times while it is read (the JSON, its parse, its validated copy) and V8 keeps what it grew to: 256 KiB pages read
 * django's 15.5k violations (6.8 MB) with 40 MiB less peak RSS than one 8 MiB page.
 */
export const RESULT_PAGE_BYTES = 256 * 1024;

export interface NativeCodeRuleDslWorkspace {
  /**
   * Attaches the content store at `storePath` (see `contentStorePath`) to the repository containing `root`; the
   * binding never chooses a store itself and refuses an empty path. A file `secretNames` names is evaluated but never
   * written to the store (not its source, syntax trees or results): it stays in the workspace's memory instead.
   */
  enableStorageAsync(root: string, storePath: string, secretNames: NativeSecretNames): Promise<NativeStorageAttachment>;
  /**
   * `placement`, when given, is a JSON `CodeRulePlacement` (repository-model.ts): this workspace runs the code rule per folder
   * at the entry's mode (`null` is off there), and a linter's suppression comment silences its violations there unless the
   * entry's `suppressions` is `"ignore"` (omitted is `"honour"`); omitted, the code rule runs wherever its scope matches at
   * its own mode and honours suppression comments.
   */
  addCodeRule(compiled: NativeCompiledCodeRuleDsl, placement?: string): void;
  acceptsPath(path: string): boolean;
  /** Whether `path` is in the scope of a code rule that reads text (as `acceptsPath`, over those rules only): an unlisted file is read only then. */
  acceptsTextPath(path: string): boolean;
  /** Whether a code rule reads `file.package.serverGraph`, so hosts send `package.serverGraph`. */
  readsServerGraph(): boolean;
  /**
   * Whether a code rule has an AST clause in a family whose files read their nearest `package.json` (ECMA), so
   * `package.json` and `tsconfig`/`jsconfig` edits change what it evaluates.
   */
  readsPackageManifests(): boolean;
  tryApply(requestJson: string): string | null;
  applyAsync(requestJson: string): Promise<string>;
  applyBufferAsync(request: Buffer): Promise<string>;
  /** One bounded chunk of a repository index over the attached Git worktree; call until `done`. */
  indexRepositoryAsync(options: NativeRepositoryIndexOptions): Promise<string>;
  /** Records the current revision as reusable in the content store, off the JS thread. */
  completeStorageRevisionAsync(): Promise<void>;
  memoryProfile(): string;
  /** One page of ordered results, computed off the JS thread; the last page releases the result index. */
  resultPageAsync(cursor: number, maximumBytes: number): Promise<string>;
  evictInactive(): boolean;
  /** A snapshot-qualified reference to the deepest node spanning exactly the range, or null. */
  nodeReference(path: string, startByte: number, endByte: number): string | null;
  /** The referenced node as JSON; throws when the reference's file version is no longer current. */
  resolveNodeReference(reference: string): string;
  setDocumentResidencyLimit(bytes: number): void;
  /**
   * The absolute folder on disk the workspace's paths are relative to: `importTarget` resolution reads the files below
   * it the workspace does not hold (installed packages, files outside every scope), and nothing above it. Without one,
   * targets resolve among the files the workspace holds.
   */
  setRoot(root: string): void;
  close(): void;
}

/** The bounds of one repository index; every chunk call carries the same. */
export interface NativeRepositoryIndexOptions {
  maxFiles: number;
  maxSourceBytes: number;
  maxFileBytes: number;
  ignoredDirectories: string[];
  /** The file label of each evaluated extension, keyed lowercase with its dot (`.py` → `python`). */
  sourceLanguages: Record<string, string>;
  /** The file label of each file evaluated by its whole name, keyed lowercase (`.npmignore` → `text`); wins over `sourceLanguages`. */
  sourceNames?: Record<string, string>;
  /** Folders below the workspace root that are checkouts judged by their own rules: not walked, only reported. */
  nestedCheckouts: string[];
  /** Leave out what Git ignores (nested `.gitignore`s, `.git/info/exclude`, `core.excludesFile`); never a tracked file. */
  followGitignore: boolean;
}

/**
 * What Rerail's Git reports about the config files it reads outside any repository (`git-access.ts` asks it): `git var
 * GIT_CONFIG_SYSTEM`, absent on a Git before 2.42 or where it failed; and `git config --list --show-origin --show-scope
 * -z --name-only --no-includes` outside any repository, absent where it failed.
 */
export interface NativeGitInstallation {
  systemConfig?: string;
  configOrigins?: string;
}

/**
 * The process's one owner of the engine's in-process Git reads. Made from what Rerail's Git reported, it names the
 * config files that Git reads to every read in the process, so the engine starts no Git of its own to find them.
 */
export interface NativeGitAccess {
  /** The system config file the engine's reads take, as Rerail's Git named it; null where it named none. */
  readonly systemConfig: string | null;
  /** A config file the reads take besides the system one (Apple Git's); null where there is none. */
  readonly installationConfig: string | null;
  /**
   * Git's exclude stack for the worktree around `root`, re-reading the index as it changes; null outside any Git
   * worktree. Throws where Git refuses the repository (another user's, outside `safe.directory`; a config file Git
   * cannot parse), as it does where the rules cannot be read.
   */
  openGitIgnoreMatcher(root: string): NativeGitIgnoreMatcher | null;
}

/** Git's exclude stack for paths below the root it was opened for; see `NativeGitAccess.openGitIgnoreMatcher`. */
export interface NativeGitIgnoreMatcher {
  /**
   * Whether Git ignores `relativePath` (to the matcher's root, `/`-separated): `git check-ignore`'s answer, so a tracked
   * path, or a folder holding one, never is. `isDirectory` lets directory-only patterns (`build/`) match. Paths outside
   * the worktree are never ignored. Throws when an ignore file on the way cannot be read.
   */
  ignored(relativePath: string, isDirectory: boolean): boolean;
  /**
   * The pattern that makes Git ignore `relativePath` (as for `ignored`), as `git check-ignore -v` names it: the file it
   * was read from (absolute; absent when it came from no file), its line there, and the pattern. A path under an
   * ignored folder names the folder's pattern. Null when Git does not ignore it; throws as `ignored` does.
   */
  ignoredBy(relativePath: string, isDirectory: boolean): NativeGitIgnoreMatch | null;
  /**
   * Whether a matcher opened now at this root would read the rules this one read: the configuration, read again as Git
   * reads it (the system, global and repository files, their includes and conditions), chooses the same excludes file
   * and pattern case, and the root `.gitignore`, `info/exclude` and the excludes file are as this matcher read them.
   * When it would, each folder's `.gitignore` is read again as the next path enters the folder.
   */
  sourcesUnchanged(): boolean;
}

/** A pattern of Git's exclude stack that ignores a path (`NativeGitIgnoreMatcher.ignoredBy`). */
export interface NativeGitIgnoreMatch {
  source?: string | null;
  line: number;
  pattern: string;
}

/**
 * An ECMA file's `package`: what the engine reads of its nearest `package.json`. `folder` is the manifest's directory
 * below the workspace root (`""` at the root); `type` is `unset` for a manifest without a valid one; `bin` holds the
 * `bin` targets relative to `folder`; `nodeVersion` the configured Node range (`engines.node`, else
 * `devEngines.runtime`'s `node` entry), a valid npm range (a `package` with any other is malformed); `dependencies`
 * every dependency name of any kind; `jsxImportSource` the `compilerOptions.jsxImportSource` of the `tsconfig.json`
 * (else `jsconfig.json`) beside it; `private` is `true` where the manifest's `private` is. `serverGraph`, which a host
 * sends only where a code rule reads it and the manifest names `next`, is whether the file is in its Next app's server
 * module graph (`serverGraphAsync` below). `published`, the file's own fact (`packageFacts` sends it, `nearestPackage`
 * does not), is whether npm publishes the file, as eslint-plugin-n reads the manifest's `files` and `main` and its
 * folder's `.npmignore`, else (without `files`) its `.gitignore`.
 */
export interface NativePackageFacts {
  folder: string;
  type: "module" | "commonjs" | "unset";
  bin: string[];
  nodeVersion?: string;
  dependencies: string[];
  jsxImportSource?: string;
  private?: boolean;
  serverGraph?: boolean;
  published?: boolean;
}

/** One walk's `packageFacts` for the files below the root it was opened for, each manifest and ignore file read once. */
export interface NativePackageFactsReader {
  facts(label: string, absolute: string): NativePackageFacts | null;
}

/**
 * A workspace request. A file without a `package` that the engine reads manifests for gets the nearest `package.json`
 * among the workspace's files (a `{ path: ".../package.json", language: "json", source }` entry, sent in any request).
 */
export function encodeCodeRuleWorkspaceRequest(request: {
  revision: number;
  files: ReadonlyArray<{ path: string; language: string; source: string; module?: string | string[]; package?: NativePackageFacts }>;
  removed: readonly string[];
  deferGlobal?: boolean;
  quiet?: boolean;
}): Buffer {
  const sources = request.files.map((file) => Buffer.from(file.source, "utf8"));
  const metadata = Buffer.from(
    JSON.stringify({
      revision: request.revision,
      files: request.files.map((file, index) => ({
        path: file.path,
        language: file.language,
        sourceBytes: sources[index]!.byteLength,
        ...(file.module === undefined ? {} : { module: file.module }),
        ...(file.package === undefined ? {} : { package: file.package }),
      })),
      removed: request.removed,
      deferGlobal: request.deferGlobal ?? false,
      quiet: request.quiet ?? false,
    }),
    "utf8",
  );
  const header = Buffer.allocUnsafe(12);
  header.write("RERAILB1", 0, "ascii");
  header.writeUInt32LE(metadata.byteLength, 8);
  const length = 12 + metadata.byteLength + sources.reduce((sum, source) => sum + source.byteLength, 0);
  return Buffer.concat([header, metadata, ...sources], length);
}

export type NativeCodeRuleDslWorkspaceConstructor = new () => NativeCodeRuleDslWorkspace;

export interface NativeCodeRuleDslRuntimeConstructor {
  /** Canonical CPython facts for a whole document demand batch; source is never executed. */
  pythonAstBatch(documentsJson: string): string;
  compile(source: string, format: CodeRuleDslSourceFormat, capabilitiesJson: string): NativeCompiledCodeRuleDsl;
  /**
   * `compile` on a thread of Node's worker pool, so the JS thread runs other work while a rule compiles. An engine built
   * before it lacks it.
   */
  compileAsync?(source: string, format: CodeRuleDslSourceFormat, capabilitiesJson: string): Promise<NativeCompiledCodeRuleDsl>;
  /**
   * The tests the `.rerail` files under `pathsJson` (a JSON array of files and directories) carry, each run: a JSON array of
   * `NativeRrTestResult` (`rerail-engine test-rr`).
   */
  testRr(pathsJson: string, filter?: string | null): string;
  /**
   * Each `.rerail` file of `pathsJson` (a JSON array of paths) read with its includes and checked, nothing run: a JSON
   * array of `RrDescription`. A file that does not read or check is a row with its error; it never throws for one.
   */
  describeRr(pathsJson: string): string;
  /** The policies of `requestJson` (a JSON `RrRunRequest`) run over its root, off the JavaScript thread: a JSON `RrRunResult`. */
  runRr(requestJson: string): Promise<string>;
}

/** What a `.rerail` rule file states of itself, as the engine reads it (`describeRr`). */
export type RrDescription =
  | {
      path: string;
      ok: true;
      id: string;
      revision: number;
      description: string | null;
      mode: "block" | "steer" | "log";
      language: string | null;
      /** The files it reads, as its run selects them: `text` names those read as text whatever their language. */
      scope: { include: string[]; exclude: string[]; text: string[]; toolCalls: boolean };
      /** The codes its findings can have, sorted. */
      codes: string[];
      /** How many `.test` blocks it holds. */
      tests: number;
      /** Whether a `.violation` carries a `suppress` line. */
      suppresses: boolean;
      /** Whether a `.violation` carries a `fix` line. */
      fixes: boolean;
      /** The rule file, then each file it includes: canonical path and the SHA-256 (hex) of its bytes. */
      files: { path: string; sha256: string }[];
    }
  | { path: string; ok: false; error: string };

/** A `runRr` request: the policies (absolute paths) over the corpus at `root`, written into the empty directory `out`. */
export interface RrRunRequest {
  root: string;
  policies: string[];
  out: string;
  /** Root-relative files the run reads alone; omitted: the whole corpus. */
  inputFiles?: string[];
}

/** A finding of a `.rerail` run. `span` has `CodeRuleViolation.span`'s bases; null only for a finding in no file. */
export interface RrFinding {
  policy: string;
  clause: string;
  code: string;
  message: string;
  path: string;
  line: number;
  span: CodeRuleSourceSpan | null;
  tag: string | null;
  parameters: Record<string, string>;
}

/** A `runRr` result. A failure's `policy` is the path of the policy whose run failed; the others ran without it. */
export interface RrRunResult {
  findings: RrFinding[];
  failures: { policy: string | null; message: string }[];
  /** The files the largest of the run's runs read. */
  files: number;
}

/** One test of a `.rerail` file, as `testRr` reports it. */
export interface NativeRrTestResult {
  file: string;
  test: string;
  line: number;
  status: "pass" | "fail";
  /** Each failed `.expect`, with its line and what the run had instead. */
  failures: string[];
  /** The run's findings, as keys: `code at "path":line`. */
  findings: string[];
}

/** A policy document for the first slice: its name (the file stem), source and format. */
export interface NativeSlicePolicy {
  name: string;
  source: string;
  format: CodeRuleDslSourceFormat;
}

/** An editor's unsaved source for `path` (relative to the directory, `/` separators); the file may not exist yet. */
export interface NativeSliceBuffer {
  path: string;
  source: string;
}

/**
 * The first slice over one content store (docs/final-shape/06-first-slice.md): the pack's lowered Python `ast`
 * clauses, kept between runs. `audit` and `apply` return JSON: `findings` ({rule, code, path, line}), `coverage`
 * ({rule, path, coverage, reason?}), `evaluated` paths, `rerun` derivations and the `changed` families.
 */
export interface NativeCodeRuleSlice {
  clauses(): number;
  /**
   * A cold audit of `scope` at `directory` as `observer` sees it: `"directory"` (default, the working tree's
   * snapshot, with `buffers` over it) or `"head"` (the `HEAD` tree of its Git checkout; no buffers).
   */
  audit(scope: string, directory: string, observer?: "directory" | "head" | null, buffers?: NativeSliceBuffer[] | null): string;
  /** `scope` at `directory` as `observer` sees it, evaluating only the documents that changed since the last run. */
  apply(scope: string, directory: string, observer?: "directory" | "head" | null, buffers?: NativeSliceBuffer[] | null): string;
  /** The lowered rules as `rir` (list step 18): programs and label tables, with `fused` the packs and groups, with `setplan` the set plan. */
  dump(fused?: boolean | null, setplan?: boolean | null): string;
}

export interface NativeCodeRuleSliceConstructor {
  /** Opens the store at `storePath` and lowers `policies` for `backend` (`"cpu"` or `"metal"`). */
  new (storePath: string, policies: NativeSlicePolicy[], capabilitiesJson: string, backend: "cpu" | "metal"): NativeCodeRuleSlice;
}

/** An OS advisory lock (`flock` / `LockFileEx`) the OS drops when the holding process exits, however it exits. */
export interface NativeExclusiveFileLock {
  /** Unlocks and closes the file before process exit; later calls do nothing. */
  release(): void;
}

/** A connection-local SQL extension context over the native content store. */
export interface NativeCodeRuleSqlExecutor {
  bindSnapshot(snapshotJson: string): void;
  /** Retain SQL results across source edits; unchanged files reuse native fact shards. */
  applySnapshot(snapshotJson: string, maxWaves?: number | null): string;
  /** Atomic {upsert: snapshot files, delete: paths}; leaves unmentioned files and metadata in place. */
  applyFileChanges(changesJson: string, maxWaves?: number | null): string;
  bindStored(scope: string, paths?: string[] | null): void;
  bindRelation(relationJson: string): void;
  /** Drop generated outputs while keeping fact bindings and registered extensions. */
  reset(): void;
  /** Per-operation limits: {max_steps?: number, timeout_ms?: number}. An empty object removes limits. */
  setLimits(limitsJson: string): void;
  createIndex(relation: string, columns: number[]): void;
  /** Register deterministic aggregate/merge/widen SQL functions for a custom lattice. */
  registerLattice(name: string, specificationJson: string): void;
  lower(program: string, dialect: string): string;
  run(program: string, dialect: string, maxWaves?: number | null): string;
  runFile(path: string, dialect: string, maxWaves?: number | null): string;
  /** Atomic [{name, insert?: rows, delete?: rows}] batch; returns deltas and refreshed strata. */
  applyChanges(changesJson: string, maxWaves?: number | null): string;
  query(sql: string, parametersJson?: string | null, maxRows?: number | null): string;
  /** Read-only SQL graph: CTE fusion, shared statement limits, and parallel SQL groups. */
  queryPlan(planJson: string): string;
}

export interface NativeCodeRuleSqlExecutorConstructor {
  new(storePath: string): NativeCodeRuleSqlExecutor;
}

export interface NativeCodeRuleDslExecution {
  evaluate(compiled: NativeCompiledCodeRuleDsl, snapshot: string, mode: string): string;
  createWorkspace(): NativeCodeRuleDslWorkspace;
  close(): void;
}

/** What changed under one watched root since the last `take` (`NativeTreeWatcher`). */
export interface NativeTreeNotice {
  root: string;
  /** Changed entries (content, size, attributes), relative `/`-paths. Capped (see `lost`). */
  changed: string[];
  /** Created, deleted or renamed entries, relative `/`-paths. Capped (see `lost`). */
  renamed: string[];
  /**
   * Notices were dropped (an OS overflow, more than 16,384 entries between two takes, or a Windows handle let go for a
   * rename above the root and opened again): read the whole root.
   */
  lost: boolean;
  /**
   * The watch stopped (the root or a folder above it renamed, moved or deleted, a watch error).
   * The root is no longer watched: read it whole and watch it again.
   */
  ended: boolean;
}

/**
 * The engine's file-system event bus: one OS watch per root (FSEvents, inotify per folder, ReadDirectoryChangesW with a
 * read-handle oplock), whose notices wait in the engine until `take`. Nothing calls into JavaScript.
 */
export interface NativeTreeWatcher {
  /**
   * Starts watching `root` (an absolute physical path). `folders`: the folders the last read walked into, relative
   * `/`-paths with `""` for the root; only the Linux adapter uses them. False when this root cannot be watched here (a
   * network or remote file system, a WSL path, past Linux's watch budget, Windows with no oplock, an OS with no
   * adapter). Watching a root already watched replaces its folder set.
   */
  watch(root: string, folders: readonly string[]): boolean;
  /**
   * Watches `root` as `watch` does, but only with room the Linux watch budget has left: a root `watch` asks for that
   * needs the room takes it, and this root's watch ends. Elsewhere, as `watch`. An engine built before it lacks it.
   */
  watchSpare?(root: string, folders: readonly string[]): boolean;
  /** Stops watching `root` and releases every handle on it, so `git worktree remove` and renames work at once. */
  unwatch(root: string): void;
  /** Takes every notice since the last take, at most one per root. Never blocks on the OS. */
  take(): NativeTreeNotice[];
  /** Stops the adapter's thread and releases every handle. Idempotent; a later `watch` is false. */
  close(): void;
}

/** The process holding the server end of a Windows named pipe ({@link NativeBinding.namedPipeServer}). */
export interface NativeNamedPipeServer {
  /** Absent when other clients kept every instance of the pipe taken until the wait ran out. */
  pid?: number | null;
  /** When it started, epoch ms; absent where this user may not ask. */
  startedAt?: number | null;
}

export interface NativeBinding {
  CodeRuleDslExecution: new () => NativeCodeRuleDslExecution;
  CodeRuleSqlExecutor: NativeCodeRuleSqlExecutorConstructor;
  executeSqlRequest(storePath: string, requestJson: string): string;
  /** The message of a workspace task's failure while another apply holds the engine's one slot: pause and call again. */
  readonly WORKSPACE_BUSY: string;
  health(): NativeHealth;
  /** Checks the model directory against its `manifest.json` (each file's SHA-256) without loading the model. */
  verifyModelFiles(modelPath: string): NativeModelCheck;
  nativeMemoryProfile(): string;
  processSessionId(pid: number): number | null;
  /** The module a file of dialect `label` carries as `module`, from the package markers on disk; null where its family names no modules or the file is outside a package. */
  modulePath(label: string, absolute: string): string | string[] | null;
  /** The `package` a file of dialect `label` carries: its nearest `package.json`, never above `root`; null without one, or where its family reads none. */
  packageFacts(label: string, absolute: string, root: string): NativePackageFacts | null;
  /**
   * The nearest `package.json` at or above `absolute` (a file, or a folder read as one: the package around it), never
   * above `root`, whatever family reads it; null without one. A host asks which package an edit of any file is in.
   */
  nearestPackage(absolute: string, root: string): NativePackageFacts | null;
  /** Whether files of dialect `label` carry a `package` (its family reads `package.json` files). */
  readsPackageManifests(label: string): boolean;
  /**
   * The files of the package in `folder` (relative to `root`) on disk that are in its Next App Router server module
   * graph, relative to `folder`: reachable from an App Router server file convention by imports that pass no
   * `"use client"` module. With `overlayPath`, that file reads as `overlaySource` (deleted without one). Walked off
   * the JS thread; null where the package has no graph (a source or folder cannot be read, or the package is past the
   * walk's file or byte bounds): a host then sends its files no `serverGraph`.
   */
  serverGraphAsync(root: string, folder: string, overlayPath?: string | null, overlaySource?: string | null): Promise<string[] | null>;
  /**
   * The project configuration files the package in `folder` (relative to `root`) reads on disk: its `tsconfig.json`
   * (else `jsconfig.json`), then the `tsconfig*.json`/`jsconfig*.json` bases it extends, relative to `root`.
   */
  moduleConfigFiles(root: string, folder: string): string[];
  /** Throws unless `pattern` compiles in the regex dialect and size limit structural rules use. */
  validateRegex(pattern: string): void;
  /** Windows only: start a detached process that inherits only NUL stdin and the appended log file; returns its pid. */
  spawnDetached(program: string, args: string[], cwd: string | null, env: Record<string, string>, logPath: string): number;
  /**
   * Windows only: the exit code of a process `spawnDetached` started from this process, or null while it runs, read from
   * the handle it kept, so a pid Windows has given to another process is no answer. A binary built before it lacks it.
   */
  detachedExitCode?(pid: number): number | null;
  /**
   * Linux only: starts Git through `posix_spawn`, which copies nothing of this process (`git-start.ts`). A binary built
   * before it lacks it.
   */
  startGit?(program: string, args: string[], env: string[], pipeStderr: boolean): EngineStartedGit;
  /** Linux only: reaps a child `startGit` started once its exit pipe has closed. A binary built before it lacks it. */
  reapGit?(pid: number): EngineGitExit;
  /**
   * Linux only: runs Git as `spawnSync` does, through `posix_spawn` (`git-start.ts`). A binary built before it lacks it.
   */
  runGit?(program: string, args: string[], env: string[], timeoutMs: number | undefined, maxBuffer: number): EngineGitRun;
  /**
   * Windows only: whether every path's owner is one Git for Windows takes as this process's user (the user, or the
   * Administrators group while the process's token is a member); false when an owner cannot be read, and on every
   * other platform. A binary built before it lacks it.
   */
  pathsOwnedByCurrentUser?(paths: string[]): boolean;
  /**
   * Windows only: the process holding the server end of the named pipe `path` and when it started, opening again while
   * other clients hold every instance until `timeoutMs` has passed, on a thread of the worker pool; null where no pipe
   * of that name takes a connection, and on every other system. A binary built before it lacks it.
   */
  namedPipeServer?(path: string, timeoutMs: number): Promise<NativeNamedPipeServer | null>;
  /** The content-store file name this binary's store schema owns (`content-v5.sqlite` at store floor 5). */
  contentStoreFileName(): string;
  /** Locks `path` exclusively, creating the file; null while any other handle, in any process, holds it. */
  tryLockExclusive(path: string): NativeExclusiveFileLock | null;
  /** The owner of the engine's in-process Git reads; make it through `nativeGitAccess` (`git-access.ts`), once per process. */
  GitAccess: new (installation: NativeGitInstallation) => NativeGitAccess;
  ConversationEngine: NativeConversationEngineConstructor;
  PackageFactsReader: new (root: string) => NativePackageFactsReader;
  CodeRuleDslRuntime: NativeCodeRuleDslRuntimeConstructor;
  CodeRuleDslWorkspace: NativeCodeRuleDslWorkspaceConstructor;
  CodeRuleSlice: NativeCodeRuleSliceConstructor;
  /**
   * `policies` lowered and dumped as `rir` without a store (`rerail engine dump`): every clause, or only `clause`
   * (`policy/clause` or a clause id); `fused` adds the packs and groups, `setplan` the set plan.
   */
  dumpRir(policies: NativeSlicePolicy[], capabilitiesJson: string, clause?: string | null, fused?: boolean | null, setplan?: boolean | null): string;
  /** Every `program` block of `text` assembled through the verifier and dumped again (`rerail engine asm`); throws the first refusal. */
  assembleRir(text: string): string;
  /** The file-system event bus; an engine built before it lacks it. */
  TreeWatcher?: new () => NativeTreeWatcher;
}

const require = createRequire(import.meta.url);

export function nativePlatformSuffix(
  platform: NodeJS.Platform = process.platform,
  architecture: string = process.arch,
): string {
  if (platform === "darwin") return `darwin-${architecture}`;
  if (platform === "linux") return `linux-${architecture}-gnu`;
  if (platform === "win32") return `win32-${architecture}-msvc`;
  return `${platform}-${architecture}`;
}

export function resolveNativeBindingPath(configuredPath = process.env.RERAIL_ENGINE_PATH): string {
  if (configuredPath !== undefined) {
    const addonPath = resolve(configuredPath);
    if (!existsSync(addonPath)) throw new Error(`RERAIL_ENGINE_PATH does not exist: ${addonPath}`);
    return addonPath;
  }
  const fileName = `rerail_engine.${nativePlatformSuffix()}.node`;
  const candidates = [join(PACKAGE_ROOT, "native", fileName), join(PACKAGE_ROOT, "native", "rerail_engine.node")];
  const addonPath = candidates.find(existsSync);
  if (addonPath === undefined) {
    throw new Error(`Unsupported platform or missing native addon: ${fileName}`);
  }
  return addonPath;
}

/**
 * The engine's busy refusal, kept when the addon first loads. `isWorkspaceBusy` runs in a retry's `catch`, after the
 * failed call loaded the addon, so it reads this instead of resolving the addon there again, where a moved addon would
 * throw over the error it classifies.
 */
let workspaceBusy: unknown;

export function loadNativeBinding(): NativeBinding {
  const binding = require(resolveNativeBindingPath()) as NativeBinding;
  workspaceBusy ??= binding.WORKSPACE_BUSY;
  return binding;
}

/** Whether `error` is a workspace task's refusal because another apply holds the engine's one slot. */
export function isWorkspaceBusy(error: unknown): boolean {
  if (!(error instanceof Error)) return false;
  const busy = workspaceBusy ?? loadNativeBinding().WORKSPACE_BUSY;
  if (typeof busy !== "string") {
    throw new Error("The native addon exports no WORKSPACE_BUSY, so it is older than this Rerail; rebuild or reinstall it", { cause: error });
  }
  return error.message === busy;
}

export function nativeHealth(): NativeHealth {
  return loadNativeBinding().health();
}

/**
 * On Windows, whether Git, started by this process, takes every one of `paths` as its user's own: the engine reads each
 * owner as Git for Windows does. Undefined on every other platform, and when no engine here can tell (none loads, one
 * built before it, or a read that throws), so the caller asks Git; the first such failure is reported
 * ({@link reportOwnerReadFailure}).
 */
export function pathsOwnedByCurrentUser(paths: readonly string[]): boolean | undefined {
  if (process.platform !== "win32") return undefined;
  try {
    const binding = loadNativeBinding();
    if (binding.pathsOwnedByCurrentUser === undefined) throw new Error("the engine was built before it could read a path's owner");
    return binding.pathsOwnedByCurrentUser([...paths]);
  } catch (error) {
    reportOwnerReadFailure(error);
    return undefined;
  }
}

/** Whether a failure of the engine's owner read was reported in this process. */
let ownerReadFailureReported = false;

/**
 * Writes the first failure of the engine's owner read to stderr, which a daemon keeps in its log, once per process:
 * without that read every worktree a repository start places is asked of Git, one process each.
 */
function reportOwnerReadFailure(error: unknown): void {
  if (ownerReadFailureReported) return;
  ownerReadFailureReported = true;
  const reason = error instanceof Error ? error.message : String(error);
  process.stderr.write(`rerail: the engine could not read path owners, so Git places each worktree a start asks after: ${reason}\n`);
}
