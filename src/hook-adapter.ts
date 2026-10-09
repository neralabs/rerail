import { createHash, randomUUID } from "node:crypto";
import { realpathSync } from "node:fs";
import { mkdir, readFile, rename, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";
import type { Writable } from "node:stream";
import type { AdapterDelivery, RerailHost } from "./adapter-contract.js";
import { RULE_MODES, steers } from "./rule-mode.js";

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

export async function readPending(path: string): Promise<AdapterDelivery[]> {
  try {
    const parsed = JSON.parse(await readFile(path, "utf8")) as unknown;
    if (!Array.isArray(parsed)) throw new Error("Pending delivery state must be an array");
    return parsed
      .filter((value): value is AdapterDelivery => {
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
      });
  } catch (error) {
    if (errno(error, "ENOENT")) return [];
    throw error;
  }
}

export async function persistPending(path: string, deliveries: readonly AdapterDelivery[]): Promise<void> {
  if (deliveries.length === 0) return;
  await mkdir(join(path, ".."), { recursive: true });
  const temporary = `${path}.${process.pid}`;
  await writeFile(temporary, JSON.stringify([...(await readPending(path)), ...deliveries]), { mode: 0o600 });
  await rename(temporary, path);
}

export async function takePending(path: string): Promise<AdapterDelivery[]> {
  const deliveries = await readPending(path);
  await rm(path, { force: true });
  return deliveries;
}

export async function replacePending(path: string, deliveries: readonly AdapterDelivery[]): Promise<void> {
  if (deliveries.length === 0) {
    await rm(path, { force: true });
    return;
  }
  await mkdir(join(path, ".."), { recursive: true });
  const temporary = `${path}.${process.pid}`;
  await writeFile(temporary, JSON.stringify(deliveries), { mode: 0o600 });
  await rename(temporary, path);
}

export async function routeDeliveries(
  path: string,
  hook: string,
  deliveries: readonly AdapterDelivery[],
  contextEvents: EventTable,
): Promise<AdapterDelivery[]> {
  const pendingSteers = [...await readPending(path), ...deliveries.filter(delivery => steers(delivery.requested))];
  const carried = contextEvents[hook] === true;
  await replacePending(path, carried ? [] : pendingSteers);
  return carried ? pendingSteers : [];
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

async function acquireLock(path: string, serializedOwner: string, deadline: number, sessionId: string): Promise<void> {
  for (;;) {
    try {
      await writeFile(path, serializedOwner, { flag: "wx", mode: 0o600 });
      return;
    } catch (error) {
      if (!errno(error, "EEXIST")) throw error;
      if (await reclaimDeadOwner(path)) continue;
      if (Date.now() >= deadline) throw new Error(`Timed out waiting for session lock: ${sessionId}`);
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
  const path = sessionFile(storageRoot, host, "locks", sessionId, "lock");
  await mkdir(join(path, ".."), { recursive: true });
  const owner: LockOwner = { pid: process.pid, token: randomUUID() };
  const serializedOwner = JSON.stringify(owner);
  await acquireLock(path, serializedOwner, Date.now() + 30_000, sessionId);
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

export async function serveHook(run: (input: HookInput) => Promise<HookOutput>, streams: HookStreams = process): Promise<void> {
  let output: HookOutput = {};
  let serialized = "{}\n";
  try {
    const parsed = JSON.parse(await readStdin(streams.stdin)) as unknown;
    if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) throw new Error("Hook stdin must be a JSON object");
    output = await run(parsed as HookInput);
    serialized = `${JSON.stringify(output)}\n`;
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
