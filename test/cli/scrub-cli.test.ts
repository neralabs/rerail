import { createHash, randomUUID } from "node:crypto";
import { spawnSync } from "node:child_process";
import { mkdirSync, readFileSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deserialize, serialize } from "node:v8";
import { afterEach, describe, expect, it, vi } from "vitest";
import { runScrubCommand, scrubHintOnce } from "../../src/cli/scrub-cli.js";
import { openRerailDatabase, recordCheckout } from "../../src/engine/index.js";

const directories: string[] = [];
afterEach(async () => {
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
