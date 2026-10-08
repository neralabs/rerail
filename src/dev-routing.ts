import { existsSync, readFileSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import type { AdapterDelivery, AdapterEvent, AdapterSession } from "./adapter-contract.js";
import { decodeRerailEventResult } from "./event-protocol.js";
import type { EngineRuntimeConfiguration } from "./engine-service.js";

const INSTANCE_FILE = ".nera/instance.env";

export interface RerailDevInstance {
  workspaceRoot: string;
  url: URL;
  publicUrl: URL;
  stateFile: string;
}

export type RerailWorkspaceRoute =
  | { kind: "workspace"; instance: RerailDevInstance }
  | { kind: "disabled"; workspaceRoot: string; configurationFile: string }
  | { kind: "host" };

export interface RerailDevLauncher {
  command: string;
  args: string[];
}

/** Checkout launcher invocation: an absolute shell running the checkout's own ops.sh, so PATH never chooses the executable. */
export function rerailDevLauncher(instance: Pick<RerailDevInstance, "workspaceRoot">, args: readonly string[]): RerailDevLauncher {
  return { command: "/bin/bash", args: [resolve(instance.workspaceRoot, "scripts", "ops.sh"), "rerail-command", ...args] };
}
function parseInstanceEnvironment(text: string, path: string): ReadonlyMap<string, string> {
  const values = new Map<string, string>();
  for (const line of text.split(/\r?\n/u)) {
    if (line === "" || line.startsWith("#")) continue;
    const match = /^([A-Z][A-Z0-9_]*)=(.*)$/u.exec(line);
    const key = match?.[1];
    const raw = match?.[2];
    if (key === undefined || raw === undefined || values.has(key)) {
      throw new Error(`Invalid Rerail development instance file: ${path}`);
    }
    const quote = raw[0];
    if ((quote === '"' || quote === "'") && !raw.endsWith(quote)) {
      throw new Error(`Unterminated value in Rerail development instance file: ${path}`);
    }
    values.set(key, quote === '"' || quote === "'" ? raw.slice(1, -1) : raw);
  }
  return values;
}

function loopbackUrl(value: string | undefined, path: string): URL {
  if (value === undefined) throw new Error(`Rerail development instance lacks NERA_RERAIL_PUBLIC_URL: ${path}`);
  let url: URL;
  try {
    url = new URL(value);
  } catch {
    throw new Error(`Invalid NERA_RERAIL_PUBLIC_URL in ${path}`);
  }
  if (url.protocol !== "http:" || (url.hostname !== "127.0.0.1" && url.hostname !== "[::1]")) {
    throw new Error(`NERA_RERAIL_PUBLIC_URL must use an explicit loopback HTTP address in ${path}`);
  }
  if (url.pathname !== "/" || url.search !== "" || url.hash !== "" || url.username !== "" || url.password !== "") {
    throw new Error(`NERA_RERAIL_PUBLIC_URL must be an origin URL in ${path}`);
  }
  return url;
}


async function readWorkspaceRoute(directory: string, path: string): Promise<RerailWorkspaceRoute> {
  const values = parseInstanceEnvironment(await readFile(path, "utf8"), path);
  const enabled = values.get("NERA_RERAIL_ENABLED");
  if (enabled === undefined) return { kind: "host" };
  if (enabled === "false") return { kind: "disabled", workspaceRoot: directory, configurationFile: path };
  if (enabled !== "true") throw new Error(`Invalid NERA_RERAIL_ENABLED in ${path}`);
  const publicOrigin = loopbackUrl(values.get("NERA_RERAIL_PUBLIC_URL"), path);
  const url = process.env.RERAIL_WORKSPACE === directory && process.env.RERAIL_INTERNAL_PORT !== undefined
    ? loopbackUrl(`http://127.0.0.1:${process.env.RERAIL_INTERNAL_PORT}`, path) : publicOrigin;
  return {
    kind: "workspace",
    instance: {
      workspaceRoot: directory,
      url,
      publicUrl: publicOrigin,
      stateFile: resolve(directory, ".nera/rerail-daemon.json"),
    },
  };
}

/**
 * Selects the nearest repository-operations workspace without collapsing an explicit opt-out
 * into the host-installed fallback used by ordinary and pre-Rerail checkouts.
 */
export async function resolveRerailWorkspaceRoute(cwd: string): Promise<RerailWorkspaceRoute> {
  let directory = resolve(cwd);
  for (;;) {
    const path = resolve(directory, INSTANCE_FILE);
    if (existsSync(path)) return await readWorkspaceRoute(directory, path);
    const parent = dirname(directory);
    if (parent === directory) return { kind: "host" };
    directory = parent;
  }
}

/**
 * Whether the nearest repository-operations workspace explicitly runs its own Rerail container
 * (`NERA_RERAIL_ENABLED=true`). Synchronous, for the start check every hook makes first.
 */
export function devWorkspaceEnabled(cwd: string): boolean {
  for (let directory = resolve(cwd); ; directory = dirname(directory)) {
    const path = resolve(directory, INSTANCE_FILE);
    if (existsSync(path)) return parseInstanceEnvironment(readFileSync(path, "utf8"), path).get("NERA_RERAIL_ENABLED") === "true";
    if (dirname(directory) === directory) return false;
  }
}

/** Posts to the checkout-local loopback bridge; it never consults a shared daemon. */
async function postRerailDev(instance: RerailDevInstance, path: string, body: unknown, label: string): Promise<string> {
  const response = await fetch(new URL(path, instance.url), {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(30_000),
  });
  const text = await response.text();
  if (!response.ok) throw new Error(`Rerail development ${label} request failed (${response.status}): ${text}`);
  return text;
}

export async function ingestRerailDevEvent(
  instance: RerailDevInstance,
  event: AdapterEvent,
  configuration: EngineRuntimeConfiguration,
): Promise<readonly AdapterDelivery[]> {
  const body = { event, configuration: { ...configuration, workspaceRoot: instance.workspaceRoot } };
  const result = decodeRerailEventResult(await postRerailDev(instance, "/kernel/events", body, "kernel"));
  if (result.eventId !== event.eventId) throw new Error("Rerail development kernel response event identity mismatch");
  return result.deliveries;
}

/** The checkout daemon alone writes its database, so a confirmed hook steer is persisted through it. */
export async function recordRerailDevReceipt(
  instance: RerailDevInstance,
  session: AdapterSession,
  receiptId: string,
  ruleIds: readonly string[] = [],
): Promise<void> {
  await postRerailDev(instance, "/kernel/steering-receipts", { session, receiptId, ruleIds: [...ruleIds] }, "receipt");
}
