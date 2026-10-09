import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, expect, it, vi } from "vitest";
import { inspectOutcome } from "../scripts/host-smoke.js";
import { defaultRerailDatabasePath } from "../src/database.js";
import { SqliteSessionStore } from "../src/session-store.js";

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
  expect(await silent("Extension error (/checkout/package/dist/default-extension.js): handler timed out after 5000ms")).toBe(false);
  expect(await silent("Extension error (/checkout/package/dist/pi-extension.js): boom")).toBe(false);
  expect(await silent("Extension error (/elsewhere/index.js): cannot load rerail_engine.darwin-arm64.node")).toBe(false);
  expect(await silent("Extension error (/elsewhere/other-extension.js): unrelated failure")).toBe(true);
});
