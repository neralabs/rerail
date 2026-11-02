import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { eq, sql } from "drizzle-orm";
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
  const urls: string[] = [];
  let responses: Array<number | "offline"> = [];
  const fetch = vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
    const next = responses.shift() ?? "offline";
    if (next === "offline") throw new TypeError("fetch failed");
    urls.push(String(url));
    requests.push(JSON.parse(String(init?.body)) as Record<string, unknown>);
    return new Response(null, { status: next });
  });
  const env: NodeJS.ProcessEnv = { RERAIL_HOME: home };
  const stdin = { value: "" };
  async function run(...args: string[]): Promise<string> {
    let out = "";
    await runFeedbackCommand(args, () => "usage", {
      home,
      packageVersion: "9.9.9",
      cwd: workspace,
      env,
      stdout: { write: (text: string) => (out += text) },
      stdin: () => stdin.value,
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
  return { home, run, notes, idIn, flushes, requests, urls, env, stdin, fetch, respond: (...next: Array<number | "offline">) => { responses = next; } };
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
      repositoryId: expect.stringMatching(/^clone:/u),
      reason: "pytest asserts are the repo convention",
      rerailVersion: "9.9.9",
      sharing: "ask",
      sentAt: null,
    })]);
    // The person is shown the very line that would go, and every field the request carries.
    expect(output).toContain("\n  pytest asserts are the repo convention\n");
    expect(output).toContain("send it to Nera");
    expect(output).toContain(`the rule id (${RULE}), its rule pack python`);
    expect(output).toContain("your coding agent's name (claude-code), the reason below word for word, the Rerail version (9.9.9), the time, and a random note id");
    expect(output).toContain(`rerail feedback answer ${id} yes`);
    expect(output).toContain(`rerail feedback answer ${id} always`);
    expect(output).toContain(`rerail feedback answer ${id} no`);
    expect(flushes).not.toHaveBeenCalled();
  });

  it("refuses control characters in the reason or the rule id, keeping nothing", async () => {
    const { run, notes } = await steeredRepository();

    await expect(run(RULE, "clear\u001b[2Jthe screen")).rejects.toThrow(/control characters/u);
    await expect(run(`${RULE}\u0007`, "a reason")).rejects.toThrow(/control characters/u);
    expect(notes()).toEqual([]);
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

  it("holds back every note still waiting to go once the answer is never, and a flush sends none", async () => {
    const { run, notes, respond, requests } = await steeredRepository();
    await run("sharing", "always");
    await run(RULE, "queued while offline");
    await run(RULE, "also queued");
    await run("sharing", "ask");
    await run(RULE, "waiting for an answer");

    const said = await run("sharing", "never");
    respond(202, 202, 202);
    await run("flush");

    expect(said).toContain("never asks");
    expect(notes().map(note => [note.sharing, note.destination])).toEqual([["local", null], ["local", null], ["local", null]]);
    expect(requests).toEqual([]);
  });

  it("answering no to one note also keeps the others that were waiting, and says so", async () => {
    const { run, notes, idIn, respond, requests } = await steeredRepository();
    const first = idIn(await run(RULE, "first"));
    await run("answer", first, "yes");
    const second = idIn(await run(RULE, "second"));

    const said = await run("answer", second, "no");
    respond(202);
    await run("flush");

    expect(said).toContain("as 1 other note that had not gone yet");
    expect(notes().map(note => note.sharing)).toEqual(["local", "local"]);
    expect(requests).toEqual([]);
  });

  it("sends only where RERAIL_FEEDBACK_URL is https or local http, and names the override to the person", async () => {
    const { run, notes, idIn, env } = await steeredRepository();

    env.RERAIL_FEEDBACK_URL = "http://evil.example/collect";
    await expect(run(RULE, "x")).rejects.toThrow(/must be https, or http on this computer/u);
    expect(notes()).toEqual([]);
    env.RERAIL_FEEDBACK_URL = "http://127.0.0.1:9/feedback";
    const output = await run(RULE, "local test");
    expect(output).toContain("send it to 127.0.0.1:9 (RERAIL_FEEDBACK_URL, not Nera)");
    const id = idIn(output);
    expect(await run("answer", id, "yes")).toContain("to 127.0.0.1:9 (RERAIL_FEEDBACK_URL, not Nera)");
  });

  it("sends an approved note only where the person was told it would go, whatever the flush's environment says", async () => {
    const { run, notes, idIn, env, respond, urls } = await steeredRepository();
    env.RERAIL_FEEDBACK_URL = "http://127.0.0.1:9/feedback";
    await run("answer", idIn(await run(RULE, "approved for the local endpoint")), "yes");
    env.RERAIL_FEEDBACK_URL = "https://other.example/collect";
    await run("sharing", "always");
    await run(RULE, "queued under always for other.example");

    // The flush runs with no override at all (the daemon's environment, say): each note still goes where it was approved.
    delete env.RERAIL_FEEDBACK_URL;
    respond(202, 202);
    await run("flush");

    expect(urls).toEqual(["http://127.0.0.1:9/feedback", "https://other.example/collect"]);
    expect(notes().map(note => [note.sharing, note.destination])).toEqual([
      ["sent", "http://127.0.0.1:9/feedback"],
      ["sent", "https://other.example/collect"],
    ]);
  });

  it("writes a flush that fails to the flush log, where the person is told to look, and still fails", async () => {
    const { home, run } = await steeredRepository();
    await run("sharing", "always");
    await run(RULE, "cannot be sent");
    const database = openRerailDatabase(defaultRerailDatabasePath());
    database.update(steerFeedback).set({ destination: "not a url" }).run();
    database.$client.close();

    await expect(run("flush")).rejects.toThrow(/Invalid URL/u);

    expect(readFileSync(join(home, "feedback-flush.log"), "utf8")).toMatch(/ failed: Invalid URL\n$/u);
  });

  it("does not open or create the database for a flush while the answer is never", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-feedback-never-"));
    directories.push(root);
    writeFileSync(join(root, "consent.json"), JSON.stringify({ feedback: { sharing: "never", answeredAt: "2026-01-01T00:00:00.000Z" } }));
    const databasePath = join(root, "rerail.sqlite");

    await runFeedbackCommand(["flush"], () => "usage", { home: root, packageVersion: "9.9.9", databasePath, stdout: { write: () => true } });

    expect(existsSync(databasePath)).toBe(false);
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

  it("refuses a yes to one note while the answer is never, so no note waits for a flush that never sends", async () => {
    const { run, notes, idIn } = await steeredRepository();
    const id = idIn(await run(RULE, "asked before never"));
    await run("sharing", "never");

    await expect(run("answer", id, "yes")).rejects.toThrow(/sharing is never/u);
    expect(notes().map(note => note.sharing)).toEqual(["local"]);
  });

  it("tells the agent what a yes did to a note already sent or refused, and starts no flush for it", async () => {
    const { run, notes, idIn, respond, flushes } = await steeredRepository();
    const sent = idIn(await run(RULE, "goes"));
    const refused = idIn(await run(RULE, "bad"));
    await run("answer", sent, "yes");
    await run("answer", refused, "yes");
    respond(202, 400);
    await run("flush");
    expect(notes().map(note => note.sharing)).toEqual(["sent", "rejected"]);
    flushes.mockClear();

    expect(await run("answer", sent, "yes")).toMatch(/^The note on .* was already sent to Nera\./u);
    expect(await run("answer", refused, "always")).toMatch(/^The note on .* was refused by Nera as a bad note and is not sent again\. From now on/u);
    expect(flushes).not.toHaveBeenCalled();
  });

  it("keeps a note queued when the endpoint, not the note, is at fault (404, 401, 500), and logs why it stopped", async () => {
    const { home, run, notes, respond, requests } = await steeredRepository();
    await run("sharing", "always");
    await run(RULE, "moved endpoint");

    for (const status of [404, 401, 500]) {
      respond(status);
      expect(await run("flush")).toContain(`Stopped: nera-rerail-feedback.azurewebsites.net answered ${status}.`);
      expect(notes().map(note => note.sharing)).toEqual(["queued"]);
    }
    expect(requests).toHaveLength(3);
    expect(readFileSync(join(home, "feedback-flush.log"), "utf8")).toMatch(/stopped: nera-rerail-feedback\.azurewebsites\.net answered 500; 1 still queued\n$/u);
    expect(await run("sharing")).toContain("1 approved note is waiting to be sent");
  });

  it("lets two flushes run at once without sending a note twice, and queues again a claim a dead flush left", async () => {
    const { run, notes, respond, requests, fetch } = await steeredRepository();
    await run("sharing", "always");
    await run(RULE, "once only");
    // The first flush's request is in flight (it has reached fetch) when the second flush runs to completion.
    const reached = Promise.withResolvers<void>();
    const held = Promise.withResolvers<void>();
    fetch.mockImplementationOnce(async () => {
      reached.resolve();
      await held.promise;
      requests.push({ reason: "once only" });
      return new Response(null, { status: 202 });
    });
    const first = run("flush");
    await reached.promise;
    respond(202);
    expect(await run("flush")).toContain("Sent 0 feedback notes, 1 still queued.");
    held.resolve();
    expect(await first).toContain("Sent 1 feedback note.");
    expect(requests.map(request => request["reason"])).toEqual(["once only"]);
    expect(notes().map(note => note.sharing)).toEqual(["sent"]);

    // A claim older than the stale limit is a dead flush's; the next flush takes the note over.
    await run(RULE, "left claimed");
    const database = openRerailDatabase(defaultRerailDatabasePath());
    database.update(steerFeedback).set({ sharing: "sending", claimedAt: Date.now() - 60_000 }).where(eq(steerFeedback.reason, "left claimed")).run();
    database.$client.close();
    respond(202);
    expect(await run("flush")).toContain("Sent 1 feedback note.");
    expect(notes().map(note => note.sharing)).toEqual(["sent", "sent"]);
  });

  it("takes the reason on stdin for a line with a quote in it", async () => {
    const { run, notes, stdin } = await steeredRepository();
    stdin.value = "I won't: it's the repo's convention\n";

    await run(RULE, "-");

    expect(notes().map(note => note.reason)).toEqual(["I won't: it's the repo's convention"]);
  });
});
