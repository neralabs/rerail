import { createHash, randomUUID } from "node:crypto";
import { realpathSync } from "node:fs";
import { readFile, rm, writeFile } from "node:fs/promises";
import { basename, dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import type { Writable } from "node:stream";
import { isSteerRule, type AdapterDelivery, type ConflictNotice, type RerailHost, type SteerRule } from "../engine/adapter-contract.js";
import { RULE_MODES, steers } from "../engine/catalog/rule-mode.js";
import { asLateSteer } from "../engine/code-rule/rerail-steer.js";
import { REQUIRED_NODE_MAJOR } from "../engine/package-root.js";
import { scrubSecrets } from "../engine/secret-scrub.js";
import { makeLedgerFolder, readLedgerFile, writeLedgerFile } from "../engine/storage/ledger-paths.js";

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

export function prepareGuidanceOutput(output: HookOutput, ruleIds: readonly string[] = []): HookOutput {
  preparedGuidance.set(output, { ruleIds });
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
    .filter((value): value is Omit<AdapterDelivery, "rules"> & { rules?: unknown; late?: unknown; shown?: unknown } => {
      if (value === null || typeof value !== "object") return false;
      const delivery = value as Partial<AdapterDelivery>;
      return typeof delivery.sessionId === "string"
        && delivery.requested !== undefined
        && RULE_MODES.includes(delivery.requested)
        && steers(delivery.requested)
        && typeof delivery.mechanism === "string"
        && typeof delivery.content === "string"
        && Array.isArray(delivery.ruleIds)
        && delivery.ruleIds.every(id => typeof id === "string");
    })
    .map(({ late, shown, ...delivery }) => ({
      ...delivery,
      // A steer held before deliveries carried their rules, or with rules that are not, shows no line.
      rules: Array.isArray(delivery.rules) ? delivery.rules.filter(isSteerRule) : [],
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
  // A note that names no rule (a Merge Assist note) shows the person no steer line, so it is not counted as a steer.
  const ruleIds = deliveryRuleIds(deliveries);
  return content === undefined || ruleIds.length === 0 ? output : prepareGuidanceOutput(output, ruleIds);
}

interface LockOwner {
  pid: number;
  token: string;
}

async function reclaimDeadOwner(path: string): Promise<boolean> {
  let serializedOwner: string;
  let owner: LockOwner;
  try {
    serializedOwner = await readFile(path, "utf8");
    const parsed = JSON.parse(serializedOwner) as Partial<LockOwner>;
    if (!Number.isInteger(parsed.pid) || typeof parsed.token !== "string" || parsed.token === "") return false;
    owner = parsed as LockOwner;
  } catch {
    return false;
  }
  try {
    process.kill(owner.pid, 0);
    return false;
  } catch (error) {
    if (!errno(error, "ESRCH")) return false;
  }
  try {
    if (await readFile(path, "utf8") !== serializedOwner) return false;
    await rm(path);
    return true;
  } catch (error) {
    if (errno(error, "ENOENT")) return true;
    throw error;
  }
}

async function acquireLock(path: string, serializedOwner: string, deadline: number, label: string): Promise<void> {
  for (;;) {
    try {
      await writeFile(path, serializedOwner, { flag: "wx", mode: 0o600 });
      return;
    } catch (error) {
      if (!errno(error, "EEXIST")) throw error;
      if (await reclaimDeadOwner(path)) continue;
      if (Date.now() >= deadline) throw new Error(`Timed out waiting for session lock: ${label}`);
      await delay(10);
    }
  }
}

async function releaseLock(path: string, serializedOwner: string): Promise<void> {
  try {
    if (await readFile(path, "utf8") === serializedOwner) await rm(path, { force: true });
  } catch (error) {
    if (!errno(error, "ENOENT")) throw error;
  }
}

export async function withSessionLock<T>(
  storageRoot: string,
  host: HookHost,
  sessionId: string,
  operation: () => Promise<T>,
): Promise<T> {
  return withLockFile(sessionFile(storageRoot, host, "locks", sessionId, "lock"), sessionId, operation);
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
  return withLockFile(join(dirname(dirname(pendingFile)), `${host}-locks`, `${digest}.lock`), digest, operation);
}

async function withLockFile<T>(path: string, label: string, operation: () => Promise<T>): Promise<T> {
  await makeLedgerFolder(dirname(path));
  const owner: LockOwner = { pid: process.pid, token: randomUUID() };
  const serializedOwner = JSON.stringify(owner);
  await acquireLock(path, serializedOwner, Date.now() + 30_000, label);
  try {
    const result = await operation();
    await releaseLock(path, serializedOwner);
    return result;
  } catch (error) {
    try {
      await releaseLock(path, serializedOwner);
    } catch {
      // Preserve the operation failure; a stale lock is reclaimable by the next process.
    }
    throw error;
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

export function launchedDirectly(moduleUrl: string): boolean {
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
