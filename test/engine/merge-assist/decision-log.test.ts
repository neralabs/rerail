import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  appendDecision,
  DECISION_ENTRY_FORMAT,
  DECISION_LOG_REF,
  type DecisionEntry,
  decisionInForce,
  decidePrecedent,
  newDecisionId,
  readDecisions,
} from "../../../src/engine/merge-assist/decision-log.js";
import type { ConflictRecord } from "../../../src/engine/merge-assist/conflict-record.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" }).trim();
}

async function repository(files: Record<string, string> = { "README.md": "seed\n" }): Promise<{ root: string; base: string }> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-decision-log-")));
  roots.push(root);
  git(root, "init", "--quiet", "-b", "main");
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  git(root, "add", ".");
  git(root, "commit", "--quiet", "-m", "seed");
  return { root, base: git(root, "rev-parse", "HEAD") };
}

function record(refs: { a: string; b: string } = { a: "refs/heads/retry-in-client", b: "refs/heads/retry-in-transport" }, question = "overlap:abc"): ConflictRecord {
  return {
    repository: "acme/app",
    base: "1".repeat(40),
    question,
    sides: {
      a: { ref: refs.a, sha: "a".repeat(40), intent: "Retry in each client call.", anchors: { files: ["src/client.ts"], symbols: ["fetchUser"] } },
      b: { ref: refs.b, sha: "b".repeat(40), intent: "Retry once, in the transport.", anchors: { files: ["src/client.ts", "src/transport.ts"], symbols: [] } },
    },
  };
}

function entry(overrides: Partial<DecisionEntry> = {}): DecisionEntry {
  return {
    format: DECISION_ENTRY_FORMAT,
    id: newDecisionId(),
    recordedAt: "2026-11-07T12:00:00.000Z",
    conflict: record(),
    step: "panel",
    reason: "The panel agrees on side b.",
    evidence: { votes: ["b", "b", "b"] },
    verdict: "b",
    decidedBy: { kind: "policy", step: "panel" },
    result: { kind: "decision", side: "a", rule: null, text: "Follow the transport's retry design." },
    ...overrides,
  };
}

describe("the decision log", () => {
  it("appends one commit per entry on refs/rerail/decisions and reads them back in order, touching no branch", async () => {
    const { root } = await repository();
    const head = git(root, "rev-parse", "HEAD");
    const first = entry();
    const second = entry({ step: "escalate", verdict: "escalated", decidedBy: { kind: "policy", step: "escalate" }, result: null, reason: "Reserved: public-api." });
    const appended = [appendDecision(root, first), appendDecision(root, second)];
    expect(appended.every(outcome => outcome.ok)).toBe(true);
    expect(git(root, "rev-list", "--count", DECISION_LOG_REF)).toBe("2");
    expect(git(root, "ls-tree", "--name-only", DECISION_LOG_REF)).toBe("entry.json");
    expect(git(root, "rev-parse", "HEAD")).toBe(head);
    expect(git(root, "status", "--porcelain")).toBe("");
    const log = await readDecisions(root);
    expect(log.problems).toEqual([]);
    expect(log.entries.map(logged => logged.entry)).toEqual([first, second]);
    expect(log.entries[1]!.commit).toBe(appended[1]!.ok ? appended[1]!.commit : "");
    expect(git(root, "log", "-1", "--format=%s", DECISION_LOG_REF)).toBe(`Decision ${second.id}: escalated, by policy step escalate`);
  });

  it("refuses an entry whose verdict, result and decider disagree", async () => {
    const { root } = await repository();
    expect(appendDecision(root, entry({ result: null }))).toMatchObject({ ok: false, reason: expect.stringContaining("escalated entry has no result") });
    expect(appendDecision(root, entry({ result: { kind: "decision", side: "b", rule: null, text: "x" } }))).toMatchObject({ ok: false, reason: expect.stringContaining("losing side") });
    expect(appendDecision(root, entry({ decidedBy: { kind: "policy", step: "criteria" } }))).toMatchObject({ ok: false });
    expect(await readDecisions(root)).toEqual({ entries: [], problems: [] });
  });

  it("skips a log commit without a valid entry and says so", async () => {
    const { root } = await repository();
    expect(appendDecision(root, entry()).ok).toBe(true);
    const tip = git(root, "rev-parse", DECISION_LOG_REF);
    const blob = execFileSync("git", ["hash-object", "-w", "--stdin"], { cwd: root, input: "{\"format\":\"other\"}\n", encoding: "utf8" }).trim();
    const tree = execFileSync("git", ["mktree"], { cwd: root, input: `100644 blob ${blob}\tentry.json\n`, encoding: "utf8" }).trim();
    const commit = git(root, "commit-tree", tree, "-p", tip, "-m", "bad");
    git(root, "update-ref", DECISION_LOG_REF, commit);
    const log = await readDecisions(root);
    expect(log.entries).toHaveLength(1);
    expect(log.problems).toEqual([expect.stringMatching(new RegExp(`^${commit}: format`, "u"))]);
  });

  it("knows a resulting decision is in force when its rule is in the base", async () => {
    const { root, base } = await repository({ ".rerail/rules/retries-in-transport.json": "{}\n" });
    expect(decisionInForce(root, base, entry({ result: { kind: "decision", side: "a", rule: ".rerail/rules/retries-in-transport.json", text: "x" } }))).toBe(true);
    expect(decisionInForce(root, base, entry({ result: { kind: "decision", side: "a", rule: ".rerail/rules/gone.json", text: "x" } }))).toBe(false);
    expect(decisionInForce(root, base, entry())).toBe(false);
  });
});

describe("decidePrecedent", () => {
  const logged = (value: DecisionEntry, commit = "c".repeat(40)) => ({ commit, entry: value });

  it("applies an earlier verdict on the same question to the ref it picked or rejected, whichever side it is now", () => {
    const earlier = entry();
    const sameRefs = decidePrecedent(record(), [logged(earlier)], () => false);
    expect(sameRefs.winner).toBe("b");
    expect(sameRefs.entry?.id).toBe(earlier.id);
    const swapped = decidePrecedent(record({ a: "refs/heads/retry-in-transport", b: "refs/heads/other" }), [logged(earlier)], () => false);
    expect(swapped.winner).toBe("a");
    const loserAgain = decidePrecedent(record({ a: "refs/heads/new", b: "refs/heads/retry-in-client" }), [logged(earlier)], () => false);
    expect(loserAgain.winner).toBe("a");
  });

  it("ignores other questions, escalations and verdicts about other refs", () => {
    expect(decidePrecedent(record(undefined, "overlap:other"), [logged(entry())], () => false).winner).toBeNull();
    const escalated = entry({ step: "escalate", verdict: "escalated", decidedBy: { kind: "policy", step: "escalate" }, result: null });
    expect(decidePrecedent(record(), [logged(escalated)], () => false)).toMatchObject({ winner: null, entry: null });
    const unrelated = decidePrecedent(record({ a: "refs/heads/x", b: "refs/heads/y" }), [logged(entry())], () => false);
    expect(unrelated.winner).toBeNull();
    expect(unrelated.reason).toContain("other refs");
  });

  it("prefers a decision in force, then a person's, over a later policy verdict", () => {
    const person = entry({ step: "escalate", verdict: "a", decidedBy: { kind: "person", name: "Tim" }, result: { kind: "amendment", side: "b", rule: null, text: "Keep client retries." } });
    const later = entry();
    expect(decidePrecedent(record(), [logged(person), logged(later)], () => false).winner).toBe("a");
    expect(decidePrecedent(record(), [logged(person), logged(later)], value => value.id === later.id).winner).toBe("b");
  });
});
