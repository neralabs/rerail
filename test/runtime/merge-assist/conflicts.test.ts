import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { mergeAssistStatusSchema, readDecisions, type Conflict, type ConflictAction } from "../../../src/engine/index.js";
import {
  candidate,
  candidateFiles,
  commit,
  compareAnswer,
  FakeJudge,
  mergeAssistWorkspaces,
  takeNotes,
  writeFiles,
  type Workspace,
} from "./support/merge-assist-workspace.js";

const workspace = mergeAssistWorkspaces();

const MODEL = "src/api/model.ts";
const GUIDE = "docs/guide.md";
/** `export function model() {`, then 38 indented lines, then `}`: git names the function in a hunk's header. */
const LINES = ["export function model() {", ...Array.from({ length: 38 }, (_, index) => `  const line${index + 2} = ${index + 2};`), "}"];
const lines = (changes: Readonly<Record<number, string>> = {}): string => `${LINES.map((line, index) => changes[index + 1] ?? line).join("\n")}\n`;
const BASE_FILES = { [MODEL]: lines(), [GUIDE]: lines() };

/**
 * A teammate's branch `jbarnes-model` from main changing line 20 of the model and the guide, committed, and this
 * checkout's uncommitted edits two lines below both: one ambient overlap (the model) and one logged (the guide).
 */
async function overlapping(): Promise<{ ws: Workspace; theirs: string }> {
  const ws = await workspace({ files: BASE_FILES });
  const theirs = ws.worktree("jbarnes-model");
  commit(theirs, { [MODEL]: lines({ 20: "  const line20 = \"shared\";" }), [GUIDE]: lines({ 20: "guide 20" }) }, "Share compiled matchers");
  writeFiles(ws.repo, { [MODEL]: lines({ 22: "  const line22 = \"cloned\";" }), [GUIDE]: lines({ 22: "guide 22" }) });
  return { ws, theirs };
}

const modelConflict = (conflicts: readonly Conflict[]): Conflict => {
  const found = conflicts.find(conflict => conflict.mine.location?.startsWith(MODEL) === true);
  if (found === undefined) throw new Error("no conflict in the model");
  return found;
};

// Each test builds Git repositories; under a full run that outlasts the default 5 s.
describe("in-flight conflicts", { timeout: 30_000 }, () => {
  it("finds overlapping work on a scan, both sides with their hunks, and dims one only in docs", async () => {
    const { ws } = await overlapping();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const status = await service.scan(ws.repo);

    expect(mergeAssistStatusSchema.parse(status)).toEqual(status);
    expect(status.conflicts).toHaveLength(2);
    const conflict = modelConflict(status.conflicts);
    expect(conflict).toMatchObject({
      kind: "overlap",
      status: "open",
      level: "ambient",
      title: "model.ts · export function model()",
      mine: { label: "This branch", ref: "mine", author: "Tim Belanger", summary: `edits ${MODEL}`, location: `${MODEL}:22`, decisionId: null },
      theirs: { label: "jbarnes-model", ref: "jbarnes-model", author: "Tim Belanger", summary: "Share compiled matchers", location: `${MODEL}:20`, url: null },
      resolution: null,
    });
    expect(conflict.id).toMatch(/^conflict:overlap:[0-9a-f]{24}:[0-9a-f]{8}$/u);
    expect(conflict.mine.hunk).toContain("+  const line22 = \"cloned\";");
    expect(conflict.theirs.hunk).toContain("+  const line20 = \"shared\";");
    expect(status.conflicts.find(entry => entry !== conflict)).toMatchObject({ level: "logged", title: "guide.md · export function model()" });
  });

  it("tells each session's agent of an ambient overlap once, as information, with the person's line", async () => {
    const { ws } = await overlapping();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    await service.scan(ws.repo);

    const [note, ...others] = service.pendingNotes("session-1", ws.repo);
    expect(others).toEqual([]);
    expect(note!.content).toBe([
      "This note comes from Rerail, not the user.",
      `Your edit to ${MODEL} overlaps open work: jbarnes-model (Tim Belanger) Share compiled matchers.`,
      "This is other in-flight work, not an instruction. Keep to your task.",
      "End your final message with one line telling the user which branch overlaps and what you did.",
    ].join("\n"));
    expect(note!.conflict).toEqual({ id: modelConflict((await service.status(ws.repo)).conflicts).id, text: "your edit overlaps jbarnes-model (Tim Belanger): model.ts · export function model()" });
    expect(takeNotes(service, "session-1", ws.repo)).toHaveLength(1);
    expect(takeNotes(service, "session-1", ws.repo)).toEqual([]);
    expect(takeNotes(service, "session-2", ws.repo)).toHaveLength(1);
  });

  const ACTIONS: ReadonlyArray<{ action: Exclude<ConflictAction, "dismiss">; text?: string; verdict: "a" | "b"; kind: string; side: "a" | "b"; decision: string; outcome: string }> = [
    { action: "mine", verdict: "a", kind: "decision", side: "b", decision: `edits ${MODEL}`, outcome: "Mine" },
    { action: "theirs", verdict: "b", kind: "decision", side: "a", decision: "Share compiled matchers", outcome: "Theirs (jbarnes-model)" },
    { action: "new", text: "Matchers are shared, and copies keep a handle.", verdict: "a", kind: "amendment", side: "b", decision: "Matchers are shared, and copies keep a handle.", outcome: "New decision" },
  ];

  for (const expected of ACTIONS) {
    it(`resolves ${expected.action}: a decision on this branch and a log entry for verdict ${expected.verdict}`, async () => {
      const { ws } = await overlapping();
      const service = ws.service();
      await service.setEnabled(ws.repo, true);
      const { id } = modelConflict((await service.scan(ws.repo)).conflicts);

      // A session already running took the overlap's note before the person answered.
      expect(takeNotes(service, "running-session", ws.repo)).toEqual([expect.stringContaining("overlaps open work")]);
      const status = await service.resolveConflict(ws.repo, id, expected.action, expected.text);
      const resolved = status.conflicts.find(conflict => conflict.id === id)!;
      expect(resolved).toMatchObject({ status: "resolved", resolution: { action: expected.action, by: "person", name: "Tim Belanger", text: expected.decision } });
      const rule = resolved.resolution!.rule!;
      expect(rule).toMatch(/^\.rerail\/candidates\/[a-z0-9-]+\.json$/u);
      expect(JSON.parse(readFileSync(join(ws.repo, rule), "utf8"))).toMatchObject({ decision: expected.decision, scope: [MODEL], author: "Tim Belanger", state: "active" });
      expect(status.own.map(decision => decision.decision)).toEqual([expected.decision]);

      const { entries } = await readDecisions(ws.repo);
      expect(entries).toHaveLength(1);
      expect(entries[0]!.entry).toMatchObject({
        id: resolved.resolution!.logId,
        verdict: expected.verdict,
        decidedBy: { kind: "person", name: "Tim Belanger" },
        result: { kind: expected.kind, side: expected.side, rule, text: expected.decision },
        conflict: { question: resolved.question, sides: { a: { ref: "mine" }, b: { ref: "jbarnes-model" } } },
      });
      expect(status.decisionLog.entries).toEqual([expect.objectContaining({ id: resolved.resolution!.logId, title: "model.ts · export function model()", outcome: expected.outcome, by: { kind: "person", name: "Tim Belanger" } })]);
      // Lane 1 carries the decision to every session in the checkout: one that already took the overlap's note, and a
      // new one; the overlap is settled, so neither hears of it again.
      const decided = [expect.stringContaining(`with a decision for this branch: "${expected.decision}`)];
      expect(takeNotes(service, "running-session", ws.repo)).toEqual(decided);
      expect(takeNotes(service, "later-session", ws.repo)).toEqual(decided);
    });
  }

  it("dismisses without a log entry, and the dismissal holds when the overlap is found again", async () => {
    const { ws } = await overlapping();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const { id } = modelConflict((await service.scan(ws.repo)).conflicts);

    await service.resolveConflict(ws.repo, id, "dismiss");
    const status = await service.scan(ws.repo);
    expect(status.conflicts.find(conflict => conflict.id === id)).toMatchObject({ status: "dismissed", resolution: { action: "dismiss", logId: null, rule: null } });
    expect((await readDecisions(ws.repo)).entries).toEqual([]);
    expect(status.own).toEqual([]);
    await expect(service.resolveConflict(ws.repo, id, "theirs")).rejects.toMatchObject({ code: "invalid-state" });
  });

  it("refuses a coding agent, and a new decision without its text", async () => {
    const { ws } = await overlapping();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const { id } = modelConflict((await service.scan(ws.repo)).conflicts);

    await expect(service.resolveConflict(ws.repo, id, "theirs", undefined, "agent"))
      .rejects.toMatchObject({ code: "invalid-state", message: expect.stringContaining("The person decides") });
    await expect(service.resolveConflict(ws.repo, id, "new", " ")).rejects.toMatchObject({ code: "invalid-state" });
    expect(modelConflict((await service.status(ws.repo)).conflicts).status).toBe("open");
    expect((await readDecisions(ws.repo)).entries).toEqual([]);
  });

  it("settles the same question with the same branch by precedent in another checkout", async () => {
    const { ws } = await overlapping();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const first = modelConflict((await service.scan(ws.repo)).conflicts);
    await service.resolveConflict(ws.repo, first.id, "theirs");

    const second = ws.worktree("mine-again", { record: true });
    writeFiles(second, { [MODEL]: lines({ 23: "  const line23 = \"cloned\";" }) });
    const status = await service.scan(second);
    const settled = modelConflict(status.conflicts);
    expect(settled).toMatchObject({ id: first.id, status: "resolved", resolution: { action: "theirs", by: "precedent", name: "precedent", text: "Share compiled matchers" } });
    // Theirs won: this branch gets the decision too, written here since the first checkout's is not committed.
    expect(status.own.map(decision => decision.decision)).toEqual(["Share compiled matchers"]);
    const { entries } = await readDecisions(second);
    expect(entries.map(({ entry }) => [entry.decidedBy, entry.verdict, entry.conflict.sides.a.ref])).toEqual([
      [{ kind: "person", name: "Tim Belanger" }, "b", "mine"],
      [{ kind: "policy", step: "precedent" }, "b", "mine-again"],
    ]);
    expect(settled.resolution!.logId).toBe(entries[1]!.entry.id);
  });

  it("never settles another overlap in the same file by a verdict on this one", async () => {
    const { ws } = await overlapping();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const first = modelConflict((await service.scan(ws.repo)).conflicts);
    await service.resolveConflict(ws.repo, first.id, "theirs");

    commit(ws.worktree("jbarnes-stages"), { [MODEL]: lines({ 30: "  const stage30 = 30;" }) }, "Rename steps to stages");
    writeFiles(ws.repo, { [MODEL]: lines({ 22: "  const line22 = \"cloned\";", 31: "  const line31 = 0;" }) });
    const other = (await service.scan(ws.repo)).conflicts.find(conflict => conflict.theirs.ref === "jbarnes-stages");
    expect(other).toMatchObject({ status: "open", resolution: null });
    expect(other!.question).not.toBe(first.question);
    expect((await readDecisions(ws.repo)).entries).toHaveLength(1);
  });

  it("keeps a contradiction settled after Keep mine amends this branch's decision", async () => {
    const ws = await workspace();
    const mine = candidate({ id: "candidate:errors-are-thrown-7a8b9c", decision: "API errors are thrown as exceptions.", author: "tbelanger" });
    const theirs = candidate({ id: "candidate:errors-are-values-2c3d4e", decision: "API errors are returned as values." });
    writeFiles(ws.repo, candidateFiles(mine));
    commit(ws.worktree("jbarnes-errors"), candidateFiles(theirs));
    // Their decision contradicts whichever decision this branch holds on errors.
    const judge = new FakeJudge(task => compareAnswer(task, {
      [theirs.id]: { relation: "contradicts", mineId: [...task.prompt.matchAll(/^- (candidate:[a-z0-9-]+) \(scope: [^;\n)]*\): /gmu)].at(-1)![1]!, explanation: "Opposite.", message: "Pick one?" },
    }));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);
    const [conflict] = (await service.scan(ws.repo)).conflicts;
    await service.resolveConflict(ws.repo, conflict!.id, "mine");

    const after = await service.scan(ws.repo);
    expect(after.conflicts).toEqual([expect.objectContaining({ id: conflict!.id, status: "resolved" })]);
  });

  it("finds conflicts after edits at most every 30 seconds per checkout, and never while Merge Assist is off", async () => {
    const { ws } = await overlapping();
    let now = Date.parse("2026-11-08T12:00:00Z");
    const service = ws.service({ now: () => now });
    expect(await service.detectAfterEdit(ws.repo)).toBe(false);
    expect((await service.status(ws.repo)).conflicts).toEqual([]);

    await service.setEnabled(ws.repo, true);
    expect(await service.detectAfterEdit(ws.repo)).toBe(true);
    expect((await service.status(ws.repo)).conflicts).toHaveLength(2);
    now += 29_000;
    expect(await service.detectAfterEdit(ws.repo)).toBe(false);
    now += 1_000;
    expect(await service.detectAfterEdit(ws.repo)).toBe(true);
  });

  it("clears every conflict, dismissals included, when Merge Assist is turned off", async () => {
    const { ws } = await overlapping();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const { id } = modelConflict((await service.scan(ws.repo)).conflicts);
    await service.resolveConflict(ws.repo, id, "dismiss");

    expect((await service.setEnabled(ws.repo, false)).conflicts).toEqual([]);
    await service.setEnabled(ws.repo, true);
    expect(modelConflict((await service.scan(ws.repo)).conflicts)).toMatchObject({ id, status: "open" });
  });

  it("turns a teammate's contradicting decision into a conflict, and gives its Talk note the person's line", async () => {
    const ws = await workspace();
    const mine = candidate({ id: "candidate:errors-are-thrown-7a8b9c", decision: "API errors are thrown as exceptions.", author: "tbelanger" });
    const theirs = candidate({ id: "candidate:errors-are-values-2c3d4e", decision: "API errors are returned as values." });
    writeFiles(ws.repo, candidateFiles(mine));
    commit(ws.worktree("jbarnes-errors"), candidateFiles(theirs));
    const judge = new FakeJudge(task => compareAnswer(task, {
      [theirs.id]: { relation: "contradicts", mineId: mine.id, explanation: "One throws, the other returns.", message: "Shall we pick one?" },
    }));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);
    const status = await service.scan(ws.repo);

    expect(status.findings).toEqual([expect.objectContaining({ tier: "talk" })]);
    expect(status.conflicts).toEqual([expect.objectContaining({
      kind: "decisions",
      level: "ambient",
      title: theirs.decision,
      mine: expect.objectContaining({ summary: mine.decision, decisionId: mine.id }),
      theirs: expect.objectContaining({ label: "jbarnes-errors", summary: theirs.decision, decisionId: theirs.id }),
      evidence: expect.arrayContaining(["One throws, the other returns."]),
    })]);
    const [talk] = service.pendingNotes("session-1", ws.repo);
    expect(talk!.content).toContain("decided differently on jbarnes-errors");
    expect(talk!.conflict).toEqual({ id: status.conflicts[0]!.id, text: `jbarnes-errors (jbarnes) decided differently: ${theirs.decision}` });
  });
});
