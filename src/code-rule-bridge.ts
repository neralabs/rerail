import { createHash } from "node:crypto";
import { constants, type Stats } from "node:fs";
import { type FileHandle, lstat, mkdtemp, open, opendir, readFile, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { isAbsolute, join, relative, resolve, sep } from "node:path";
import type {
  CodeRuleEdit,
  CodeRuleEditBatch,
  CodeRuleAuditor,
  CodeRuleResponse,
  CodeRuleCheckpoint,
  AuditSettings,
  CodeRuleSourceLanguage,
} from "./code-rule.js";
import type { AuditMode } from "./audit.js";
import { compareDeterministicStrings } from "./stable-json.js";
import { languageOfPath, SOURCE_LANGUAGES } from "./code-rule-languages.js";


const MAX_SOURCE_BYTES = 1_000_000;
/** Paths observed per wave; on network or virtualised mounts each lstat/read is a round trip, so latency, not bandwidth, bounds the walk. */
const OBSERVATION_CONCURRENCY = 16;
const IGNORED_DIRECTORIES = new Set([
  ".git", ".hg", ".svn", ".rerail", ".venv", "__pycache__", "node_modules", "dist", "build", ".next",
  ".turbo", "coverage", "target", "vendor",
]);

interface Preimage {
  readonly text?: string;
  readonly readError?: string;
}
interface Source {
  readonly path: string;
  readonly language: CodeRuleSourceLanguage;
  readonly text?: string;
  readonly deleted?: boolean;
  readonly readError?: string;
  readonly stamp?: string;
  readonly digest?: string;
  readonly modifiedAt?: number;
  /** Filesystem-clock reading taken before the walk that produced this observation. */
  readonly observedAt?: number;
}
export type ObservedSource = Omit<Source, "text" | "deleted">;
/** Baseline observations keyed by workspace directory, shared so later sessions reuse digests of unchanged sources. */
export type SourceObservationCache = Map<string, ReadonlyMap<string, ObservedSource>>;
interface SourceLocation {
  readonly absolute: string;
  readonly path: string;
  readonly language: CodeRuleSourceLanguage;
}
function observation({ text: _text, deleted: _deleted, ...source }: Source): ObservedSource {
  return source;
}
interface PendingEdit {
  readonly paths: readonly string[];
  readonly preimages: ReadonlyMap<string, Preimage>;
}

/** Which boundary asked for a discovery pass; a stronger request absorbs weaker ones queued behind the same walk. */
type DiscoveryCheckpoint = "edit" | "turn" | "final";
const DISCOVERY_PRIORITY: Readonly<Record<DiscoveryCheckpoint, number>> = { edit: 0, turn: 1, final: 2 };
function strongerCheckpoint(checkpoint: DiscoveryCheckpoint, other: DiscoveryCheckpoint | undefined): DiscoveryCheckpoint {
  return other !== undefined && DISCOVERY_PRIORITY[other] > DISCOVERY_PRIORITY[checkpoint] ? other : checkpoint;
}
function discoveryCheckpoints(checkpoint: DiscoveryCheckpoint): CodeRuleCheckpoint[] {
  return checkpoint === "edit" ? ["edit", "function"] : [checkpoint];
}

interface SourceDiscovery {
  /** Filesystem-clock reading taken before the walk; files stamped at or after it may hold this session's shell writes. */
  readonly startedAt: number;
  readonly root: string;
  cancelled: boolean;
  /** Resolves once every requested pass has run; undefined while no walk is in flight. */
  idle: Promise<void> | undefined;
  baselineComplete: boolean;
  requested: DiscoveryCheckpoint | undefined;
  readonly versions: Map<string, number>;
  /** Subtrees whose first successful observation must still be a baseline. */
  baselineRoots: Set<string>;
  error: string | undefined;
}

/**
 * Samples the clock file timestamps are stamped with. `Date.now()` is not that clock: on Linux file stamps
 * advance per scheduler tick and lag it, so a write made right after session start carries an earlier mtime,
 * while on macOS stamps carry sub-millisecond precision that `Date.now()` truncates away.
 */
async function filesystemClock(): Promise<number> {
  try {
    const probe = await mkdtemp(join(tmpdir(), "rerail-clock-"));
    try {
      const info = await lstat(probe);
      return Math.min(info.mtimeMs, info.ctimeMs);
    } finally {
      await rm(probe, { recursive: true, force: true });
    }
  } catch {
    return Date.now();
  }
}

interface SourceScan {
  readonly sources: Map<string, Source>;
  readonly failedRoots: Set<string>;
  readonly errors: string[];
}
/** One traversal of the workspace: what it compares against, when it started, and what it has found so far. */
interface SourceWalk {
  readonly discovery: SourceDiscovery;
  readonly observedAt: number;
  readonly previousSources: ReadonlyMap<string, ObservedSource>;
  readonly captureChanges: boolean;
  readonly scan: SourceScan;
  readonly stack: string[];
}
/**
 * A stamp only proves a file unchanged when it is older than the observation it is compared with:
 * a same-size rewrite within the same filesystem-clock tick leaves the stamp identical (git's racy-index rule).
 */
function staleStamp(previous: ObservedSource | undefined, stamp: string, modifiedAt: number): boolean {
  return previous?.stamp === stamp && previous.observedAt !== undefined && modifiedAt < previous.observedAt;
}

function strings(value: unknown, output: string[]): void {
  if (typeof value === "string") {
    output.push(value);
    return;
  }
  if (Array.isArray(value)) {
    for (const item of value) strings(item, output);
    return;
  }
  if (value === null || typeof value !== "object") return;
  for (const [key, item] of Object.entries(value)) {
    if (/^(?:path|paths|file|filename|file_path|filePath|patch|input|command)$/u.test(key)) strings(item, output);
  }
}
function patchPaths(text: string): string[] {
  return [
    ...[...text.matchAll(/^\[([^#\r\n]+)#[0-9A-F]{4}\]$/gmu)].map(match => match[1]!),
    ...[...text.matchAll(/^\*\*\* (?:Add File|Update File|Delete File|Move to): ([^\r\n]+)$/gmu)].map(match => match[1]!),
  ];
}
function sameSource(left: ObservedSource | undefined, right: Source | undefined): boolean {
  return left?.digest === right?.digest && left?.readError === right?.readError;
}
function emptyResponse(revision: number): CodeRuleResponse {
  return { revision, disposition: "continue", violations: [], failures: [] };
}
function underBaselineRoot(roots: ReadonlySet<string>, path: string): boolean {
  for (const root of roots) if (root === "" || path.startsWith(`${root}/`)) return true;
  return false;
}
function uncapturedPreimage(baselinePath: boolean, before: ObservedSource | undefined): Pick<CodeRuleEdit, "previousReadError"> {
  if (baselinePath) return { previousReadError: "Source changed during initial discovery without a captured preimage" };
  if (before === undefined) return {};
  return { previousReadError: "Source changed outside a tool with a captured preimage" };
}
function uncapturedEdit(
  path: string,
  after: Source,
  preimage: Pick<CodeRuleEdit, "previousReadError">,
): CodeRuleEdit | undefined {
  if (after.readError !== undefined) return { path, language: after.language, ...preimage, readError: after.readError };
  if (after.text !== undefined) return { path, language: after.language, ...preimage, generatedText: after.text };
  return undefined;
}

export function generatedEditPaths(toolName: string, input: unknown): string[] {
  if (!/^(?:write|edit|multiedit|ast_edit|apply_patch)$/iu.test(toolName)) return [];
  const candidates: string[] = [];
  strings(input, candidates);
  const paths = candidates.flatMap(candidate => {
    const patched = patchPaths(candidate);
    return patched.length > 0 ? patched : [candidate];
  });
  return [...new Set(paths.filter(path => languageOfPath(SOURCE_LANGUAGES, path) !== undefined))];
}

/** Tools that run arbitrary commands can write any source without a preimage, so their results reconcile the workspace. */
export function isCommandTool(toolName: string): boolean {
  return /bash|shell|exec|terminal|command|cmd/iu.test(toolName);
}

/** Captures only changed source snapshots; the startup snapshot is discovery state, never evaluator input. */
export class CodeRuleBridge {
  readonly #cwd: string;
  readonly #auditor: CodeRuleAuditor;
  readonly #onRevision: ((sessionId: string, revision: number, paths: readonly string[]) => void) | undefined;
  readonly #observations: SourceObservationCache | undefined;
  readonly #pending = new Map<string, PendingEdit>();
  /** Command tool calls whose results must reconcile shell writes. */
  readonly #commands = new Set<string>();
  readonly #observed = new Map<string, ObservedSource>();
  #lastResponse = emptyResponse(0);
  #sessionId: string | undefined;
  #revision = 0;
  #queue: Promise<void> = Promise.resolve();
  #discovery: SourceDiscovery | undefined;

  constructor(
    cwd: string,
    auditor: CodeRuleAuditor,
    options: {
      onRevision?(sessionId: string, revision: number, paths: readonly string[]): void;
      observations?: SourceObservationCache;
    } = {},
  ) {
    this.#cwd = resolve(cwd);
    this.#auditor = auditor;
    this.#onRevision = options.onRevision;
    this.#observations = options.observations;
  }

  /** Resolves once the evaluator is open; the baseline walk runs behind it and blocks only the next checkpoint or command result. */
  async openWorkspace(sessionId: string): Promise<void> {
    await this.#enqueue(async () => {
      const startedAt = await filesystemClock();
      if (this.#discovery !== undefined) this.#discovery.cancelled = true;
      if (this.#sessionId !== undefined) {
        this.#sessionId = undefined;
        await this.#auditor.close();
      }
      this.#pending.clear();
      this.#commands.clear();
      this.#observed.clear();
      this.#discovery = undefined;
      const root = await realpath(this.#cwd);
      await this.#auditor.openWorkspace(sessionId);
      this.#sessionId = sessionId;
      this.#revision = 0;
      this.#lastResponse = emptyResponse(0);
      const discovery: SourceDiscovery = {
        startedAt, root, cancelled: false, idle: undefined, baselineComplete: false, requested: undefined,
        versions: new Map(), baselineRoots: new Set(), error: undefined,
      };
      this.#discovery = discovery;
      // The lowest priority: whichever boundary first waits on the baseline labels what it finds.
      void this.#requestDiscovery(discovery, "edit");
    });
  }

  async begin(
    toolCallId: string,
    toolName: string,
    input: unknown,
    restoredPreimages?: Readonly<Record<string, Preimage>>,
  ): Promise<Readonly<Record<string, Preimage>>> {
    const discovery = this.#requireDiscovery();
    this.refreshCodeRules();
    const paths = generatedEditPaths(toolName, input);
    if (paths.length === 0) {
      if (isCommandTool(toolName)) this.#commands.add(toolCallId);
      return {};
    }
    return await this.#enqueue(async () => {
      this.#assertDiscovery(discovery);
      const preimages = restoredPreimages === undefined
        ? await this.#capturePreimages(paths)
        : new Map(Object.entries(restoredPreimages));
      for (const path of preimages.keys()) this.#touchSource(discovery, path);
      this.#pending.set(toolCallId, { paths, preimages });
      return Object.fromEntries(preimages);
    });
  }

  /** A failed command may still have written files, so command results reconcile regardless of `failed`. */
  async finish(toolCallId: string, failed = false): Promise<readonly CodeRuleResponse[]> {
    const discovery = this.#requireDiscovery();
    if (this.#commands.delete(toolCallId)) return await this.#reconcile(discovery);
    return await this.#enqueue(async () => {
      this.#assertDiscovery(discovery);
      const pending = this.#pending.get(toolCallId);
      this.#pending.delete(toolCallId);
      if (pending === undefined || failed) return [];
      this.refreshCodeRules();
      const edits = await this.#readEdits(discovery, pending.paths, pending.preimages);
      return edits.length === 0 ? [] : [await this.#auditEdits(edits, ["edit", "function"])];
    });
  }

  /** Resolves after every requested walk has run, so the checkpoint's violations are delivered with this boundary. */
  async checkpoint(checkpoint: Extract<CodeRuleCheckpoint, "turn" | "final">): Promise<CodeRuleResponse> {
    const discovery = this.#requireDiscovery();
    await this.#requestDiscovery(discovery, checkpoint);
    this.#assertDiscovery(discovery);
    return this.#discoveryResponse(discovery);
  }

  async #reconcile(discovery: SourceDiscovery): Promise<readonly CodeRuleResponse[]> {
    const revision = this.#revision;
    this.refreshCodeRules();
    await this.#requestDiscovery(discovery, "edit");
    this.#assertDiscovery(discovery);
    return this.#revision === revision ? [] : [this.#lastResponse];
  }
  async settleGeneratedEdits(): Promise<void> {
    this.#requireSession();
    await this.#queue;
    if (this.#auditor.settleGeneratedEdits !== undefined) {
      await this.#auditor.settleGeneratedEdits();
    } else {
      await this.#auditor.settle?.();
    }
  }

  async settle(): Promise<void> {
    this.#requireSession();
    await this.#queue;
    await this.#auditor.settle?.();
  }

  refreshCodeRules(): void {
    this.#auditor.refreshCodeRules?.();
  }

  updateSettings(settings: Required<AuditSettings>): void {
    this.#auditor.updateSettings?.(settings);
  }

  async audit(input: Parameters<NonNullable<CodeRuleAuditor["audit"]>>[1]): Promise<readonly string[]> {
    const discovery = this.#requireDiscovery();
    const sessionId = this.#requireSession();
    if (this.#auditor.audit === undefined) throw new Error("Policy evaluator does not support scans");
    // Serialize the launch with edits, not the repository traversal with subsequent tool events.
    const { work } = await this.#enqueue(async () => {
      this.#assertDiscovery(discovery);
      return { work: this.#auditor.audit!(sessionId, input) };
    });
    return await work;
  }
  interrupt(): void {
    this.#auditor.interrupt?.();
  }


  async close(): Promise<void> {
    const discovery = this.#discovery;
    if (discovery === undefined || discovery.cancelled) return;
    // Cancel before joining the operation queue: a blocked filesystem read is not shutdown work.
    discovery.cancelled = true;
    await this.#enqueue(async () => {
      if (this.#discovery !== discovery) return;
      this.#sessionId = undefined;
      this.#pending.clear();
      this.#commands.clear();
      this.#observed.clear();
      this.#discovery = undefined;
      await this.#auditor.close();
    });
  }
  async #capturePreimages(paths: readonly string[]): Promise<ReadonlyMap<string, Preimage>> {
    const root = await realpath(this.#cwd);
    const preimages = new Map<string, Preimage>();
    for (const configuredPath of [...paths].sort(compareDeterministicStrings)) {
      const source = await this.#readSource(root, configuredPath);
      if (source === undefined) continue;
      if (source.readError === undefined) {
        if (source.deleted) this.#observed.delete(source.path);
        else this.#observed.set(source.path, observation(source));
      }
      if (source.text !== undefined) preimages.set(source.path, { text: source.text });
      else if (source.readError !== undefined) preimages.set(source.path, { readError: source.readError });
      else if (source.deleted) preimages.set(source.path, {});
    }
    return preimages;
  }

  async #readEdits(
    discovery: SourceDiscovery,
    paths: readonly string[],
    preimages: ReadonlyMap<string, Preimage>,
  ): Promise<CodeRuleEdit[]> {
    const root = await realpath(this.#cwd);
    const edits: CodeRuleEdit[] = [];
    for (const configuredPath of [...paths].sort(compareDeterministicStrings)) {
      const source = await this.#readSource(root, configuredPath);
      if (source === undefined) continue;
      this.#touchSource(discovery, source.path);
      const edit = this.#readEdit(source, preimages.get(source.path));
      if (edit !== undefined) edits.push(edit);
    }
    return edits;
  }

  /** Queues a pass for `checkpoint` and resolves once the walk is idle, i.e. every pass requested so far has run. */
  #requestDiscovery(discovery: SourceDiscovery, checkpoint: DiscoveryCheckpoint): Promise<void> {
    if (discovery.cancelled) return Promise.resolve();
    discovery.requested = strongerCheckpoint(checkpoint, discovery.requested);
    // The traversal never occupies #queue. Only completed snapshots join ordered tool revisions.
    discovery.idle ??= this.#runDiscovery(discovery);
    return discovery.idle;
  }

  async #runDiscovery(discovery: SourceDiscovery): Promise<void> {
    try {
      while (!discovery.cancelled && discovery.requested !== undefined) {
        try {
          await this.#discoveryPass(discovery);
        } catch (error) {
          if (this.#discovery === discovery && !discovery.cancelled)
            discovery.error = error instanceof Error ? error.message : String(error);
        }
      }
    } finally {
      discovery.idle = undefined;
    }
  }

  async #discoveryPass(discovery: SourceDiscovery): Promise<void> {
    const checkpoint = discovery.requested!;
    discovery.requested = undefined;
    const baseline = !discovery.baselineComplete;
    const previous = new Map(this.#observed);
    const versions = new Map(discovery.versions);
    // An earlier session's baseline of this workspace lets unchanged sources skip the read.
    const seed = baseline ? this.#observations?.get(this.#cwd) : undefined;
    const scan = await this.#observeSources({
      discovery, observedAt: await filesystemClock(), previousSources: seed === undefined ? previous : new Map([...seed, ...previous]),
      captureChanges: !baseline, scan: { sources: new Map(), failedRoots: new Set(), errors: [] }, stack: [discovery.root],
    });
    if (discovery.cancelled || this.#discovery !== discovery) return;
    await this.#enqueue(async () => {
      if (discovery.cancelled || this.#discovery !== discovery) return;
      const edits = this.#discoverEdits(discovery, scan.sources, previous, versions, baseline);
      try {
        if (edits.length > 0)
          await this.#auditEdits(edits, discoveryCheckpoints(strongerCheckpoint(checkpoint, discovery.requested)));
      } catch (error) {
        // A rejected evaluator submission is not an observation we have enforced.
        // Restore these baselines so the next requested scan can retry the changes.
        for (const edit of edits) {
          const before = previous.get(edit.path);
          if (before === undefined) this.#observed.delete(edit.path);
          else this.#observed.set(edit.path, before);
        }
        throw error;
      }
      discovery.baselineComplete = true;
      discovery.baselineRoots = scan.failedRoots;
      discovery.error = scan.errors.length === 0 ? undefined : scan.errors.join("; ");
      this.#observations?.set(this.#cwd, new Map(this.#observed));
    });
    // Reconcile changes to already-observed files made during the initial traversal.
    if (baseline && discovery.requested === undefined) discovery.requested = checkpoint;
  }

  #discoverEdits(
    discovery: SourceDiscovery,
    current: ReadonlyMap<string, Source>,
    previous: ReadonlyMap<string, ObservedSource>,
    versions: ReadonlyMap<string, number>,
    baseline: boolean,
  ): CodeRuleEdit[] {
    const pendingPaths = new Set<string>();
    for (const pending of this.#pending.values())
      for (const path of pending.preimages.keys()) pendingPaths.add(path);
    const edits: CodeRuleEdit[] = [];
    for (const path of [...new Set([...previous.keys(), ...current.keys()])].sort(compareDeterministicStrings)) {
      // The walk covers the session folder only: a path a tool wrote outside it is never observed missing here.
      if (path === ".." || path.startsWith("../")) continue;
      // A begin/finish after scan launch wins even when its bytes happen to match the old snapshot.
      if (discovery.versions.get(path) !== versions.get(path) || pendingPaths.has(path)) continue;
      const edit = this.#discoveredEdit(discovery, path, previous.get(path), current.get(path), baseline);
      if (edit !== undefined) edits.push(edit);
    }
    return edits;
  }

  #discoveredEdit(
    discovery: SourceDiscovery,
    path: string,
    before: ObservedSource | undefined,
    after: Source | undefined,
    baseline: boolean,
  ): CodeRuleEdit | undefined {
    const baselinePath = baseline || (before === undefined && underBaselineRoot(discovery.baselineRoots, path));
    // A first observation made after a shell write is not a trustworthy preimage.
    // Preserve it as an uncaptured change, never as newly generated source.
    const changedDuringBaseline = baselinePath && after?.modifiedAt !== undefined && after.modifiedAt >= discovery.startedAt;
    if ((baselinePath && !changedDuringBaseline) || sameSource(before, after)) {
      if (after !== undefined) this.#observed.set(path, observation(after));
      return undefined;
    }
    if (after === undefined) {
      this.#observed.delete(path);
      if (before === undefined) return undefined;
      return { path, language: before.language, previousReadError: "Source was deleted without a captured preimage", deleted: true };
    }
    this.#observed.set(path, observation(after));
    return uncapturedEdit(path, after, uncapturedPreimage(baselinePath, before));
  }

  async #observeSources(walk: SourceWalk): Promise<SourceScan> {
    const { discovery, stack, scan } = walk;
    while (!discovery.cancelled && stack.length > 0) {
      const wave = stack.splice(-OBSERVATION_CONCURRENCY);
      await Promise.all(wave.map(absolute => this.#observePath(walk, absolute)));
    }
    for (const source of scan.sources.values())
      if (source.readError !== undefined) scan.errors.push(`${source.path}: ${source.readError}`);
    return scan;
  }

  async #observePath(walk: SourceWalk, absolute: string): Promise<void> {
    const { discovery } = walk;
    try {
      const info = await lstat(absolute);
      if (discovery.cancelled || info.isSymbolicLink()) return;
      if (info.isDirectory()) await this.#expandDirectory(walk, absolute);
      else if (info.isFile()) await this.#observeRegularFile(walk, absolute, info);
    } catch (error) {
      if (discovery.cancelled) return;
      this.#recordObservationFailure(walk, absolute, error);
    }
  }

  async #expandDirectory(walk: SourceWalk, absolute: string): Promise<void> {
    for await (const entry of await opendir(absolute)) {
      if (walk.discovery.cancelled) return;
      if (entry.isSymbolicLink() || (entry.isDirectory() && IGNORED_DIRECTORIES.has(entry.name))) continue;
      if (entry.isDirectory() || entry.isFile()) walk.stack.push(join(absolute, entry.name));
    }
  }

  async #observeRegularFile(walk: SourceWalk, absolute: string, info: Stats): Promise<void> {
    const path = relative(walk.discovery.root, absolute).split(sep).join("/");
    const language = languageOfPath(SOURCE_LANGUAGES, path);
    if (language !== undefined) await this.#observeFile(walk, { absolute, path, language }, info);
  }

  async #observeFile(walk: SourceWalk, location: SourceLocation, info: Stats): Promise<void> {
    const { discovery, observedAt, previousSources, captureChanges, scan: { sources: observed } } = walk;
    const { absolute, path, language } = location;
    const stamp = `${info.dev}:${info.ino}:${info.size}:${info.mtimeMs}:${info.ctimeMs}`;
    let modifiedAt = Math.max(info.mtimeMs, info.ctimeMs);
    const previous = previousSources.get(path);
    if (!this.#auditor.acceptsPath(path)) {
      observed.set(path, { path, language, stamp, observedAt, ...(previous?.digest === undefined ? {} : { digest: previous.digest }) });
      return;
    }
    if (staleStamp(previous, stamp, modifiedAt)) {
      observed.set(path, previous!);
      return;
    }
    if (info.size > MAX_SOURCE_BYTES) {
      observed.set(path, { path, language, stamp, modifiedAt, observedAt, readError: `Source exceeds ${MAX_SOURCE_BYTES} byte policy limit` });
      return;
    }
    const source = await readFile(absolute);
    if (discovery.cancelled) return;
    // A shell may mutate the file while readFile is pending. Keep the original stamp
    // so the next reconciliation re-reads it, but do not silently baseline those bytes.
    if (!captureChanges) {
      const after = await lstat(absolute);
      if (discovery.cancelled) return;
      modifiedAt = Math.max(modifiedAt, after.mtimeMs, after.ctimeMs);
    }
    if (source.length > MAX_SOURCE_BYTES || source.includes(0)) {
      observed.set(path, { path, language, stamp, modifiedAt, observedAt, readError: "Source exceeds its byte budget or contains binary data" });
      return;
    }
    const digest = createHash("sha256").update(source).digest("hex");
    observed.set(path, {
      path, language, stamp, digest, modifiedAt, observedAt,
      ...((captureChanges || modifiedAt >= discovery.startedAt) && previous?.digest !== digest
        ? { text: source.toString("utf8") } : {}),
    });
  }

  #recordObservationFailure(walk: SourceWalk, absolute: string, error: unknown): void {
    const { discovery, previousSources, scan } = walk;
    const path = relative(discovery.root, absolute).split(sep).join("/");
    const language = languageOfPath(SOURCE_LANGUAGES, path);
    const readError = error instanceof Error ? error.message : String(error);
    if (language !== undefined) {
      scan.sources.set(path, { path, language, readError });
      return;
    }
    scan.errors.push(`${path || "."}: ${readError}`);
    scan.failedRoots.add(path);
    for (const [sourcePath, source] of previousSources) {
      if (path === "" || sourcePath.startsWith(`${path}/`))
        scan.sources.set(sourcePath, { path: sourcePath, language: source.language, readError });
    }
  }

  #readEdit(source: Source, preimage: Preimage | undefined): CodeRuleEdit | undefined {
    if (source.text !== undefined && preimage?.text === source.text) {
      this.#observed.set(source.path, observation(source));
      return undefined;
    }
    const edit = this.#editFromToolSource(source, preimage);
    this.#updateObservedSource(source);
    return edit;
  }

  #editFromToolSource(source: Source, preimage: Preimage | undefined): CodeRuleEdit | undefined {
    const previous = preimage === undefined
      ? { previousReadError: "Original source was not captured for this tool call" }
      : { ...(preimage.text === undefined ? {} : { previousText: preimage.text }), ...(preimage.readError === undefined ? {} : { previousReadError: preimage.readError }) };
    if (source.text !== undefined) return { path: source.path, language: source.language, ...previous, generatedText: source.text };
    if (source.deleted) return { path: source.path, language: source.language, ...previous, deleted: true };
    if (source.readError !== undefined) return { path: source.path, language: source.language, ...previous, readError: source.readError };
    return undefined;
  }

  #updateObservedSource(source: Source): void {
    if (source.readError !== undefined) return;
    if (source.deleted) this.#observed.delete(source.path);
    else this.#observed.set(source.path, observation(source));
  }

  async #readSource(root: string, configuredPath: string): Promise<Source | undefined> {
    const location = this.#sourceLocation(root, configuredPath);
    if (location === undefined) return undefined;
    try {
      return await this.#readLocatedSource(root, location);
    } catch (error) {
      return this.#failedSourceRead(location, error);
    }
  }

  /** Paths outside the session folder are kept, keyed relative to it (`../sibling/x.ts`); the auditor decides. */
  #sourceLocation(root: string, configuredPath: string): SourceLocation | undefined {
    const absolute = isAbsolute(configuredPath) ? configuredPath : resolve(root, configuredPath);
    const path = relative(root, absolute).split(sep).join("/");
    if (path === "" || isAbsolute(path)) return undefined;
    const language = languageOfPath(SOURCE_LANGUAGES, path);
    if (language === undefined || !this.#auditor.acceptsPath(path)) return undefined;
    return { absolute, path, language };
  }

  /**
   * Reads the file through one descriptor opened without following a final symlink. Its real path must stay inside
   * the checkout that owns it (the session folder for an auditor that judges one folder), and still name the file
   * that was opened, so a folder swapped for a symlink mid-read cannot bring outside bytes in. A file the auditor
   * finds no owner for is not read.
   */
  async #readLocatedSource(root: string, location: SourceLocation): Promise<Source | undefined> {
    let handle: FileHandle;
    try {
      handle = await open(location.absolute, constants.O_RDONLY | constants.O_NOFOLLOW);
    } catch (error) {
      if (error instanceof Error && "code" in error && error.code === "ELOOP") return undefined;
      throw error;
    }
    try {
      const info = await handle.stat();
      if (!info.isFile()) return { path: location.path, language: location.language, readError: "Source is not a regular file" };
      const owner = this.#auditor.sourceRoot === undefined ? root : this.#auditor.sourceRoot(location.path);
      if (owner === undefined) return undefined;
      const canonical = await realpath(location.absolute);
      if (canonical !== owner && !canonical.startsWith(`${owner}${sep}`)) return undefined;
      const named = await lstat(canonical);
      if (named.dev !== info.dev || named.ino !== info.ino) return undefined;
      if (info.size > MAX_SOURCE_BYTES)
        return { path: location.path, language: location.language, readError: `Source exceeds ${MAX_SOURCE_BYTES} byte policy limit` };
      const source = await handle.readFile();
      if (source.length > MAX_SOURCE_BYTES || source.includes(0))
        return { path: location.path, language: location.language, readError: "Source exceeds its byte budget or contains binary data" };
      return {
        path: location.path, language: location.language, text: source.toString("utf8"),
        stamp: `${info.dev}:${info.ino}:${info.size}:${info.mtimeMs}:${info.ctimeMs}`,
        digest: createHash("sha256").update(source).digest("hex"),
      };
    } finally {
      await handle.close();
    }
  }

  #failedSourceRead(location: SourceLocation, error: unknown): Source {
    if (error instanceof Error && "code" in error && error.code === "ENOENT")
      return { path: location.path, language: location.language, deleted: true };
    return {
      path: location.path,
      language: location.language,
      readError: error instanceof Error ? error.message : String(error),
    };
  }

  async #auditEdits(edits: CodeRuleEditBatch["edits"], checkpoints: CodeRuleCheckpoint[]): Promise<CodeRuleResponse> {
    const sessionId = this.#requireSession();
    const paths = edits.map(edit => edit.path).sort(compareDeterministicStrings);
    this.#revision += 1;
    this.#onRevision?.(sessionId, this.#revision, paths);
    this.#lastResponse = await this.#auditor.auditGeneratedEdits(sessionId, { revision: this.#revision, checkpoints, edits });
    return this.#lastResponse;
  }

  #touchSource(discovery: SourceDiscovery, path: string): void {
    discovery.versions.set(path, (discovery.versions.get(path) ?? 0) + 1);
  }

  #requireDiscovery(): SourceDiscovery {
    const discovery = this.#discovery;
    if (discovery === undefined || discovery.cancelled) throw new Error("Policy workspace is not open");
    return discovery;
  }

  #assertDiscovery(discovery: SourceDiscovery): void {
    if (this.#discovery !== discovery) throw new Error("Policy workspace is not open");
  }

  #discoveryResponse(discovery: SourceDiscovery): CodeRuleResponse {
    let message = discovery.error;
    if (message === undefined && !discovery.baselineComplete) message = "Initial source discovery is still pending";
    else if (message === undefined && discovery.idle !== undefined) message = "Source reconciliation is still pending";
    if (message === undefined) return this.#lastResponse;
    return {
      ...this.#lastResponse,
      failures: [...this.#lastResponse.failures, {
        clauseId: "source-discovery", code: "source-incomplete", message, stage: "scan",
      }],
    };
  }

  #requireSession(): string {
    if (this.#sessionId === undefined) throw new Error("Policy workspace is not open");
    return this.#sessionId;
  }
  async #enqueue<T>(operation: () => Promise<T>): Promise<T> {
    const pending = this.#queue.then(operation, operation);
    this.#queue = pending.then(() => undefined, () => undefined);
    return await pending;
  }
}
