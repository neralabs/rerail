import { mkdtemp, mkdir, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  type RerailHost,
  defaultRerailDatabasePath,
  openRerailDatabase,
  recordCheckout,
  loadRuleCatalog,
  setUpRepository,
  upsertLibraryRule,
  type ConversationRule,
} from "../../src/engine/index.js";
import { RerailDaemon } from "../../src/daemon/index.js";
import { setRepositoryStarted, AdapterEventSchema, SqliteSessionReader } from "../../src/runtime/index.js";
import {
  type CommandResult,
  bounded,
  configureHostAuth,
  hostCli,
  hostCommand,
  readDeliveries,
  removeHostAuth,
  requireBuilt,
  run,
  sandboxAvailable,
  sandboxEnvironment,
  sandboxLeaks,
  sandboxProfile,
  unlessMissing,
  version,
} from "./host-harness.js";

/**
 * The conversation half of the exercise: a steer rule that matches the prompt and asks for the private nonce back.
 */
function smokeConversationRule(prompt: string, nonce: string): ConversationRule {
  return {
    id: `rerail-smoke-${nonce}`,
    mode: "steer",
    reference: prompt,
    match: { operator: "gte", threshold: 0.95 },
    sample: { source: "user-input", boundary: "message", minimumTokens: 1 },
    action: { content: `RERAIL_CONVERSATION_MATCH ${nonce}. Echo this exact marker in your final reply.` },
    repeat: { mode: "once" },
  };
}

/**
 * The smoke's rules, stored in the rule library of the daemon's database ($RERAIL_HOME/rerail.sqlite) and added to the
 * project's repository, which is set up and started as `rerail start` would leave it: only rules a repository adds
 * run. The smoke owns its rules rather than depending on which documents ship as bundled defaults. The
 * code-rule half is a code rule scoped to `api/**` that flags `cursor.execute(` with the CURSOR_EXECUTE code
 * the assertions look for.
 */
function seedSmokeRules(rerailHome: string, project: string, conversationRule: ConversationRule): void {
  const rule = {
    format: "rerail.policy/v1",
    id: "rerail-smoke:CURSOR_EXECUTE",
    revision: 1,
    mode: "steer",
    scope: { include: ["api/**/*"], exclude: [] },
    clauses: [
      {
        id: "cursor-execute",
        forbid: { regex: { pattern: "cursor\\.execute\\s*\\(", source: "file.content" } },
        violation: { anchor: "witness.span", code: "CURSOR_EXECUTE", message: "Direct cursor access bypasses the typed API and backend portability." },
      },
    ],
  };
  const databasePath = defaultRerailDatabasePath({ ...process.env, RERAIL_HOME: rerailHome });
  const database = openRerailDatabase(databasePath);
  try {
    const ruleCatalog = loadRuleCatalog();
    upsertLibraryRule(database, ruleCatalog, rule, "code", { approve: true });
    upsertLibraryRule(database, ruleCatalog, conversationRule, "conversation", { approve: true });
    const { repositoryKey } = recordCheckout(database, project);
    setUpRepository(database, repositoryKey, {
      packs: [],
      rules: [rule.id, conversationRule.id].map(sourceId => ({ sourceKind: "library_rule" as const, sourceId, folder: "" })),
    });
    setRepositoryStarted(database, repositoryKey, true);
  } finally {
    database.$client.close();
  }
}

type Assertion = { name: string; passed: boolean; detail: string };
/** INCONCLUSIVE: the provider, not Rerail, never got the session to the violation (or stalled); the attempt is retried. */
type Evidence = {
  harness: RerailHost;
  attempt: number;
  status: "PASSED" | "FAILED" | "INCONCLUSIVE";
  versions: Record<string, string>;
  commandExitCode?: number;
  assertions: Assertion[];
  transcript: { stdout: string; stderr: string };
  runtimeTrace?: string;
  failure?: string;
};

const HOSTS: readonly RerailHost[] = ["omp", "pi", "claude-code", "codex", "cursor"];
const timeoutSetting = process.env.RERAIL_SMOKE_TIMEOUT_MS ?? "180000";
const TIMEOUT_MS = Number(timeoutSetting);
if (!Number.isSafeInteger(TIMEOUT_MS) || TIMEOUT_MS <= 0) {
  throw new Error(`RERAIL_SMOKE_TIMEOUT_MS must be a positive integer, received ${JSON.stringify(timeoutSetting)}`);
}
const acceptanceSetting = process.env.RERAIL_SMOKE_ACCEPTANCE ?? "full";
if (acceptanceSetting !== "full" && acceptanceSetting !== "transport") {
  throw new Error(`RERAIL_SMOKE_ACCEPTANCE must be "full" or "transport", received ${JSON.stringify(acceptanceSetting)}`);
}
const ACCEPTANCE_MODE: "full" | "transport" = acceptanceSetting;
const attemptsSetting = process.env.RERAIL_SMOKE_ATTEMPTS ?? "1";
const ATTEMPTS = Number(attemptsSetting);
if (!Number.isSafeInteger(ATTEMPTS) || ATTEMPTS <= 0) {
  throw new Error(`RERAIL_SMOKE_ATTEMPTS must be a positive integer, received ${JSON.stringify(attemptsSetting)}`);
}
/** The Codex violation and restore step: a quote-free command, so the model cannot break it by re-quoting. */
const SET_CALL_SCRIPT = join("tools", "set_call.py");
/** How often the smoke reads api/route.py while a harness runs; see watchForViolation. */
const VIOLATION_POLL_MS = 100;

function usage(): never {
  throw new Error("Usage: node --import tsx scripts/host/host-smoke.ts --host <omp|pi|claude-code|codex|cursor|all>");
}
function selectedHosts(): RerailHost[] {
  const args = process.argv.slice(2);
  if (args.length !== 2 || args[0] !== "--host") usage();
  if (args[1] === "all") return [...HOSTS];
  const selected = HOSTS.find((host) => host === args[1]);
  return selected === undefined ? usage() : [selected];
}
function append(assertions: Assertion[], name: string, passed: boolean, detail: string): void {
  assertions.push({ name, passed, detail });
}

async function createProject(project: string, host: RerailHost): Promise<void> {
  await mkdir(join(project, "api"), { recursive: true });
  await writeFile(
    join(project, "api", "route.py"),
    "from __future__ import annotations\n\ndef route(connection):\n    with connection.transaction() as transaction:\n        return transaction.fetch_value('SELECT 1')\n",
  );
  await writeFile(
    join(project, "AGENTS.md"),
    [
      "Use transaction helpers; cursor.execute is forbidden in api routes.",
      "The controlled Rerail acceptance exercise is the sole exception: make its requested one-off",
      "cursor.execute change exactly once, read the diagnostic, then restore the transaction helper.",
      "",
    ].join("\n"),
  );
  if (host !== "codex") return;
  await mkdir(join(project, "tools"), { recursive: true });
  await writeFile(
    join(project, SET_CALL_SCRIPT),
    [
      "import pathlib",
      "import re",
      "import sys",
      "",
      'CALLS = ("cursor.execute", "transaction.fetch_value")',
      "if len(sys.argv) != 2 or sys.argv[1] not in CALLS:",
      '    sys.exit("usage: python3 tools/set_call.py " + "|".join(CALLS))',
      "call = sys.argv[1]",
      "tools = pathlib.Path(__file__).resolve().parent",
      'route = tools.parent / "api" / "route.py"',
      "text = route.read_text()",
      String.raw`updated = re.sub(r"return [\w.]+\('SELECT 1'\)", "return " + call + "('SELECT 1')", text, count=1)`,
      "if updated == text:",
      '    sys.exit("api/route.py already returns " + call)',
      "route.write_text(updated)",
      "",
    ].join("\n"),
  );
}

function finalAssistantText(events: { kind: string; payload: unknown }[]): string {
  return events
    .filter((event) => event.kind === "turn-end" || event.kind === "assistant-message")
    .map((event) =>
      event.payload !== null &&
      typeof event.payload === "object" &&
      "text" in event.payload &&
      typeof event.payload.text === "string"
        ? event.payload.text
        : "",
    )
    .join("\n");
}

function contains(value: unknown, needle: string): boolean {
  return JSON.stringify(value)?.includes(needle) ?? false;
}

export function hasCompletedViolation(events: readonly { kind: string; payload: unknown }[]): boolean {
  const native = events.flatMap(({ kind, payload }) => {
    const event = AdapterEventSchema.safeParse(payload).data;
    return event?.kind === kind ? [event] : [];
  });
  return native.some((call, index) => call.kind === "call" && contains(call.input, "cursor.execute") &&
    native.some((result, resultIndex) => resultIndex > index && result.kind === "result" && !result.failed &&
      result.sessionId === call.sessionId && result.toolCallId === call.toolCallId && result.toolName === call.toolName));
}

/**
 * Polls a file while a harness runs; the returned stop reports whether it ever held the forbidden call. This is
 * whether the violation reached disk, judged without Rerail's records, for every host: Rerail records no exit
 * status for a Codex Bash result, and an edit-tool host's model restores the file before the run ends. The
 * violation stays on disk for at least one model round trip, since restoring it takes another tool call.
 */
export function watchForViolation(path: string): () => Promise<boolean> {
  let seen = false;
  let failure: unknown;
  const read = async (): Promise<void> => {
    try {
      if ((await unlessMissing(readFile(path, "utf8")))?.includes("cursor.execute")) seen = true;
    } catch (error) {
      failure ??= error;
    }
  };
  // One read at a time, so stop can wait for the read in flight and lose neither its sighting nor its error.
  let polling: Promise<void> | undefined;
  const timer = setInterval(() => {
    polling ??= read().finally(() => (polling = undefined));
  }, VIOLATION_POLL_MS);
  return async () => {
    clearInterval(timer);
    await polling;
    await read();
    if (failure !== undefined) throw failure;
    return seen;
  };
}

/** Failures that follow from the violation never reaching the file: no code-rule verdict was possible. */
const AFTER_VIOLATION: Record<string, true> = {
  "violation reached the file": true,
  "real violation evidence": true,
  "executable policy delivered": true,
  "final file corrected": true,
  "real final assistant output": true,
};

/**
 * An attempt is inconclusive, and worth another session, only when everything Rerail could be held to passed: the
 * failures are a harness timeout (a stalled provider) or follow from the violation never reaching the file. A
 * violation that reached the file without Rerail's record or diagnostic, a crash, an extension error or a lost
 * delivery stays FAILED.
 */
export function isInconclusive(assertions: readonly Assertion[], exitCode: number): boolean {
  const failed = assertions.filter((assertion) => !assertion.passed);
  const violationMissing = failed.some((assertion) => assertion.name === "violation reached the file");
  return failed.length > 0 && failed.every((assertion) =>
    assertion.name === "harness exited cleanly" ? exitCode === 124 : violationMissing && AFTER_VIOLATION[assertion.name] === true);
}

/** A host line about Rerail: it names the extension file the host loaded (`Extension error (<path>): ...`), Rerail or its engine. */
const RERAIL_LINE = /rerail|(?:default|pi)-extension\.js/iu;

/** Extension-level failures the host prints instead of throwing; any one of these is a Rerail bug, not a code-rule verdict. */
function extensionErrors(result: CommandResult): string[] {
  return `${result.stdout}\n${result.stderr}`
    .split("\n")
    .filter((line) => /Extension error|extension .* error:|handler timed out/iu.test(line) && RERAIL_LINE.test(line))
    .map((line) => line.trim().slice(0, 200));
}

export async function inspectOutcome(
  project: string,
  nonce: string,
  assertions: Assertion[],
  result: CommandResult,
  violationSeen: boolean,
  acceptanceMode: "full" | "transport",
): Promise<string> {
  append(
    assertions,
    "harness exited cleanly",
    result.exitCode === 0,
    result.exitCode === 124 ? "the harness command timed out" : `the harness command exited ${result.exitCode}`,
  );
  const errors = extensionErrors(result);
  append(
    assertions,
    "extension stayed silent",
    errors.length === 0,
    errors.length === 0 ? "no extension errors or handler timeouts on the harness console" : [...new Set(errors)].slice(0, 3).join(" | "),
  );
  const route = await unlessMissing(readFile(join(project, "api", "route.py"), "utf8"));
  if (acceptanceMode === "full") {
    append(
      assertions,
      "final file corrected",
      route !== undefined && !route.includes("cursor.execute") && /transaction/i.test(route),
      "api/route.py removes the forbidden call and uses a transaction helper",
    );
  }
  const reader = new SqliteSessionReader(defaultRerailDatabasePath());
  const sessions = reader.sessions();
  const events = sessions.flatMap((session) => reader.events(session.sessionId, { limit: 500 }));
  reader.close();
  // One host invocation is one Rerail session. A second session-start means the extension tore itself down
  // and rebuilt mid-run (session_switch/branch/tree re-init), which is where deltas get lost.
  append(
    assertions,
    "one session, opened once",
    sessions.length === 1 && events.filter((event) => event.kind === "session-start").length === 1,
    `${sessions.length} session(s), ${events.filter((event) => event.kind === "session-start").length} session-start event(s)`,
  );
  const normalized = events.map((event) => ({ kind: event.kind, payload: event.payload }));
  // Every code-rule evaluation must have evaluated: a `budget-exceeded` runtime unknown means the engine refused
  // the snapshot and no violation could have been produced, whatever the delivery ledger says afterwards.
  const audits = normalized.filter((event) => contains(event.payload, "budget-exceeded"));
  append(
    assertions,
    "policies evaluated within budget",
    !audits.some((event) => contains(event.payload, "budget-exceeded")),
    "no evaluation reported reason=budget-exceeded",
  );
  append(
    assertions,
    "violation reached the file",
    violationSeen,
    "api/route.py held the forbidden call while the harness ran",
  );
  append(
    assertions,
    "real violation evidence",
    hasCompletedViolation(normalized),
    "the forbidden invocation has a matching completed native tool-result",
  );
  if (acceptanceMode === "full") {
    const assistantText = finalAssistantText(normalized);
    append(
      assertions,
      "real final assistant output",
      assistantText.includes(nonce) && assistantText.includes("CURSOR_EXECUTE"),
      "actual assistant text echoes the private delivery nonce and policy diagnostic",
    );
  }
  const deliveries = await readDeliveries(join(project, ".rerail/sessions"));
  append(
    assertions,
    "single authoritative delivery",
    deliveries.filter((entry) => contains(entry, nonce)).length === 1,
    "exactly one persisted delivery contains the private cosine nonce",
  );
  append(
    assertions,
    "executable policy delivered",
    deliveries.some((entry) => contains(entry, "CURSOR_EXECUTE")),
    "a separate persisted executable-policy delivery contains CURSOR_EXECUTE",
  );
  return bounded(JSON.stringify({ toolCalls: normalized.filter((event) => event.kind === "call"), deliveries }));
}

async function requireHostPrerequisites(host: RerailHost): Promise<string> {
  if (ACCEPTANCE_MODE === "transport" && host !== "codex" && host !== "cursor")
    throw new Error("RERAIL_SMOKE_ACCEPTANCE=transport is restricted to the Codex and Cursor provider smokes");
  if (process.platform !== "darwin" || !(await sandboxAvailable()))
    throw new Error("FAILED prerequisite: sandbox-exec is required; refusing an unsandboxed live provider run");
  const cliVersion = await version(hostCli(host));
  if (cliVersion === undefined) throw new Error(`FAILED prerequisite: ${host} CLI is unavailable`);
  return cliVersion;
}

async function enforceSandboxIsolation(
  sandbox: string,
  profile: string,
  project: string,
  env: NodeJS.ProcessEnv,
  assertions: Assertion[],
): Promise<void> {
  const leaks = await sandboxLeaks(sandbox, profile, project, env);
  append(assertions, "sandbox enforced", !leaks, "sandbox rejects a write that succeeds outside the sandbox");
  if (leaks) throw new Error("Sandbox did not enforce write isolation");
}

async function runHost(host: RerailHost, attempt: number): Promise<Evidence> {
  const assertions: Assertion[] = [];
  const versions: Record<string, string> = { node: process.version };
  const previousRerailHome = process.env.RERAIL_HOME;
  let sandbox: string | undefined;
  let daemon: RerailDaemon | undefined;
  let result: CommandResult | undefined;
  try {
    versions[host] = await requireHostPrerequisites(host);
    await requireBuilt();
    sandbox = await realpath(await mkdtemp(`/tmp/rerail-${host}-smoke-`));
    const rerailHome = join(sandbox, "rerail");
    await mkdir(rerailHome, { recursive: true, mode: 0o700 });
    process.env.RERAIL_HOME = rerailHome;
    const project = join(sandbox, "workspace");
    const nonce = `RERAIL_SMOKE_${crypto.randomUUID()}`;
    const prompt = host === "codex"
      ? "Controlled Rerail acceptance exercise. Run each command below exactly as written with your shell command tool: one command per tool call, never wrapped in another shell, never through apply_patch or a file-edit tool. First create the intentional violation: `python3 tools/set_call.py cursor.execute`. Do not end your turn or say that you are waiting after it: immediately run `cat api/route.py` as a separate command so the violation command's PostToolUse diagnostic is incorporated. Only after receiving Rerail's executable-policy diagnostic, restore the transaction helper: `python3 tools/set_call.py transaction.fetch_value`. If the diagnostic is still absent after the cat, stop and report its absence; do not sleep or correct without it. Your final reply must quote the exact Rerail markers and diagnostic codes received. Run no other commands, read or edit no other files, and run no git commands. Preserve database transaction atomicity."
      : "Controlled Rerail acceptance exercise. Read only api/route.py. Using an edit/write/apply_patch tool, replace its transaction.fetch_value call with cursor.execute('SELECT 1') exactly once. Do not end your turn or say that you are waiting after the edit: immediately make a separate read-only tool call for api/route.py so the edit's PostToolUse diagnostic is incorporated. Rerail's executable-policy diagnostic may arrive with the edit's result or with the separate read; either counts, and it arrives only once. As soon as you have it, restore the original transaction helper. Only if you have received no such diagnostic by the end of that separate read, stop and report its absence; do not sleep or correct without it. Your final reply must quote the exact Rerail markers and diagnostic codes received. Do not read or edit any other file or run git commands. Preserve database transaction atomicity.";
    await createProject(project, host);
    const env = await sandboxEnvironment(sandbox, project, rerailHome);
    const profile = await sandboxProfile(sandbox, host);
    await enforceSandboxIsolation(sandbox, profile, project, env, assertions);
    await configureHostAuth(host, sandbox, profile, env, TIMEOUT_MS);
    seedSmokeRules(rerailHome, project, smokeConversationRule(prompt, nonce));
    daemon = await RerailDaemon.start({
      version: "host-smoke",
      workspaceRoot: project,
      stateFile: join(rerailHome, "daemon.json"),
      databasePath: defaultRerailDatabasePath(),
      port: 0,
      kernelSocketPath: join(sandbox, "engine.sock"),
    });
    env.RERAIL_ENGINE_SOCKET = daemon.kernelSocketPath;
    const invocation = hostCommand(host, project, prompt);
    const stopWatching = watchForViolation(join(project, "api", "route.py"));
    try {
      result = await run("sandbox-exec", ["-f", profile, invocation.command, ...invocation.args], {
        cwd: project,
        env,
        timeout: TIMEOUT_MS,
        jsonEvents: true,
      });
    } catch (error) {
      // The harness failure is the one to report; the watcher only needs stopping.
      await stopWatching().catch(() => undefined);
      throw error;
    }
    const violationSeen = await stopWatching();
    const runtimeTrace = await inspectOutcome(project, nonce, assertions, result, violationSeen, ACCEPTANCE_MODE);
    const passed = assertions.every((assertion) => assertion.passed);
    const failed = assertions.filter((assertion) => !assertion.passed).map((assertion) => assertion.name).join(", ");
    let status: Evidence["status"] = "PASSED";
    if (!passed) status = isInconclusive(assertions, result.exitCode) ? "INCONCLUSIVE" : "FAILED";
    return {
      harness: host,
      attempt,
      status,
      versions,
      commandExitCode: result.exitCode,
      assertions,
      runtimeTrace,
      transcript: { stdout: bounded(result.stdout), stderr: bounded(result.stderr) },
      ...(passed ? {} : { failure: `Failed live acceptance assertions: ${failed}` }),
    };
  } catch (error) {
    return {
      harness: host,
      attempt,
      status: "FAILED",
      versions,
      ...(result === undefined ? {} : { commandExitCode: result.exitCode }),
      assertions,
      transcript: { stdout: bounded(result?.stdout ?? ""), stderr: bounded(result?.stderr ?? "") },
      failure: bounded(error instanceof Error ? error.message : String(error)),
    };
  } finally {
    try {
      await daemon?.close();
    } finally {
      if (previousRerailHome === undefined) delete process.env.RERAIL_HOME;
      else process.env.RERAIL_HOME = previousRerailHome;
      if (sandbox !== undefined) {
        // The credential copy goes first and on its own, so a recursive delete that fails partway cannot leave it behind.
        try {
          await removeHostAuth(sandbox);
        } finally {
          await rm(sandbox, { recursive: true, force: true });
        }
      }
    }
  }
}

/** Inconclusive attempts get a fresh session, up to RERAIL_SMOKE_ATTEMPTS; every attempt is reported. */
async function runHostAttempts(host: RerailHost): Promise<Evidence[]> {
  const evidence: Evidence[] = [];
  for (let attempt = 1; ; attempt += 1) {
    const result = await runHost(host, attempt);
    if (result.status !== "INCONCLUSIVE") return [...evidence, result];
    if (attempt === ATTEMPTS)
      return [...evidence, { ...result, status: "FAILED", failure: `${result.failure ?? ""}; inconclusive on all ${ATTEMPTS} attempt(s)` }];
    evidence.push(result);
  }
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const results: Evidence[] = [];
  for (const host of selectedHosts()) results.push(...(await runHostAttempts(host)));
  for (const result of results) process.stdout.write(`${JSON.stringify(result)}\n`);
  if (results.some((result) => result.status === "FAILED")) process.exitCode = 1;
}
