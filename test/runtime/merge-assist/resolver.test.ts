import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { type CheckResult, type ConflictRecord, defaultPolicy, type Policy, questionOf, type SideId } from "../../../src/engine/index.js";
import { type CheckRunner, createLocalCheckRunner, panelOrder, resolveConflict } from "../../../src/runtime/index.js";
import { FakeJudge } from "./support/merge-assist-workspace.js";

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
});

function git(cwd: string, ...args: string[]): string {
  return execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "-c", "commit.gpgsign=false", ...args], { cwd, encoding: "utf8" }).trim();
}

function commit(root: string, files: Record<string, string>, message: string): string {
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
  git(root, "add", "-A");
  git(root, "commit", "--quiet", "-m", message);
  return git(root, "rev-parse", "HEAD");
}

const CLIENT = "export async function fetchUser(id: string): Promise<User> {\n  return transport.get(`/users/${id}`);\n}\n";

/** A base and two branches that change the client's retries differently: `client` loops in the caller, `transport` retries once below it. */
async function conflict(extra: { client?: Record<string, string>; transport?: Record<string, string> } = {}): Promise<{ root: string; record: ConflictRecord }> {
  const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-resolver-")));
  roots.push(root);
  git(root, "init", "--quiet", "-b", "main");
  const base = commit(root, { "src/client.ts": CLIENT, "src/transport.ts": "export const transport = { get: (path: string) => fetch(path) };\n", "README.md": "# App\nThe client calls the transport.\n" }, "seed");
  git(root, "checkout", "--quiet", "-b", "client");
  const client = commit(root, { "src/client.ts": CLIENT.replace("return transport", "for (let i = 0; i < 3; i++) try { return await transport.get(`/users/${id}`); } catch {}\n  return transport"), ...extra.client }, "retry in the client");
  git(root, "checkout", "--quiet", "-b", "transport", base);
  const transport = commit(root, { "src/transport.ts": "export const transport = { get: (path: string) => fetch(path).catch(() => fetch(path)) };\n", "src/client.ts": `${CLIENT}// retried by the transport\n`, ...extra.transport }, "retry in the transport");
  const anchors = { a: { files: ["src/client.ts"], symbols: [] }, b: { files: ["src/client.ts", "src/transport.ts"], symbols: [] } };
  return {
    root,
    record: {
      repository: "acme/app",
      base,
      question: questionOf(anchors.a, anchors.b),
      sides: {
        a: { ref: "refs/pull/12/head", sha: client, intent: "Retry user fetches in the client. Supersedes #10.", anchors: anchors.a },
        b: { ref: "refs/pull/14/head", sha: transport, intent: "Retry once in the transport so callers stay simple.", anchors: anchors.b },
      },
    },
  };
}

function policy(overrides: Partial<Policy> = {}): Policy {
  return { ...defaultPolicy(), reserve: [], ...overrides };
}

function runner(results: Record<SideId, CheckResult["status"]>): CheckRunner & { calls: SideId[] } {
  const calls: SideId[] = [];
  return {
    calls,
    run: async side => {
      calls.push(side);
      return { checks: [{ name: "unit", kind: "tests", status: results[side], detail: "", log: "" }], benchmarks: {} };
    },
  };
}

/** A judge that prefers whichever design retries in the transport, wherever it is shown. */
function transportJudge(): FakeJudge {
  return new FakeJudge(task => {
    const x = /design x diff>>>\n([\s\S]*?)\n<<<END/u.exec(task.prompt)![1]!;
    const scores = { conventions: 4, simplicity: 4, compatibility: 4, goal: 4 };
    return { scores: { X: scores, Y: scores }, preferred: x.includes("src/transport.ts") ? "X" : "Y", reasons: ["Retries belong below the callers."] };
  });
}

const withChecks = (base: Policy): Policy => ({ ...base, checks: { ...base.checks, run: [{ name: "unit", kind: "tests", run: "npm test" }] } });

describe("resolveConflict", () => {
  it("decides at the hard checks when one side fails a check the other passes, before any judge", async () => {
    const { root, record } = await conflict();
    const judge = transportJudge();
    const checks = runner({ a: "pass", b: "fail" });
    const resolution = await resolveConflict({ root, repositoryKey: "test", record, policy: withChecks(policy()), judge, checks });
    expect(resolution).toMatchObject({ verdict: "a", step: "hard-checks" });
    expect(resolution.steps.map(step => step.step)).toEqual(["hard-checks"]);
    expect(checks.calls.sort()).toEqual(["a", "b"]);
    expect(judge.tasks).toHaveLength(0);
  });

  it("asks a blind panel, records every vote, and picks the same design whichever slot it is in", async () => {
    const { root, record } = await conflict();
    const judge = transportJudge();
    const resolution = await resolveConflict({ root, repositoryKey: "test", record, policy: policy({ criteria: { ...defaultPolicy().criteria, blastRadius: { enabled: false, ratio: 3, minLines: 100 } } }), judge, checks: null, seed: 7 });
    expect(resolution).toMatchObject({ verdict: "b", step: "panel" });
    const panel = resolution.evidence.panel!;
    expect(panel.votes.map(vote => [vote.perspective, vote.vote, vote.readings.map(reading => reading.shown)])).toEqual([0, 1, 2].map(index => {
      const order = panelOrder(7, record.question, index);
      return [["conventions", "simplicity", "users"][index], "b", [order, { X: order.Y, Y: order.X }]];
    }));
    expect(panel.votes.every(vote => vote.readings.every(reading => reading.vote === "b" && reading.reasons.length === 1 && reading.scores !== null))).toBe(true);
    expect(judge.tasks).toHaveLength(6);
    for (const task of judge.tasks) {
      expect(task.kind).toBe("resolve");
      expect(task.prompt).not.toMatch(/refs\/pull|#10|#12|Supersedes|\b(?:first|second|earlier|later)\b/iu);
      expect(task.prompt).not.toContain(record.sides.a.sha);
    }
    const swapped = { ...record, sides: { a: record.sides.b, b: record.sides.a } };
    expect((await resolveConflict({ root, repositoryKey: "test", record: swapped, policy: policy({ criteria: { ...defaultPolicy().criteria, blastRadius: { enabled: false, ratio: 3, minLines: 100 } } }), judge: transportJudge(), checks: null, seed: 7 })).verdict).toBe("a");
  });

  it("escalates when the panel does not agree, or a judge fails", async () => {
    const { root, record } = await conflict();
    let calls = 0;
    const split = new FakeJudge(() => {
      calls += 1;
      if (calls === 3) return { ok: false, reason: "timeout", detail: "no answer" };
      const scores = { conventions: 3, simplicity: 3, compatibility: 3, goal: 3 };
      return { scores: { X: scores, Y: scores }, preferred: "X", reasons: ["X fits."] };
    });
    const resolution = await resolveConflict({ root, repositoryKey: "test", record, policy: policy({ steps: ["panel"] }), judge: split, checks: null });
    expect(resolution.verdict).toBe("escalated");
    expect(resolution.reason).toMatch(/does not agree/u);
    expect(resolution.evidence.panel!.votes.flatMap(vote => vote.readings).filter(reading => reading.error !== null).map(reading => reading.error)).toEqual(["timeout: no answer"]);
  });

  it("escalates a reserved class before the panel, and when no judge is available", async () => {
    const { root, record } = await conflict({ transport: { "src/auth/token.ts": "export const ttl = 60;\n" } });
    const judge = transportJudge();
    const reserved = await resolveConflict({ root, repositoryKey: "test", record, policy: policy({ reserve: ["security"], steps: ["panel"] }), judge, checks: null });
    expect(reserved).toMatchObject({ verdict: "escalated", step: "escalate", evidence: { reserved: ["security"] } });
    expect(judge.tasks).toHaveLength(0);
    expect((await resolveConflict({ root, repositoryKey: "test", record, policy: policy({ steps: ["panel"] }), judge: null, checks: null })).reason).toBe("The panel is needed, and no judge is available.");
  });

  it("decides by blast radius when one side changes many times the source lines", async () => {
    const { root, record } = await conflict({ client: { "src/retry.ts": "export const x = 1;\n".repeat(150) } });
    const resolution = await resolveConflict({ root, repositoryKey: "test", record, policy: policy(), judge: null, checks: null });
    expect(resolution).toMatchObject({ verdict: "b", step: "criteria" });
    expect(resolution.reason).toMatch(/^Blast radius/u);
  });
});

describe("createLocalCheckRunner", () => {
  it("runs the setup and each check in an exported tree: pass, fail, and unknown after a failed setup", async () => {
    const { root, record } = await conflict({ client: { "check.sh": "test -f src/client.ts && test -f installed\n" } });
    const checks = createLocalCheckRunner({ root });
    const base = defaultPolicy();
    const run: Policy = { ...base, checks: { setup: "touch installed", run: [{ name: "has-client", kind: "tests", run: "sh check.sh" }, { name: "fails", kind: "typecheck", run: "echo broken; exit 3" }], timeoutMinutes: 1 } };
    const outcome = await checks.run("a", record.sides.a.sha, run);
    expect(outcome.checks.map(result => [result.name, result.status, result.detail])).toEqual([["has-client", "pass", "exit 0"], ["fails", "fail", "exit 3"]]);
    expect(outcome.checks[1]!.log).toContain("broken");
    const failedSetup = await checks.run("a", record.sides.a.sha, { ...run, checks: { ...run.checks, setup: "exit 1" } });
    expect(failedSetup.checks.map(result => result.status)).toEqual(["unknown", "unknown"]);
    expect(git(root, "status", "--porcelain")).toBe("");
  });
});
