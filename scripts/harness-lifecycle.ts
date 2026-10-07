/**
 * End-to-end proof that Rerail rule guidance reaches a REAL agent session, run against the real `omp`
 * or `pi` binary in print mode. CI runs both harnesses concurrently with repository provider keys;
 * local runs use the harness on PATH with the user's own HOME configuration and credentials.
 *
 *   npm run test:e2e:lifecycle                       # omp, the user's default model
 *   npm run test:e2e:lifecycle:pi                    # pi, the user's default model
 *   node --import tsx scripts/harness-lifecycle.ts pi -- --provider <provider> --model <model>
 *                                                    # anything after `--` goes to the harness verbatim
 *   LIFECYCLE_KEEP=1 ...                             # keep the scratch project and print its path
 *   LIFECYCLE_DUMP=1 ...                             # print every harness transcript entry (truncated)
 *
 * The harness runs with extension discovery off and exactly this checkout's `dist/` build loaded, and
 * with `--session-dir` under the scratch project so the harness's own transcript is at a known path.
 * The proof owns its rule rather than depending on which documents ship as bundled defaults: the run's
 * private Rerail catalog holds one enabled policy, flagging `.execute(f"... VALUES ({...})")` as
 * SQL_EXECUTE_FSTRING, and the prompt asks the agent to write `api/probe.py` with exactly that call.
 *
 * What one run proves, from records only (nothing depends on the model's wording):
 *   1. The extension prints no `Extension error` / `handler timed out` lines.
 *   2. Rerail records exactly one session for the run, opened once, with the prompt and a clean end,
 *      and its session-start row lands within 12 s of the process spawn.
 *   3. Rerail's ledger holds exactly one `steer` delivery carrying SQL_EXECUTE_FSTRING for the first
 *      tool result that writes api/probe.py; later model-chosen edits are outside this assertion.
 *   4. The harness's own session transcript recorded receiving that steer (the `custom_message` entry
 *      `sendMessage` appends, tagged `customType: "rerail.rule"` by both extensions), and an assistant
 *      message follows it in the same transcript, so it was in the model's context.
 *
 * Anything that depends on what the model chose to say is printed as INFO, never asserted.
 */
import { spawn } from "node:child_process";
import { mkdir, mkdtemp, readdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { RULE_MESSAGE_TYPE } from "../src/adapter-contract.js";
import { openRerailDatabase } from "../src/database.js";
import { startRepository } from "../src/repository-start.js";
import { openScopedRuleCatalog, resolveRepositoryContext } from "../src/scoped-rule-catalog.js";
import { SqliteTranscriptReader, type TranscriptEventRecord } from "../src/transcript-store.js";

type Harness = "omp" | "pi";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DIST = join(ROOT, "dist");
const TIMEOUT_MS = 180_000;
/**
 * Ceiling on the extension's own session start, measured from Rerail's records (process spawn to the
 * session-start row), so it does not depend on model latency. The 30 s handler timeout that used to
 * fire on every start is the failure this guards against.
 */
const MAX_SESSION_START_MS = 12_000;
const CODE = "SQL_EXECUTE_FSTRING";
/** The one enabled policy: a Python `.execute(...)` call whose SQL is an f-string interpolating into the statement. */
const POLICY = {
  format: "rerail.policy/v1",
  id: "rerail-lifecycle:sql-execute-fstring",
  revision: 1,
  mode: "steer",
  scope: { include: ["**/*.py"], exclude: [] },
  clauses: [
    {
      id: "execute-fstring",
      forbid: {
        ast: {
          language: "python",
          rule: {
            kind: "call",
            all: [
              { pattern: "$C.execute($$$ARGS)" },
              {
                has: {
                  kind: "argument_list",
                  has: { kind: "string", all: [{ regex: "^([rR]?[fF]|[fF][rR])[\"']" }, { regex: "(?is)\\b(WHERE|VALUES|SET|HAVING|LIKE|IN\\s*\\().*\\{" }] },
                },
              },
            ],
          },
        },
      },
      violation: { code: CODE, anchor: "witness.span", message: "Do not build SQL queries with string formatting; use parameterized queries." },
    },
  ],
};
const PROBE = "api/probe.py";
const PROMPT =
  `Use the write tool to create ${PROBE} containing exactly:\n\n` +
  'def persist(conn, payload):\n    conn.execute(f"INSERT INTO probe VALUES ({payload})")\n\n' +
  "Then reply with one sentence describing any diagnostic you received. Do not read other files first.";
const EXTENSION: Record<Harness, string> = {
  omp: join(DIST, "default-extension.js"),
  pi: join(DIST, "pi-extension.js"),
};

/** Validate proof-consumed records; unrelated ledger metadata stays opaque, not a second closed union. */
const LedgerEntry = z.union([
  z.object({
    type: z.literal("sample"),
    item: z.object({ metadata: z.object({ fields: z.object({ eventId: z.string(), sampleSource: z.string(), boundary: z.string() }) }) }),
  }),
  z.object({ type: z.literal("delivery"), delivery: z.object({ requested: z.string(), mechanism: z.string(), content: z.string() }) }),
  z.object({ type: z.string().refine((type) => type !== "sample" && type !== "delivery") }).transform(() => ({ type: "other" as const })),
]);
/** Rerail transcript payload for a `result` event; enough to name the tool and its target. */
const ToolPayload = z.object({ toolName: z.string(), input: z.unknown().optional(), output: z.unknown().optional() });
/** Text content as both harnesses store it: a string, or parts with `text`. */
const Content = z.union([z.string(), z.array(z.union([z.string(), z.object({ text: z.string().optional() })]))]);
/**
 * One line of the harness's own session jsonl (OMP and Pi share the session-manager format). The steer
 * arrives as a `custom_message` entry, the model's replies as `message` entries with role `assistant`.
 * Every other entry type is kept opaque as `other`.
 */
const HarnessEntry = z.union([
  z.object({ type: z.literal("custom_message"), customType: z.string(), content: Content }),
  z.object({ type: z.literal("message"), message: z.object({ role: z.string(), content: Content }) }),
  z.object({ type: z.string() }).transform(() => ({ type: "other" as const })),
]);

type Assertion = { name: string; passed: boolean; detail: string };
type Run = { exitCode: number; stdout: string; stderr: string; ms: number; startedAt: number };
type Workspace = { root: string; env: NodeJS.ProcessEnv; databasePath: string; sessionDir: string };
/** Rerail's SQLite records for the run: every event, the shape of each session, and how fast it started. */
type RerailRecords = {
  events: TranscriptEventRecord[];
  /** Per session: session-start rows, event count, and whether the prompt and a clean end were recorded. */
  shapes: { starts: number; events: number; prompted: boolean; ended: boolean }[];
  /** Milliseconds from process spawn to the first session's start row; NaN when no session was recorded. */
  startMs: number;
};
/** What Rerail's ledger says about the SQL_EXECUTE_FSTRING steer produced by the requested probe write. */
type SteerRecord = {
  deliveries: number;
  /** `steer` deliveries carrying SQL_EXECUTE_FSTRING whose producing tool result writes the requested probe. */
  steers: number;
  /** Text of the first matching probe-write steer; "" when there is none. */
  content: string;
  /** The sample that produced it, and the Rerail transcript event its eventId names; "?" where unresolved. */
  sample: { source: string; boundary: string };
  event: { kind: string; toolName: string; mentionsProbe: boolean };
};
/** The harness's own session transcript: where it recorded the steer, and what the model said. */
type HarnessTranscript = {
  files: number;
  /** Every jsonl line, one per entry. */
  lines: string[];
  /** Indexes of `custom_message` entries tagged `rerail.rule` whose text is the steer. */
  steerEntries: number[];
  /** Index of the first assistant message after the single steer entry, when both exist. */
  assistantAfter: number | undefined;
  replies: number;
  /** Text of the model's last reply; "" when it never spoke. */
  lastReply: string;
};
type Evidence = { run: Run; rerail: RerailRecords; steer: SteerRecord; transcript: HarnessTranscript };

function check(assertions: Assertion[], name: string, passed: boolean, detail: string): void {
  assertions.push({ name, passed, detail });
}

function harnessCommand(harness: Harness, project: string, sessionDir: string, extra: string[]): { command: string; args: string[] } {
  if (harness === "omp")
    return {
      command: process.env.RERAIL_OMP_EXECUTABLE ?? "omp",
      args: [
        "--print", "--auto-approve", "--no-title",
        "--cwd", project,
        "--session-dir", sessionDir,
        "--no-extensions", "--extension", EXTENSION.omp,
        ...extra,
        PROMPT,
      ],
    };
  // Pi has no approval prompts to bypass; the discovery flags keep the user's skills/templates out.
  return {
    command: "pi",
    args: [
      "--print",
      "--no-context-files", "--no-skills", "--no-prompt-templates", "--no-themes",
      "--session-dir", sessionDir,
      "--no-extensions", "--extension", EXTENSION.pi,
      ...extra,
      PROMPT,
    ],
  };
}

async function runHarness(command: string, args: string[], cwd: string, env: NodeJS.ProcessEnv): Promise<Run> {
  const started = Date.now();
  const { promise, resolve: settle, reject } = Promise.withResolvers<Run>();
  const child = spawn(command, args, { cwd, env, stdio: ["ignore", "pipe", "pipe"] });
  let stdout = "";
  let stderr = "";
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => { stdout += chunk; });
  child.stderr.on("data", (chunk: string) => { stderr += chunk; });
  // The proof does not require a provider to finish its reply. Let OMP handle SIGTERM and emit its
  // session_shutdown record, then retain a five-second SIGKILL backstop for a bounded cleanup.
  const timer = setTimeout(() => child.kill("SIGTERM"), TIMEOUT_MS);
  const force = setTimeout(() => child.kill("SIGKILL"), TIMEOUT_MS + 5_000);
  const clearTimers = (): void => {
    clearTimeout(timer);
    clearTimeout(force);
  };
  child.once("error", (error) => {
    clearTimers();
    reject(error);
  });
  child.once("close", (code) => {
    clearTimers();
    settle({ exitCode: code ?? 1, stdout, stderr, ms: Date.now() - started, startedAt: started });
  });
  return promise;
}

function extensionErrors(run: Run): string[] {
  return `${run.stdout}\n${run.stderr}`
    .split("\n")
    .filter((line) => /Extension error|handler timed out/iu.test(line) && /rerail|cosine-rules/iu.test(line))
    .map((line) => line.trim().replace(/^.*(default|pi)-extension\.js\): /u, "").slice(0, 160));
}

/** Refuse to prove anything about a `dist/` that is older than `src/`: the run would test stale code. */
async function ensureFresh(extension: string): Promise<void> {
  const built = await stat(extension).catch(() => undefined);
  if (built === undefined) throw new Error(`${extension} is missing: run \`npm run build:ts\` first`);
  for (const name of await readdir(join(ROOT, "src"))) {
    const source = await stat(join(ROOT, "src", name));
    if (source.isFile() && source.mtimeMs > built.mtimeMs) throw new Error(`${extension} is older than src/${name}: run \`npm run build:ts\` first`);
  }
}

async function project(): Promise<Workspace> {
  const root = await mkdtemp(join(tmpdir(), "rerail-lifecycle-"));
  await mkdir(join(root, "api"), { recursive: true });
  await writeFile(join(root, "api", "existing.py"), "def existing():\n    return 1\n");
  const rerailHome = join(root, ".rerail-home");
  const databasePath = join(rerailHome, "rerail.sqlite");
  const catalog = openScopedRuleCatalog(root, databasePath);
  try {
    catalog.upsertPolicy(POLICY, { applicability: "project", enabled: true });
  } finally {
    catalog.close();
  }
  // Rerail is off until started; start the project as `rerail start` would.
  const database = openRerailDatabase(databasePath);
  try {
    startRepository(database, resolveRepositoryContext(root).repositoryId);
  } finally {
    database.$client.close();
  }
  // Orca's terminal injects ORCA_* variables that redirect OMP's agent directory; the run must see the
  // user's real HOME (credentials, model choice) and nothing else. RERAIL_AUTOSTART=0 keeps the daemon out.
  const env: NodeJS.ProcessEnv = { RERAIL_AUTOSTART: "0" };
  for (const [key, value] of Object.entries(process.env)) {
    if (value !== undefined && !key.startsWith("ORCA_")) env[key] = value;
  }
  env.RERAIL_HOME = rerailHome;
  return { root, env, databasePath, sessionDir: join(root, ".harness-sessions") };
}

async function jsonlFiles(directory: string): Promise<string[]> {
  const entries = await readdir(directory, { withFileTypes: true }).catch(() => []);
  const files = await Promise.all(
    entries.map((entry) => {
      const path = join(directory, entry.name);
      if (entry.isDirectory()) return jsonlFiles(path);
      return entry.name.endsWith(".jsonl") ? [path] : [];
    }),
  );
  return files.flat();
}

function textOf(content: z.infer<typeof Content>): string {
  if (typeof content === "string") return content;
  return content.map((part) => (typeof part === "string" ? part : part.text ?? "")).join("");
}

function parseArgs(argv: string[]): { harness: Harness; extra: string[] } {
  const harness = argv[2] ?? "omp";
  if (harness !== "omp" && harness !== "pi") throw new Error(`unknown harness "${harness}": expected omp or pi`);
  const separator = argv.indexOf("--", 3);
  return { harness, extra: separator === -1 ? [] : argv.slice(separator + 1) };
}

/** Rerail's records: every session's events, and the session-start latency measured from the spawn. */
function readRerailRecords(databasePath: string, spawnedAt: number): RerailRecords {
  const reader = new SqliteTranscriptReader(databasePath);
  const sessions = reader.sessions();
  const events = sessions.flatMap((session) => reader.events(session.sessionId, { limit: 500 }));
  reader.close();
  const shapes = sessions.map((session) => {
    const kinds = events.filter((event) => event.sessionId === session.sessionId).map((event) => event.kind);
    return {
      starts: kinds.filter((kind) => kind === "session-start").length,
      events: kinds.length,
      prompted: kinds.includes("user-prompt"),
      ended: kinds.includes("session-end"),
    };
  });
  const startedAt = sessions[0]?.startedAt;
  return { events, shapes, startMs: startedAt === undefined ? Number.NaN : startedAt - spawnedAt };
}

/** Rerail's ledger (`.rerail/sessions/<id>/events.jsonl`): the SQL_EXECUTE_FSTRING steer produced by the first requested probe write. */
export async function readLedger(root: string, events: readonly TranscriptEventRecord[]): Promise<SteerRecord> {
  const directory = join(root, ".rerail", "sessions");
  const directories = (await readdir(directory, { withFileTypes: true }).catch(() => [])).filter((entry) => entry.isDirectory());
  const texts = await Promise.all(directories.map((entry) => readFile(join(directory, entry.name, "events.jsonl"), "utf8").catch(() => "")));
  const ledger = texts.flatMap((text) => text.split("\n").filter(Boolean).map((line) => LedgerEntry.parse(JSON.parse(line))));
  const deliveries = ledger.filter((entry) => entry.type === "delivery");
  const steers = ledger.flatMap((entry, index) =>
    entry.type === "delivery" && entry.delivery.requested === "steer" && entry.delivery.content.includes(CODE)
      ? [{ index, content: entry.delivery.content }]
      : [],
  );
  const requestedWrite = events.find((event) => {
    if (event.kind !== "result") return false;
    const payload = ToolPayload.safeParse(event.payload).data;
    return payload?.toolName === "write" && JSON.stringify(payload).includes(PROBE);
  });
  const evidence = steers.map((steer) => {
    // Each delivery follows its producing sample in the ledger. Resolve that sample's eventId back to
    // the native tool result so later model-chosen rewrites of the same path cannot change this assertion.
    const sample = ledger.slice(0, steer.index).reverse().find((entry) => entry.type === "sample");
    const fields = sample?.type === "sample" ? sample.item.metadata.fields : undefined;
    const sourceEvent = fields === undefined ? undefined : events.find((event) => event.eventId === fields.eventId);
    const payload = sourceEvent === undefined ? undefined : ToolPayload.safeParse(sourceEvent.payload).data;
    return {
      eventId: fields?.eventId,
      content: steer.content,
      sample: { source: fields?.sampleSource ?? "?", boundary: fields?.boundary ?? "?" },
      event: { kind: sourceEvent?.kind ?? "?", toolName: payload?.toolName ?? "?", mentionsProbe: JSON.stringify(payload ?? {}).includes(PROBE) },
    };
  });
  const probeSteers = evidence.filter((steer) => steer.eventId === requestedWrite?.eventId);
  const steer = probeSteers[0];
  return {
    deliveries: deliveries.length,
    steers: probeSteers.length,
    content: steer?.content ?? "",
    sample: steer?.sample ?? { source: "?", boundary: "?" },
    event: steer?.event ?? { kind: "?", toolName: "?", mentionsProbe: false },
  };
}

/** The harness's session jsonl under `sessionDir`: the steer entry, and the assistant messages around it. */
async function readHarnessTranscript(sessionDir: string, steerContent: string): Promise<HarnessTranscript> {
  const files = await jsonlFiles(sessionDir);
  const lines = (await Promise.all(files.map((file) => readFile(file, "utf8")))).flatMap((text) => text.split("\n").filter(Boolean));
  const entries = lines.map((line) => HarnessEntry.parse(JSON.parse(line)));
  const steerEntries = entries.flatMap((entry, index) =>
    steerContent !== "" && entry.type === "custom_message" && entry.customType === RULE_MESSAGE_TYPE && textOf(entry.content) === steerContent ? [index] : [],
  );
  const assistants = entries.flatMap((entry, index) => (entry.type === "message" && entry.message.role === "assistant" ? [index] : []));
  const steerEntry = steerEntries[0] ?? -1;
  const assistantAfter = steerEntries.length === 1 ? assistants.find((index) => index > steerEntry) : undefined;
  const last = entries[assistants.at(-1) ?? -1];
  return {
    files: files.length,
    lines,
    steerEntries,
    assistantAfter,
    replies: assistants.length,
    lastReply: last?.type === "message" ? textOf(last.message.content) : "",
  };
}

/** Runs the harness once in the workspace, then reads back every record the proof is made from. */
async function gather(harness: Harness, extra: string[], { root, env, databasePath, sessionDir }: Workspace): Promise<Evidence> {
  const { command, args } = harnessCommand(harness, root, sessionDir, extra);
  const run = await runHarness(command, args, root, env);
  const rerail = readRerailRecords(databasePath, run.startedAt);
  const steer = await readLedger(root, rerail.events);
  const transcript = await readHarnessTranscript(sessionDir, steer.content);
  return { run, rerail, steer, transcript };
}

function print(assertions: Assertion[], infos: string[]): void {
  for (const line of infos) process.stdout.write(`INFO  ${line}\n`);
  for (const assertion of assertions) process.stdout.write(`${assertion.passed ? "PASS" : "FAIL"}  ${assertion.name} — ${assertion.detail}\n`);
  if (assertions.some((assertion) => !assertion.passed)) process.exitCode = 1;
}

/** Turns the evidence into the assertions and INFO lines of the proof, prints them, and sets the exit code. */
function report(harness: Harness, { run, rerail, steer, transcript }: Evidence): void {
  const assertions: Assertion[] = [];
  const infos = [`${harness} exit ${run.exitCode} after ${run.ms} ms wall (model latency; not asserted)`];
  const errors = extensionErrors(run);
  check(assertions, "extension silent", errors.length === 0, [...new Set(errors)].slice(0, 2).join(" | ") || "no Extension error / handler timed out lines");

  // Rerail's records: one session, opened once, started fast.
  check(assertions, "session start within ceiling", Number.isFinite(rerail.startMs) && rerail.startMs <= MAX_SESSION_START_MS, `${Math.round(rerail.startMs)} ms from spawn to session-start (ceiling ${MAX_SESSION_START_MS} ms)`);
  const perSession = rerail.shapes.map((shape) => `${shape.starts} start, ${shape.events} events, end ${shape.ended}`).join("; ");
  check(
    assertions,
    "one session opened once",
    rerail.shapes.length === 1 && rerail.shapes.every((shape) => shape.starts === 1 && shape.prompted && shape.ended),
    `${rerail.shapes.length} session(s); per session: ${perSession}`,
  );

  // Rerail's ledger: exactly one SQL_EXECUTE_FSTRING steer produced by the first requested probe write.
  check(assertions, "probe violation steered exactly once", steer.steers === 1, `${steer.steers} probe-write ${CODE} steer(s); ${steer.deliveries} deliveries total`);
  check(
    assertions,
    "steer produced on the probe write's tool result",
    steer.event.kind === "result" && steer.event.toolName === "write" && steer.event.mentionsProbe,
    `sample source ${steer.sample.source}/${steer.sample.boundary} -> event ${steer.event.kind} tool ${steer.event.toolName}, mentions ${PROBE}: ${steer.event.mentionsProbe}`,
  );

  // The harness's own transcript: it recorded receiving the steer, and the model spoke after it.
  if (process.env.LIFECYCLE_DUMP) infos.push(...transcript.lines.map((line, index) => `harness entry ${index}: ${line.slice(0, 400)}`));
  const steerEntry = transcript.steerEntries[0] ?? -1;
  const plural = transcript.steerEntries.length === 1 ? "entry" : "entries";
  const at = transcript.steerEntries.join(",") || "-";
  check(
    assertions,
    `${harness} transcript recorded the steer once`,
    transcript.files === 1 && transcript.steerEntries.length === 1,
    `${transcript.files} transcript file(s), ${transcript.lines.length} entries, ${transcript.steerEntries.length} custom_message/${RULE_MESSAGE_TYPE} ${plural} with the steer text at index ${at}`,
  );
  const placement =
    transcript.assistantAfter === undefined
      ? "no assistant message after the steer entry"
      : `assistant message at index ${transcript.assistantAfter} follows steer at index ${steerEntry}`;
  check(assertions, `${harness} put the steer in the model's context`, transcript.assistantAfter !== undefined, placement);
  infos.push(
    `assistant's last reply: ${transcript.lastReply.replace(/\s+/gu, " ").trim().slice(0, 240) || "(none)"}`,
    `assistant mentioned the policy by name: ${/SQL_EXECUTE_FSTRING|f-string|raw sql/iu.test(transcript.lastReply)}`,
  );
  if (run.exitCode !== 0 || transcript.replies === 0) {
    const tail = `${run.stdout}\n${run.stderr}`.trim().split("\n").slice(-3).join(" | ").slice(0, 400);
    infos.push(`harness output tail: ${tail}`);
  }
  print(assertions, infos);
}

async function main(): Promise<void> {
  const { harness, extra } = parseArgs(process.argv);
  await ensureFresh(EXTENSION[harness]);
  const workspace = await project();
  const evidence = await gather(harness, extra, workspace).finally(async () => {
    if (process.env.LIFECYCLE_KEEP) process.stdout.write(`kept ${workspace.root}\n`);
    else await rm(workspace.root, { recursive: true, force: true });
  });
  report(harness, evidence);
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) await main();
