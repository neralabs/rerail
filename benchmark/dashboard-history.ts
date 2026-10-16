// `just rerail-dashboard-history`: seed a throwaway database with a long audit history, serve the daemon's API over
// it, and time what the dashboard asks for: each history read alone, one full poll, and how long a cheap request
// (`/healthz`) waits while dashboard tabs poll (NER-1746, NER-1750). Nothing is written outside a temporary folder.
import { createHash } from "node:crypto";
import { createServer, type Server } from "node:http";
import { mkdtempSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parseArgs } from "node:util";
import { getRequestListener } from "@hono/node-server";
import type Database from "better-sqlite3";
import { Hono } from "hono";
import { createRerailApi } from "../src/api/http.js";
import { operations } from "../src/api/operations.js";
import { DatabaseApiStores } from "../src/api/stores.js";
import { openRerailDatabase } from "../src/engine/index.js";

const DAY = 86_400_000;
const REPOSITORY = "remote:github.com/example/benchmark";
const USAGE = "usage: dashboard-history [--audits N] [--rules N] [--backlog N] [--sessions N] [--tabs N] [--seconds N] [--seed N]";

interface Options { audits: number; rules: number; backlog: number; sessions: number; tabs: number; seconds: number; seed: number }

function options(): Options {
  const { values } = parseArgs({
    options: {
      audits: { type: "string", default: "4000" },
      rules: { type: "string", default: "166" },
      backlog: { type: "string", default: "700" },
      sessions: { type: "string", default: "7" },
      tabs: { type: "string", default: "2" },
      seconds: { type: "string", default: "10" },
      seed: { type: "string", default: "1" },
    },
  });
  const number = (name: keyof typeof values): number => {
    const value = Number(values[name]);
    if (!Number.isSafeInteger(value) || value < 0) throw new Error(`--${name} must be a non-negative integer\n${USAGE}`);
    return value;
  };
  return {
    audits: number("audits"), rules: number("rules"), backlog: number("backlog"), sessions: number("sessions"),
    tabs: number("tabs"), seconds: number("seconds"), seed: number("seed"),
  };
}

/** Deterministic so two runs (before and after a change) seed the same history. */
function random(seed: number): () => number {
  let state = seed >>> 0 || 1;
  return () => {
    state ^= state << 13; state >>>= 0;
    state ^= state >>> 17;
    state ^= state << 5; state >>>= 0;
    return state / 2 ** 32;
  };
}

/**
 * A repository with `rules` rules on: every 25th audit is a repository audit that rewrites the whole standing
 * backlog of unfixed violations, the rest are touched-file audits reporting a handful of them. Rows are written
 * with plain SQL in one transaction, the way any writer (this build or an older one) fills `audits`.
 */
function seed(path: string, { audits, rules, backlog, sessions, seed: seedValue }: Options, now: number): void {
  const database = openRerailDatabase(path);
  const client = database.$client;
  const next = random(seedValue);
  const ruleIds = Array.from({ length: rules }, (_, index) => `benchmark-rule-${index}`);
  const placement = { folder: "", on: true, mode: "steer", suppressions: "honour" };
  const provenance = JSON.stringify(ruleIds.map(ruleId => ({
    ruleId, revision: 1, payloadHash: `sha256:${createHash("sha256").update(ruleId).digest("hex")}`,
    source: { kind: "pack", id: "typescript", packId: "typescript" }, repositoryKey: REPOSITORY,
    checkoutPath: "/benchmark/checkout", placement: [placement],
  })));
  const revisions = JSON.stringify(ruleIds.map(ruleId => ({ ruleId, revision: 1 })));
  const standing = Array.from({ length: backlog }, (_, index) => ({
    id: `sha256:backlog-${index}`, ruleId: ruleIds[Math.floor(next() * rules)]!, clauseId: "clause", code: "CODE",
    mode: next() < 0.3 ? "log" : "steer", message: "A violation message of typical length for a rule.",
    path: `src/file-${index % 300}.ts`,
    span: { startByte: index, endByte: index + 9, startLine: index + 1, startColumn: 1, endLine: index + 1, endColumn: 10 },
    captures: {}, variables: {},
  }));
  const sessionIds = Array.from({ length: sessions }, (_, index) => `benchmark-session-${index}`);
  client.transaction(() => {
    client.prepare("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES (?, ?, 1, 1)")
      .run(REPOSITORY, "benchmark");
    const session = client.prepare(
      "INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES (?, 'claude-code', '/benchmark/checkout', ?, ?, NULL)",
    );
    sessionIds.forEach((id, index) => session.run(id, REPOSITORY, now - 7 * DAY + index));
    const audit = client.prepare(
      "INSERT INTO audits (audit_id, session_id, workspace_root, scope, mode, revision, catalog_revision, rule_revisions_json, "
        + "rule_provenance_json, paths_json, started_at, finished_at, status, violations_json, failures_json) "
        + "VALUES (?, ?, '/benchmark/checkout', ?, 'steer', 1, 'catalog', ?, ?, '[]', ?, ?, 'completed', ?, '[]')",
    );
    for (let index = 0; index < audits; index += 1) {
      const startedAt = now - Math.floor(next() * 6 * DAY);
      const repositoryAudit = index % 25 === 0;
      const violations = repositoryAudit
        ? standing
        : Array.from({ length: 15 }, () => standing[Math.floor(next() * standing.length)]!);
      audit.run(
        `benchmark-audit-${index}`, sessionIds[Math.floor(next() * sessions)]!, repositoryAudit ? "repository" : "touched-files",
        revisions, provenance, startedAt, startedAt + 500, JSON.stringify(violations),
      );
    }
  })();
  client.pragma("wal_checkpoint(TRUNCATE)");
  client.close();
}

const median = (values: readonly number[]): number => [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)]!;
const percentile = (values: readonly number[], p: number): number => [...values].sort((a, b) => a - b)[Math.min(values.length - 1, Math.floor(values.length * p))]!;
const ms = (value: number): string => `${value.toFixed(1).padStart(8)} ms`;

async function timed(url: string): Promise<number> {
  const started = performance.now();
  const response = await fetch(url);
  await response.arrayBuffer();
  if (!response.ok) throw new Error(`${url} answered ${response.status}`);
  return performance.now() - started;
}

function sleep(milliseconds: number): Promise<void> {
  const { promise, resolve } = Promise.withResolvers<void>();
  setTimeout(resolve, milliseconds);
  return promise;
}

async function main(): Promise<void> {
  const settings = options();
  const root = mkdtempSync(join(tmpdir(), "rerail-dashboard-history-"));
  const path = join(root, "rerail.sqlite");
  let server: Server | undefined;
  let stores: DatabaseApiStores | undefined;
  let writer: Database.Database | undefined;
  let committing: NodeJS.Timeout | undefined;
  try {
    const now = Date.now();
    const seeding = performance.now();
    seed(path, settings, now);
    console.log(`seeded ${settings.audits} audits, ${settings.rules} rules, ${settings.backlog} standing violations in ${((performance.now() - seeding) / 1000).toFixed(1)} s`);
    stores = new DatabaseApiStores(path, {
      health: () => ({ pid: process.pid, database: { path, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1",
    });
    console.log(`database ${(statSync(path).size / 1e6).toFixed(0)} MB (after opening, which runs any migration)`);
    const app = new Hono();
    app.get("/healthz", context => context.json({ status: "ok" }));
    app.route("/", createRerailApi(stores, operations, { version: "benchmark" }));
    server = createServer(getRequestListener(app.fetch));
    const listening = Promise.withResolvers<void>();
    server.listen(0, "127.0.0.1", listening.resolve);
    await listening.promise;
    const address = server.address();
    if (address === null || typeof address === "string") throw new Error("no port");
    const base = `http://127.0.0.1:${address.port}`;
    const week = Math.floor((now - 7 * DAY) / 600_000) * 600_000;
    const scope = `repository=${encodeURIComponent(REPOSITORY)}`;
    const reads: Record<string, string> = {
      "rules/activity (7 days)": `/api/rules/activity?since=${week}&${scope}`,
      "rules/activity (since=0)": `/api/rules/activity?since=0&${scope}`,
      "violations (7 days, 200)": `/api/violations?since=${week}&limit=200&${scope}`,
      "sessions": `/api/sessions?${scope}`,
    };
    console.log("\neach read alone: first (computed), then repeated with nothing changed (median of 4)");
    for (const [name, url] of Object.entries(reads)) {
      const first = await timed(base + url);
      const repeats: number[] = [];
      for (let run = 0; run < 4; run += 1) repeats.push(await timed(base + url));
      console.log(`  ${name.padEnd(28)}${ms(first)}${ms(median(repeats))}`);
    }

    // A session at work commits all the time, so every poll then computes its reads afresh.
    writer = openRerailDatabase(path).$client;
    const appendEvent = writer.prepare(
      "INSERT INTO events (session_id, event_id, kind, observed_at, payload_json) VALUES ('benchmark-session-0', ?, 'turn-end', ?, '{}')",
    );
    let events = 0;
    const commit = (): void => { events += 1; appendEvent.run(`benchmark-event-${events}`, Date.now()); };
    const poll = async (): Promise<number> => {
      const started = performance.now();
      await Promise.all(Object.values(reads).map(url => timed(base + url)));
      return performance.now() - started;
    };
    const polls: number[] = [];
    for (let run = 0; run < 5; run += 1) {
      commit();
      polls.push(await poll());
    }
    console.log(`  ${"one full poll after a commit".padEnd(28)}${ms(median(polls))}`);

    console.log(`\nwriting one repository audit of ${settings.backlog} violations`);
    const audit = writer.prepare(
      "INSERT INTO audits (audit_id, session_id, workspace_root, scope, mode, revision, catalog_revision, rule_revisions_json, "
        + "rule_provenance_json, paths_json, started_at, finished_at, status, violations_json, failures_json) "
        + "SELECT 'benchmark-write', session_id, workspace_root, scope, mode, revision, catalog_revision, rule_revisions_json, "
        + "rule_provenance_json, paths_json, ?, NULL, 'running', '[]', '[]' FROM audits WHERE scope = 'repository' LIMIT 1",
    );
    const backlogOf = writer.prepare("SELECT violations_json FROM audits WHERE scope = 'repository' LIMIT 1").pluck();
    const standing = backlogOf.get() as string;
    let started = performance.now();
    audit.run(now);
    const inserted = performance.now() - started;
    started = performance.now();
    writer.prepare("UPDATE audits SET violations_json = ?, status = 'completed' WHERE audit_id = 'benchmark-write'").run(standing);
    console.log(`  insert ${ms(inserted)}, record its ${settings.backlog} violations ${ms(performance.now() - started)}`);

    // Each tab polls back to back (the worst case of the dashboard's 5 s interval with slow polls) while a session
    // commits every 250 ms, and a status-line-like client reads /healthz every 100 ms; the status line gives up after 2 s.
    console.log(`\n/healthz while ${settings.tabs} tab(s) poll for ${settings.seconds} s and a session commits every 250 ms`);
    committing = setInterval(commit, 250);
    const until = performance.now() + settings.seconds * 1000;
    const tabs = Array.from({ length: settings.tabs }, async () => { while (performance.now() < until) await poll(); });
    const probes: number[] = [];
    while (performance.now() < until) {
      probes.push(await timed(`${base}/healthz`));
      await sleep(100);
    }
    await Promise.all(tabs);
    const over = probes.filter(value => value > 2_000).length;
    console.log(`  samples ${probes.length}, median ${ms(median(probes))}, p99 ${ms(percentile(probes, 0.99))}, max ${ms(Math.max(...probes))}, over 2 s: ${over}`);
  } finally {
    clearInterval(committing);
    writer?.close();
    if (server !== undefined) {
      const closed = Promise.withResolvers<void>();
      server.close(() => closed.resolve());
      await closed.promise;
    }
    stores?.close();
    rmSync(root, { recursive: true, force: true });
  }
}

await main();
