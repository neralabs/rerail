import { createRequire } from "node:module";
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { PACKAGE_ROOT } from "../package-root.js";
import type { MatchOperator, SampleBoundary, SampleSource } from "../catalog/rule-schema.js";

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
 * The bytes of violations a host reads a whole result in per `resultPageAsync` page. Small, since a page is held three
 * times while it is read (the JSON, its parse, its validated copy) and V8 keeps what it grew to: 256 KiB pages read
 * django's 15.5k violations (6.8 MB) with 40 MiB less peak RSS than one 8 MiB page.
 */
export const RESULT_PAGE_BYTES = 256 * 1024;

export interface NativeCodeRuleDslWorkspace {
  /**
   * Attaches the content store at `storePath` (see `contentStorePath`) to the repository containing `root`; the
   * binding never chooses a store itself and refuses an empty path.
   */
  enableStorageAsync(root: string, storePath: string): Promise<NativeStorageAttachment>;
  /**
   * `placement`, when given, is a JSON `CodeRulePlacement` (repository-model.ts): this workspace runs the code rule per folder
   * at the entry's mode (`null` is off there), and a linter's suppression comment silences its violations there unless the
   * entry's `suppressions` is `"ignore"` (omitted is `"honour"`); omitted, the code rule runs wherever its scope matches at
   * its own mode and honours suppression comments.
   */
  addCodeRule(compiled: NativeCompiledCodeRuleDsl, placement?: string): void;
  acceptsPath(path: string): boolean;
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
  /** Folders below the workspace root that are checkouts judged by their own rules: not walked, only reported. */
  nestedCheckouts: string[];
}

/**
 * An ECMA file's `package`: what the engine reads of its nearest `package.json`. `folder` is the manifest's directory
 * below the workspace root (`""` at the root); `type` is `unset` for a manifest without a valid one; `bin` holds the
 * `bin` targets relative to `folder`; `nodeVersion` the configured Node range (`engines.node`, else
 * `devEngines.runtime`'s `node` entry), a valid npm range (a `package` with any other is malformed); `dependencies`
 * every dependency name of any kind; `jsxImportSource` the `compilerOptions.jsxImportSource` of the `tsconfig.json`
 * (else `jsconfig.json`) beside it. `serverGraph`, which a host sends only where a code rule reads it and the manifest
 * names `next`, is whether the file is in its Next app's server module graph (`serverGraphAsync` below).
 */
export interface NativePackageFacts {
  folder: string;
  type: "module" | "commonjs" | "unset";
  bin: string[];
  nodeVersion?: string;
  dependencies: string[];
  jsxImportSource?: string;
  serverGraph?: boolean;
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
  compile(source: string, format: CodeRuleDslSourceFormat, capabilitiesJson: string): NativeCompiledCodeRuleDsl;
}

/** An OS advisory lock (`flock` / `LockFileEx`) the OS drops when the holding process exits, however it exits. */
export interface NativeExclusiveFileLock {
  /** Unlocks and closes the file before process exit; later calls do nothing. */
  release(): void;
}

export interface NativeBinding {
  health(): NativeHealth;
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
  /** The content-store file name this binary's store schema owns (`content-v1.sqlite` at store floor 1). */
  contentStoreFileName(): string;
  /** Locks `path` exclusively, creating the file; null while any other handle, in any process, holds it. */
  tryLockExclusive(path: string): NativeExclusiveFileLock | null;
  ConversationEngine: NativeConversationEngineConstructor;
  CodeRuleDslRuntime: NativeCodeRuleDslRuntimeConstructor;
  CodeRuleDslWorkspace: NativeCodeRuleDslWorkspaceConstructor;
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

export function loadNativeBinding(): NativeBinding {
  return require(resolveNativeBindingPath()) as NativeBinding;
}

export function nativeHealth(): NativeHealth {
  return loadNativeBinding().health();
}
