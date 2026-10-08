import { createRequire } from "node:module";
import { existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";
import type { MatchOperator, SampleBoundary, SampleSource } from "./rule-schema.js";

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
  canonicalJson(): string;
  /** Whether a repository-relative path is inside the code rule's scope; the matcher evaluation uses. */
  inScope(path: string): boolean;
  digest(): string;
  requiredCapabilitiesJson(): string;
  evaluate(snapshotJson: string, mode: CodeRuleDslEvaluationMode): string;
  openEditor(snapshotJson: string): NativeCodeRuleDslEditor;
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
  addCodeRule(compiled: NativeCompiledCodeRuleDsl): void;
  acceptsPath(path: string): boolean;
  tryApply(requestJson: string): string | null;
  applyAsync(requestJson: string): Promise<string>;
  applyBufferAsync(request: Buffer): Promise<string>;
  /** One bounded chunk of a repository index over the attached Git worktree; call until `done`. */
  indexRepositoryAsync(options: NativeRepositoryIndexOptions): Promise<string>;
  maintainStorage(): string;
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
}

export function encodeCodeRuleWorkspaceRequest(request: {
  revision: number;
  files: ReadonlyArray<{ path: string; language: string; source: string; module?: string | string[] }>;
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
      })),
      removed: request.removed,
      deferGlobal: request.deferGlobal ?? false,
      quiet: request.quiet ?? false,
    }),
    "utf8",
  );
  const header = Buffer.allocUnsafe(12);
  header.write("TEMPORB1", 0, "ascii");
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
  /** The Python module path hosts send as a file's `module`, from the package markers on disk. */
  pythonModulePath(absolute: string): string | string[] | null;
  /** Throws unless `pattern` compiles in the regex dialect and size limit structural rules use. */
  validateRegex(pattern: string): void;
  /** Windows only: start a detached process that inherits only NUL stdin and the appended log file; returns its pid. */
  spawnDetached(program: string, args: string[], cwd: string | null, env: Record<string, string>, logPath: string): number;
  /** The content-store file name this binary's store schema owns (`content-v3.sqlite` at schema floor 17). */
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

export function resolveNativeBindingPath(configuredPath = process.env.RERAIL_NATIVE_KERNEL_PATH): string {
  if (configuredPath !== undefined) {
    const addonPath = resolve(configuredPath);
    if (!existsSync(addonPath)) throw new Error(`RERAIL_NATIVE_KERNEL_PATH does not exist: ${addonPath}`);
    return addonPath;
  }
  const moduleDirectory = dirname(fileURLToPath(import.meta.url));
  const packageRoot = dirname(moduleDirectory);
  const fileName = `omp_cosine_rules.${nativePlatformSuffix()}.node`;
  const candidates = [join(packageRoot, "native", fileName), join(packageRoot, "native", "omp_cosine_rules.node")];
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
