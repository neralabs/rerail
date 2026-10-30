import { createHash, randomBytes } from "node:crypto";
import { constants, type Stats } from "node:fs";
import { type FileHandle, lstat, mkdtemp, open, opendir, readFile, realpath, rename, rm, unlink } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import type {
  AuditorCloseOptions,
  CodeRuleEdit,
  CodeRuleEditBatch,
  CodeRuleAuditor,
  CodeRuleResponse,
  CodeRuleCheckpoint,
  AuditSettings,
  CodeRuleSourceLanguage,
} from "./code-rule.js";
import type { AuditMode } from "../audit/audit.js";
import { readGitBlobs } from "../git.js";
import type { RewoundPaths } from "./rerail-steer.js";
import { compareDeterministicStrings } from "../stable-json.js";
import { languageOfPath, SOURCE_LANGUAGES } from "./code-rule-languages.js";
import { sensitiveByName } from "../path-boundary.js";


const MAX_SOURCE_BYTES = 1_000_000;
/** The most source text the session keeps as later preimages (`#texts`); the least recently changed goes first. */
const MAX_SESSION_TEXT_BYTES = 64 * 1024 * 1024;
/** Paths observed per wave; on network or virtualised mounts each lstat/read is a round trip, so latency, not bandwidth, bounds the walk. */
const OBSERVATION_CONCURRENCY = 16;
const IGNORED_DIRECTORIES = new Set([
  ".git", ".hg", ".svn", ".rerail", ".venv", "__pycache__", "node_modules", "dist", "build", ".next",
  ".turbo", "coverage", "target", "vendor",
]);

/**
 * A source as a tool call found it: its text, why it could not be read, or neither when it did not exist. A file named
 * like one holding secrets (`sensitiveByName`) keeps only its SHA-256 and size (`withheld`): preimages go into the
 * session ledger in the checkout, so its text is never captured, and `rewind` refuses to put it back.
 */
export interface Preimage {
  readonly text?: string;
  readonly readError?: string;
  readonly withheld?: { readonly sha256: string; readonly bytes: number };
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
/** A finished edit `rewind` may still roll back: the revision its finish audited, and each path's digest after it (none when absent). */
interface RewindableEdit {
  readonly revision: number;
  readonly preimages: ReadonlyMap<string, Preimage>;
  readonly after: ReadonlyMap<string, string | undefined>;
}
/** The paths `rewind` put back, and the revision that audited them back (in `log` mode). */
export interface RewoundEdit extends RewoundPaths {
  readonly revision: number;
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
  /** A boundary answered while the current walk was running; closing before it finishes says so again. */
  pendingReported: boolean;
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
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
function underBaselineRoot(roots: ReadonlySet<string>, path: string): boolean {
  for (const root of roots) if (root === "" || path.startsWith(`${root}/`)) return true;
  return false;
}
/** `absolute` relative to the walk's root (the session folder), `/`-separated, as the auditor names paths. */
function sessionPath(root: string, absolute: string): string {
  return relative(root, absolute).split(sep).join("/");
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

/**
 * How long a turn checkpoint or a command result waits for a checkout-wide source walk, and a code-rule boundary
 * waits for the session's evaluator to open, before answering with what it has. The work goes on, and what it finds
 * is audited and delivered at a later boundary; waiting longer would hold the session's event lane past the host's
 * per-event timeout.
 */
export const BOUNDARY_DISCOVERY_WAIT_MS = 10_000;

/** A boundary answered while a source walk was still running: its shell writes are checked later, or never for `close`. */
export type DiscoveryPendingBoundary = "turn" | "final" | "command" | "close";

/** Captures only changed source snapshots; the startup snapshot is discovery state, never evaluator input. */
export class CodeRuleBridge {
  readonly #cwd: string;
  readonly #auditor: CodeRuleAuditor;
  readonly #onRevision: ((sessionId: string, revision: number, paths: readonly string[]) => void) | undefined;
  readonly #observations: SourceObservationCache | undefined;
  readonly #pending = new Map<string, PendingEdit>();
  /** Finished edits kept rewindable by their tool call, from `finish` until `release`. */
  readonly #rewindable = new Map<string, RewindableEdit>();
  /** Command tool calls whose results must reconcile shell writes. */
  readonly #commands = new Set<string>();
  readonly #observed = new Map<string, ObservedSource>();
  /**
   * The text each source this session changed was last seen with: the preimage of its next change made without one
   * captured, once its digest proves it (`#recoverPreimages`). Only changed sources are kept, never the baseline, and at
   * most `MAX_SESSION_TEXT_BYTES` of them, the least recently changed evicted first: an evicted source falls back to Git.
   */
  readonly #texts = new Map<string, { text: string; bytes: number }>();
  #textBytes = 0;
  #lastResponse = emptyResponse(0);
  #sessionId: string | undefined;
  #revision = 0;
  #queue: Promise<void> = Promise.resolve();
  #discovery: SourceDiscovery | undefined;
  readonly #discoveryWaitMs: number;
  readonly #onDiscoveryPending: ((boundary: DiscoveryPendingBoundary) => void) | undefined;

  constructor(
    cwd: string,
    auditor: CodeRuleAuditor,
    options: {
      onRevision?(sessionId: string, revision: number, paths: readonly string[]): void;
      observations?: SourceObservationCache;
      /** How long a boundary waits for a source walk; `BOUNDARY_DISCOVERY_WAIT_MS` by default. */
      discoveryWaitMs?: number;
      /** Told when a boundary answers, or the bridge closes, while a source walk is still running. */
      onDiscoveryPending?(boundary: DiscoveryPendingBoundary): void;
    } = {},
  ) {
    this.#cwd = resolve(cwd);
    this.#auditor = auditor;
    this.#onRevision = options.onRevision;
    this.#observations = options.observations;
    this.#discoveryWaitMs = options.discoveryWaitMs ?? BOUNDARY_DISCOVERY_WAIT_MS;
    this.#onDiscoveryPending = options.onDiscoveryPending;
  }

  /** Resolves once the evaluator is open; the baseline walk runs behind it and delays only the next checkpoint or command result. */
  async openWorkspace(sessionId: string): Promise<void> {
    await this.#enqueue(async () => {
      const startedAt = await filesystemClock();
      if (this.#discovery !== undefined) this.#discovery.cancelled = true;
      if (this.#sessionId !== undefined) {
        this.#sessionId = undefined;
        await this.#auditor.close();
      }
      this.#pending.clear();
      this.#rewindable.clear();
      this.#commands.clear();
      this.#observed.clear();
      this.#texts.clear();
      this.#textBytes = 0;
      this.#discovery = undefined;
      const root = await realpath(this.#cwd);
      await this.#auditor.openWorkspace(sessionId);
      this.#sessionId = sessionId;
      this.#revision = 0;
      this.#lastResponse = emptyResponse(0);
      const discovery: SourceDiscovery = {
        startedAt, root, cancelled: false, idle: undefined, baselineComplete: false, requested: undefined,
        versions: new Map(), baselineRoots: new Set(), error: undefined, pendingReported: false,
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

  /**
   * A failed command may still have written files, so command results reconcile regardless of `failed`. With
   * `rewindable` the edit's preimages are kept for `rewind` until `release`.
   */
  async finish(toolCallId: string, failed = false, options: { rewindable?: boolean } = {}): Promise<readonly CodeRuleResponse[]> {
    const discovery = this.#requireDiscovery();
    if (this.#commands.delete(toolCallId)) return await this.#reconcile(discovery);
    return await this.#enqueue(async () => {
      this.#assertDiscovery(discovery);
      const pending = this.#pending.get(toolCallId);
      this.#pending.delete(toolCallId);
      if (pending === undefined || failed) return [];
      this.refreshCodeRules();
      const { edits, uncaptured } = await this.#readEdits(discovery, pending.paths, pending.preimages);
      if (edits.length === 0) return [];
      // Kept before the audit starts, so a steer its evaluation publishes finds it.
      if (options.rewindable === true) {
        const after = new Map([...pending.preimages.keys()].map(path => [path, this.#observed.get(path)?.digest]));
        this.#rewindable.set(toolCallId, { revision: this.#revision + 1, preimages: pending.preimages, after });
      }
      return [await this.#auditEdits(discovery, edits, ["edit", "function"], uncaptured)];
    });
  }

  /** Ends the window in which `rewind` may roll the call's edit back. */
  release(toolCallId: string): void {
    this.#rewindable.delete(toolCallId);
  }

  /** Whether a steer at `revision` over `scope` is the check of the call's finished edit, from `finish` until `release`. */
  checks(toolCallId: string, revision: number, scope: readonly string[]): boolean {
    const rewindable = this.#rewindable.get(toolCallId);
    return rewindable !== undefined && rewindable.revision === revision
      && scope.length > 0 && scope.every(path => rewindable.preimages.has(path));
  }

  /**
   * Rolls a finished, still rewindable edit back to its captured preimages, every file or none: a file that did not
   * exist is deleted. Only for a steer at the revision its finish audited whose `scope` lies within the captured
   * paths, while every preimage was read, every file still holds what the edit left, and no other call begun and
   * unfinished, or still rewindable, covers any of them (rolling back over its write would undo it). Each file is
   * replaced whole, so a failed write leaves it as it was; when putting already rolled-back files back fails too, those
   * files stay rolled back and are reported. The rollback is observed as Rerail's own write, so no walk reports it as an
   * uncaptured edit, and audited in `log` mode, so the evaluator holds the restored source without a steer for it; that
   * audit failing is logged, never a reason to lose what was rolled back. Undefined when nothing was rolled back.
   */
  async rewind(toolCallId: string, revision: number, scope: readonly string[]): Promise<RewoundEdit | undefined> {
    const rewindable = this.#rewindable.get(toolCallId);
    if (rewindable === undefined || rewindable.revision !== revision) return undefined;
    this.#rewindable.delete(toolCallId);
    const { preimages, after } = rewindable;
    if (scope.length === 0 || scope.some(path => !preimages.has(path))) return undefined;
    if ([...preimages.values()].some(preimage => preimage.readError !== undefined)) return undefined;
    const secret = [...preimages].find(([, preimage]) => preimage.withheld !== undefined);
    if (secret !== undefined)
      throw new Error(`${secret[0]} is named like a file holding secrets, so Rerail kept no copy of it to put back; the edit stays as it is`);
    const discovery = this.#discovery;
    if (discovery === undefined || discovery.cancelled) return undefined;
    return await this.#enqueue(async () => {
      if (this.#discovery !== discovery || discovery.cancelled) return undefined;
      if (this.#shared(toolCallId, preimages.keys())) return undefined;
      const root = await realpath(this.#cwd);
      const targets: Array<{ location: SourceLocation; preimage: Preimage; current: Source }> = [];
      for (const [path, preimage] of preimages) {
        const location = this.#sourceLocation(root, path);
        const current = await this.#readSource(root, path);
        if (location === undefined || current === undefined || current.readError !== undefined) return undefined;
        if (current.digest !== after.get(path)) return undefined;
        if (current.deleted && preimage.text !== undefined && !await this.#ownsFolder(root, location)) return undefined;
        targets.push({ location, preimage, current });
      }
      let written: typeof targets = [];
      try {
        for (const target of targets) {
          if (target.current.text === target.preimage.text) continue;
          await this.#writeSource(target.location, target.preimage.text);
          written.push(target);
        }
      } catch (error) {
        // Every file or none: what was already put back takes the edit's text again.
        const stuck: typeof targets = [];
        let undoError: unknown;
        for (const target of written) {
          await this.#writeSource(target.location, target.current.text).catch((failure: unknown) => {
            stuck.push(target);
            undoError ??= failure;
          });
        }
        const kept = stuck.length === 0
          ? ""
          : `; ${stuck.map(target => target.location.path).join(", ")} stay rolled back: ${errorMessage(undoError)}`;
        process.stderr.write(`rerail: could not roll back tool call ${toolCallId}: ${errorMessage(error)}${kept}\n`);
        if (stuck.length === 0) return undefined;
        written = stuck;
      }
      try {
        const edits: CodeRuleEdit[] = [];
        for (const { location, current } of written) {
          const source = await this.#readSource(root, location.path);
          this.#touchSource(discovery, location.path);
          if (source === undefined) continue;
          this.#updateObservedSource(source);
          const edit = this.#editFromToolSource(source, current.text === undefined ? {} : { text: current.text });
          if (edit !== undefined) edits.push(edit);
        }
        // Every rolled-back file has its preimage (the edit's text), so nothing is left to recover.
        if (edits.length > 0) await this.#auditEdits(discovery, edits, ["edit", "function"], new Map(), "log");
      } catch (error) {
        process.stderr.write(`rerail: could not audit the rollback of tool call ${toolCallId}: ${errorMessage(error)}\n`);
      }
      return {
        revision: this.#revision,
        restored: written.filter(target => target.preimage.text !== undefined).map(target => target.location.path),
        deleted: written.filter(target => target.preimage.text === undefined).map(target => target.location.path),
      };
    });
  }

  /** Whether a call other than `toolCallId`, begun and unfinished or still rewindable, covers any of `paths`. */
  #shared(toolCallId: string, paths: Iterable<string>): boolean {
    const others = [...this.#pending, ...this.#rewindable].flatMap(([id, edit]) => (id === toolCallId ? [] : [edit.preimages]));
    for (const path of paths) if (others.some(preimages => preimages.has(path))) return true;
    return false;
  }

  /** Whether the folder a file is to be created in lies in the checkout that owns the file. */
  async #ownsFolder(root: string, location: SourceLocation): Promise<boolean> {
    const owner = this.#auditor.sourceRoot === undefined ? root : this.#auditor.sourceRoot(location.path);
    if (owner === undefined) return false;
    const folder = await realpath(dirname(location.absolute)).catch(() => undefined);
    return folder !== undefined && (folder === owner || folder.startsWith(`${owner}${sep}`));
  }

  /**
   * Replaces the file whole with `text`, written to a temporary file beside it and renamed over it, so a failed write
   * leaves the file as it was; a symlink or other non-regular file there is refused, never followed or replaced. An
   * existing file keeps its permissions. Without text the file is removed.
   */
  async #writeSource(location: SourceLocation, text: string | undefined): Promise<void> {
    if (text === undefined) {
      await unlink(location.absolute).catch((error: unknown) => {
        if (!(error instanceof Error && "code" in error && error.code === "ENOENT")) throw error;
      });
      return;
    }
    const existing = await lstat(location.absolute).catch((error: unknown) => {
      if (error instanceof Error && "code" in error && error.code === "ENOENT") return undefined;
      throw error;
    });
    if (existing !== undefined && !existing.isFile()) throw new Error(`${location.path} is not a regular file`);
    const temporary = join(dirname(location.absolute), `.${basename(location.absolute)}.rerail-${randomBytes(6).toString("hex")}.tmp`);
    const handle = await open(temporary, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o666);
    try {
      try {
        if (existing !== undefined) await handle.chmod(existing.mode & 0o7777);
        await handle.writeFile(text, "utf8");
      } finally {
        await handle.close();
      }
      await rename(temporary, location.absolute);
    } catch (error) {
      await unlink(temporary).catch(() => undefined);
      throw error;
    }
  }

  /**
   * Resolves after every requested walk has run, so the checkpoint's violations are delivered with this boundary; a walk
   * that outlasts the boundary's wait is reported pending, and what it finds reaches a later boundary.
   */
  async checkpoint(checkpoint: Extract<CodeRuleCheckpoint, "turn" | "final">): Promise<CodeRuleResponse> {
    const discovery = this.#requireDiscovery();
    await this.#awaitDiscovery(discovery, checkpoint, checkpoint);
    this.#assertDiscovery(discovery);
    return this.#discoveryResponse(discovery);
  }

  async #reconcile(discovery: SourceDiscovery): Promise<readonly CodeRuleResponse[]> {
    const revision = this.#revision;
    this.refreshCodeRules();
    await this.#awaitDiscovery(discovery, "edit", "command");
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
  interrupt(options?: AuditorCloseOptions): void {
    this.#auditor.interrupt?.(options);
  }


  async close(options?: AuditorCloseOptions): Promise<void> {
    const discovery = this.#discovery;
    if (discovery === undefined || discovery.cancelled) return;
    // Cancel before reporting and before joining the operation queue: neither the report nor a blocked filesystem read
    // may keep the walk going.
    discovery.cancelled = true;
    // A walk a boundary already gave up on is abandoned here: what it had not reached is never checked.
    if (discovery.idle !== undefined && discovery.pendingReported) this.#reportDiscoveryPending("close");
    await this.#enqueue(async () => {
      if (this.#discovery !== discovery) return;
      this.#sessionId = undefined;
      this.#pending.clear();
      this.#rewindable.clear();
      this.#commands.clear();
      this.#observed.clear();
      this.#texts.clear();
      this.#textBytes = 0;
      this.#discovery = undefined;
      await this.#auditor.close(options);
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
      if (source.text !== undefined && sensitiveByName(source.path))
        preimages.set(source.path, { withheld: { sha256: source.digest!, bytes: Buffer.byteLength(source.text) } });
      else if (source.text !== undefined) preimages.set(source.path, { text: source.text });
      else if (source.readError !== undefined) preimages.set(source.path, { readError: source.readError });
      else if (source.deleted) preimages.set(source.path, {});
    }
    return preimages;
  }

  /** The tool call's edits, and the digest each source had before it where the call's start captured no preimage. */
  async #readEdits(
    discovery: SourceDiscovery,
    paths: readonly string[],
    preimages: ReadonlyMap<string, Preimage>,
  ): Promise<{ edits: CodeRuleEdit[]; uncaptured: Map<string, string> }> {
    const root = await realpath(this.#cwd);
    const edits: CodeRuleEdit[] = [];
    const uncaptured = new Map<string, string>();
    for (const configuredPath of [...paths].sort(compareDeterministicStrings)) {
      const source = await this.#readSource(root, configuredPath);
      if (source === undefined) continue;
      this.#touchSource(discovery, source.path);
      const preimage = preimages.get(source.path);
      const before = this.#observed.get(source.path)?.digest;
      const edit = this.#readEdit(source, preimage);
      if (edit === undefined) continue;
      edits.push(edit);
      // A secret-like file's text was not captured: its digest finds it again where this session or Git holds it.
      const prior = preimage === undefined ? before : preimage.withheld?.sha256;
      if (prior !== undefined && edit.generatedText !== undefined) uncaptured.set(edit.path, prior);
    }
    return { edits, uncaptured };
  }

  /** Queues a pass for `checkpoint` and resolves once the walk is idle, i.e. every pass requested so far has run. */
  #requestDiscovery(discovery: SourceDiscovery, checkpoint: DiscoveryCheckpoint): Promise<void> {
    if (discovery.cancelled) return Promise.resolve();
    discovery.requested = strongerCheckpoint(checkpoint, discovery.requested);
    // The traversal never occupies #queue. Only completed snapshots join ordered tool revisions.
    discovery.idle ??= this.#runDiscovery(discovery);
    return discovery.idle;
  }

  /** A requested walk, waited for at most `#discoveryWaitMs`; a walk that outlasts it is reported pending at `boundary`. */
  async #awaitDiscovery(
    discovery: SourceDiscovery,
    checkpoint: DiscoveryCheckpoint,
    boundary: Exclude<DiscoveryPendingBoundary, "close">,
  ): Promise<void> {
    const budget = Promise.withResolvers<boolean>();
    const timer = setTimeout(() => budget.resolve(false), this.#discoveryWaitMs);
    try {
      const walked = await Promise.race([this.#requestDiscovery(discovery, checkpoint).then(() => true), budget.promise]);
      if (!walked && !discovery.cancelled) {
        discovery.pendingReported = true;
        this.#reportDiscoveryPending(boundary);
      }
    } finally {
      clearTimeout(timer);
    }
  }

  /** Recording a boundary that answered before its shell-write check is best-effort: it never fails the boundary. */
  #reportDiscoveryPending(boundary: DiscoveryPendingBoundary): void {
    try {
      this.#onDiscoveryPending?.(boundary);
    } catch (error) {
      process.stderr.write(`rerail: policy execution recording failed: ${error instanceof Error ? error.message : String(error)}\n`);
    }
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
      discovery.pendingReported = false;
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
      const uncaptured = new Map<string, string>();
      const edits = this.#discoverEdits(discovery, scan.sources, previous, versions, baseline, uncaptured);
      try {
        if (edits.length > 0)
          await this.#auditEdits(discovery, edits, discoveryCheckpoints(strongerCheckpoint(checkpoint, discovery.requested)), uncaptured);
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

  /** The changes the walk found; `uncaptured` receives the digest each changed source was observed with before. */
  #discoverEdits(
    discovery: SourceDiscovery,
    current: ReadonlyMap<string, Source>,
    previous: ReadonlyMap<string, ObservedSource>,
    versions: ReadonlyMap<string, number>,
    baseline: boolean,
    uncaptured: Map<string, string>,
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
      if (edit === undefined) continue;
      edits.push(edit);
      const before = previous.get(path)?.digest;
      if (edit.previousReadError !== undefined && edit.generatedText !== undefined && before !== undefined && !baseline)
        uncaptured.set(path, before);
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

  /** Folders Git ignores (when the session follows `.gitignore`) are never entered, so nothing below them is observed. */
  async #expandDirectory(walk: SourceWalk, absolute: string): Promise<void> {
    for await (const entry of await opendir(absolute)) {
      if (walk.discovery.cancelled) return;
      if (entry.isSymbolicLink() || (entry.isDirectory() && IGNORED_DIRECTORIES.has(entry.name))) continue;
      const child = join(absolute, entry.name);
      if (entry.isDirectory() && this.#auditor.ignoresPath?.(sessionPath(walk.discovery.root, child), true)) continue;
      if (entry.isDirectory() || entry.isFile()) walk.stack.push(child);
    }
  }

  /** A file Git ignores is not observed at all: no baseline entry, so no later read or edit of it is audited. */
  async #observeRegularFile(walk: SourceWalk, absolute: string, info: Stats): Promise<void> {
    const path = sessionPath(walk.discovery.root, absolute);
    const language = languageOfPath(SOURCE_LANGUAGES, path);
    if (language !== undefined && !this.#auditor.ignoresPath?.(path, false))
      await this.#observeFile(walk, { absolute, path, language }, info);
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
    const path = sessionPath(discovery.root, absolute);
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
    const unchanged = preimage?.withheld === undefined ? preimage?.text === source.text : preimage.withheld.sha256 === source.digest;
    if (source.text !== undefined && unchanged) {
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
      : preimage.withheld !== undefined
        ? { previousReadError: "Rerail keeps no copy of a file named like one holding secrets" }
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

  async #auditEdits(
    discovery: SourceDiscovery,
    captured: CodeRuleEditBatch["edits"],
    checkpoints: CodeRuleCheckpoint[],
    uncaptured: ReadonlyMap<string, string>,
    mode?: AuditMode,
  ): Promise<CodeRuleResponse> {
    const sessionId = this.#requireSession();
    const edits = await this.#recoverPreimages(discovery.root, captured, uncaptured);
    for (const edit of edits) {
      this.#forgetText(edit.path);
      if (edit.generatedText !== undefined) this.#rememberText(edit.path, edit.generatedText);
    }
    const paths = edits.map(edit => edit.path).sort(compareDeterministicStrings);
    this.#revision += 1;
    this.#onRevision?.(sessionId, this.#revision, paths);
    this.#lastResponse = await this.#auditor.auditGeneratedEdits(sessionId, {
      revision: this.#revision, checkpoints, edits, ...(mode === undefined ? {} : { mode }),
    });
    return this.#lastResponse;
  }

  /**
   * Gives each edit `uncaptured` names (made with no preimage captured: a shell write, a tool call whose start was not
   * seen) the source it changed, where that is known exactly: the text this session last saw the file with, else its
   * Git index or `HEAD` blob, each taken only when its SHA-256 is the digest observed before the change. Any other
   * keeps its `previousReadError`, so the generated lines stay unknown rather than guessed.
   */
  async #recoverPreimages(
    root: string,
    edits: CodeRuleEditBatch["edits"],
    uncaptured: ReadonlyMap<string, string>,
  ): Promise<CodeRuleEditBatch["edits"]> {
    if (uncaptured.size === 0) return edits;
    const recovered = new Map<string, string>();
    const unresolved: string[] = [];
    for (const [path, digest] of uncaptured) {
      const text = this.#texts.get(path)?.text;
      if (text !== undefined && createHash("sha256").update(text).digest("hex") === digest) recovered.set(path, text);
      else unresolved.push(path);
    }
    if (unresolved.length > 0) {
      const revisions = unresolved.flatMap(path => [`:./${path}`, `HEAD:./${path}`]);
      // A blob larger than a source could never match its digest: it is skipped, never read into memory.
      const blobs = await readGitBlobs(root, revisions, { maxBlobBytes: MAX_SOURCE_BYTES });
      for (const path of unresolved) {
        const blob = [blobs.get(`:./${path}`), blobs.get(`HEAD:./${path}`)]
          .find(candidate => candidate !== undefined && createHash("sha256").update(candidate).digest("hex") === uncaptured.get(path));
        if (blob !== undefined) recovered.set(path, blob.toString("utf8"));
      }
    }
    return edits.map(edit => {
      const previousText = recovered.get(edit.path);
      if (previousText === undefined) return edit;
      const { previousReadError: _uncaptured, ...rest } = edit;
      return { ...rest, previousText };
    });
  }

  /** Keeps `text` as the source's latest; past `MAX_SESSION_TEXT_BYTES`, the least recently changed sources go. */
  #rememberText(path: string, text: string): void {
    const bytes = Buffer.byteLength(text);
    this.#texts.set(path, { text, bytes });
    this.#textBytes += bytes;
    for (const [oldest, entry] of this.#texts) {
      if (this.#textBytes <= MAX_SESSION_TEXT_BYTES) break;
      this.#texts.delete(oldest);
      this.#textBytes -= entry.bytes;
    }
  }

  #forgetText(path: string): void {
    const entry = this.#texts.get(path);
    if (entry === undefined) return;
    this.#texts.delete(path);
    this.#textBytes -= entry.bytes;
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
