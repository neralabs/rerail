import { createHash, randomUUID } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { appendFileSync, mkdirSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp, open, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { deserialize, serialize } from "node:v8";
import { afterEach, describe, expect, it, vi } from "vitest";
import { rewriteAppendedFile, runScrubCommand, scrubHintOnce } from "../../src/cli/scrub-cli.js";
import { openRerailDatabase, recordCheckout } from "../../src/engine/index.js";
import { withPendingFileLock } from "../../src/harness/index.js";

/** Set to make `eraseFreedPages` fail as a busy database or a full disk makes it fail. */
const erase = vi.hoisted(() => ({ failure: undefined as Error | undefined }));
vi.mock("../../src/engine/storage/database.js", async () => {
  const actual = await vi.importActual<typeof import("../../src/engine/storage/database.js")>("../../src/engine/storage/database.js");
  return {
    ...actual,
    eraseFreedPages: (database: Parameters<typeof actual.eraseFreedPages>[0]) => {
      if (erase.failure !== undefined) throw erase.failure;
      return actual.eraseFreedPages(database);
    },
  };
});

const directories: string[] = [];
afterEach(async () => {
  erase.failure = undefined;
  vi.restoreAllMocks();
  vi.unstubAllEnvs();
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-scrub-"));
  directories.push(root);
  return root;
}

/**
 * The bytes of the database and its WAL, read by a child process: closing a descriptor of the file in this process
 * would drop the locks this process's connections hold on it.
 */
function databaseBytes(path: string): string {
  const script = "const fs = require('node:fs'); for (const file of process.argv.slice(1)) if (fs.existsSync(file)) process.stdout.write(fs.readFileSync(file).toString('latin1'));";
  const read = spawnSync(process.execPath, ["-e", script, path, `${path}-wal`], { encoding: "latin1", maxBuffer: 256 * 1024 * 1024 });
  if (read.status !== 0) throw new Error(read.stderr);
  return read.stdout;
}

describe("rerail scrub", () => {
  // The daemon holds the database open across a scrub, so no closing connection checkpoints for it.
  it("redacts secrets earlier builds stored in the database, the session ledgers and daemon.log, and a second run changes nothing", async () => {
    const root = await temporaryRoot();
    const home = join(root, "home");
    vi.stubEnv("RERAIL_HOME", home);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const checkout = join(root, "repo");
    mkdirSync(checkout);
    const path = join(home, "rerail.sqlite");
    const token = `ghp_${randomUUID().replaceAll("-", "")}abcd`;
    const live = openRerailDatabase(path);
    try {
      const { repositoryKey } = recordCheckout(live, checkout);
      const recordedCheckout = live.$client.prepare("SELECT checkout_path FROM checkouts").pluck().get() as string;
      live.$client.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES ('s1', 'omp', ?, ?, 1, 2)")
        .run(checkout, repositoryKey);
      const event = live.$client.prepare("INSERT INTO events (session_id, event_id, kind, observed_at, payload_json, payload_v8) VALUES ('s1', ?, ?, 1, ?, ?)");
      const output = { kind: "result", toolName: "Bash", output: `GITHUB_TOKEN=${token}\n` };
      event.run("in-file", "result", JSON.stringify(output), serialize(output));
      const violation = {
        id: "v1", ruleId: "acme:no-token", clauseId: "token", code: "TOKEN", mode: "log", message: `Token ${token} is committed`,
        path: "src/a.ts", captures: {}, variables: { value: token },
      };
      live.$client.prepare(
        "INSERT INTO audits (audit_id, session_id, workspace_root, scope, mode, revision, rule_provenance_json, paths_json, started_at, finished_at, status, violations_json, failures_json) "
          + "VALUES ('a1', 's1', ?, 'repository', 'log', 1, '[]', '[]', 1, 2, 'completed', ?, '[]')",
      ).run(checkout, JSON.stringify([violation]));
      live.$client.pragma("wal_checkpoint(TRUNCATE)");
      // One copy still in the WAL.
      event.run("in-wal", "user-prompt", JSON.stringify({ text: `use ${token} for the release` }), null);
      expect(databaseBytes(path)).toContain(token);
      // The copy `rerail update` keeps for a rollback holds the same history.
      const copy = `${path}.pre-update`;
      live.$client.exec(`VACUUM INTO '${copy}'`);
      expect(databaseBytes(copy)).toContain(token);
      const keyBefore = live.$client.prepare("SELECT violation_key FROM audit_violations WHERE audit_id = 'a1'").pluck().all();

      const ledgerFolder = join(recordedCheckout, ".rerail", "sessions", createHash("sha256").update("s1").digest("hex"));
      mkdirSync(ledgerFolder, { recursive: true });
      const ledger = join(ledgerFolder, "events.jsonl");
      const preimage = `TOKEN = "${token}"\n`;
      writeFileSync(ledger, [
        { type: "event", eventId: "in-file" },
        { type: "sample", item: { ref: { source: "host:event", id: "in-file:tool-call:0" }, text: `GITHUB_TOKEN=${token}\n`, metadata: {} } },
        { type: "policy-call", toolCallId: "w1", toolName: "write", paths: ["src/a.py"], preimages: { "src/a.py": { text: preimage } } },
      ].map(line => `${JSON.stringify(line)}\n`).join(""));
      writeFileSync(join(home, "daemon.log"), `rerail: GET /api failed: token ${token} rejected\nstarted\n`);

      expect(await runScrubCommand([], () => "usage")).toBe(0);

      const bytes = databaseBytes(path);
      expect(bytes).not.toContain(token);
      expect(bytes).toContain("[redacted:github-token:");
      expect(databaseBytes(copy)).not.toContain(token);
      const stored = live.$client.prepare("SELECT event_id AS id, payload_json AS json, payload_v8 AS v8 FROM events ORDER BY sequence").all() as Array<{ id: string; json: string; v8: Buffer | null }>;
      expect(JSON.parse(stored[0]!.json)).toMatchObject({ output: expect.stringMatching(/^GITHUB_TOKEN=\[redacted:github-token:[0-9a-f]{6}\]\n$/u) });
      expect(deserialize(stored[0]!.v8!)).toEqual(JSON.parse(stored[0]!.json));
      expect(stored[1]!.v8).toBeNull();
      const audit = JSON.parse(live.$client.prepare("SELECT violations_json FROM audits WHERE audit_id = 'a1'").pluck().get() as string) as Array<{ message: string }>;
      expect(audit[0]!.message).toMatch(/^Token \[redacted:github-token:[0-9a-f]{6}\] is committed$/u);
      // History still counts the violation as the same one.
      expect(live.$client.prepare("SELECT violation_key FROM audit_violations WHERE audit_id = 'a1'").pluck().all()).toEqual(keyBefore);

      const ledgerText = readFileSync(ledger, "utf8");
      expect(ledgerText).not.toContain(token);
      const lines = ledgerText.split("\n").filter(Boolean).map(line => JSON.parse(line) as Record<string, unknown>);
      expect(lines[0]).toEqual({ type: "event", eventId: "in-file" });
      expect(lines[2]).toMatchObject({
        preimages: { "src/a.py": { withheld: { sha256: createHash("sha256").update(preimage).digest("hex"), bytes: Buffer.byteLength(preimage) } } },
      });
      const log = readFileSync(join(home, "daemon.log"), "utf8");
      expect(log).not.toContain(token);
      expect(log.endsWith("started\n")).toBe(true);

      const rowsAfterFirst = live.$client.prepare("SELECT payload_json FROM events ORDER BY sequence").pluck().all();
      expect(await runScrubCommand([], () => "usage")).toBe(0);
      expect(live.$client.prepare("SELECT payload_json FROM events ORDER BY sequence").pluck().all()).toEqual(rowsAfterFirst);
      expect(readFileSync(ledger, "utf8")).toBe(ledgerText);
      expect(readFileSync(join(home, "daemon.log"), "utf8")).toBe(log);
    } finally {
      live.$client.close();
    }
  });

  it("follows no link at a checkout's .rerail folder", async () => {
    const root = await temporaryRoot();
    const home = join(root, "home");
    vi.stubEnv("RERAIL_HOME", home);
    vi.spyOn(process.stdout, "write").mockReturnValue(true);
    const checkout = join(root, "repo");
    const elsewhere = join(root, "elsewhere");
    const token = `ghp_${randomUUID().replaceAll("-", "")}abcd`;
    const outside = join(elsewhere, "sessions", "x", "events.jsonl");
    mkdirSync(join(elsewhere, "sessions", "x"), { recursive: true });
    const line = `${JSON.stringify({ type: "sample", item: { ref: { source: "host:event", id: "e" }, text: token, metadata: {} } })}\n`;
    writeFileSync(outside, line);
    mkdirSync(checkout);
    // A Git checkout keeps its identity in .git, so the planted .rerail link is only where its ledgers would be.
    spawnSync("git", ["init", "-q", checkout]);
    symlinkSync(elsewhere, join(checkout, ".rerail"));
    const database = openRerailDatabase(join(home, "rerail.sqlite"));
    try {
      recordCheckout(database, checkout);
    } finally {
      database.$client.close();
    }

    expect(await runScrubCommand([], () => "usage")).toBe(0);
    expect(readFileSync(outside, "utf8")).toBe(line);
  });
});

/** A Rerail home recording one checkout, with an event that holds a token earlier builds stored. */
async function scrubFixture() {
  const root = await temporaryRoot();
  const home = join(root, "home");
  vi.stubEnv("RERAIL_HOME", home);
  const output = vi.spyOn(process.stdout, "write").mockReturnValue(true);
  const checkout = join(root, "repo");
  mkdirSync(checkout);
  const token = `ghp_${randomUUID().replaceAll("-", "")}abcd`;
  const live = openRerailDatabase(join(home, "rerail.sqlite"));
  const { repositoryKey } = recordCheckout(live, checkout);
  const recordedCheckout = live.$client.prepare("SELECT checkout_path FROM checkouts").pluck().get() as string;
  live.$client.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES ('s1', 'omp', ?, ?, 1, 2)")
    .run(checkout, repositoryKey);
  live.$client.prepare("INSERT INTO events (session_id, event_id, kind, observed_at, payload_json) VALUES ('s1', 'e1', 'user-prompt', 1, ?)")
    .run(JSON.stringify({ text: `use ${token}` }));
  const sessions = join(recordedCheckout, ".rerail", "sessions");
  const ledger = join(sessions, createHash("sha256").update("s1").digest("hex"), "events.jsonl");
  mkdirSync(join(ledger, ".."), { recursive: true });
  const printed = () => output.mock.calls.map(([chunk]) => String(chunk)).join("");
  return { root, home, sessions, ledger, token, live, printed };
}

function sampleLine(text: string): string {
  return `${JSON.stringify({ type: "sample", item: { ref: { source: "host:event", id: `e:${randomUUID()}` }, text, metadata: {} } })}\n`;
}

describe("rerail scrub alongside running sessions", () => {
  it("reports a database rewrite that fails, still scrubs the rows, ledgers and daemon.log, and exits 1", async () => {
    const { home, ledger, token, live, printed } = await scrubFixture();
    try {
      writeFileSync(ledger, sampleLine(`GITHUB_TOKEN=${token}`));
      writeFileSync(join(home, "daemon.log"), `token ${token} rejected\n`);
      erase.failure = new Error("database is locked");

      expect(await runScrubCommand([], () => "usage")).toBe(1);

      expect(printed()).toContain("database is locked");
      expect(live.$client.prepare("SELECT payload_json FROM events").pluck().get()).not.toContain(token);
      expect(readFileSync(ledger, "utf8")).not.toContain(token);
      expect(readFileSync(join(home, "daemon.log"), "utf8")).not.toContain(token);
      erase.failure = undefined;
      expect(await runScrubCommand([], () => "usage")).toBe(0);
    } finally {
      live.$client.close();
    }
  });

  it("loses no line a process appends to daemon.log or a ledger while it scrubs them", async () => {
    const { root, home, ledger, token, live } = await scrubFixture();
    const log = join(home, "daemon.log");
    try {
      // Redacted, the token is shorter and each password longer, so the ledger is rewritten in place and the log
      // after making room.
      writeFileSync(ledger, sampleLine(`GITHUB_TOKEN=${token}`).repeat(20));
      writeFileSync(log, "password=k7Qm2xVp9Lw4\n".repeat(20));
      const stop = join(root, "stop");
      const appender = "const fs = require('node:fs'); const [log, ledger, stop] = process.argv.slice(1); "
        + "const pause = new Int32Array(new SharedArrayBuffer(4)); "
        + "for (let i = 0; !fs.existsSync(stop); i += 1) { fs.appendFileSync(log, `appended ${i}\\n`); "
        + "fs.appendFileSync(ledger, JSON.stringify({ type: 'event', eventId: `appended-${i}` }) + '\\n'); Atomics.wait(pause, 0, 0, 1); }";
      const child = spawn(process.execPath, ["-e", appender, log, ledger, stop], { stdio: "ignore" });
      const exited = new Promise(resolve => child.once("exit", resolve));
      try {
        while (!readFileSync(log, "utf8").includes("appended 0")) await delay(5);
        const appendedBefore = readFileSync(ledger, "utf8").split("appended-").length;
        expect(await runScrubCommand([], () => "usage")).toBe(0);
        // The appender kept writing for the whole run.
        expect(readFileSync(ledger, "utf8").split("appended-").length).toBeGreaterThan(appendedBefore);
      } finally {
        writeFileSync(stop, "");
        await exited;
      }

      const logLines = readFileSync(log, "utf8").split("\n").slice(0, -1).map(line => line.trimEnd());
      expect(logLines.slice(0, 20)).toEqual(Array(20).fill("password=[redacted:password]"));
      const appended = logLines.slice(20);
      expect(appended).toEqual(appended.map((_, index) => `appended ${index}`));
      const entries = readFileSync(ledger, "utf8").split("\n").slice(0, -1).map(line => JSON.parse(line) as { type: string; eventId?: string });
      expect(JSON.stringify(entries.slice(0, 20))).not.toContain(token);
      expect(entries.slice(20).map(entry => entry.eventId)).toEqual(appended.map((_, index) => `appended-${index}`));
    } finally {
      live.$client.close();
    }
  });

  it("scrubs held steers under the lock their hook takes, keeping a steer a hook holds meanwhile", async () => {
    const { sessions, token, live } = await scrubFixture();
    try {
      const file = join(sessions, "claude-code-pending", `${createHash("sha256").update("s1").digest("hex")}.json`);
      mkdirSync(join(file, ".."), { recursive: true });
      const steer = (content: string) => ({ sessionId: "s1", requested: "steer", mechanism: "deferred-context", content, ruleIds: [], rules: [] });
      writeFileSync(file, JSON.stringify([steer(`use ${token}`)]));
      const locked = Promise.withResolvers<void>();
      const release = Promise.withResolvers<void>();
      const hook = withPendingFileLock(file, async () => {
        locked.resolve();
        await release.promise;
        writeFileSync(file, JSON.stringify([steer(`use ${token}`), steer("a later steer")]));
      });
      await locked.promise;
      const scrub = runScrubCommand([], () => "usage");
      await delay(100);
      release.resolve();
      await hook;
      expect(await scrub).toBe(0);

      const held = JSON.parse(readFileSync(file, "utf8")) as Array<{ content: string }>;
      expect(held.map(delivery => delivery.content)).toEqual([expect.stringMatching(/^use \[redacted:github-token:[0-9a-f]{6}\]$/u), "a later steer"]);
    } finally {
      live.$client.close();
    }
  });
});

describe("rewriting a file other processes append to", () => {
  async function appendedFile(text: string): Promise<string> {
    const path = join(await temporaryRoot(), "appended.log");
    writeFileSync(path, text);
    return path;
  }
  const opener = (path: string) => async (flags: number) => await open(path, flags);
  const lines = (path: string) => readFileSync(path, "utf8").split("\n").map(line => line.trimEnd());

  it("writes shorter lines over the old ones and keeps a line appended between its read and its write", async () => {
    const path = await appendedFile("keep 1\nsecret-0123456789abcdef\nkeep 2\n");
    let appended = false;
    const changed = await rewriteAppendedFile(opener(path), line => {
      if (!appended) {
        appended = true;
        appendFileSync(path, "appended\n");
      }
      return line.startsWith("secret-") ? "[redacted]" : undefined;
    });
    expect(changed).toBe(1);
    expect(lines(path)).toEqual(["keep 1", "[redacted]", "keep 2", "appended", ""]);
  });

  it("makes room for longer lines without writing over a line appended before or after the room", async () => {
    const path = await appendedFile("keep 1\nsecret\nkeep 2\n");
    let reads = 0;
    const changed = await rewriteAppendedFile(opener(path), line => {
      if (line === "keep 1") {
        reads += 1;
        appendFileSync(path, `appended ${reads}\n`);
      }
      return line === "secret" ? "[redacted:a-longer-replacement]" : undefined;
    });
    expect(changed).toBe(1);
    expect(lines(path)).toEqual(["keep 1", "[redacted:a-longer-replacement]", "keep 2", "appended 1", "appended 2", ""]);
  });

  it("leaves a file whose unfinished last line would change", async () => {
    const path = await appendedFile("keep\npassword=k7Qm2");
    expect(await rewriteAppendedFile(opener(path), line => (line.startsWith("password=") ? "x" : undefined))).toBeUndefined();
    expect(readFileSync(path, "utf8")).toBe("keep\npassword=k7Qm2");
  });
});

describe("the hint after an update", () => {
  it("is given once where there is a database to scrub, and never where there is none", async () => {
    const withDatabase = join(await temporaryRoot(), "home");
    openRerailDatabase(join(withDatabase, "rerail.sqlite")).$client.close();
    expect(scrubHintOnce(withDatabase)).toContain("rerail scrub");
    expect(scrubHintOnce(withDatabase)).toBeUndefined();

    const empty = await temporaryRoot();
    expect(scrubHintOnce(empty)).toBeUndefined();
    openRerailDatabase(join(empty, "rerail.sqlite")).$client.close();
    expect(scrubHintOnce(empty)).toBeUndefined();
  });
});
