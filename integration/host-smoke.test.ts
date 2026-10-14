import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { afterEach, expect, it, vi } from "vitest";
import { inspectOutcome, isInconclusive, watchForViolation } from "../scripts/host/host-smoke.js";
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
      await inspectOutcome(project, nonce, assertions, { exitCode: 0, stdout: "", stderr: "" }, true, "transport");
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
    await inspectOutcome(project, "nonce", assertions, { exitCode: 0, stdout: "", stderr }, true, "transport");
    return assertions.find((assertion) => assertion.name === "extension stayed silent")?.passed;
  };
  expect(await silent("Extension error (/checkout/package/dist/hosts/omp/default-extension.js): handler timed out after 5000ms")).toBe(false);
  expect(await silent("Extension error (/checkout/package/dist/hosts/pi/pi-extension.js): boom")).toBe(false);
  expect(await silent("Extension error (/elsewhere/index.js): cannot load rerail_engine.darwin-arm64.node")).toBe(false);
  expect(await silent("Extension error (/elsewhere/other-extension.js): unrelated failure")).toBe(true);
});

it("sees a forbidden call that was on disk only briefly, and reports read failures instead of absence", async () => {
  const project = await mkdtemp(join(tmpdir(), "rerail-smoke-watch-"));
  temporaryDirectories.push(project);
  const route = join(project, "route.py");
  // The file does not exist yet, then never holds the call.
  const quiet = watchForViolation(route);
  await sleep(150);
  await writeFile(route, "return transaction.fetch_value('SELECT 1')\n");
  await sleep(300);
  expect(await quiet()).toBe(false);
  // A violation restored before the run ends, as an edit-tool host's model does after the diagnostic.
  const watching = watchForViolation(route);
  await writeFile(route, "return cursor.execute('SELECT 1')\n");
  await sleep(300);
  await writeFile(route, "return transaction.fetch_value('SELECT 1')\n");
  expect(await watching()).toBe(true);
  // A path that can't be read (here a directory) is an error, never "the violation was absent".
  await expect(watchForViolation(project)()).rejects.toThrow();
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
