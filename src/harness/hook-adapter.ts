import { createHash, randomUUID } from "node:crypto";
import { readFileSync, realpathSync, rmSync } from "node:fs";
import { readFile, rm, stat, utimes, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import type { Writable } from "node:stream";
import { isConflictNotice, isSteerRule, type AdapterDelivery, type ConflictNotice, type RerailHost, type SteerRule } from "../engine/adapter-contract.js";
import { RULE_MODES, steers } from "../engine/catalog/rule-mode.js";
import { processState } from "../engine/process-liveness.js";
import { asLateSteer } from "../engine/code-rule/rerail-steer.js";
import { heldOnWindows, nameTaken, removeWithRetry } from "../engine/file-replace.js";
import { REQUIRED_NODE_MAJOR } from "../engine/package-root.js";
import { scrubSecrets } from "../engine/secret-scrub.js";
import { makeLedgerFolder, readLedgerFile, writeLedgerFile } from "../engine/storage/ledger-paths.js";
import { exitWhenParentGone, type ParentWatch } from "./parent-watch.js";

export type HookInput = Record<string, unknown>;
export type HookOutput = Record<string, unknown>;
export type HookHost = Extract<RerailHost, "claude-code" | "codex" | "cursor">;
export type EventTable = Readonly<Record<string, true>>;

type ReceiptRecorder = (receiptId: string, ruleIds: readonly string[]) => void | Promise<void>;
interface PreparedGuidance {
  ruleIds: readonly string[];
  record?: ReceiptRecorder;
}

const preparedGuidance = new WeakMap<HookOutput, PreparedGuidance>();

/**
 * `output` as a steer whose receipt is recorded once it is written, crediting `ruleIds`. A note that names no rule (a
 * Merge Assist note) shows the person no steer line, so it is not counted as a steer: nothing is recorded for it.
 */
export function prepareGuidanceOutput(output: HookOutput, ruleIds: readonly string[]): HookOutput {
  if (ruleIds.length > 0) preparedGuidance.set(output, { ruleIds });
  return output;
}

export function bindSteerReceipt(output: HookOutput, record: ReceiptRecorder): HookOutput {
  const prepared = preparedGuidance.get(output);
  if (prepared !== undefined) prepared.record = record;
  return output;
}

function carriesText(delivery: AdapterDelivery): boolean {
  return steers(delivery.requested) && delivery.content.trim() !== "";
}

export function deliveryText(deliveries: readonly AdapterDelivery[]): string | undefined {
  const content = deliveries
    .filter(carriesText)
    .map(delivery => delivery.content.trim())
    .join("\n\n");
  return content === "" ? undefined : content;
}

/** The rules behind the deliveries `deliveryText` rendered, in order: what the person's lines are built from. */
export function deliveryRules(deliveries: readonly AdapterDelivery[]): SteerRule[] {
  return deliveries.filter(carriesText).flatMap(delivery => [...delivery.rules]);
}

/** The conflicts behind the deliveries `deliveryText` rendered, in order: the person's line for each, after the rules'. */
export function deliveryConflicts(deliveries: readonly AdapterDelivery[]): ConflictNotice[] {
  return deliveries.filter(carriesText).flatMap(delivery => (delivery.conflict === undefined ? [] : [delivery.conflict]));
}

/** Distinct rule ids behind the deliveries `deliveryText` rendered, so the hook's receipt can credit them. */
export function deliveryRuleIds(deliveries: readonly AdapterDelivery[]): string[] {
  return [...new Set(deliveries.filter(carriesText).flatMap(delivery => [...delivery.ruleIds]))];
}

function sessionFile(storageRoot: string, host: HookHost, kind: "pending" | "locks", sessionId: string, extension: string): string {
  return join(storageRoot, `${host}-${kind}`, `${createHash("sha256").update(sessionId).digest("hex")}.${extension}`);
}

export function pendingPath(storageRoot: string, host: HookHost, sessionId: string): string {
  return sessionFile(storageRoot, host, "pending", sessionId, "json");
}

function errno(error: unknown, code: string): boolean {
  return error instanceof Error && "code" in error && error.code === code;
}

/** A delivery as a pending file holds it: its rules, conflict and marks still to be checked. */
type HeldDeliveryShape = Omit<AdapterDelivery, "rules" | "conflict"> & { rules?: unknown; conflict?: unknown; late?: unknown; shown?: unknown };

/** Whether `requested`, read back from a file, is a rule mode that steers. */
function isSteerRequest(requested: unknown): requested is AdapterDelivery["requested"] {
  return RULE_MODES.includes(requested as AdapterDelivery["requested"]) && steers(requested as AdapterDelivery["requested"]);
}

/** Whether `value`, read back from a pending file, is a held steer: its session, mechanism, text and rule ids, and a mode that steers. */
function isHeldDeliveryShape(value: unknown): value is HeldDeliveryShape {
  if (value === null || typeof value !== "object") return false;
  const delivery = value as Partial<Record<keyof AdapterDelivery, unknown>>;
  return [delivery.sessionId, delivery.mechanism, delivery.content].every(field => typeof field === "string")
    && isSteerRequest(delivery.requested)
    && Array.isArray(delivery.ruleIds) && delivery.ruleIds.every(id => typeof id === "string");
}

/**
 * The steers held at `path`; a symlink on the way to it under `.rerail` is refused (`readLedgerFile`). A steer held
 * at `Stop` is `late`; the 0.1.2 hook marked one `shown`, which a session updated mid-turn still holds.
 */
export async function readPending(path: string): Promise<HeldDelivery[]> {
  const text = await readLedgerFile(path, dirname(path));
  if (text === undefined) return [];
  const parsed = JSON.parse(text) as unknown;
  if (!Array.isArray(parsed)) throw new Error("Pending delivery state must be an array");
  return parsed
    .filter(isHeldDeliveryShape)
    .map(({ late, shown, conflict, ...delivery }) => ({
      ...delivery,
      // A steer held before deliveries carried their rules, or with rules that are not, shows no line.
      rules: Array.isArray(delivery.rules) ? delivery.rules.filter(isSteerRule) : [],
      // A conflict that is not one shows no line; its note still reaches the agent.
      ...(isConflictNotice(conflict) ? { conflict } : {}),
      ...(late === true || shown === true ? { late: true as const } : {}),
    }));
}

/**
 * Deliveries as a pending file holds them: each one's content scrubbed of secrets (`scrubSecrets`), as the session
 * ledger stores a delivery. The caller keeps the original, so a steer delivered now carries its text whole; one held
 * for a later event goes out as it was written here.
 */
function pendingForDisk(deliveries: readonly HeldDelivery[]): HeldDelivery[] {
  return deliveries.map(delivery => {
    const content = scrubSecrets(delivery.content);
    return content === delivery.content ? delivery : { ...delivery, content };
  });
}

export async function persistPending(path: string, deliveries: readonly HeldDelivery[]): Promise<void> {
  if (deliveries.length === 0) return;
  await writeLedgerFile(path, dirname(path), JSON.stringify(pendingForDisk([...(await readPending(path)), ...deliveries])));
}

export async function takePending(path: string): Promise<HeldDelivery[]> {
  const deliveries = await readPending(path);
  await rm(path, { force: true });
  return deliveries;
}

export async function replacePending(path: string, deliveries: readonly HeldDelivery[]): Promise<void> {
  if (deliveries.length === 0) {
    await rm(path, { force: true });
    return;
  }
  await writeLedgerFile(path, dirname(path), JSON.stringify(pendingForDisk(deliveries)));
}

/**
 * A pending file's text as `rerail scrub` rewrites it: each entry's `content` scrubbed of secrets, every other part of
 * the file as it was; undefined when nothing changes or the text is not a JSON array.
 */
export function scrubbedPendingText(text: string): string | undefined {
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return undefined;
  }
  if (!Array.isArray(parsed)) return undefined;
  let changed = false;
  const scrubbed = parsed.map((entry: unknown) => {
    if (entry === null || typeof entry !== "object" || !("content" in entry) || typeof entry.content !== "string") return entry;
    const content = scrubSecrets(entry.content);
    if (content === entry.content) return entry;
    changed = true;
    return { ...entry, content };
  });
  return changed ? JSON.stringify(scrubbed) : undefined;
}

export async function routeDeliveries(
  path: string,
  hook: string,
  deliveries: readonly AdapterDelivery[],
  contextEvents: EventTable,
): Promise<HeldDelivery[]> {
  const pendingSteers = [...await readPending(path), ...deliveries.filter(delivery => steers(delivery.requested))];
  const carried = contextEvents[hook] === true;
  await replacePending(path, carried ? [] : pendingSteers);
  return carried ? pendingSteers : [];
}

/** A steer held for the next context event; `late` once a `Stop` that did not carry it held it past the end of a turn. */
export type HeldDelivery = AdapterDelivery & { readonly late?: true };

/** The person's lines for `deliveries`, as the host draws them, in one `systemMessage`; nothing without a line. */
export type SteerLines = (deliveries: readonly AdapterDelivery[]) => { systemMessage: string } | Record<string, never>;

/**
 * Holds a `Stop`'s steers for the next context event, each marked `late`, where that `Stop` does not carry them: in
 * Codex, which records a `Stop` block's reason as the user's turn, and in Claude Code once Rerail has already kept the
 * turn going. Nothing goes out at that `Stop`, the person's lines included: they show, and the steer counts, when the
 * block reaches the agent.
 */
export async function holdStopSteers(path: string, deliveries: readonly AdapterDelivery[]): Promise<void> {
  await persistPending(path, deliveries.filter(delivery => steers(delivery.requested)).map(delivery => ({ ...delivery, late: true as const })));
}

/**
 * Steers reach the model as `additionalContext` at a context event, after `note`, and the person as one line per rule,
 * in the same output, which records the receipt once written (D5): what the person sees, what the agent is told and
 * what is counted agree. A steer held past the end of a turn (`late`) is led as a late steer.
 */
export function steerHookOutput(
  hook: string,
  deliveries: readonly HeldDelivery[],
  contextEvents: EventTable,
  lines: SteerLines,
  note?: string,
): HookOutput {
  if (contextEvents[hook] !== true) return {};
  const content = deliveryText(deliveries.map(delivery => (delivery.late === true ? { ...delivery, content: asLateSteer(delivery.content) } : delivery)));
  const context = [note, content].filter(text => text !== undefined).join("\n\n");
  if (context === "") return {};
  const output = { ...lines(deliveries), hookSpecificOutput: { hookEventName: hook, additionalContext: context } };
  return content === undefined ? output : prepareGuidanceOutput(output, deliveryRuleIds(deliveries));
}

interface LockOwner {
  pid: number;
  token: string;
}

/** A lock file as a waiter found it: its text, the holder it names if it names one, and when it last beat. */
interface FoundLock {
  text: string;
  owner: LockOwner | undefined;
  beat: number;
}

async function foundLock(path: string): Promise<FoundLock | undefined> {
  try {
    const text = await readFile(path, "utf8");
    const { mtimeMs } = await stat(path);
    let owner: LockOwner | undefined;
    try {
      const parsed = JSON.parse(text) as Partial<LockOwner>;
      if (Number.isInteger(parsed.pid) && typeof parsed.token === "string" && parsed.token !== "") owner = parsed as LockOwner;
    } catch {
      // A holder made the file and has not written itself into it yet, or ended before it did.
    }
    return { text, owner, beat: mtimeMs };
  } catch {
    return undefined;
  }
}

/** How often a hook holding its session's lock touches the lock file, the beat that says it still runs. */
const HEARTBEAT_MS = 1_000;

/** How many beats in a row a lock's holder may miss before a waiter takes the lock from it. */
const SILENT_BEATS = 10;

/**
 * Judges, over one waiter's tries, whether the hook a lock file names has abandoned it: its process is gone, or the
 * file has not beaten (`hold`) for `silentMs` while this waiter watched. Its pid alone never says the holder still runs:
 * Windows hands a freed pid to the next process at once, and a hook that ended holding its lock, whose pid a shell or
 * node the agent started next then took, held the session's lock for as long as that process ran, while each of the
 * session's hooks waited 30 s for it and lost its event. The silence is timed on this waiter's clock from the beat it
 * last saw change, so no other process's clock, nor a file server's, enters into it.
 */
function abandonmentJudge(silentMs: number): (found: FoundLock) => boolean {
  let watched: { text: string; beat: number; since: number } | undefined;
  return found => {
    if (found.owner !== undefined && processState(found.owner.pid) === "gone") return true;
    if (watched?.text !== found.text || watched.beat !== found.beat) {
      watched = { text: found.text, beat: found.beat, since: Date.now() };
      return false;
    }
    return Date.now() - watched.since >= silentMs;
  };
}

/** Removes the lock file at `path` when its holder has abandoned it (`abandoned`); true when the next try may take it. */
async function reclaimAbandoned(path: string, platform: NodeJS.Platform, abandoned: (found: FoundLock) => boolean): Promise<boolean> {
  const found = await foundLock(path);
  return found !== undefined && abandoned(found) && await removeAbandoned(path, found.text, platform);
}

/**
 * Removes the abandoned lock file at `path` while it still reads `text`; true when the next try may take the lock. On
 * Windows a lock file another hook is already removing refuses this removal (EPERM, EACCES, EBUSY): false then, so the
 * hook waits.
 */
async function removeAbandoned(path: string, text: string, platform: NodeJS.Platform): Promise<boolean> {
  try {
    if (await readFile(path, "utf8") !== text) return false;
    await rm(path);
    return true;
  } catch (error) {
    if (errno(error, "ENOENT")) return true;
    if (heldOnWindows(error, platform)) return false;
    throw error;
  }
}

/** How long a hook waits for its session's lock before it gives up on its event. */
const LOCK_WAIT_MS = 30_000;

interface LockTiming {
  platform: NodeJS.Platform;
  heartbeatMs: number;
}

/**
 * Keeps the lock file this hook made at `path`, whose text is `owner`, showing that its holder runs: touches it every
 * `heartbeatMs` (`abandonmentJudge`), and removes it if the process exits holding it, as `exitWhenParentGone` ends a
 * hook mid-event. Returns the function that stops both.
 */
function hold(path: string, owner: string, heartbeatMs: number): () => void {
  const timer = setInterval(() => {
    const now = new Date();
    utimes(path, now, now).catch(() => undefined);
  }, heartbeatMs);
  timer.unref();
  const removeOnExit = (): void => {
    try {
      if (readFileSync(path, "utf8") === owner) rmSync(path, { force: true });
    } catch {
      // Its process gone and its beat stopped, the next hook takes the lock all the same.
    }
  };
  process.on("exit", removeOnExit);
  return () => {
    clearInterval(timer);
    process.off("exit", removeOnExit);
  };
}

/** Takes the lock at `path` for `owner` and holds it (`hold`); returns the function that stops holding it. */
async function acquireLock(path: string, owner: string, label: string, timing: LockTiming): Promise<() => void> {
  const deadline = Date.now() + LOCK_WAIT_MS;
  const abandoned = abandonmentJudge(timing.heartbeatMs * SILENT_BEATS);
  while (!await created(path, owner, timing.platform)) {
    if (await reclaimAbandoned(path, timing.platform, abandoned)) continue;
    if (Date.now() >= deadline) throw new Error(`Timed out waiting for session lock: ${label}`);
    await delay(10);
  }
  return hold(path, owner, timing.heartbeatMs);
}

/** Makes the lock file at `path`; false while another hook holds the lock, or on Windows is still deleting its file. */
async function created(path: string, owner: string, platform: NodeJS.Platform): Promise<boolean> {
  try {
    await writeFile(path, owner, { flag: "wx", mode: 0o600 });
    return true;
  } catch (error) {
    if (nameTaken(error, platform)) return false;
    throw error;
  }
}

/**
 * Removes the lock file at `path` while it is still `owner`'s. On Windows the file this hook made a moment ago may be
 * held by a scanner, which refuses its removal (EPERM, EBUSY): the removal waits that out (`removeWithRetry`), where it
 * threw at once and cost the hook its event or its answer.
 */
async function releaseLock(path: string, owner: string, platform: NodeJS.Platform): Promise<void> {
  try {
    if (await readFile(path, "utf8") !== owner) return;
  } catch (error) {
    if (errno(error, "ENOENT")) return;
    throw error;
  }
  await removeWithRetry(path, { platform });
}

/**
 * The lock a hook holds for its session, which guards the session's files: its held steers, and Cursor's transcript
 * sidecar. `released` lets go of it while `work` runs, for work that touches no session file, such as waiting for the
 * daemon's answer, and takes it again after. `kept` holds it while `work` runs, through every `released` within: for
 * work that reads a session file before such a wait and writes it after, as Cursor's transcript ingest does.
 */
export interface SessionLock {
  released<T>(work: () => Promise<T>): Promise<T>;
  kept<T>(work: () => Promise<T>): Promise<T>;
}

export interface SessionLockOptions {
  /** The platform whose errors for a lock file being deleted count as the lock being taken; this process's by default. */
  platform?: NodeJS.Platform;
  /** How often the holder touches its lock file (`HEARTBEAT_MS` by default); every hook of a session must use the same. */
  heartbeatMs?: number;
}

export async function withSessionLock<T>(
  storageRoot: string,
  host: HookHost,
  sessionId: string,
  operation: (lock: SessionLock) => Promise<T>,
  options: SessionLockOptions = {},
): Promise<T> {
  const timing = { platform: options.platform ?? process.platform, heartbeatMs: options.heartbeatMs ?? HEARTBEAT_MS };
  return withLockFile(sessionFile(storageRoot, host, "locks", sessionId, "lock"), sessionId, operation, timing);
}

/**
 * Runs `operation` holding the lock a hook holds for the session whose held steers are `pendingFile`
 * (`<storageRoot>/<host>-pending/<digest>.json`, `pendingPath`): the same `<storageRoot>/<host>-locks/<digest>.lock`,
 * so rewriting the file races no hook of that session.
 */
export async function withPendingFileLock<T>(pendingFile: string, operation: () => Promise<T>): Promise<T> {
  const folder = basename(dirname(pendingFile));
  const host = /^(.+)-pending$/.exec(folder)?.[1];
  const digest = /^([0-9a-f]{64})\.json$/.exec(basename(pendingFile))?.[1];
  if (host === undefined || digest === undefined) throw new Error(`Not a pending steer file: ${pendingFile}`);
  return withLockFile(join(dirname(dirname(pendingFile)), `${host}-locks`, `${digest}.lock`), digest, operation, {
    platform: process.platform,
    heartbeatMs: HEARTBEAT_MS,
  });
}

async function withLockFile<T>(
  path: string,
  label: string,
  operation: (lock: SessionLock) => Promise<T>,
  timing: LockTiming,
): Promise<T> {
  await makeLedgerFolder(dirname(path));
  const owner = JSON.stringify({ pid: process.pid, token: randomUUID() } satisfies LockOwner);
  let stopHolding = await acquireLock(path, owner, label, timing);
  let held = true;
  let keeping = 0;
  const lock: SessionLock = {
    async kept(work) {
      keeping += 1;
      try {
        return await work();
      } finally {
        keeping -= 1;
      }
    },
    async released(work) {
      if (!held || keeping > 0) return await work();
      stopHolding();
      try {
        await releaseLock(path, owner, timing.platform);
      } catch {
        // The lock file would not go: the work runs holding the lock, where its event was lost.
        stopHolding = hold(path, owner, timing.heartbeatMs);
        return await work();
      }
      held = false;
      try {
        return await work();
      } finally {
        stopHolding = await acquireLock(path, owner, label, timing);
        held = true;
      }
    },
  };
  try {
    return await operation(lock);
  } finally {
    if (held) {
      stopHolding();
      // The hook's answer, or its failure, stands: the steers it carries are already gone from the pending file. A lock
      // file left behind names a holder that is about to end and beats no more, which the next hook takes.
      await releaseLock(path, owner, timing.platform).catch(() => undefined);
    }
  }
}

export interface HookStreams {
  stdin: AsyncIterable<Buffer | string>;
  stdout: {
    write(chunk: string, callback: (error?: Error | null) => void): unknown;
  } & Partial<Pick<Writable, "once" | "removeListener">>;
  stderr: { write(chunk: string): unknown };
}

export async function emitHookOutput(output: HookOutput, stdout: HookStreams["stdout"] = process.stdout): Promise<void> {
  await emitSerializedHookOutput(output, `${JSON.stringify(output)}\n`, stdout);
}

async function emitSerializedHookOutput(output: HookOutput, serialized: string, stdout: HookStreams["stdout"]): Promise<void> {
  const written = Promise.withResolvers<void>();
  stdout.once?.("error", written.reject);
  try {
    stdout.write(serialized, error => {
      if (error != null) written.reject(error);
      else written.resolve();
    });
    await written.promise;
  } finally {
    stdout.removeListener?.("error", written.reject);
  }
  const prepared = preparedGuidance.get(output);
  await prepared?.record?.(randomUUID(), prepared.ruleIds);
}

export async function readStdin(stdin: AsyncIterable<Buffer | string> = process.stdin): Promise<string> {
  const chunks: Buffer[] = [];
  for await (const chunk of stdin) chunks.push(Buffer.from(chunk));
  return Buffer.concat(chunks).toString("utf8");
}

function launchedDirectly(moduleUrl: string): boolean {
  const launchedPath = process.argv[1];
  return launchedPath !== undefined && realpathSync(fileURLToPath(moduleUrl)) === realpathSync(resolve(launchedPath));
}

/**
 * A hook the recorded-first fallback started on an older `node` from PATH, once the node set-up recorded was gone,
 * does nothing and says so in one line on stderr, which the coding agents keep out of the session: a hook must never
 * break it.
 */
export async function serveHook(
  run: (input: HookInput) => Promise<HookOutput>,
  streams: HookStreams = process,
  nodeVersion: string = process.versions.node,
): Promise<void> {
  let output: HookOutput = {};
  let serialized = "{}\n";
  try {
    const parsed = JSON.parse(await readStdin(streams.stdin)) as unknown;
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("Hook stdin must be a JSON object");
    if (Number.parseInt(nodeVersion, 10) >= REQUIRED_NODE_MAJOR) {
      output = await run(parsed as HookInput);
      serialized = `${JSON.stringify(output)}\n`;
    } else {
      streams.stderr.write(
        `rerail hook: Rerail needs Node ${REQUIRED_NODE_MAJOR} or newer and this is Node ${nodeVersion}, so it is off in this session. `
          + `Put Node ${REQUIRED_NODE_MAJOR} or newer first on PATH, then run rerail install --refresh.\n`,
      );
    }
  } catch (error) {
    output = {};
    streams.stderr.write(`rerail hook: ${error instanceof Error ? error.message : String(error)}\n`);
  }
  try {
    await emitSerializedHookOutput(output, serialized, streams.stdout);
  } catch (error) {
    streams.stderr.write(`rerail hook: ${error instanceof Error ? error.message : String(error)}\n`);
  }
}

/** What a hook entry uses in place of this process's own: the parent watch's facts and the hook's streams. */
export interface HookEntryOptions {
  watch?: ParentWatch;
  streams?: HookStreams;
}

/**
 * A coding agent's hook process, the one call each hook host's entry module makes: when `moduleUrl` is the module node
 * was started with, it watches the process that started it (`exitWhenParentGone`, which on Windows ends a hook the
 * agent gave up on, so it stops holding its session's lock) and then serves the hook. Imported, as by tests and the
 * package index, it does nothing. The watch runs until the process ends: its timer never keeps the process alive, and
 * a handle still open after the answer then ends with the agent too.
 */
export async function serveHookEntry(
  moduleUrl: string,
  run: (input: HookInput) => Promise<HookOutput>,
  options: HookEntryOptions = {},
): Promise<void> {
  if (!launchedDirectly(moduleUrl)) return;
  exitWhenParentGone(options.watch);
  await serveHook(run, options.streams);
}
