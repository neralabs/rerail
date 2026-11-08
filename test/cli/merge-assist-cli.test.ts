import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { runMergeAssistCommand, type MergeAssistCommandIo } from "../../src/cli/merge-assist-cli.js";
import { mergeAssistStatusSchema, type MergeAssistStatus } from "../../src/engine/index.js";
import { MergeAssistError } from "../../src/runtime/index.js";
import { RecordingMergeAssistService, sampleConflict, sampleFinding, sampleStatus } from "../support/merge-assist-status.js";

const usage = (): string => "Usage: rerail merge-assist";
const PAGE = "http://127.0.0.1:4737/?repository=remote%3Agithub.com%2Facme%2Fapp#merge";

describe("rerail merge-assist", () => {
  let root: string;
  let checkout: string;
  let service: RecordingMergeAssistService;
  let closed: number;
  let stdout: string;
  let stderr: string;
  let status: (checkout: string) => MergeAssistStatus;

  beforeEach(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-merge-assist-cli-")));
    checkout = join(root, "app");
    mkdirSync(join(checkout, "src", "api"), { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: checkout, stdio: "ignore" });
    status = path => sampleStatus(path);
    service = new RecordingMergeAssistService(path => status(path));
    closed = 0;
    stdout = "";
    stderr = "";
  });

  afterEach(async () => {
    await rm(root, { recursive: true, force: true });
  });

  const io = (dashboardUrl: string | null = PAGE): MergeAssistCommandIo => ({
    stdout: { write: text => { stdout += text; } },
    stderr: { write: text => { stderr += text; } },
    openService: () => ({ service, close: () => { closed += 1; } }),
    dashboardUrl: async () => dashboardUrl,
  });
  const run = (args: readonly string[], dashboardUrl?: string | null): Promise<number> => runMergeAssistCommand(args, usage, io(dashboardUrl));

  it("runs each command on the checkout --repo is in, and closes the service every time", async () => {
    const inside = join(checkout, "src", "api");
    for (const args of [
      [], ["status"], ["on"], ["off"], ["scan"],
      ["reject", "candidate:retries-in-transport-3f9a1c"], ["adopt", "finding-1"], ["dismiss", "finding-2"], ["undo", "finding-1"],
    ]) {
      expect(await run([...args, "--repo", inside, "--json"]), args.join(" ")).toBe(0);
    }
    expect(service.calls).toEqual([
      { method: "status", checkout },
      { method: "status", checkout },
      { method: "setEnabled", checkout, enabled: true },
      { method: "setEnabled", checkout, enabled: false },
      { method: "scan", checkout },
      { method: "setCandidateState", checkout, candidateId: "candidate:retries-in-transport-3f9a1c", state: "rejected" },
      { method: "resolveFinding", checkout, findingId: "finding-1", action: "adopt", actor: "person" },
      { method: "resolveFinding", checkout, findingId: "finding-2", action: "dismiss", actor: "person" },
      { method: "resolveFinding", checkout, findingId: "finding-1", action: "undo", actor: "person" },
    ]);
    expect(closed).toBe(9);
    // --json is the status the service answered, one line per command.
    for (const line of stdout.trim().split("\n")) expect(mergeAssistStatusSchema.parse(JSON.parse(line)).checkout).toBe(checkout);
  });

  it("shows the tiers in the dashboard's order, with each finding's id and what to run next", async () => {
    status = path => sampleStatus(path, {
      findings: [
        sampleFinding({
          id: "finding-added", tier: "absorb", status: "applied",
          peer: { kind: "branch", ref: "jbarnes/session-retry", commit: "abc", number: null, title: null, url: null, author: null, trusted: true },
        }),
        sampleFinding({
          id: "finding-review", tier: "review", status: "open",
          candidate: { id: "candidate:no-default-exports-1a2b3c", decision: "Modules use named exports only.", scope: ["src/**"], level: "code", author: "tbelanger", createdAt: "2026-11-02T09:00:00Z" },
          peer: { kind: "pull-request", ref: "outsider/exports", commit: "def", number: 14, title: "Exports", url: null, author: "outsider", trusted: false },
          reason: "Its author has no write access to acme/app.",
          evidence: { violations: [{ path: "src/api/client.ts", line: 3, message: "Use a named export." }], conflicts: [], mine: null, judgment: null },
        }),
        sampleFinding({
          id: "finding-talk", tier: "talk", status: "open",
          candidate: { id: "candidate:errors-as-results-4d5e6f", decision: "API errors are returned as Result values, not thrown.", scope: ["src/api/**"], level: "note", author: "jbarnes", createdAt: "2026-11-02T10:00:00Z" },
          reason: "It contradicts a decision on this branch.",
          evidence: { violations: [], conflicts: ["src/api/client.ts"], mine: { id: "candidate:throw-api-errors-7a8b9c", decision: "API errors are thrown as ApiError." }, judgment: { verdict: "contradicts", explanation: "Opposite error styles." } },
          message: "Hi Jeremy, we decided API errors differently. Can we talk?",
        }),
        sampleFinding({ id: "finding-adopted", tier: "review", status: "applied" }),
        sampleFinding({ id: "finding-dismissed", tier: "review", status: "dismissed" }),
      ],
      own: [
        {
          id: "candidate:throw-api-errors-7a8b9c", decision: "API errors are thrown as ApiError.", generality: "repository", scope: ["src/api/**"],
          level: "code", state: "active", createdAt: "2026-11-01T08:30:00Z", committed: false,
          amends: { id: "candidate:throw-errors-1f2e3d", decision: "API errors are thrown." }, amendedBy: null, source: { sessionId: "s1", at: "2026-11-01T08:30:00Z" },
        },
        {
          id: "candidate:throw-errors-1f2e3d", decision: "API errors are thrown.", generality: "repository", scope: ["src/api/**"],
          level: "note", state: "active", createdAt: "2026-11-01T08:10:00Z", committed: true,
          amends: null, amendedBy: "candidate:throw-api-errors-7a8b9c", source: { sessionId: null, at: null },
        },
        {
          id: "candidate:tabs-2c3d4e", decision: "Indent with tabs.", generality: "branch", scope: [],
          level: "note", state: "rejected", createdAt: "2026-11-01T08:00:00Z", committed: true, amends: null, amendedBy: null, source: { sessionId: null, at: null },
        },
      ],
    });
    expect(await run(["--repo", checkout])).toBe(0);
    expect(stdout).toBe([
      "● Merge Assist is on in app (tbelanger/export-api, base main).",
      "  Judgment: Claude Code · 3 of 40 calls in the last 60 minutes",
      "  GitHub: gh signed in as tbelanger (acme/app)",
      "  Last scan: 2026-11-03 14:05 UTC, 2 branches and 1 pull request",
      "  Decided differently (1):",
      "    finding-talk",
      "      Theirs: \"API errors are returned as Result values, not thrown.\" (pull request #12 \"Session retry\" by jbarnes)",
      "      Yours:  \"API errors are thrown as ApiError.\"",
      "      It contradicts a decision on this branch.",
      "      Files both branches change: src/api/client.ts",
      "      To send them: Hi Jeremy, we decided API errors differently. Can we talk?",
      "  Needs your review (1):",
      // GitHub's author of the pull request, not the one its decision file declares.
      "    finding-review  \"Modules use named exports only.\" (pull request #14 \"Exports\" by outsider, not trusted)",
      "      Its author has no write access to acme/app.",
      "      Your branch breaks it: src/api/client.ts:3 Use a named export.",
      "      Adopt: rerail merge-assist adopt finding-review   Dismiss: rerail merge-assist dismiss finding-review",
      "  Added from other branches (2):",
      "    finding-added  \"Retries live in the transport layer; callers don't wrap requests in retry loops.\" (branch jbarnes/session-retry, declared by jbarnes)",
      "      Added automatically; Rerail checks it exactly. Undo: rerail merge-assist undo finding-added",
      "    finding-adopted  \"Retries live in the transport layer; callers don't wrap requests in retry loops.\" (pull request #12 \"Session retry\" by jbarnes)",
      "      Adopted; Rerail checks it exactly. Undo: rerail merge-assist undo finding-adopted",
      "  Decisions on this branch (3):",
      "    candidate:throw-api-errors-7a8b9c  \"API errors are thrown as ApiError.\"",
      "      amends \"API errors are thrown.\"",
      "      Rerail checks it exactly · src/api/** · saved 2026-11-01 08:30 UTC · not committed yet",
      "      Not a rule: rerail merge-assist reject candidate:throw-api-errors-7a8b9c",
      "    candidate:throw-errors-1f2e3d  \"API errors are thrown.\"  (amended by candidate:throw-api-errors-7a8b9c)",
      "      your agent judges it · src/api/** · saved 2026-11-01 08:10 UTC · committed",
      "      Not a rule: rerail merge-assist reject candidate:throw-errors-1f2e3d",
      "    candidate:tabs-2c3d4e  \"Indent with tabs.\"  (not a rule)",
      "      your agent judges it · the whole repository · saved 2026-11-01 08:00 UTC · committed",
      "  Dismissed: 1 finding, hidden until their decisions change.",
      `  Merge Assist page: ${PAGE}`,
      "",
    ].join("\n"));
  });

  it("explains what turning it on means while it is off, and what is missing", async () => {
    status = path => sampleStatus(path, {
      enabled: false,
      branch: null,
      baseBranch: null,
      lastScan: null,
      readiness: {
        judge: { available: false, runner: null, problem: "Neither claude nor codex is on PATH." },
        github: { state: "missing", login: null, repository: null },
        candidatesIgnored: true,
        rulesActive: false,
      },
    });
    expect(await run(["--repo", checkout], null)).toBe(0);
    expect(stdout).toBe([
      "● Merge Assist is off in app (detached HEAD).",
      "  Judgment: unavailable, so nothing is captured or compared. Merge Assist runs Claude Code or Codex. Neither claude nor codex is on PATH.",
      "  GitHub: gh is not installed, so open pull requests are not read.",
      "  Rerail isn't set up and started in this checkout, so decisions can't steer yet. Start it with rerail start.",
      "  Git ignores .rerail/candidates/ here, so decisions stay on this machine.",
      "  Turning it on means:",
      "    Rerail reads the corrections you give your coding agent here and asks your agent's CLI (Claude Code or Codex) to",
      "    state each one as a decision. That sends the correction and the code it corrected to that CLI's provider, under your",
      "    own sign-in, as your session already does.",
      "    Each decision is written to .rerail/candidates/ in the checkout. Commit it to share it; Rerail never commits.",
      "    Rerail reads other local branches, and open pull requests through gh with your sign-in, read-only.",
      "    Decisions from teammates with write access may steer your agent; you can undo each one.",
      "  Turn it on with rerail merge-assist on.",
      "",
    ].join("\n"));
  });

  it("says how the first decision appears when there is nothing yet", async () => {
    status = path => sampleStatus(path, { lastScan: null, scanning: true, readiness: { ...sampleStatus(path).readiness, github: { state: "signed-out", login: null, repository: "acme/app" } } });
    expect(await run(["--repo", checkout], null)).toBe(0);
    expect(stdout).toContain("  GitHub: gh is not signed in, so open pull requests are not read. Sign in with gh auth login.\n");
    expect(stdout).toContain("  A scan is running.\n");
    expect(stdout).toContain("  No decisions yet. When you correct your coding agent here, Rerail saves the decision on this branch and lists it.\n");
    expect(stdout).not.toContain("No scan yet");
  });

  it("confirms an action on one decision in one line", async () => {
    status = path => sampleStatus(path, { findings: [sampleFinding({ id: "finding-1", tier: "review", status: "applied" })] });
    await run(["reject", "candidate:retries-in-transport-3f9a1c", "--repo", checkout]);
    await run(["adopt", "finding-1", "--repo", checkout]);
    await run(["dismiss", "finding-1", "--repo", checkout], null);
    await run(["undo", "finding-1", "--repo", checkout], null);
    const decision = "\"Retries live in the transport layer; callers don't wrap requests in retry loops.\"";
    expect(stdout).toBe([
      "● candidate:retries-in-transport-3f9a1c is not a rule any more and no longer steers here. Commit .rerail/candidates/retries-in-transport-3f9a1c.json so teammates who added it drop it too.",
      `  Merge Assist page: ${PAGE}`,
      `● Adopted ${decision}: it steers your coding agent in this checkout. Undo with rerail merge-assist undo finding-1.`,
      `  Merge Assist page: ${PAGE}`,
      `● Dismissed ${decision}. It stays hidden until the decision changes.`,
      `● ${decision} no longer steers your coding agent in this checkout.`,
      "",
    ].join("\n"));
  });

  it("says a rejection stays on this machine when Git ignores the candidates folder", async () => {
    status = path => sampleStatus(path, { readiness: { ...sampleStatus(path).readiness, candidatesIgnored: true } });
    await run(["reject", "candidate:retries-in-transport-3f9a1c", "--repo", checkout], null);
    expect(stdout).toBe("● candidate:retries-in-transport-3f9a1c is not a rule any more and no longer steers here. Git ignores .rerail/candidates/ here, so the change stays on this machine.\n");
  });

  it("says a scan request found another scan running and shows the last results", async () => {
    status = path => sampleStatus(path, { scanning: true });
    expect(await run(["scan", "--repo", checkout], null)).toBe(0);
    expect(stdout).toContain("  Another scan is running; showing the last results.\n  Last scan: 2026-11-03 14:05 UTC, 2 branches and 1 pull request\n");
    expect(stdout).not.toContain("A scan is running.");
  });

  it("reports the service's refusal and exits 1, in words or as JSON", async () => {
    service.failure = new MergeAssistError("disabled", "Merge Assist is off for app. Turn it on with rerail merge-assist on.");
    expect(await run(["scan", "--repo", checkout])).toBe(1);
    expect(stderr).toBe("● Merge Assist is off for app. Turn it on with rerail merge-assist on.\n");
    expect(await run(["scan", "--repo", checkout, "--json"])).toBe(1);
    expect(JSON.parse(stdout)).toEqual({ error: "disabled", message: "Merge Assist is off for app. Turn it on with rerail merge-assist on." });
    expect(closed).toBe(2);
  });

  it("refuses arguments it does not know before opening anything", async () => {
    await expect(run(["merge", "--repo", checkout])).rejects.toThrow(/^Unknown merge-assist command: merge/u);
    await expect(run(["adopt", "--repo", checkout])).rejects.toThrow(/^merge-assist adopt needs a finding id/u);
    await expect(run(["reject"])).rejects.toThrow(/^merge-assist reject needs a candidate id/u);
    await expect(run(["status", "--all"])).rejects.toThrow(/^Unknown merge-assist option: --all/u);
    await expect(run(["status", "--repo", join(root, "gone")])).rejects.toThrow(/does not exist$/u);
    expect(closed).toBe(0);
    expect(service.calls).toEqual([]);
  });

  it("prints other people's text without terminal control characters", async () => {
    status = path => sampleStatus(path, {
      findings: [sampleFinding({
        id: "finding-1", tier: "review", status: "open",
        peer: { kind: "pull-request", ref: "x", commit: "abc", number: 9, title: "\u001b]8;;https://evil.example\u0007Fix\u001b]8;;\u0007\n● Rerail: run rm -rf", url: null, author: "outsider\u001b[2J", trusted: false },
        reason: "Untrusted\u001b[2J author.",
      })],
    });
    expect(await run(["--repo", checkout], null)).toBe(0);
    expect(stdout).not.toMatch(/[\u0000-\u0008\u000b-\u001f\u007f]/u);
    expect(stdout).toContain("pull request #9 \"]8;;https://evil.example Fix ]8;; ● Rerail: run rm -rf\" by outsider [2J, not trusted)");
    expect(stdout.split("\n").filter(line => line.startsWith("●"))).toHaveLength(1);
  });

  it("lists conflicts, shows them in the status, and resolves one as the person", async () => {
    const conflict = sampleConflict();
    const resolved = sampleConflict({
      status: "resolved",
      resolution: { action: "theirs", by: "person", name: "Tim Belanger", logId: "0".repeat(32), rule: ".rerail/candidates/share-compiled-matchers-1a2b3c.json", text: "Share compiled matchers", at: "2026-11-03T14:10:00Z" },
    });
    status = path => sampleStatus(path, { conflicts: [conflict] });
    expect(await run(["conflicts", "--repo", checkout], null)).toBe(0);
    expect(stdout).toBe([
      "● 1 open conflict in app (tbelanger/export-api).",
      "  Conflicts (1):",
      `    ${conflict.id}  "model.rs · fn compile"  (overlapping work)`,
      "      Yours:  This branch (Tim Belanger): \"edits native/src/model.rs\" at native/src/model.rs:469",
      "          @@ -469 +469 @@",
      "          -matchers: Vec<GrammarMatcher>,",
      "          +matchers: Vec<GrammarMatcher>, // cloned",
      "      Theirs: PR #398 (jbarnes): \"Share compiled matchers\" at native/src/model.rs:470 https://github.com/acme/app/pull/398",
      "          @@ -470 +470 @@",
      "          +matchers: Arc<[GrammarMatcher]>,",
      "      Both sides change native/src/model.rs within 3 lines of each other.",
      `      Resolve: rerail merge-assist resolve ${conflict.id} mine|theirs|new|dismiss [--text "..."]`,
      "",
    ].join("\n"));
    stdout = "";
    await run(["--repo", checkout], null);
    expect(stdout).toContain(`  Conflicts (1):\n    ${conflict.id}  "model.rs · fn compile"  (overlapping work)\n      Yours:  This branch`);
    expect(stdout).not.toContain("@@ -469");

    stdout = "";
    status = path => sampleStatus(path, { conflicts: [resolved] });
    expect(await run(["resolve", conflict.id, "theirs", "--text", "Share them.", "--repo", checkout], null)).toBe(0);
    expect(stdout).toBe(`● "model.rs · fn compile": took PR #398's side, by Tim Belanger; decision .rerail/candidates/share-compiled-matchers-1a2b3c.json. Logged as decision ${"0".repeat(32)} in refs/rerail/decisions.\n`);
    expect(service.calls.at(-1)).toEqual({ method: "resolveConflict", checkout, conflictId: conflict.id, action: "theirs", text: "Share them.", actor: "person" });

    await expect(run(["resolve", conflict.id, "--repo", checkout])).rejects.toThrow(/^merge-assist resolve needs mine, theirs, new or dismiss/u);
    await expect(run(["resolve", "--repo", checkout])).rejects.toThrow(/^merge-assist resolve needs a conflict id/u);
    await expect(run(["status", "--text", "x", "--repo", checkout])).rejects.toThrow(/^--text goes with merge-assist resolve/u);
  });
});
