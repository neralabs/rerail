import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";
import { mergeAssistStatusSchema, readWorktreeDecisions, type Conflict, type ConflictAction } from "../../../src/engine/index.js";
import {
  candidate,
  candidateFiles,
  commit,
  compareAnswer,
  FakeJudge,
  git,
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

/** Their change to line 20 of the model: the line, and five lines that build on it. */
const SHARED = ["  const line20 = \"shared\";", ...Array.from({ length: 5 }, (_, index) => `  const shared${index} = line20;`)].join("\n");

/**
 * A teammate's branch `jbarnes-model` from main changing line 20 of the model (and building five lines on it) and of
 * the guide, committed, and this checkout's uncommitted edits two lines below both: one ambient overlap (the model:
 * 7 lines in the hour, 1 caller, so 168 lines of expected rework) and one logged (the guide is docs).
 */
async function overlapping(): Promise<{ ws: Workspace; theirs: string }> {
  const ws = await workspace({ files: BASE_FILES });
  const theirs = ws.worktree("jbarnes-model");
  commit(theirs, { [MODEL]: lines({ 20: SHARED }), [GUIDE]: lines({ 20: "guide 20" }) }, "Share compiled matchers");
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
  it("finds overlapping work on a scan, both sides with their hunks, levelled by the dimmer, and logs one only in docs", async () => {
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
      title: "model.ts · export function model",
      mine: { label: "This branch", ref: "mine", author: "Tim Belanger", summary: `edits ${MODEL}`, location: `${MODEL}:22`, decisionId: null },
      theirs: { label: "jbarnes-model", ref: "jbarnes-model", author: "Tim Belanger", summary: "Share compiled matchers", location: `${MODEL}:20`, url: null },
      resolution: null,
    });
    expect(conflict.id).toMatch(/^conflict:overlap:[0-9a-f]{24}:[0-9a-f]{8}$/u);
    expect(conflict.mine.hunk).toContain("+  const line22 = \"cloned\";");
    expect(conflict.theirs.hunk).toContain("+  const line20 = \"shared\";");
    expect(conflict).toMatchObject({
      shortId: expect.stringMatching(/^[0-9a-f]{6}$/u),
      signals: { quiet: false, lockIn: 1, lockInClass: "local", callers: 1, otherRefs: 0, linesTheirs: 6, linesMine: 1, windowHours: 1, hoursToMerge: 24, pullRequest: false, verdict: false },
      score: 168,
      reasons: ["local code: ×1", "+6 lines on theirs and +1 on this branch in 1 h", "1 caller", "about 24 h to merge", "no verdict yet: even odds either side loses"],
      queued: false,
      snooze: null,
    });
    expect(status.conflicts.find(entry => entry !== conflict)).toMatchObject({ level: "logged", title: "guide.md · export function model", signals: { quiet: true } });
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
    expect(note!.conflict).toEqual({ id: modelConflict((await service.status(ws.repo)).conflicts).id, text: "your edit overlaps jbarnes-model (Tim Belanger): model.ts · export function model" });
    expect(takeNotes(service, "session-1", ws.repo)).toHaveLength(1);
    expect(takeNotes(service, "session-1", ws.repo)).toEqual([]);
    expect(takeNotes(service, "session-2", ws.repo)).toHaveLength(1);
  });

  const ACTIONS: ReadonlyArray<{ action: Exclude<ConflictAction, "dismiss">; text?: string; verdict: "a" | "b" | "new"; kind: string; decision: string; outcome: string }> = [
    // With no text, the winning side's change and where: this branch's (mine) is uncommitted, theirs is a commit's subject.
    { action: "mine", verdict: "a", kind: "decision", decision: `The change on mine (${MODEL}:22)`, outcome: "Mine" },
    { action: "theirs", verdict: "b", kind: "decision", decision: `Share compiled matchers (${MODEL}:20)`, outcome: "Theirs (jbarnes-model)" },
    { action: "new", text: "Matchers are shared, and copies keep a handle.", verdict: "new", kind: "amendment", decision: "Matchers are shared, and copies keep a handle.", outcome: "New decision" },
  ];

  for (const expected of ACTIONS) {
    it(`resolves ${expected.action}: a decision and a ledger entry for verdict ${expected.verdict} in this branch's working tree`, async () => {
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
      const written = JSON.parse(readFileSync(join(ws.repo, rule), "utf8")) as { id: string };
      expect(written).toMatchObject({ decision: expected.decision, scope: [MODEL], author: "Tim Belanger", state: "active" });
      expect(status.own.map(decision => decision.decision)).toEqual([expected.decision]);

      const { entries } = await readWorktreeDecisions(ws.repo);
      expect(entries).toHaveLength(1);
      expect(entries[0]).toMatchObject({
        id: resolved.resolution!.logId,
        question: resolved.question,
        verdict: expected.verdict,
        decidedBy: { kind: "person", name: "Tim Belanger" },
        result: { candidateId: written.id, kind: expected.kind, rule, text: expected.decision, follows: null },
        sides: { a: { ref: "mine", label: "mine", at: { kind: "hunks", path: MODEL } }, b: { ref: "jbarnes-model", label: "jbarnes-model" } },
        supersedes: null,
      });
      // Nothing is committed: the person's branch carries the files once they commit them.
      expect(git(ws.repo, "status", "--porcelain", "--", ".rerail/decisions")).toBe(`?? .rerail/decisions/`);
      expect(status.decisionLog.questions).toEqual([{ question: resolved.question, title: "model.ts · export function model", items: [expect.objectContaining({
        id: resolved.resolution!.logId, status: "provisional", outcome: expected.outcome, by: { kind: "person", name: "Tim Belanger" }, source: { ref: "mine", label: "This branch" },
      })] }]);
      // Lane 1 carries the decision to every session in the checkout: one that already took the overlap's note, and a
      // new one; the overlap is settled, so neither hears of it again.
      const decided = [expect.stringContaining(`with a decision for this branch: "${expected.decision}`)];
      expect(takeNotes(service, "running-session", ws.repo)).toEqual(decided);
      expect(takeNotes(service, "later-session", ws.repo)).toEqual(decided);
    });
  }

  it("dismisses without a ledger entry, and the dismissal holds when the overlap is found again", async () => {
    const { ws } = await overlapping();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const { id } = modelConflict((await service.scan(ws.repo)).conflicts);

    await service.resolveConflict(ws.repo, id, "dismiss");
    const status = await service.scan(ws.repo);
    expect(status.conflicts.find(conflict => conflict.id === id)).toMatchObject({ status: "dismissed", resolution: { action: "dismiss", logId: null, rule: null } });
    expect((await readWorktreeDecisions(ws.repo)).entries).toEqual([]);
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
    expect((await readWorktreeDecisions(ws.repo)).entries).toEqual([]);
  });

  it("settles the same dispute by precedent in another checkout once the verdict is committed, writing nothing there, and only shows it for another dispute", async () => {
    const { ws } = await overlapping();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const first = modelConflict((await service.scan(ws.repo)).conflicts);
    const answered = (await service.resolveConflict(ws.repo, first.id, "theirs")).conflicts.find(conflict => conflict.id === first.id)!;

    // The same change as this checkout's, against the same change of theirs: the same dispute.
    const second = ws.worktree("mine-again", { record: true });
    writeFiles(second, { [MODEL]: lines({ 22: "  const line22 = \"cloned\";" }) });
    // Not committed yet: no other checkout can read the verdict.
    expect(modelConflict((await service.scan(second)).conflicts)).toMatchObject({ status: "open", resolution: null });

    git(ws.repo, "add", "--", ".rerail");
    git(ws.repo, "commit", "-q", "-m", "Take their matchers");
    await service.setEnabled(second, false);
    await service.setEnabled(second, true);
    const status = await service.scan(second);
    const settled = modelConflict(status.conflicts);
    expect(settled).toMatchObject({ id: first.id, status: "resolved", resolution: { action: "theirs", by: "precedent", name: "precedent", logId: answered.resolution!.logId, rule: null } });
    // Precedent is local state: no entry and no decision in this worktree.
    expect((await readWorktreeDecisions(second)).entries).toEqual([]);
    expect(status.own).toEqual([]);
    expect(git(second, "status", "--porcelain", "--", ".rerail")).toBe("");
    expect(status.decisionLog.questions[0]!.items.map(item => [item.id, item.status, item.source.label])).toEqual([[answered.resolution!.logId, "provisional", "mine"]]);

    // Another change on the same lines is another dispute: the person decides, with the earlier verdict as context.
    const third = ws.worktree("other-change", { record: true });
    writeFiles(third, { [MODEL]: lines({ 23: "  const line23 = \"cloned\";" }) });
    const open = modelConflict((await service.scan(third)).conflicts);
    expect(open).toMatchObject({ question: first.question, status: "open", resolution: null });
    expect(open.evidence.at(-1)).toMatch(/^Earlier: Tim Belanger chose jbarnes-model on \d{4}-\d{2}-\d{2}\.$/u);
  });

  it("keeps a verdicts conflict apart from the overlap on the same question, and gives agents neither side's verdict while it is open", async () => {
    const ws = await workspace({ files: BASE_FILES });
    const theirsPath = ws.worktree("jbarnes-model", { record: true });
    commit(theirsPath, { [MODEL]: lines({ 20: "  const line20 = \"shared\";" }) }, "Share compiled matchers");
    // A later commit of theirs to the same file, away from the contested line, is not what their side says.
    commit(theirsPath, { [MODEL]: lines({ 20: "  const line20 = \"shared\";", 36: "  const line36 = 0;" }) }, "Tidy the tail");
    commit(ws.repo, { [MODEL]: lines({ 20: "  const line20 = \"cloned\";" }) }, "Clone matchers");
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const mineOverlap = modelConflict((await service.scan(ws.repo)).conflicts);
    const theirOverlap = (await service.scan(theirsPath)).conflicts.find(conflict => conflict.kind === "overlap")!;
    // Both sides changed the same base line: one question from either side.
    expect(theirOverlap.question).toBe(mineOverlap.question);
    // Each side is the commit that changed the contested line.
    expect([mineOverlap.mine.summary, mineOverlap.theirs.summary]).toEqual(["Clone matchers", "Share compiled matchers"]);

    // Each keeps its own side and commits it.
    await service.resolveConflict(ws.repo, mineOverlap.id, "mine");
    git(ws.repo, "add", "-A");
    git(ws.repo, "commit", "-q", "-m", "Keep cloned matchers");
    await service.resolveConflict(theirsPath, theirOverlap.id, "mine");
    git(theirsPath, "add", "-A");
    git(theirsPath, "commit", "-q", "-m", "Keep shared matchers");

    const conflicts = (await service.scan(ws.repo)).conflicts.filter(conflict => conflict.theirs.ref === "jbarnes-model");
    expect(conflicts.map(conflict => [conflict.kind, conflict.status])).toEqual([["verdicts", "open"], ["overlap", "resolved"]]);
    expect(new Set(conflicts.map(conflict => conflict.id)).size).toBe(2);
    const verdicts = conflicts[0]!;
    expect(verdicts.question).toBe(mineOverlap.question);
    // This branch's entry may not be pushed: no time for it; theirs is on their committed branch.
    expect(verdicts.mine.pushedAt).toBeNull();
    expect(verdicts.theirs.pushedAt).not.toBeNull();

    // While it is open, neither verdict is guidance: not this branch's own, not theirs.
    const guidance = /Rerail added a decision|settled a conflict with|recorded a verdict on a conflict that has not merged/u;
    expect(takeNotes(service, "session-open", ws.repo).filter(note => guidance.test(note))).toEqual([]);
    // Once it is settled, their verdict reaches agents only as information about unmerged work.
    await service.resolveConflict(ws.repo, verdicts.id, "dismiss");
    const notes = takeNotes(service, "session-after", ws.repo);
    expect(notes.some(note => note.includes("Rerail added a decision from jbarnes-model"))).toBe(false);
    expect(notes).toEqual(expect.arrayContaining([expect.stringContaining("jbarnes-model recorded a verdict on a conflict that has not merged yet.")]));
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
    expect((await readWorktreeDecisions(ws.repo)).entries).toHaveLength(1);
  });

  it("reopens a settled overlap when the winner's position moves, and the verdict no longer counts", async () => {
    const { ws, theirs } = await overlapping();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const { id } = modelConflict((await service.scan(ws.repo)).conflicts);
    await service.resolveConflict(ws.repo, id, "theirs");
    expect(modelConflict((await service.scan(ws.repo)).conflicts)).toMatchObject({ id, status: "resolved" });

    commit(theirs, { [MODEL]: lines({ 20: SHARED.replace("\"shared\"", "\"shared, behind an Arc\"") }) }, "Arc the matchers");
    const status = await service.scan(ws.repo);
    expect(modelConflict(status.conflicts)).toMatchObject({ id, status: "open", resolution: null });
    expect(status.decisionLog.questions[0]!.items.map(item => item.status)).toEqual(["reopened"]);
  });

  it("raises a verdicts conflict when another branch decided the same question the other way, and taking theirs follows their decision", async () => {
    const { ws } = await overlapping();
    const service = ws.service();
    await service.setEnabled(ws.repo, true);
    const other = ws.worktree("other", { record: true });
    writeFiles(other, { [MODEL]: lines({ 22: "  const line22 = \"cloned\";" }) });
    await service.setEnabled(other, true);
    const otherOverlap = modelConflict((await service.scan(other)).conflicts);

    // This checkout keeps its own side and commits; the other takes jbarnes-model's.
    const mine = modelConflict((await service.scan(ws.repo)).conflicts);
    await service.resolveConflict(ws.repo, mine.id, "mine");
    git(ws.repo, "add", "-A");
    git(ws.repo, "commit", "-q", "-m", "Keep cloned matchers");
    await service.resolveConflict(other, otherOverlap.id, "theirs");

    const status = await service.scan(other);
    const verdicts = status.conflicts.find(conflict => conflict.kind === "verdicts");
    expect(verdicts).toMatchObject({
      status: "open", question: mine.question, title: "model.ts · export function model",
      mine: { label: "This branch", summary: `Share compiled matchers (${MODEL}:20)` },
      theirs: { label: "mine", ref: "mine", author: "Tim Belanger", summary: `The change on mine (${MODEL}:22)` },
    });
    expect(verdicts!.theirs.decisionId).toMatch(/^candidate:the-change-on-mine-/u);

    const answered = await service.resolveConflict(other, verdicts!.id, "theirs");
    const followed = JSON.parse(readFileSync(join(other, answered.conflicts.find(conflict => conflict.id === verdicts!.id)!.resolution!.rule!), "utf8")) as { follows?: unknown };
    expect(followed.follows).toEqual({ ref: "mine", candidateId: verdicts!.theirs.decisionId });
    const after = await service.scan(other);
    expect(after.conflicts.find(conflict => conflict.id === verdicts!.id)).toMatchObject({ status: "resolved" });
    const own = after.decisionLog.questions.find(question => question.question === mine.question)!.items.filter(item => item.source.label === "This branch");
    expect(own.map(item => item.status)).toEqual(["provisional", "superseded"]);
  });

  it("gives a contradiction the same question on both branches, so their verdicts can meet", async () => {
    const ws = await workspace();
    const mine = candidate({ id: "candidate:errors-are-thrown-7a8b9c", decision: "API errors are thrown as exceptions." });
    const theirs = candidate({ id: "candidate:errors-are-values-2c3d4e", decision: "API errors are returned as values." });
    commit(ws.repo, candidateFiles(mine));
    const theirsPath = ws.worktree("jbarnes-errors", { record: true });
    commit(theirsPath, candidateFiles(theirs));
    const judge = new FakeJudge(task => compareAnswer(task, {
      [theirs.id]: { relation: "contradicts", mineId: mine.id, explanation: "Opposite.", message: "Pick one?" },
      [mine.id]: { relation: "contradicts", mineId: theirs.id, explanation: "Opposite.", message: "Pick one?" },
    }));
    const service = ws.service({ judge });
    await service.setEnabled(ws.repo, true);
    const here = (await service.scan(ws.repo)).conflicts.find(conflict => conflict.kind === "decisions");
    const there = (await service.scan(theirsPath)).conflicts.find(conflict => conflict.kind === "decisions");
    expect(here).toMatchObject({ mine: { decisionId: mine.id }, theirs: { decisionId: theirs.id } });
    expect(there).toMatchObject({ mine: { decisionId: theirs.id }, theirs: { decisionId: mine.id } });
    expect(there!.question).toBe(here!.question);
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
    // Ten lines of theirs build on their decision in its scope: 120 lines of expected rework, so ambient.
    const errors = Array.from({ length: 10 }, (_, index) => `export const error${index} = { ok: false, code: ${index} };`).join("\n");
    commit(ws.worktree("jbarnes-errors"), { ...candidateFiles(theirs), "src/api/errors.ts": `${errors}\n` });
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
