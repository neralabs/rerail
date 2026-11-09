import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { CANDIDATE_FORMAT, serializeCandidate, type Candidate } from "../../../src/engine/merge-assist/candidate.js";
import {
  DECISION_ENTRY_FORMAT,
  decisionEntryId,
  type DecisionDraft,
  type DecisionEntry,
  sealDecisionEntry,
} from "../../../src/engine/merge-assist/decision-entry.js";
import { positionFingerprint, readWorktreeDecisions, writeWorktreeDecision } from "../../../src/engine/merge-assist/decision-files.js";
import {
  decidePrecedent,
  followedDecision,
  hasVerdict,
  type LedgerItem,
  type LedgerMemory,
  ledgerLog,
  readLedger,
  storedLedger,
  verdictDisagreements,
} from "../../../src/engine/merge-assist/ledger.js";
import type { LedgerStatus } from "../../../src/engine/merge-assist/model.js";

const folders: string[] = [];
afterEach(async () => {
  await Promise.all(folders.splice(0).map(folder => rm(folder, { recursive: true, force: true })));
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" }).trim();
}

function write(root: string, files: Record<string, string | null>): void {
  for (const [path, content] of Object.entries(files)) {
    if (content === null) rmSync(join(root, path), { force: true });
    else {
      mkdirSync(dirname(join(root, path)), { recursive: true });
      writeFileSync(join(root, path), content);
    }
  }
}

function commit(root: string, files: Record<string, string | null>, message = "change"): string {
  write(root, files);
  git(root, "add", "-A");
  git(root, "commit", "--quiet", "-m", message);
  return git(root, "rev-parse", "HEAD");
}

const FILE = "src/a.ts";
/** Twelve lines; `changes` replaces some, by line number. */
const lines = (changes: Record<number, string> = {}): string => `${Array.from({ length: 12 }, (_, index) => changes[index + 1] ?? `const line${index + 1} = ${index + 1};`).join("\n")}\n`;
/** The contested lines: line 5 of the base. */
const AT = { kind: "hunks" as const, path: FILE, lines: [{ lo: 5, hi: 5 }] };

interface Repo {
  readonly folder: string;
  /** The checkout under test, on branch `work` from main, with nothing of its own yet. */
  readonly root: string;
  readonly base: string;
  /** A linked worktree on a new branch from main. */
  branch(name: string, from?: string): string;
  /** A linked worktree on main itself. */
  main(): string;
}

async function repository(): Promise<Repo> {
  const folder = realpathSync(await mkdtemp(join(tmpdir(), "rerail-ledger-")));
  folders.push(folder);
  const root = join(folder, "repo");
  mkdirSync(root);
  git(root, "init", "--quiet", "-b", "main");
  const base = commit(root, { [FILE]: lines() }, "seed");
  git(root, "switch", "--quiet", "-c", "work");
  return {
    folder, root, base,
    branch: (name, from = "main") => {
      const path = join(folder, name);
      git(root, "worktree", "add", "--quiet", "-b", name, path, from);
      return path;
    },
    main: () => {
      const path = join(folder, "main-checkout");
      git(root, "worktree", "add", "--quiet", path, "main");
      return path;
    },
  };
}

interface SideInput {
  readonly ref: string;
  readonly sha: string;
  /** Where the position is read: a commit, or null for `root`'s working files. */
  readonly at: string | null;
}

/** An entry on question `q`: side a's and b's positions at line 5, as of where each is read. */
async function entry(repo: Repo, input: { a: SideInput; b: SideInput; verdict: "a" | "b" | "new"; by?: DecisionDraft["decidedBy"]; recordedAt: string; question?: string; text?: string; supersedes?: string | null }): Promise<DecisionEntry> {
  const side = async ({ ref, sha, at }: SideInput) => ({ ref, label: ref, sha, author: null, fingerprint: (await positionFingerprint(repo.root, repo.base, at, AT))!, at: AT });
  return sealDecisionEntry({
    format: DECISION_ENTRY_FORMAT,
    question: input.question ?? "overlap:line5",
    title: "a.ts · line 5",
    base: repo.base,
    sides: { a: await side(input.a), b: await side(input.b) },
    verdict: input.verdict,
    decidedBy: input.by ?? { kind: "person", name: "Tim" },
    result: { candidateId: null, kind: input.verdict === "new" ? "amendment" : "decision", text: input.text ?? `Verdict ${input.verdict}.`, rule: null, follows: null },
    supersedes: input.supersedes ?? null,
    recordedAt: input.recordedAt,
  });
}

const entryFile = (item: DecisionEntry): Record<string, string> => ({ [`.rerail/decisions/${item.id}.json`]: `${JSON.stringify(item, null, 2)}\n` });

function statuses(items: readonly LedgerItem[]): Record<string, [LedgerStatus, string, string]> {
  return Object.fromEntries(items.map(item => [item.entry.id, [item.status, item.place, item.source.label]]));
}

describe("ledger entries", () => {
  it("are files whose id is the hash of their content, and an edited one is refused", async () => {
    const repo = await repository();
    const made = await entry(repo, { a: { ref: "work", sha: repo.base, at: null }, b: { ref: "theirs", sha: repo.base, at: repo.base }, verdict: "a", recordedAt: "2026-11-08T10:00:00.000Z" });
    expect(made.id).toMatch(/^[0-9a-f]{32}$/u);
    expect(decisionEntryId(made)).toBe(made.id);
    expect(await writeWorktreeDecision(repo.root, made)).toBe(`.rerail/decisions/${made.id}.json`);
    expect(await readWorktreeDecisions(repo.root)).toEqual({ entries: [made], problems: [] });

    const path = join(repo.root, ".rerail/decisions", `${made.id}.json`);
    writeFileSync(path, (await readFile(path, "utf8")).replace("Verdict a.", "Verdict b."));
    const edited = await readWorktreeDecisions(repo.root);
    expect(edited.entries).toEqual([]);
    expect(edited.problems).toEqual([expect.stringContaining("the id is the hash of the entry's content")]);
  });
});

describe("the ledger view", { timeout: 30_000 }, () => {
  it("is provisional on this branch and live refs, in force on the base, and superseded there by a later entry", async () => {
    const repo = await repository();
    const theirs = commit(repo.branch("theirs"), { [FILE]: lines({ 5: "const line5 = \"theirs\";" }) });
    write(repo.root, { [FILE]: lines({ 5: "const line5 = \"mine\";" }) });
    const here = await entry(repo, { a: { ref: "work", sha: repo.base, at: null }, b: { ref: "theirs", sha: theirs, at: theirs }, verdict: "a", recordedAt: "2026-11-08T10:00:00.000Z" });
    await writeWorktreeDecision(repo.root, here);

    const featurePath = repo.branch("feature");
    const featureSha = commit(featurePath, { [FILE]: lines({ 5: "const line5 = \"feature\";" }) });
    const live = await entry(repo, { a: { ref: "feature", sha: featureSha, at: featureSha }, b: { ref: "theirs", sha: theirs, at: theirs }, verdict: "b", recordedAt: "2026-11-08T11:00:00.000Z" });
    commit(featurePath, entryFile(live));

    const older = await entry(repo, { a: { ref: "x", sha: repo.base, at: repo.base }, b: { ref: "y", sha: repo.base, at: repo.base }, verdict: "new", question: "overlap:other", recordedAt: "2026-11-01T00:00:00.000Z", text: "Old." });
    const newer = await entry(repo, { a: { ref: "x", sha: repo.base, at: repo.base }, b: { ref: "y", sha: repo.base, at: repo.base }, verdict: "new", question: "overlap:other", recordedAt: "2026-11-02T00:00:00.000Z", text: "New." });
    commit(repo.main(), { ...entryFile(older), ...entryFile(newer) });

    const view = await readLedger(repo.root);
    expect(view.problems).toEqual([]);
    expect(statuses(view.items)).toEqual({
      [here.id]: ["provisional", "branch", "This branch"],
      [live.id]: ["provisional", "live", "feature"],
      [older.id]: ["superseded", "base", "main"],
      [newer.id]: ["in-force", "base", "main"],
    });
    expect(hasVerdict(view, "overlap:line5")).toBe(true);
    expect(Object.keys(view.memory.entries).sort()).toEqual([here.id, live.id, older.id, newer.id].sort());
  });

  it("runs the merge check: in force once merged with matching code, flagged when the merge contradicts it, void when its branch merged without it or went away", async () => {
    const repo = await repository();
    const theirs = commit(repo.branch("theirs"), { [FILE]: lines({ 5: "const line5 = \"theirs\";" }) });
    const featurePath = repo.branch("feature");
    const featureSha = commit(featurePath, { [FILE]: lines({ 5: "const line5 = \"feature\";" }) });
    const kept = await entry(repo, { a: { ref: "feature", sha: featureSha, at: featureSha }, b: { ref: "theirs", sha: theirs, at: theirs }, verdict: "a", recordedAt: "2026-11-08T10:00:00.000Z" });
    commit(featurePath, entryFile(kept));

    const dropperPath = repo.branch("dropper");
    const dropped = await entry(repo, { a: { ref: "dropper", sha: repo.base, at: repo.base }, b: { ref: "theirs", sha: theirs, at: theirs }, verdict: "b", question: "overlap:dropped", recordedAt: "2026-11-08T11:00:00.000Z" });
    commit(dropperPath, entryFile(dropped));
    // reverted keeps its own side, then puts the line back before it merges: the merge contradicts its entry.
    const revertedPath = repo.branch("reverted");
    const revertedSha = commit(revertedPath, { [FILE]: lines({ 9: "const line9 = \"reverted\";" }) });
    const contradicted = await entry(repo, { a: { ref: "reverted", sha: revertedSha, at: revertedSha }, b: { ref: "theirs", sha: theirs, at: theirs }, verdict: "a", question: "overlap:reverted", recordedAt: "2026-11-08T13:00:00.000Z" });
    const at9 = { kind: "hunks" as const, path: FILE, lines: [{ lo: 9, hi: 9 }] };
    const flaggedEntry = sealDecisionEntry({ ...contradicted, sides: { ...contradicted.sides, a: { ...contradicted.sides.a, at: at9, fingerprint: (await positionFingerprint(repo.root, repo.base, revertedSha, at9))! } } });
    commit(revertedPath, { ...entryFile(flaggedEntry), [FILE]: lines() }, "keep the entry, drop the change");
    const abandonedPath = repo.branch("abandoned");
    const abandoned = await entry(repo, { a: { ref: "abandoned", sha: repo.base, at: repo.base }, b: { ref: "theirs", sha: theirs, at: theirs }, verdict: "b", question: "overlap:abandoned", recordedAt: "2026-11-08T12:00:00.000Z" });
    commit(abandonedPath, entryFile(abandoned));

    let memory: LedgerMemory = (await readLedger(repo.root)).memory;
    const step = async (): Promise<Record<string, [LedgerStatus, string, string]>> => {
      const view = await readLedger(repo.root, { memory });
      memory = view.memory;
      return statuses(view.items);
    };
    // reverted's own tip no longer holds its winning change: other checkouts see it reopened before it merges.
    expect(Object.values(await step()).map(([status]) => status)).toEqual(["provisional", "provisional", "provisional", "reopened"]);

    // feature merges with its entry and its code: in force.
    git(repo.root, "branch", "--force", "main", "feature");
    expect((await step())[kept.id]).toEqual(["in-force", "base", "main"]);

    // dropper removes its entry, then merges; abandoned is deleted unmerged.
    commit(dropperPath, { [`.rerail/decisions/${dropped.id}.json`]: null }, "drop the entry");
    const mainPath = repo.main();
    git(mainPath, "-c", "user.name=Rerail", "-c", "user.email=rerail@local", "merge", "--quiet", "--no-ff", "--no-edit", "dropper");
    git(repo.root, "worktree", "remove", "--force", abandonedPath);
    git(repo.root, "branch", "--quiet", "-D", "abandoned");
    let now = await step();
    expect(now[dropped.id]).toEqual(["void", "gone", "dropper"]);
    expect(now[abandoned.id]).toEqual(["void", "gone", "abandoned"]);
    expect(now[kept.id]![0]).toBe("in-force");

    // Later base work on the same lines does not undo what the merge decided.
    commit(mainPath, { [FILE]: lines() }, "revert line 5");
    now = await step();
    expect(now[kept.id]).toEqual(["in-force", "base", "main"]);
    // Void entries stay known, so they stay void.
    expect(now[abandoned.id]![0]).toBe("void");

    // reverted merges with its entry but without its change.
    git(mainPath, "-c", "user.name=Rerail", "-c", "user.email=rerail@local", "merge", "--quiet", "--no-ff", "--no-edit", "reverted");
    now = await step();
    expect(now[flaggedEntry.id]).toEqual(["flagged", "base", "main"]);
    // The status read shows what the last read stored, without Git.
    expect(statuses(storedLedger(memory).items)).toEqual(now);
  });

  it("reopens a provisional verdict when the winner's position moves at its ref's tip, not for work elsewhere in the file", async () => {
    const repo = await repository();
    const theirsPath = repo.branch("theirs");
    const theirs = commit(theirsPath, { [FILE]: lines({ 5: "const line5 = \"theirs\";" }) });
    write(repo.root, { [FILE]: lines({ 5: "const line5 = \"mine\";" }) });
    const tookTheirs = await entry(repo, { a: { ref: "work", sha: repo.base, at: null }, b: { ref: "theirs", sha: theirs, at: theirs }, verdict: "b", recordedAt: "2026-11-08T10:00:00.000Z" });
    await writeWorktreeDecision(repo.root, tookTheirs);
    expect(statuses((await readLedger(repo.root)).items)[tookTheirs.id]![0]).toBe("provisional");

    commit(theirsPath, { [FILE]: lines({ 5: "const line5 = \"theirs\";", 12: "const line12 = 0;" }) }, "elsewhere");
    expect(statuses((await readLedger(repo.root)).items)[tookTheirs.id]![0]).toBe("provisional");

    commit(theirsPath, { [FILE]: lines({ 5: "const line5 = \"theirs, reworked\";", 12: "const line12 = 0;" }) }, "rework line 5");
    const view = await readLedger(repo.root);
    expect(statuses(view.items)[tookTheirs.id]![0]).toBe("reopened");
    expect(hasVerdict(view, "overlap:line5")).toBe(false);
    expect(decidePrecedent({ question: "overlap:line5", sides: { a: { ref: "other", fingerprint: null }, b: { ref: "theirs", fingerprint: null } } }, view).winner).toBeNull();
  });

  it("never reopens a verdict for the checkout whose branch holds the winning position; other checkouts compare its committed tip", async () => {
    const repo = await repository();
    const theirsPath = repo.branch("theirs");
    const theirs = commit(theirsPath, { [FILE]: lines({ 5: "const line5 = \"theirs\";" }) });
    write(repo.root, { [FILE]: lines({ 5: "const line5 = \"mine\";" }) });
    const keptMine = await entry(repo, { a: { ref: "work", sha: repo.base, at: null }, b: { ref: "theirs", sha: theirs, at: theirs }, verdict: "a", recordedAt: "2026-11-08T10:00:00.000Z" });
    await writeWorktreeDecision(repo.root, keptMine);

    // The winner keeps working on the lines it won.
    write(repo.root, { [FILE]: lines({ 5: "const line5 = \"mine, deeper\";" }) });
    expect(statuses((await readLedger(repo.root)).items)[keptMine.id]![0]).toBe("provisional");
    git(repo.root, "add", "-A");
    git(repo.root, "commit", "--quiet", "-m", "deeper");
    expect(statuses((await readLedger(repo.root)).items)[keptMine.id]![0]).toBe("provisional");

    // The side that followed sees the move: at work's committed tip, not anyone's working files.
    expect(statuses((await readLedger(theirsPath)).items)[keptMine.id]).toEqual(["reopened", "live", "work"]);
  });

  it("keeps a merged human verdict in force over a later precedent copy of it", async () => {
    const repo = await repository();
    const theirs = commit(repo.branch("theirs"), { [FILE]: lines({ 5: "const line5 = \"theirs\";" }) });
    const human = await entry(repo, { a: { ref: "one", sha: repo.base, at: repo.base }, b: { ref: "theirs", sha: theirs, at: theirs }, verdict: "b", recordedAt: "2026-11-08T10:00:00.000Z" });
    const copy = await entry(repo, { a: { ref: "two", sha: repo.base, at: repo.base }, b: { ref: "theirs", sha: theirs, at: theirs }, verdict: "b", by: { kind: "precedent", entry: human.id }, recordedAt: "2026-11-09T10:00:00.000Z" });
    commit(repo.main(), { ...entryFile(human), ...entryFile(copy) });
    const view = await readLedger(repo.root);
    expect(statuses(view.items)).toEqual({ [human.id]: ["in-force", "base", "main"], [copy.id]: ["superseded", "base", "main"] });
  });

  it("resolves a followed decision to its latest text: the candidate's amendment at its ref's tip", async () => {
    const repo = await repository();
    const candidate = (id: string, decision: string, supersedes: string | null): Candidate => ({
      format: CANDIDATE_FORMAT, id, decision, generality: "repository", scope: ["src/**"], author: "jbarnes", createdAt: "2026-11-08T10:00:00Z", state: "active", supersedes,
    });
    const first = candidate("candidate:share-matchers-1a2b3c", "Share compiled matchers.", null);
    const amended = candidate("candidate:share-matchers-arc-4d5e6f", "Share compiled matchers behind an Arc.", first.id);
    const path = repo.branch("theirs");
    commit(path, { ".rerail/candidates/share-matchers-1a2b3c.json": serializeCandidate(first) });
    const follows = { ref: "theirs", candidateId: first.id };
    expect(await followedDecision(repo.root, follows)).toBe("Share compiled matchers.");
    commit(path, { ".rerail/candidates/share-matchers-arc-4d5e6f.json": serializeCandidate(amended) });
    expect(await followedDecision(repo.root, follows)).toBe("Share compiled matchers behind an Arc.");
    expect(await followedDecision(repo.root, { ref: "gone", candidateId: "candidate:missing-000000" })).toBeNull();
  });
});

describe("precedent from the view", () => {
  const SHA = "1".repeat(40);
  const fp = (char: string): string => char.repeat(64);
  const item = (id: string, status: LedgerStatus, input: { winner: "a" | "b"; won: string; lost: string; person: boolean; at: string; place?: LedgerItem["place"] }): LedgerItem => {
    const side = (fingerprint: string) => ({ ref: `ref-${fingerprint[0]}`, label: `ref-${fingerprint[0]}`, sha: SHA, author: null, fingerprint, at: AT });
    const sides = input.winner === "a" ? { a: side(input.won), b: side(input.lost) } : { a: side(input.lost), b: side(input.won) };
    const entry = sealDecisionEntry({
      format: DECISION_ENTRY_FORMAT, question: "q", title: id, base: SHA, sides, verdict: input.winner,
      decidedBy: input.person ? { kind: "person", name: "Tim" } : { kind: "precedent", entry: "0".repeat(32) },
      result: { candidateId: null, kind: "decision", text: id, rule: null, follows: null }, supersedes: null, recordedAt: input.at,
    });
    return { entry, status, place: input.place ?? "live", source: { ref: "feature", label: "feature", sha: SHA } };
  };
  /** A conflict whose side a holds position `a…`, side b position `b…`. */
  const target = { question: "q", sides: { a: { ref: "mine", fingerprint: fp("a") }, b: { ref: "theirs", fingerprint: fp("b") } } };

  it("prefers a verdict in force, then a person's provisional one, then the latest, only in the same dispute, and never a void, superseded, reopened or flagged one", () => {
    const inForce = item("in force", "in-force", { winner: "a", won: fp("b"), lost: fp("a"), person: false, at: "2026-11-01T00:00:00.000Z", place: "base" });
    const person = item("person", "provisional", { winner: "a", won: fp("a"), lost: fp("b"), person: true, at: "2026-11-02T00:00:00.000Z" });
    const latest = item("latest", "provisional", { winner: "b", won: fp("b"), lost: fp("a"), person: false, at: "2026-11-03T00:00:00.000Z" });
    // Same question, but another dispute: its loser is a position neither side holds.
    const other = item("other", "in-force", { winner: "b", won: fp("b"), lost: fp("c"), person: true, at: "2026-11-04T00:00:00.000Z", place: "base" });
    const ignored = (["void", "superseded", "reopened", "flagged"] as const).map((status, index) =>
      item(status, status, { winner: "a", won: fp("a"), lost: fp("b"), person: true, at: `2026-11-1${index}T00:00:00.000Z`, place: status === "void" ? "gone" : "base" }));

    expect(decidePrecedent(target, { items: [inForce, person, latest, ...ignored] })).toMatchObject({ winner: "b", entry: { id: inForce.entry.id } });
    expect(decidePrecedent(target, { items: [person, latest, ...ignored] })).toMatchObject({ winner: "a", entry: { id: person.entry.id } });
    expect(decidePrecedent(target, { items: [latest, ...ignored] })).toMatchObject({ winner: "b", entry: { id: latest.entry.id } });
    expect(decidePrecedent(target, { items: ignored })).toMatchObject({ winner: null, entry: null });
    expect(decidePrecedent(target, { items: [other, latest] })).toMatchObject({ winner: "b", entry: { id: latest.entry.id } });
    expect(decidePrecedent(target, { items: [other] })).toEqual({ step: "precedent", winner: null, reason: "Earlier: Tim chose ref-b on 2026-11-04.", entry: { id: other.entry.id } });
    expect(decidePrecedent({ ...target, question: "other" }, { items: [inForce] }).winner).toBeNull();
  });

  it("finds a verdict conflict when this branch and a live ref picked different winners on one question", () => {
    const mine = item("mine", "provisional", { winner: "a", won: fp("a"), lost: fp("b"), person: true, at: "2026-11-02T00:00:00.000Z", place: "branch" });
    const theirs = item("theirs", "provisional", { winner: "a", won: fp("b"), lost: fp("a"), person: true, at: "2026-11-03T00:00:00.000Z" });
    const agrees = item("agrees", "provisional", { winner: "b", won: fp("a"), lost: fp("b"), person: true, at: "2026-11-04T00:00:00.000Z" });
    expect(verdictDisagreements({ items: [mine, theirs] })).toEqual([{ mine, theirs }]);
    expect(verdictDisagreements({ items: [mine, { ...agrees, source: { ref: "other", label: "other", sha: SHA } }] })).toEqual([]);
    expect(verdictDisagreements({ items: [mine, { ...theirs, status: "void" }] })).toEqual([]);
    // This branch's latest human verdict counts, not a later precedent copy that picked otherwise.
    const copy = { ...item("copy", "provisional", { winner: "a", won: fp("b"), lost: fp("a"), person: false, at: "2026-11-05T00:00:00.000Z", place: "branch" }) };
    expect(verdictDisagreements({ items: [mine, copy, { ...agrees, source: { ref: "other", label: "other", sha: SHA } }] })).toEqual([]);

    const groups = ledgerLog({ items: [mine, theirs] }, "mine");
    expect(groups).toEqual([{ question: "q", title: "theirs", items: [
      expect.objectContaining({ id: theirs.entry.id, status: "provisional", outcome: "Theirs (feature)", source: { ref: "feature", label: "feature" } }),
      expect.objectContaining({ id: mine.entry.id, outcome: "Mine", by: { kind: "person", name: "Tim" } }),
    ] }]);
  });
});
