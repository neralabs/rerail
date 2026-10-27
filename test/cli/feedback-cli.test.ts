import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { sql } from "drizzle-orm";
import { afterEach, describe, expect, it, vi } from "vitest";
import { runFeedbackCommand } from "../../src/cli/feedback-cli.js";
import { defaultRerailDatabasePath, openRerailDatabase, steerFeedback } from "../../src/engine/index.js";
import { SqliteSessionStore } from "../../src/runtime/index.js";
import { installPack } from "../support/installed-packs.js";
import { startTestRepository } from "../support/started-repository.js";

const RULE = "bandit-b101-assert-used";
const directories: string[] = [];

afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(directories.splice(0).map(directory => rm(directory, { recursive: true, force: true })));
});

/**
 * A Rerail home with the Python pack installed and a started git repository in which a Claude Code session was
 * steered about `RULE`; `run` is `rerail feedback` from that repository, with the flush and the API faked.
 */
async function steeredRepository() {
  const root = await mkdtemp(join(tmpdir(), "rerail-feedback-"));
  directories.push(root);
  const home = join(root, "home");
  vi.stubEnv("RERAIL_HOME", home);
  installPack("python", home);
  const workspace = join(root, "repo");
  mkdirSync(workspace);
  execFileSync("git", ["init", "-q", workspace]);
  const repositoryId = startTestRepository(workspace);
  const store = new SqliteSessionStore(defaultRerailDatabasePath(), { repositoryId });
  const session = { host: "claude-code" as const, sessionId: "steered", cwd: workspace };
  store.openSession(session);
  store.recordSteer(session, "receipt", [RULE]);
  store.close();

  const flushes = vi.fn();
  const requests: Array<Record<string, unknown>> = [];
  let responses: Array<number | "offline"> = [];
  const fetch = vi.fn(async (_url: string | URL | Request, init?: RequestInit) => {
    const next = responses.shift() ?? "offline";
    if (next === "offline") throw new TypeError("fetch failed");
    requests.push(JSON.parse(String(init?.body)) as Record<string, unknown>);
    return new Response(null, { status: next });
  });
  async function run(...args: string[]): Promise<string> {
    let out = "";
    await runFeedbackCommand(args, () => "usage", {
      home,
      packageVersion: "9.9.9",
      cwd: workspace,
      stdout: { write: (text: string) => (out += text) },
      startFlush: flushes,
      fetch: fetch as typeof globalThis.fetch,
    });
    return out;
  }
  const notes = () => {
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      return database.select().from(steerFeedback).orderBy(sql`rowid`).all();
    } finally {
      database.$client.close();
    }
  };
  const idIn = (output: string): string => /note ([0-9a-f-]{36})/u.exec(output)![1]!;
  return { home, run, notes, idIn, flushes, requests, respond: (...next: Array<number | "offline">) => { responses = next; } };
}

describe("rerail feedback", () => {
  it("keeps the note with its rule's pack version and the session the rule steered, and asks the user before sending anything", async () => {
    const { run, notes, idIn, flushes } = await steeredRepository();

    const output = await run(RULE, "pytest", "asserts\nare the   repo convention");

    const id = idIn(output);
    expect(notes()).toEqual([expect.objectContaining({
      id,
      ruleId: RULE,
      packId: "python",
      packVersion: JSON.parse(readFileSync(join("packs", "python", "package.json"), "utf8")).version,
      host: "claude-code",
      sessionId: "steered",
      reason: "pytest asserts are the repo convention",
      rerailVersion: "9.9.9",
      sharing: "ask",
      sentAt: null,
    })]);
    expect(output).toContain(`rerail feedback answer ${id} yes`);
    expect(output).toContain(`rerail feedback answer ${id} always`);
    expect(output).toContain(`rerail feedback answer ${id} no`);
    expect(flushes).not.toHaveBeenCalled();
  });

  it("refuses a rule Rerail does not know, keeping nothing", async () => {
    const { run, notes } = await steeredRepository();

    await expect(run("no-such-rule", "it does not apply")).rejects.toThrow(/no rule no-such-rule/u);
    expect(notes()).toEqual([]);
  });

  it("sends the note on yes and asks again about the next one", async () => {
    const { run, notes, idIn, flushes } = await steeredRepository();
    const id = idIn(await run(RULE, "tests assert"));

    await run("answer", id, "yes");
    const next = await run(RULE, "still tests");

    expect(notes().map(note => note.sharing)).toEqual(["queued", "ask"]);
    expect(flushes).toHaveBeenCalledTimes(1);
    expect(next).toContain("rerail feedback answer");
  });

  it("sends the note and every later one without asking on always, keeping the sessions approval", async () => {
    const { home, run, notes, idIn, flushes } = await steeredRepository();
    writeFileSync(join(home, "consent.json"), JSON.stringify({ sessions: { approvedAt: "2026-01-01T00:00:00.000Z" } }));
    const id = idIn(await run(RULE, "tests assert"));

    await run("answer", id, "always");
    const next = await run(RULE, "still tests");

    expect(notes().map(note => note.sharing)).toEqual(["queued", "queued"]);
    expect(flushes).toHaveBeenCalledTimes(2);
    expect(next).not.toContain("rerail feedback answer");
    expect(JSON.parse(readFileSync(join(home, "consent.json"), "utf8"))).toMatchObject({
      sessions: { approvedAt: "2026-01-01T00:00:00.000Z" },
      feedback: { sharing: "always" },
    });
  });

  it("keeps the note and every later one local on no, never asking again until sharing is set back to ask", async () => {
    const { run, notes, idIn, flushes } = await steeredRepository();
    const id = idIn(await run(RULE, "tests assert"));

    await run("answer", id, "no");
    const next = await run(RULE, "still tests");
    expect(await run("sharing")).toContain("never asks");
    await run("sharing", "ask");
    const asked = await run(RULE, "third time");

    expect(notes().map(note => note.sharing)).toEqual(["local", "local", "ask"]);
    expect(flushes).not.toHaveBeenCalled();
    expect(next).not.toContain("rerail feedback answer");
    expect(asked).toContain("rerail feedback answer");
  });

  it("flushes approved notes only, without the session, never retries one the API refused, and keeps the rest queued while it is unreachable", async () => {
    const { run, notes, idIn, respond, requests } = await steeredRepository();
    const local = idIn(await run(RULE, "not approved"));
    await run("sharing", "always");
    for (const reason of ["accepted", "malformed", "offline one", "offline two"]) await run(RULE, reason);

    respond(202, 400, "offline");
    await run("flush");

    const byReason = Object.fromEntries(notes().map(note => [note.reason, note]));
    expect(byReason["not approved"]).toMatchObject({ id: local, sharing: "ask" });
    expect(byReason["accepted"]).toMatchObject({ sharing: "sent", sentAt: expect.any(Number) });
    expect(byReason["malformed"]).toMatchObject({ sharing: "rejected", sentAt: null });
    expect(byReason["offline one"]).toMatchObject({ sharing: "queued" });
    expect(byReason["offline two"]).toMatchObject({ sharing: "queued" });
    expect(requests[0]).toEqual({
      id: byReason["accepted"]!.id,
      ruleId: RULE,
      packId: "python",
      packVersion: byReason["accepted"]!.packVersion,
      agent: "claude-code",
      reason: "accepted",
      rerailVersion: "9.9.9",
      recordedAt: new Date(byReason["accepted"]!.recordedAt).toISOString(),
    });

    respond(202, 202);
    await run("flush");
    expect(notes().filter(note => note.sharing === "sent").map(note => note.reason).sort()).toEqual(["accepted", "offline one", "offline two"]);
    expect(requests.map(request => request["reason"])).toEqual(["accepted", "malformed", "offline one", "offline two"]);
  });
});
