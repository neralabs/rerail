import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { lookupCheckout, mergeAssistItems, newCandidateId } from "../../../src/engine/index.js";
import type { SessionEventRecord } from "../../../src/runtime/session-store.js";
import { ownRuleId } from "../../../src/runtime/merge-assist/activation.js";
import { captureCorrections, deriveSignals, draftDecision, validateSignals, type CaptureContext, type SessionEventSource } from "../../../src/runtime/merge-assist/capture.js";
import type { CaptureInput, CaptureVerdict, CheckRule } from "../../../src/runtime/merge-assist/judge-prompts.js";
import {
  candidate,
  candidateFiles,
  captureVerdict,
  FakeJudge,
  forbidRule,
  mergeAssistWorkspaces,
  ORDERS_CLEAN,
  ORDERS_RETRY,
  takeNotes,
  writeFiles,
} from "./support/merge-assist-workspace.js";

const workspace = mergeAssistWorkspaces();

const CORRECTION = "No, don't wrap API calls in retry loops; retries belong in the transport.";

/** A session's stored events, as the session store pages them. */
class EventLog implements SessionEventSource {
  readonly records: SessionEventRecord[] = [];

  constructor(readonly sessionId: string) {}

  add(kind: string, fields: Record<string, unknown> = {}): this {
    const sequence = this.records.length + 1;
    this.records.push({ sequence, sessionId: this.sessionId, eventId: `event-${sequence}`, kind, observedAt: sequence, payload: { kind, ...fields } });
    return this;
  }

  events(_sessionId: string, { after = 0, before, limit = 500 }: { after?: number; before?: number; limit?: number }): readonly SessionEventRecord[] {
    const rows = this.records.filter(row => row.sequence > after && (before === undefined || row.sequence < before));
    return before === undefined ? rows.slice(0, limit) : rows.slice(-limit);
  }
}

/** The task, the agent's retry loop, the person's correction, and the agent's fix (plus edits it must not pass on). */
function correctedSession(repo: string, sessionId = "session-1"): EventLog {
  return new EventLog(sessionId)
    .add("user-prompt", { text: "Add an orders endpoint, and don't forget the tests." })
    .add("call", { toolCallId: "1", toolName: "Write", input: { file_path: join(repo, "src/api/orders.ts"), content: ORDERS_RETRY } })
    .add("result", { toolCallId: "1", toolName: "Write", output: "ok" })
    .add("turn-end", { text: "Done." })
    .add("user-prompt", { text: CORRECTION })
    .add("call", { toolCallId: "2", toolName: "Edit", input: { file_path: join(repo, "src/api/orders.ts"), old_string: ORDERS_RETRY, new_string: ORDERS_CLEAN } })
    .add("call", { toolCallId: "3", toolName: "Edit", input: { file_path: join(repo, ".env"), old_string: "TOKEN=a", new_string: "TOKEN=b" } })
    .add("call", { toolCallId: "4", toolName: "Write", input: { file_path: "/etc/elsewhere.ts", content: "outside" } })
    .add("turn-end", { text: "Fixed." });
}

/** A checkout whose transport layer retries in a loop of its own, and whose callers go through it. */
const RETRY_FILES = {
  "src/api/client.ts": "export async function request(path: string): Promise<unknown> {\n  const response = await fetch(path);\n  return response.json();\n}\n",
  "src/api/transport.ts": [
    "/** The one place a request is retried. */",
    "export async function withRetry<T>(call: () => Promise<T>, attempts = 3): Promise<T> {",
    "  let last: unknown;",
    "  for (let attempt = 0; attempt < attempts; attempt += 1) {",
    "    try {",
    "      return await call();",
    "    } catch (error) {",
    "      last = error;",
    "    }",
    "  }",
    "  throw last;",
    "}",
    "",
  ].join("\n"),
  "src/api/users.ts": "import { request } from \"./client\";\nimport { withRetry } from \"./transport\";\n\nexport async function loadUsers(): Promise<unknown> {\n  return withRetry(() => request(\"/users\"));\n}\n",
};

const USERS_LOOP = [
  "import { request } from \"./client\";",
  "",
  "export async function loadUsers(): Promise<unknown> {",
  "  for (let attempt = 0; attempt < 3; attempt += 1) {",
  "    try {",
  "      return await request(\"/users\");",
  "    } catch {}",
  "  }",
  "  throw new Error(\"failed\");",
  "}",
  "",
].join("\n");

const LOOP_THAT_REQUESTS: CheckRule = { pattern: null, nodeKind: "for_statement", has: { pattern: "request($$$ARGS)", nodeKind: null }, inside: null };
const ANY_LOOP: CheckRule = { pattern: null, nodeKind: "for_statement", has: null, inside: null };

/** A verdict whose AST check is `rule`, with the loop the agent wrote as its caught case and the fix as its passed case. */
function loopVerdict(rule: CheckRule): CaptureVerdict {
  return captureVerdict({
    check: { kind: "ast", language: "typescript", rule },
    cases: { caught: [{ path: "src/api/users.ts", content: USERS_LOOP }], passed: [{ path: "src/api/users.ts", content: RETRY_FILES["src/api/users.ts"] }] },
  });
}

// Each test builds Git repositories and compiles rules in the Engine; under a full run that outlasts the default 5 s.
describe("Merge Assist capture", { timeout: 30_000 }, () => {
  it("saves a correction as a code-rule candidate that steers this checkout and no other, and says so once", async () => {
    const ws = await workspace();
    const sibling = ws.worktree("sibling", { record: true });
    const judge = new FakeJudge(() => captureVerdict());
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);

    const result = await service.captureSession({ sessionId: "session-1", host: "claude-code", checkoutPath: ws.repo, events: correctedSession(ws.repo) });

    expect(judge.count("capture")).toBe(1);
    const prompt = judge.tasks[0]!.prompt;
    expect(prompt).toContain(CORRECTION);
    expect(prompt).toContain(`Write ${join(ws.repo, "src/api/orders.ts")}`);
    expect(prompt).toContain("path: src/api/orders.ts");
    expect(prompt).not.toContain("TOKEN=");
    expect(prompt).not.toContain("outside");
    expect(prompt).not.toContain("Add an orders endpoint");
    const [saved] = result.saved;
    expect(saved).toMatchObject({ decision: captureVerdict().decision, scope: ["src/api/**"], author: "Tim Belanger", state: "active", rule: { id: saved!.id, mode: "steer" } });
    expect(existsSync(join(ws.repo, ".rerail/candidates", `${saved!.id.slice("candidate:".length)}.json`))).toBe(true);
    expect(ws.rulesOn(ws.repo)).toEqual([ownRuleId(ws.repo, saved!.id)]);
    expect(ws.rulesOn(sibling)).toEqual([]);

    const status = await service.status(ws.repo);
    expect(status.own).toEqual([expect.objectContaining({ id: saved!.id, level: "code", committed: false, source: { sessionId: "session-1", at: expect.any(String) } })]);
    expect(takeNotes(service, "session-1", ws.repo)).toEqual([
      "This note comes from Rerail, not the user.\n"
        + `Rerail saved your correction as a decision for this branch: "${saved!.decision}" It applies from now on. `
        + `Undo: Merge Assist page, or \`rerail merge-assist reject ${saved!.id}\`.\nTell the user so in one line.`,
    ]);
    expect(takeNotes(service, "session-1", ws.repo)).toEqual([]);
    expect(takeNotes(service, "session-2", ws.repo)).toEqual([]);
  });

  it("stores the most general signal forms that hold against the correction on the candidate", async () => {
    const ws = await workspace();
    // "client.get(" is in the corrected code; " retry( " trims to the first one, and both widen to "retry".
    const judge = new FakeJudge(() => captureVerdict({ signals: ["retry(", "client.get(", " retry( "] }));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);

    const [saved] = (await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved;

    expect(saved!.signals).toEqual(["retry"]);
    const file = JSON.parse(readFileSync(join(ws.repo, ".rerail/candidates", `${saved!.id.slice("candidate:".length)}.json`), "utf8")) as { signals: unknown };
    expect(file.signals).toEqual(["retry"]);
  });

  it("drafts the decision capture would save without writing anything", async () => {
    const ws = await workspace();
    const judge = new FakeJudge(() => captureVerdict({ signals: ["retry("] }));
    const context: CaptureContext = {
      database: ws.database,
      checkout: lookupCheckout(ws.database, ws.repo)!,
      judge,
      author: async () => "tbelanger",
      now: () => Date.parse("2026-11-03T14:00:00Z"),
    };
    const input: CaptureInput = {
      correction: CORRECTION,
      priorCalls: [],
      edits: [{ path: "src/api/orders.ts", before: ORDERS_RETRY, after: ORDERS_CLEAN }],
      ownDecisions: [],
    };

    const drafted = await draftDecision(context, input);

    expect(drafted).toMatchObject({ candidate: { decision: captureVerdict().decision, signals: ["retry"], rule: { mode: "steer" } }, dropped: null });
    expect(existsSync(join(ws.repo, ".rerail/candidates"))).toBe(false);
    expect(ws.database.select().from(mergeAssistItems).all()).toEqual([]);
    expect(await draftDecision({ ...context, judge: new FakeJudge(() => captureVerdict({ generality: "one-off" })) }, input)).toEqual({ oneOff: true });
    expect(await draftDecision({ ...context, judge: new FakeJudge(() => captureVerdict({ duplicateOf: "candidate:retries-3f9a1c" })) }, input)).toEqual({ duplicateOf: "candidate:retries-3f9a1c" });
    // An amendment keeps the scope of the decision it amends, with what the correction touched.
    const own = { ...input, ownDecisions: [{ id: "candidate:retries-3f9a1c", decision: "Retries live in the transport layer.", scope: ["src/http/**"] }] };
    expect(await draftDecision({ ...context, judge: new FakeJudge(() => captureVerdict({ amends: "candidate:retries-3f9a1c" })) }, own))
      .toMatchObject({ candidate: { decision: captureVerdict().decision, scope: ["src/http/**", "src/api/**"], supersedes: "candidate:retries-3f9a1c" } });
    expect(drafted).toMatchObject({ candidate: { supersedes: null } });
    expect(await draftDecision({ ...context, judge: new FakeJudge(() => ({ ok: false, reason: "budget", detail: "the hourly budget is spent" })) }, input))
      .toEqual({ unjudged: "the hourly budget is spent" });
  });

  it("never writes a new decision over another candidate's file when the random id repeats", async () => {
    const ws = await workspace();
    const decision = captureVerdict().decision;
    const existing = candidate({ id: newCandidateId(decision, () => "aaaaaa"), author: "tbelanger", decision: "Something else entirely." });
    writeFiles(ws.repo, candidateFiles(existing));
    const before = readFileSync(join(ws.repo, ".rerail/candidates", `${existing.id.slice("candidate:".length)}.json`), "utf8");
    const suffixes = ["aaaaaa", "bbbbbb"];

    const { saved } = await captureCorrections({
      database: ws.database,
      checkout: lookupCheckout(ws.database, ws.repo)!,
      judge: new FakeJudge(() => captureVerdict()),
      author: async () => "tbelanger",
      now: () => Date.parse("2026-11-03T14:00:00Z"),
      idSuffix: () => suffixes.shift()!,
    }, { sessionId: "session-1", events: correctedSession(ws.repo) });

    expect(saved.map(entry => entry.id)).toEqual([newCandidateId(decision, () => "bbbbbb")]);
    expect(readFileSync(join(ws.repo, ".rerail/candidates", `${existing.id.slice("candidate:".length)}.json`), "utf8")).toBe(before);
  });

  it("keeps a decision whose drafted check still fails its case after the repair round as a note, and says why", async () => {
    const ws = await workspace();
    const judge = new FakeJudge(() => captureVerdict({ check: { kind: "regex", pattern: String.raw`\bnever_written\(` } }));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);

    const [saved] = (await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved;

    expect(saved).toBeDefined();
    expect(saved!.rule).toBeUndefined();
    expect(ws.rulesOn(ws.repo)).toEqual([]);
    expect((await service.status(ws.repo)).own[0]).toMatchObject({ id: saved!.id, level: "note" });
    expect(judge.count("capture")).toBe(2);
    expect(judge.tasks[1]!.prompt).toContain("the caught case src/api/orders.ts (\"the code the agent wrote (1)\") must be caught but is not caught");
    const [item] = ws.database.select().from(mergeAssistItems).all();
    expect(item!.evidence).toMatchObject({ ruleDropped: "the drafted check fails its case \"the code the agent wrote (1)\" (after a repair round)" });
  });

  it("drafts an AST check from a node kind and a node it has, and keeps it when the repository stays clean", async () => {
    const ws = await workspace({ files: RETRY_FILES });
    const judge = new FakeJudge(() => loopVerdict(LOOP_THAT_REQUESTS));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);

    const [saved] = (await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved;

    expect(judge.count("capture")).toBe(1);
    expect(saved!.rule!.clauses[0]).toMatchObject({
      forbid: { ast: { language: "typescript", rule: { kind: "for_statement", has: { pattern: "request($$$ARGS)", stopBy: "end" } } } },
    });
    expect(ws.rulesOn(ws.repo)).toEqual([ownRuleId(ws.repo, saved!.id)]);
  });

  it("rejects a check that matches anywhere in the corrected repository, and keeps the one the repair round corrects", async () => {
    const ws = await workspace({ files: RETRY_FILES });
    let calls = 0;
    const judge = new FakeJudge(() => loopVerdict(++calls === 1 ? ANY_LOOP : LOOP_THAT_REQUESTS));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);

    const [saved] = (await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved;

    expect(judge.tasks.map(task => task.kind)).toEqual(["capture", "capture"]);
    expect(judge.tasks[1]!.cacheKey).not.toBe(judge.tasks[0]!.cacheKey);
    const repair = judge.tasks[1]!.prompt;
    expect(repair).toContain("the check matches 1 place in the repository after the correction, which must match none:");
    expect(repair).toContain("src/api/transport.ts:4: for (let attempt = 0; attempt < attempts; attempt += 1) {");
    expect(repair).toContain("\"nodeKind\": \"for_statement\"");
    expect(saved!.rule!.clauses[0]).toMatchObject({ forbid: { ast: { rule: { kind: "for_statement", has: { pattern: "request($$$ARGS)" } } } } });
    expect(ws.database.select().from(mergeAssistItems).all()[0]!.evidence).toMatchObject({ ruleDropped: null });
  });

  it("keeps a note naming the first match when the repaired check still matches the corrected repository", async () => {
    const ws = await workspace({ files: RETRY_FILES });
    const judge = new FakeJudge(() => loopVerdict(ANY_LOOP));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);

    const [saved] = (await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved;

    expect(judge.count("capture")).toBe(2);
    expect(saved!.rule).toBeUndefined();
    expect(ws.database.select().from(mergeAssistItems).all()[0]!.evidence).toMatchObject({
      ruleDropped: "the drafted check matches 1 place in the corrected repository, first src/api/transport.ts:4 (after a repair round)",
    });
  });

  it("keeps a note when the judge cannot answer the repair round", async () => {
    const files = { "src/api/resource.ts": "retry(() => load());\n" };
    const ws = await workspace({ files });
    let calls = 0;
    const judge = new FakeJudge(() => (++calls === 1 ? captureVerdict() : { ok: false, reason: "budget", detail: "the hourly budget is spent" }));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);

    const [saved] = (await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved;

    expect(saved!.rule).toBeUndefined();
    expect(ws.database.select().from(mergeAssistItems).all()[0]!.evidence).toMatchObject({
      ruleDropped: "the drafted check matches 1 place in the corrected repository, first src/api/resource.ts:1; the repair round failed: the hourly budget is spent",
    });
  });

  it("writes nothing for a one-off fix, and judges each correction once", async () => {
    const ws = await workspace();
    const judge = new FakeJudge(() => captureVerdict({ generality: "one-off" }));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);
    const log = correctedSession(ws.repo);

    expect((await service.captureSession({ sessionId: "session-1", host: "claude-code", checkoutPath: ws.repo, events: log })).saved).toEqual([]);
    expect(existsSync(join(ws.repo, ".rerail/candidates"))).toBe(false);
    expect(judge.count("capture")).toBe(1);

    await service.captureSession({ sessionId: "session-1", host: "claude-code", checkoutPath: ws.repo, events: log });
    expect(judge.count("capture")).toBe(1);

    log.add("user-prompt", { text: "Stop adding comments to every line." }).add("turn-end", { text: "Ok." });
    await service.captureSession({ sessionId: "session-1", host: "claude-code", checkoutPath: ws.repo, events: log });
    expect(judge.count("capture")).toBe(2);
    expect(judge.tasks[1]!.prompt).toContain("Stop adding comments to every line.");
  });

  it("writes nothing for a correction that restates a decision already made", async () => {
    const ws = await workspace();
    const service = ws.service({ judge: new FakeJudge(() => captureVerdict({ duplicateOf: "candidate:retries-in-transport-3f9a1c" })) });
    await service.setEnabled(ws.repo, true);
    expect((await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved).toEqual([]);
  });

  it("saves a correction that changes a decision already made as its amendment, which replaces it", async () => {
    const ws = await workspace();
    const original = candidate({ id: "candidate:no-sleep-in-api-0a0b0c", author: "Tim Belanger", decision: "API code never sleeps.", rule: forbidRule("candidate:no-sleep-in-api-0a0b0c", String.raw`\bsleep\(`) });
    writeFiles(ws.repo, candidateFiles(original));
    const judge = new FakeJudge(() => captureVerdict({ amends: original.id }));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);
    expect(ws.rulesOn(ws.repo)).toEqual([ownRuleId(ws.repo, original.id)]);

    const [saved] = (await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved;

    expect(judge.tasks[0]!.prompt).toContain(`- ${original.id}: API code never sleeps.`);
    expect(saved).toMatchObject({ decision: captureVerdict().decision, supersedes: original.id, state: "active" });
    expect(saved!.id).not.toBe(original.id);
    expect(ws.rulesOn(ws.repo)).toEqual([ownRuleId(ws.repo, saved!.id)]);
    const status = await service.status(ws.repo);
    expect(status.own).toEqual(expect.arrayContaining([
      expect.objectContaining({ id: saved!.id, amends: { id: original.id, decision: original.decision }, amendedBy: null }),
      expect.objectContaining({ id: original.id, state: "active", amends: null, amendedBy: saved!.id }),
    ]));

    // Not a rule, for the amendment: the decision it amended is in force again.
    await service.setCandidateState(ws.repo, saved!.id, "rejected");
    expect(ws.rulesOn(ws.repo)).toEqual([ownRuleId(ws.repo, original.id)]);
    expect((await service.status(ws.repo)).own).toEqual(expect.arrayContaining([expect.objectContaining({ id: original.id, amendedBy: null })]));
  });

  it("does nothing while Merge Assist is off", async () => {
    const ws = await workspace();
    const judge = new FakeJudge(() => captureVerdict());
    const service = ws.service({ judge });
    expect((await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved).toEqual([]);
    expect(judge.tasks).toEqual([]);
  });

  it("saves the decision but applies nothing while Rerail is not set up here", async () => {
    const ws = await workspace({ setUp: false });
    const service = ws.service({ judge: new FakeJudge(() => captureVerdict()) });
    await service.setEnabled(ws.repo, true);
    const [saved] = (await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved;
    expect(saved!.rule).toBeDefined();
    const status = await service.status(ws.repo);
    expect(status.readiness.rulesActive).toBe(false);
    expect(status.own).toMatchObject([{ id: saved!.id, level: "code" }]);
    expect(ws.database.$client.prepare("SELECT count(*) AS n FROM rule_settings WHERE target_id GLOB 'candidate:*'").get()).toEqual({ n: 0 });
  });
});

describe("validateSignals", () => {
  const roots: string[] = [];
  afterEach(async () => {
    await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
  });

  async function repository(files: Record<string, string>): Promise<string> {
    const root = await mkdtemp(join(tmpdir(), "rerail-signals-"));
    roots.push(root);
    for (const [path, content] of Object.entries(files)) {
      mkdirSync(dirname(join(root, path)), { recursive: true });
      writeFileSync(join(root, path), content);
    }
    return root;
  }

  const quietRepository = (): Promise<string> => repository({
    "src/a.ts": "export const a = 1;\n",
    "src/b.ts": "logger.info(\"b\");\n",
    "src/c.ts": "export const c = 3;\n",
  });

  it("keeps the most general form that holds, and a more specific one when the general form is in the corrected code", async () => {
    const root = await quietRepository();

    expect(validateSignals(["console.log"], { before: "console.log(user);", after: "logger.info(user);" }, { root, scope: ["src/**"] })).toEqual(["console."]);
    expect(validateSignals(["console.log"], { before: "console.log(user);", after: "console.info(user);" }, { root, scope: ["src/**"] })).toEqual(["console.log"]);
  });

  it("falls back to a more specific form when the general one is noisy in the repository", async () => {
    const root = await repository({
      "src/a.ts": "console.warn(\"a\");\n",
      "src/b.ts": "console.warn(\"b\");\n",
      "src/c.ts": "export const c = 3;\n",
      "src/d.ts": "export const d = 4;\n",
      "src/e.ts": "export const e = 5;\n",
    });

    expect(validateSignals(["console.log("], { before: "console.log(user);", after: "logger.info(user);" }, { root, scope: ["src/**"] })).toEqual(["console.log"]);
  });

  it("drops a form that is in the corrected code or missing from the code before, and adds what the correction removed", () => {
    const example = { before: "console.log(user);\nexport default handler;", after: "logger.info(user);\nexport { handler };" };

    // user) is in the corrected code and process.env never in the code before; console.log( and export default are mined.
    expect(validateSignals(["user)", "process.env"], example)).toEqual(["console.", "export default"]);
    // throw new and new ApiError( are mined; new ApiError contains ApiError, so only the more general one stays.
    expect(validateSignals([], { before: "throw new ApiError(res);", after: "return err(res);" })).toEqual(["throw", "ApiError"]);
  });

  it("generalizes only the judge's signals without an example", () => {
    expect(validateSignals(["console.log(", "process.env.API_URL"], null)).toEqual(["console.", "process.env"]);
  });

  it("drops a form found in more than a fifth of the files in scope, counting only files in scope", async () => {
    const files: Record<string, string> = {
      "src/a.ts": "export const a = fetch(\"/a\");\n",
      "src/b.ts": "export const b = 1;\n",
      "src/c.ts": "export const c = 2;\n",
      "src/d.ts": "export const d = 3;\n",
      "src/e.ts": "export const e = process.env.E;\n",
      "src/f.ts": "export const f = process.env.F;\n",
      "docs/console.md": "console. console. console.\n",
      "node_modules/x/index.ts": "console.log(1);\n",
    };
    const root = await repository(files);

    // fetch is in 1 of 6 files in scope, process. and process.env in 2 of 6 (more than a fifth); docs/ and node_modules/ do not count.
    expect(validateSignals(["fetch(", "process.env", "console."], null, { root, scope: ["src/**"] })).toEqual(["fetch", "console."]);
    expect(validateSignals(["process.env"], null, { root, scope: ["lib/**"] })).toEqual(["process."]);
  });

  it("ranks the most general first and keeps at most eight", () => {
    const names = [..."abcdefghij"].map((letter, index) => letter.repeat(13 - index));
    const kept = validateSignals(names.map(name => `${name}(`), null);

    expect(kept).toEqual([...names].reverse().slice(0, 8));
  });
});

describe("deriveSignals", () => {
  it("asks the judge for a decision's signals and validates the answer", async () => {
    const judge = new FakeJudge(() => ({ signals: ["console.", "logger.", " console. "] }));

    const signals = await deriveSignals(judge, {
      repositoryKey: "remote:github.com/acme/app",
      decision: "Log through logger, not console.",
      scope: ["src/**"],
      example: { before: "console.log(user);", after: "logger.info(user);" },
    });

    expect(signals).toEqual(["console."]);
    expect(judge.tasks).toHaveLength(1);
    expect(judge.tasks[0]).toMatchObject({ kind: "signals", repositoryKey: "remote:github.com/acme/app" });
    expect(judge.tasks[0]!.prompt).toContain("Log through logger, not console.");
    expect(judge.tasks[0]!.prompt).not.toContain("remote:github.com/acme/app");
  });

  it("keeps the rest of an answer that carries a malformed signal", async () => {
    const judge = new FakeJudge(() => ({ signals: ["toFixed(", "x", "Math.\nround("] }));
    const signals = await deriveSignals(judge, { repositoryKey: "local:app", decision: "Format money with formatCents.", scope: ["src/**"], example: null });
    expect(signals).toEqual(["toFixed"]);
  });

  it("throws when the judge cannot answer, so a failure never reads as no signals", async () => {
    const judge = new FakeJudge(() => ({ ok: false, reason: "unavailable", detail: "no agent CLI is signed in" }));
    await expect(deriveSignals(judge, { repositoryKey: "local:app", decision: "Log through logger.", scope: ["src/**"], example: null }))
      .rejects.toThrow("no agent CLI is signed in");
  });
});
