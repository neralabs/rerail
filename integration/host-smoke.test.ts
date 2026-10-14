import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, it, vi } from "vitest";
import { inspectOutcome, isInconclusive } from "../scripts/host/host-smoke.js";
import { defaultRerailDatabasePath } from "../src/engine/index.js";
import { SqliteSessionStore } from "../src/runtime/index.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(temporaryDirectories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});

it("accepts a completed violation retry without accepting unrelated or mismatched results", async () => {
  const project = await mkdtemp(join(tmpdir(), "rerail-smoke-evidence-"));
  temporaryDirectories.push(project);
  vi.stubEnv("RERAIL_HOME", join(project, "home"));
  const sessionId = "retry-session";
  const nonce = "private-smoke-nonce";
  const ledger = join(project, ".rerail", "sessions", sessionId);
  await mkdir(ledger, { recursive: true });
  await writeFile(join(ledger, "events.jsonl"), [
    { type: "delivery", delivery: { content: nonce } },
    { type: "delivery", delivery: { content: "CURSOR_EXECUTE" } },
  ].map((entry) => JSON.stringify(entry)).join("\n"));
  const store = new SqliteSessionStore(defaultRerailDatabasePath());
  try {
    const session: Parameters<SqliteSessionStore["openSession"]>[0] = { host: "codex", sessionId, cwd: project };
    store.openSession(session, 100);
    let sequence = 0;
    const append = (kind: "session-start" | "call" | "result", payload: Record<string, unknown>): void => {
      sequence += 1;
      const event = { ...payload, sessionId, eventId: `event-${sequence}`, kind };
      store.append({ ...event, observedAt: 100 + sequence, payload: event });
    };
    const accepted = async (): Promise<boolean> => {
      const assertions: Parameters<typeof inspectOutcome>[2] = [];
      await inspectOutcome(project, nonce, assertions, { exitCode: 0, stdout: "", stderr: "" }, "transport");
      return assertions.every((assertion) => assertion.passed);
    };
    const input = { command: "printf 'cursor.execute(1)' > api/route.py" };
    append("session-start", { session });
    append("call", { toolCallId: "pty-denied", toolName: "Bash", input });
    append("result", { toolCallId: "unrelated", toolName: "Bash", output: { exitCode: 0 } });
    expect(await accepted()).toBe(false);

    append("call", { toolCallId: "completed-retry", toolName: "Bash", input });
    append("result", { toolCallId: "completed-retry", toolName: "Read", output: "contents" });
    expect(await accepted()).toBe(false);

    append("result", { toolCallId: "completed-retry", toolName: "Bash", output: { exitCode: 0 } });
    expect(await accepted()).toBe(true);
  } finally {
    store.close();
  }
});

it("counts extension errors from this package's extension or engine and ignores other extensions", async () => {
  const project = await mkdtemp(join(tmpdir(), "rerail-smoke-errors-"));
  temporaryDirectories.push(project);
  vi.stubEnv("RERAIL_HOME", join(project, "home"));
  await mkdir(join(project, ".rerail", "sessions"), { recursive: true });
  new SqliteSessionStore(defaultRerailDatabasePath()).close();
  const silent = async (stderr: string): Promise<boolean | undefined> => {
    const assertions: Parameters<typeof inspectOutcome>[2] = [];
    await inspectOutcome(project, "nonce", assertions, { exitCode: 0, stdout: "", stderr }, "transport");
    return assertions.find((assertion) => assertion.name === "extension stayed silent")?.passed;
  };
  expect(await silent("Extension error (/checkout/package/dist/hosts/omp/default-extension.js): handler timed out after 5000ms")).toBe(false);
  expect(await silent("Extension error (/checkout/package/dist/hosts/pi/pi-extension.js): boom")).toBe(false);
  expect(await silent("Extension error (/elsewhere/index.js): cannot load rerail_engine.darwin-arm64.node")).toBe(false);
  expect(await silent("Extension error (/elsewhere/other-extension.js): unrelated failure")).toBe(true);
});

it("counts a Codex shell violation as written only once the set-call helper logged writing it", async () => {
  const project = await mkdtemp(join(tmpdir(), "rerail-smoke-helper-"));
  temporaryDirectories.push(project);
  vi.stubEnv("RERAIL_HOME", join(project, "home"));
  await mkdir(join(project, ".rerail", "sessions"), { recursive: true });
  await mkdir(join(project, "tools"), { recursive: true });
  await writeFile(join(project, "tools", "set_call.py"), "");
  const sessionId = "helper-session";
  const store = new SqliteSessionStore(defaultRerailDatabasePath());
  try {
    const session: Parameters<SqliteSessionStore["openSession"]>[0] = { host: "codex", sessionId, cwd: project };
    store.openSession(session, 100);
    const input = { command: "bash -lc \"python3 tools/set_call.py cursor.execute\"" };
    for (const [sequence, kind, payload] of [
      [1, "call", { toolCallId: "violation", toolName: "Bash", input }],
      [2, "result", { toolCallId: "violation", toolName: "Bash", output: "bash: syntax error" }],
    ] as const) {
      const event = { ...payload, sessionId, eventId: `event-${sequence}`, kind };
      store.append({ ...event, observedAt: 100 + sequence, payload: event });
    }
  } finally {
    store.close();
  }
  const outcome = async (): Promise<Record<string, boolean>> => {
    const assertions: Parameters<typeof inspectOutcome>[2] = [];
    await inspectOutcome(project, "nonce", assertions, { exitCode: 0, stdout: "", stderr: "" }, "transport");
    return Object.fromEntries(assertions.map((assertion) => [assertion.name, assertion.passed]));
  };
  // Rerail recorded a completed call either way; only the helper's log says whether the file changed.
  expect(await outcome()).toMatchObject({ "real violation evidence": true, "violation reached the file": false });
  await writeFile(join(project, "tools", "set_call.log"), "transaction.fetch_value\n");
  expect(await outcome()).toMatchObject({ "violation reached the file": false });
  await writeFile(join(project, "tools", "set_call.log"), "cursor.execute\ntransaction.fetch_value\n");
  expect(await outcome()).toMatchObject({ "violation reached the file": true });
});

it("retries only attempts where the provider, not Rerail, kept the violation from being judged", () => {
  const transport = [
    "harness exited cleanly", "extension stayed silent", "one session, opened once", "policies evaluated within budget",
    "violation reached the file", "real violation evidence", "single authoritative delivery", "executable policy delivered",
  ];
  const full = [...transport, "final file corrected", "real final assistant output"];
  const outcome = (names: readonly string[], ...failed: string[]) =>
    names.map((name) => ({ name, passed: !failed.includes(name), detail: "" }));
  const missing = ["violation reached the file", "real violation evidence", "executable policy delivered"];
  // The violation never reached the file: a mis-quoted command (exit 0) or a stalled provider (timeout).
  expect(isInconclusive(outcome(transport, ...missing), 0)).toBe(true);
  expect(isInconclusive(outcome(transport, "harness exited cleanly", ...missing), 124)).toBe(true);
  expect(isInconclusive(outcome(full, ...missing, "real final assistant output"), 0)).toBe(true);
  // A stall after every transport assertion already passed.
  expect(isInconclusive(outcome(transport, "harness exited cleanly"), 124)).toBe(true);
  // The violation reached the file and Rerail lost its record or never delivered its diagnostic.
  expect(isInconclusive(outcome(transport, "real violation evidence", "executable policy delivered"), 0)).toBe(false);
  expect(isInconclusive(outcome(transport, "executable policy delivered"), 0)).toBe(false);
  expect(isInconclusive(outcome(transport, "harness exited cleanly", "executable policy delivered"), 124)).toBe(false);
  expect(isInconclusive(outcome(full, "real violation evidence", "executable policy delivered", "final file corrected"), 0)).toBe(false);
  // Rerail-owned failures stay red even when the violation is also missing; so does a crash.
  expect(isInconclusive(outcome(transport, ...missing, "single authoritative delivery"), 0)).toBe(false);
  expect(isInconclusive(outcome(transport, ...missing, "extension stayed silent"), 0)).toBe(false);
  expect(isInconclusive(outcome(transport, "harness exited cleanly", ...missing), 1)).toBe(false);
  expect(isInconclusive(outcome(transport), 0)).toBe(false);
});
