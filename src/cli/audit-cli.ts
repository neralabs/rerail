import { daemonEndpoint, findRunningDaemon, type DaemonEndpoint } from "../runtime/daemon-control.js";

import {
  auditSettingsPatchSchema, auditSettingsSchema, type AuditControls, type AuditSettings, type AuditSettingsPatch, type AuditRequest,
} from "../engine/audit/audit-control-schema.js";

const WAIT_TIMEOUT_MS = 10_000;
const WAIT_INTERVAL_MS = 250;
type Scope = AuditSettings["scope"];
type Mode = AuditSettings["mode"];
type SteerStrategy = AuditSettings["steerStrategy"];

export interface AuditCliOptions {
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
  const parsed = auditSettingsSchema.shape.scope.safeParse(value);
  if (!parsed.success) throw new Error(`${option} must be repository, touched-files, or ai-output`);
  return parsed.data;
}

function auditRequestScope(value: string | undefined): "repository" | "git-diff" | undefined {
  if (value === undefined) return undefined;
  if (value !== "repository" && value !== "git-diff") throw new Error("--scope must be repository or git-diff");
  return value;
}

function mode(value: string | undefined, option: string): Mode | undefined {
  if (value === undefined) return undefined;
  const parsed = auditSettingsSchema.shape.mode.safeParse(value);
  if (!parsed.success) throw new Error(`${option} must be log or steer`);
  return parsed.data;
}

function steerStrategy(value: string | undefined, option: string): SteerStrategy | undefined {
  if (value === undefined) return undefined;
  const parsed = auditSettingsPatchSchema.shape.steerStrategy.safeParse(value);
  if (!parsed.success || parsed.data === undefined) throw new Error(`${option} must be message, rewind, or subagent`);
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
    if (["--session", "--scope", "--mode", "--steer-strategy"].includes(argument)) {
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
async function targetDaemon(options: AuditCliOptions): Promise<DaemonEndpoint> {
  const daemon = await findRunningDaemon(options.home);
  if (daemon === undefined) throw new Error("Rerail is not running. Start it with rerail daemon start.");
  return daemonEndpoint(daemon.state);
}

async function api<T>(base: DaemonEndpoint, path: string, method: "GET" | "PATCH" | "POST", body?: unknown): Promise<T> {
  const response = await fetch(new URL(path, base.url), {
    signal: AbortSignal.timeout(WAIT_TIMEOUT_MS),
    method,
    headers: { ...base.headers, ...(body === undefined ? {} : { "content-type": "application/json" }) },
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
  const payload = await response.json() as T | { error?: string; message?: string };
  if (!response.ok) {
    const error = payload as { error?: string; message?: string };
    throw new Error(error.message === undefined ? `Rerail API request failed (${response.status})` : `${error.error ?? "request-failed"}: ${error.message}`);
  }
  return payload as T;
}

function print(options: AuditCliOptions, json: boolean, value: unknown, text: string): void {
  (options.stdout ?? process.stdout).write(json ? `${JSON.stringify(value)}\n` : `${text}\n`);
}

function describeAuditRequest(request: AuditRequest): string {
  const result = request.auditIds.length === 0 ? "no policy runs yet" : `runs: ${request.auditIds.join(", ")}`;
  const error = request.error === null ? "" : `: ${request.error}`;
  return `Scan ${request.id}: ${request.status} (${request.scope}/${request.mode}; ${result})${error}`;
}

async function waitForAuditRequest(base: DaemonEndpoint, sessionId: string, id: string): Promise<AuditRequest> {
  const deadline = Date.now() + WAIT_TIMEOUT_MS;
  for (;;) {
    const { auditRequests } = await api<{ auditRequests: AuditRequest[] }>(base, `/api/sessions/${encodeURIComponent(sessionId)}/audit-requests`, "GET");
    const request = auditRequests.find(item => item.id === id);
    if (request === undefined) throw new Error(`Scan ${id} disappeared before it could be observed`);
    if (!["queued", "running"].includes(request.status)) return request;
    if (Date.now() >= deadline) return request;
    await new Promise<void>(resolveWait => setTimeout(resolveWait, WAIT_INTERVAL_MS));
  }
}

/** The settings `rerail settings` changes; checked before Rerail is reached, so a wrong value fails on its own. */
function settingsPatch(parsed: ParsedCommon): AuditSettingsPatch {
  if (parsed.flags.has("--wait") || parsed.paths.length !== 0) throw new Error("--wait and --paths apply only to rerail scan");
  const selectedScope = settingsScope(parsed.values.get("--scope"), "--scope");
  const selectedMode = mode(parsed.values.get("--mode"), "--mode");
  const selectedStrategy = steerStrategy(parsed.values.get("--steer-strategy"), "--steer-strategy");
  return {
    ...(selectedScope === undefined ? {} : { scope: selectedScope }),
    ...(selectedMode === undefined ? {} : { mode: selectedMode }),
    ...(selectedStrategy === undefined ? {} : { steerStrategy: selectedStrategy }),
  };
}

async function runSettingsCommand(
  base: DaemonEndpoint,
  session: string,
  patch: AuditSettingsPatch,
  json: boolean,
  options: AuditCliOptions,
): Promise<number> {
  const controls = Object.keys(patch).length === 0
    ? await api<AuditControls>(base, `/api/sessions/${session}/audit-settings`, "GET")
    : await api<AuditControls>(base, `/api/sessions/${session}/audit-settings`, "PATCH", patch);
  const status = controls.active ? controls.delivery : "inactive";
  const { scope, mode: runMode, steerStrategy: strategy } = controls.settings;
  print(options, json, controls, `Ongoing settings for ${controls.workspaceRoot}: ${scope}/${runMode}, steer strategy ${strategy} (${status})`);
  return 0;
}

/** Internal testing: `rerail scan`, an explicit full-repository or git-diff audit of one session's checkout. */
async function runAuditRequestCommand(
  base: DaemonEndpoint,
  session: string,
  parsed: ParsedCommon,
  options: AuditCliOptions,
): Promise<number> {
  if (parsed.values.has("--mode")) throw new Error("--mode applies only to rerail settings; one-shot scans are log-only");
  if (parsed.values.has("--steer-strategy")) throw new Error("--steer-strategy applies only to rerail settings");
  if (parsed.paths.length > 0) throw new Error("--paths is not supported; use --scope repository or --scope git-diff");
  const selectedScope = auditRequestScope(parsed.values.get("--scope"));
  const input = selectedScope === undefined ? {} : { scope: selectedScope };
  const created = await api<AuditRequest>(base, `/api/sessions/${session}/audit-requests`, "POST", input);
  const result = parsed.flags.has("--wait") ? await waitForAuditRequest(base, parsed.common.sessionId, created.id) : created;
  print(options, parsed.common.json, result, describeAuditRequest(result));
  if (parsed.flags.has("--wait") && (result.status === "queued" || result.status === "running")) return 2;
  return result.status === "failed" || result.status === "interrupted" ? 1 : 0;
}

export async function runAuditCommand(command: "scan" | "settings", args: readonly string[], options: AuditCliOptions): Promise<number> {
  const parsed = parseCommon(args);
  const patch = command === "settings" ? settingsPatch(parsed) : undefined;
  const base = await targetDaemon(options);
  const session = encodeURIComponent(parsed.common.sessionId);
  return patch !== undefined
    ? runSettingsCommand(base, session, patch, parsed.common.json, options)
    : runAuditRequestCommand(base, session, parsed, options);
}
