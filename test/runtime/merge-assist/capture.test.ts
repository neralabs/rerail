import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { lookupCheckout, mergeAssistItems, newCandidateId } from "../../../src/engine/index.js";
import type { SessionEventRecord } from "../../../src/runtime/session-store.js";
import { ownRuleId } from "../../../src/runtime/merge-assist/activation.js";
import { captureCorrections, type SessionEventSource } from "../../../src/runtime/merge-assist/capture.js";
import {
  candidate,
  candidateFiles,
  captureVerdict,
  FakeJudge,
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
        + `Rerail saved your correction as a decision for this branch: "${saved!.decision}". It applies from now on. `
        + `Undo: Merge Assist page, or \`rerail merge-assist reject ${saved!.id}\`.\nTell the user so in one line.`,
    ]);
    expect(takeNotes(service, "session-1", ws.repo)).toEqual([]);
    expect(takeNotes(service, "session-2", ws.repo)).toEqual([]);
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

  it("keeps a decision whose drafted check fails as a note, and says why", async () => {
    const ws = await workspace();
    const service = ws.service({ judge: new FakeJudge(() => captureVerdict({ check: { kind: "regex", pattern: String.raw`\bnever_written\(` } })) });
    await service.setEnabled(ws.repo, true);

    const [saved] = (await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved;

    expect(saved).toBeDefined();
    expect(saved!.rule).toBeUndefined();
    expect(ws.rulesOn(ws.repo)).toEqual([]);
    expect((await service.status(ws.repo)).own[0]).toMatchObject({ id: saved!.id, level: "note" });
    const [item] = ws.database.select().from(mergeAssistItems).all();
    expect(item!.evidence).toMatchObject({ ruleDropped: "the drafted check fails its case \"the code the agent wrote (1)\"" });
  });

  it("keeps a decision whose drafted check matches too much of the corrected repository as a note", async () => {
    const files = Object.fromEntries(Array.from({ length: 11 }, (_, index) => [`src/api/resource-${index}.ts`, "retry(() => load());\n"]));
    const ws = await workspace({ files });
    const service = ws.service({ judge: new FakeJudge(() => captureVerdict()) });
    await service.setEnabled(ws.repo, true);
    const [saved] = (await service.captureSession({ sessionId: "s", host: "codex", checkoutPath: ws.repo, events: correctedSession(ws.repo, "s") })).saved;
    expect(saved!.rule).toBeUndefined();
    expect(ws.database.select().from(mergeAssistItems).all()[0]!.evidence).toMatchObject({
      ruleDropped: "the drafted check matches 11 places in the corrected repository",
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
