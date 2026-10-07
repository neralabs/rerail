import { findRunningDaemon } from "./daemon-control.js";

import { policySettingsSchema, type PolicyControls, type PolicySettings, type ScanRequest } from "./scan-control-schema.js";

const WAIT_TIMEOUT_MS = 10_000;
const WAIT_INTERVAL_MS = 250;
type Scope = PolicySettings["scope"];
type Mode = PolicySettings["mode"];

export interface ScanCliOptions {
  home: string;
  stdout?: Pick<NodeJS.WriteStream, "write">;
}

interface CommonOptions {
  sessionId: string;
  json: boolean;
}

interface ParsedCommon {
  common: CommonOptions;
  values: Map<string, string>;
  flags: Set<string>;
  paths: string[];
}

function requireValue(args: readonly string[], index: number, option: string): string {
  const value = args[index + 1];
  if (value === undefined || value.startsWith("--")) throw new Error(`${option} requires a value`);
  return value;
}

function settingsScope(value: string | undefined, option: string): Scope | undefined {
  if (value === undefined) return undefined;
  const parsed = policySettingsSchema.shape.scope.safeParse(value);
  if (!parsed.success) throw new Error(`${option} must be repository, touched-files, or ai-output`);
  return parsed.data;
}

function scanScope(value: string | undefined): "repository" | "git-diff" | undefined {
  if (value === undefined) return undefined;
  if (value !== "repository" && value !== "git-diff") throw new Error("--scope must be repository or git-diff");
  return value;
}

function mode(value: string | undefined, option: string): Mode | undefined {
  if (value === undefined) return undefined;
  const parsed = policySettingsSchema.shape.mode.safeParse(value);
  if (!parsed.success) throw new Error(`${option} must be log or resteer`);
  return parsed.data;
}

function parseCommon(args: readonly string[]): ParsedCommon {
  const values = new Map<string, string>();
  const flags = new Set<string>();
  const paths: string[] = [];
  let index = 0;
  while (index < args.length) {
    const argument = args[index]!;
    if (argument === "--paths") {
      index += 1;
      const firstPath = index;
      while (index < args.length && !args[index]!.startsWith("--")) paths.push(args[index++]!);
      if (index === firstPath) throw new Error("--paths requires at least one path");
      continue;
    }
    if (argument === "--json" || argument === "--wait") {
      flags.add(argument);
      index += 1;
      continue;
    }
    if (["--session", "--scope", "--mode"].includes(argument)) {
      values.set(argument, requireValue(args, index, argument));
      index += 2;
      continue;
    }
    throw new Error(`Unknown option: ${argument}`);
  }
  const sessionId = values.get("--session");
  if (sessionId === undefined) throw new Error("--session is required; Rerail never guesses a latest session");
  return {
    common: { sessionId, json: flags.has("--json") },
    values,
    flags,
    paths,
  };
}

/** The Rerail home's daemon serves every session recorded in its database, whichever repository ran it. */
async function targetDaemon(options: ScanCliOptions): Promise<URL> {
  const daemon = await findRunningDaemon(options.home);
  if (daemon === undefined) throw new Error("Rerail is not running. Start it with rerail start.");
  return new URL(daemon.state.url);
}

async function api<T>(base: URL, path: string, method: "GET" | "PATCH" | "POST", body?: unknown): Promise<T> {
  const response = await fetch(new URL(path, base), {
    signal: AbortSignal.timeout(WAIT_TIMEOUT_MS),
    method,
    ...(body === undefined ? {} : { headers: { "content-type": "application/json" }, body: JSON.stringify(body) }),
  });
  const payload = await response.json() as T | { error?: string; message?: string };
  if (!response.ok) {
    const error = payload as { error?: string; message?: string };
    throw new Error(error.message === undefined ? `Rerail API request failed (${response.status})` : `${error.error ?? "request-failed"}: ${error.message}`);
  }
  return payload as T;
}

function print(options: ScanCliOptions, json: boolean, value: unknown, text: string): void {
  (options.stdout ?? process.stdout).write(json ? `${JSON.stringify(value)}\n` : `${text}\n`);
}

function describeScan(scan: ScanRequest): string {
  const result = scan.runIds.length === 0 ? "no policy runs yet" : `runs: ${scan.runIds.join(", ")}`;
  const error = scan.error === null ? "" : `: ${scan.error}`;
  return `Scan ${scan.id}: ${scan.status} (${scan.scope}/${scan.mode}; ${result})${error}`;
}

async function waitForScan(base: URL, sessionId: string, id: string): Promise<ScanRequest> {
  const deadline = Date.now() + WAIT_TIMEOUT_MS;
  for (;;) {
    const { scans } = await api<{ scans: ScanRequest[] }>(base, `/api/sessions/${encodeURIComponent(sessionId)}/scans`, "GET");
    const scan = scans.find(item => item.id === id);
    if (scan === undefined) throw new Error(`Scan ${id} disappeared before it could be observed`);
    if (!["queued", "running"].includes(scan.status)) return scan;
    if (Date.now() >= deadline) return scan;
    await new Promise<void>(resolveWait => setTimeout(resolveWait, WAIT_INTERVAL_MS));
  }
}

async function runSettingsCommand(
  base: URL,
  session: string,
  parsed: ParsedCommon,
  options: ScanCliOptions,
): Promise<number> {
  if (parsed.flags.has("--wait") || parsed.paths.length !== 0) throw new Error("--wait and --paths apply only to rerail scan");
  const selectedScope = settingsScope(parsed.values.get("--scope"), "--scope");
  const selectedMode = mode(parsed.values.get("--mode"), "--mode");
  const patch = {
    ...(selectedScope === undefined ? {} : { scope: selectedScope }),
    ...(selectedMode === undefined ? {} : { mode: selectedMode }),
  };
  const controls = Object.keys(patch).length === 0
    ? await api<PolicyControls>(base, `/api/sessions/${session}/policy-settings`, "GET")
    : await api<PolicyControls>(base, `/api/sessions/${session}/policy-settings`, "PATCH", patch);
  const status = controls.active ? controls.delivery : "inactive";
  print(options, parsed.common.json, controls, `Ongoing settings for ${controls.workspaceRoot}: ${controls.settings.scope}/${controls.settings.mode} (${status})`);
  return 0;
}

async function runExplicitScanCommand(
  base: URL,
  session: string,
  parsed: ParsedCommon,
  options: ScanCliOptions,
): Promise<number> {
  if (parsed.values.has("--mode")) throw new Error("--mode applies only to rerail settings; one-shot scans are log-only");
  if (parsed.paths.length > 0) throw new Error("--paths is not supported; use --scope repository or --scope git-diff");
  const selectedScope = scanScope(parsed.values.get("--scope"));
  const input = selectedScope === undefined ? {} : { scope: selectedScope };
  const created = await api<ScanRequest>(base, `/api/sessions/${session}/scans`, "POST", input);
  const result = parsed.flags.has("--wait") ? await waitForScan(base, parsed.common.sessionId, created.id) : created;
  print(options, parsed.common.json, result, describeScan(result));
  if (parsed.flags.has("--wait") && (result.status === "queued" || result.status === "running")) return 2;
  return result.status === "failed" || result.status === "interrupted" ? 1 : 0;
}

export async function runScanCommand(command: "scan" | "settings", args: readonly string[], options: ScanCliOptions): Promise<number> {
  const parsed = parseCommon(args);
  const base = await targetDaemon(options);
  const session = encodeURIComponent(parsed.common.sessionId);
  return command === "settings"
    ? runSettingsCommand(base, session, parsed, options)
    : runExplicitScanCommand(base, session, parsed, options);
}
