import { mkdtempSync, realpathSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, describe, expect, it, vi } from "vitest";
import type { AuditInput, AuditRecorder, AuditResult, AuditScope } from "../../../src/engine/audit/audit.js";
import type { CodeRulePlacement } from "../../../src/engine/catalog/repository-model.js";
import type { CodeRuleEdit, RuleSteerDelivery, SteerMetadata } from "../../../src/engine/code-rule/code-rule.js";
import type { RerailRuleEntry } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { RrAuditRun } from "../../../src/engine/rr/rr-audit-run.js";
import { RerailEditRuns } from "../../../src/engine/rr/rr-edit-runs.js";
import { RrRunQueue } from "../../../src/engine/rr/rr-run-queue.js";
import type { RrFinding, RrRun, RrRunOutcome } from "../../../src/engine/rr/rr-runner.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

const root = realpathSync(mkdtempSync(join(tmpdir(), "rr-edit-runs-")));

afterAll(async () => {
  await removeFolder(root);
});

function rule(id: string, placement: CodeRulePlacement): RerailRuleEntry {
  return {
    id,
    path: `/packs/rr-sample/rules/${id}.rerail`,
    placement,
    scope: { include: ["**/*.py"], exclude: [] },
    language: "python",
    files: [],
    provenance: {
      ruleId: id, revision: 1, payloadHash: `hash-${id}`, source: { kind: "pack", id: "rr-sample", packId: "rr-sample" },
      repositoryKey: "repository", checkoutPath: root, placement: [],
    },
  };
}

/** A finding of `policy` on `path` over the bytes `[startByte, endByte)` of line `line` (0-based). */
function finding(policy: string, path: string, line: number, startByte: number, endByte: number): RrFinding {
  return {
    policy, clause: "Found", code: "F001", message: `found by ${policy}`, path, line: line + 1, tag: null, parameters: {},
    span: { startByte, endByte, startLine: line, startColumn: 0, endLine: line, endColumn: endByte - startByte },
  };
}

const completed = (findings: RrFinding[]): RrRunOutcome => ({ status: "completed", findings, failures: [], files: 2, wallMs: 1, peakRssBytes: 1 });

/** A session's `.rerail` runs over fakes: a runner whose runs end when the test says, the audits and steers it records. */
function session(rules: RerailRuleEntry[], scope: AuditScope = "touched-files") {
  const runs: { run: RrRun; end: (outcome: RrRunOutcome) => void }[] = [];
  const runner = (run: RrRun): Promise<RrRunOutcome> => {
    const { promise, resolve } = Promise.withResolvers<RrRunOutcome>();
    runs.push({ run, end: resolve });
    run.signal?.addEventListener("abort", () => resolve({ status: "killed", reason: "aborted", wallMs: 1, peakRssBytes: 1 }));
    return promise;
  };
  const started: AuditInput[] = [];
  const finished: AuditResult[] = [];
  const audits: AuditRecorder = {
    start: input => `audit-${started.push(input)}`,
    finish: (_auditId, result) => void finished.push(result),
    progress: () => undefined,
    interrupt: () => undefined,
  };
  const steers: { content: string; metadata: SteerMetadata | undefined }[] = [];
  const delivery: RuleSteerDelivery = {
    requestSteer: async (_sessionId, _delivery, content, metadata) => {
      steers.push({ content, metadata });
    },
  };
  const queue = new RrRunQueue(root, { deadlineMs: 30_000, quietMs: 1 }, runner);
  const edits = new RerailEditRuns({
    sessionId: "session",
    workspaceRoot: root,
    rules: () => rules,
    catalogRevision: () => "catalog",
    audits,
    steerTarget: { sessionId: "session", delivery, workspaceRoot: root, presentPath: path => path, steerShell: undefined },
    nestedCheckouts: () => [],
    queue,
  });
  const write = (revision: number, ...written: CodeRuleEdit[]) => edits.gather(written, revision, scope, "steer");
  return { runs, started, finished, steers, edits, write };
}

const written = (path: string, generatedText = "x = 1\n", previousText?: string): CodeRuleEdit =>
  ({ path, language: "python", generatedText, ...(previousText === undefined ? {} : { previousText }) });

describe("a session's .rerail runs after its edits", () => {
  it("records each finding at the mode its rule's placement gives the path, drops those where the rule is off, and steers only what steers", async () => {
    const legacyLogged = rule("rr-legacy-logged", [{ folder: "", mode: "steer" }, { folder: "legacy", mode: "log" }]);
    const appOnly = rule("rr-app-only", [{ folder: "app", mode: "steer" }]);
    const { runs, started, finished, steers, write } = session([legacyLogged, appOnly]);
    write(1, written("app/a.py"), written("legacy/b.py"));
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    expect(runs[0]!.run.policies).toEqual([appOnly.path, legacyLogged.path]);
    expect(started).toMatchObject([{ scope: "touched-files", mode: "steer", revision: 1, paths: ["app/a.py", "legacy/b.py"] }]);
    expect(started[0]!.ruleProvenance?.map(entry => entry.ruleId)).toEqual(["rr-legacy-logged", "rr-app-only"]);
    runs[0]!.end(completed([
      finding("rr-legacy-logged", "app/a.py", 0, 0, 5),
      finding("rr-legacy-logged", "legacy/b.py", 0, 0, 5),
      finding("rr-app-only", "legacy/b.py", 0, 0, 5),
    ]));
    await vi.waitFor(() => expect(steers).toHaveLength(1));
    expect(finished).toHaveLength(1);
    expect(finished[0]!.status).toBe("completed");
    expect(finished[0]!.violations?.map(({ ruleId, path, mode, span }) => ({ ruleId, path, mode, startByte: span?.startByte }))).toEqual([
      { ruleId: "rr-legacy-logged", path: "app/a.py", mode: "steer", startByte: 0 },
      { ruleId: "rr-legacy-logged", path: "legacy/b.py", mode: "log", startByte: 0 },
    ]);
    expect(steers[0]!.metadata).toMatchObject({ revision: 1, background: true, ruleIds: ["rr-legacy-logged"] });
    expect(steers[0]!.metadata?.generatedEdit?.violations.map(violation => violation.path)).toEqual(["app/a.py"]);
  });

  it("in ai-output scope keeps only the findings on the lines the agent wrote since the file's last run", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    const { runs, finished, write } = session([assert], "ai-output");
    // Two writes before the run: the first one's preimage is what the run measures from.
    write(1, written("a.py", "x = 1\nassert y\n", "x = 1\n"));
    write(2, written("a.py", "x = 1\nassert y\nassert z\n", "x = 1\nassert y\n"));
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    runs[0]!.end(completed([finding("rr-sample-assert", "a.py", 0, 0, 5), finding("rr-sample-assert", "a.py", 1, 6, 14), finding("rr-sample-assert", "a.py", 2, 15, 23)]));
    await vi.waitFor(() => expect(finished).toHaveLength(1));
    expect(finished[0]!.violations?.map(violation => violation.span?.startLine)).toEqual([1, 2]);
  });

  it("records a run killed past its deadline as a failure of each rule it carried, saying how to raise the deadline, and steers nothing", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    const { runs, finished, steers, write } = session([assert]);
    write(1, written("a.py"));
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    runs[0]!.end({ status: "killed", reason: "deadline", wallMs: 30_000, peakRssBytes: 1 });
    await vi.waitFor(() => expect(finished).toHaveLength(1));
    expect(finished[0]).toMatchObject({ status: "failed", violations: [], failures: [{ ruleId: "rr-sample-assert", code: "budget-exceeded" }] });
    expect(finished[0]!.failures?.[0]?.message).toContain("RERAIL_RR_DEADLINE_MS");
    expect(steers).toEqual([]);
  });

  it("gathers only the writes some rule takes: in its placement, its scope and its language", async () => {
    const appOnly = rule("rr-app-only", [{ folder: "app", mode: "steer" }, { folder: "app/off", mode: null }]);
    const { runs, started, edits, write } = session([appOnly]);
    expect(edits.takes("app/a.py")).toBe(true);
    expect(edits.takes("app/off/a.py")).toBe(false);
    expect(edits.takes("lib/a.py")).toBe(false);
    expect(edits.takes("app/a.ts")).toBe(false);
    write(1, written("lib/a.py"), written("app/off/b.py"));
    write(2, written("app/c.py"));
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    expect(started.map(audit => audit.paths)).toEqual([["app/c.py"]]);
  });
});

describe("a session's .rerail runs when the session or its rules change", () => {
  it("kills the run when the session closes and records its audit interrupted", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    const { runs, finished, edits, write } = session([assert]);
    write(1, written("a.py"));
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    edits.close();
    expect(runs[0]!.run.signal?.aborted).toBe(true);
    expect(finished).toEqual([{ status: "interrupted" }]);
  });

  it("kills the run when the rules change, records it superseded, and runs its files again with the new rules", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    const rules = [assert];
    const { runs, finished, edits, write } = session(rules);
    write(1, written("a.py"));
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    // The rule turned off in `legacy/`: a placement change.
    rules[0] = rule("rr-sample-assert", [{ folder: "", mode: "steer" }, { folder: "legacy", mode: null }]);
    edits.rulesChanged();
    expect(runs[0]!.run.signal?.aborted).toBe(true);
    expect(finished).toEqual([{ status: "superseded" }]);
    await vi.waitFor(() => expect(runs).toHaveLength(2));
    runs[1]!.end(completed([finding("rr-sample-assert", "a.py", 0, 0, 5)]));
    await vi.waitFor(() => expect(finished).toHaveLength(2));
    expect(finished[1]!.violations?.map(violation => violation.path)).toEqual(["a.py"]);
  });
});

describe("an audit's .rerail run", () => {
  it("keeps the findings in the files a touched-files audit was given, outside the checkouts nested in it", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    const off = rule("rr-off", [{ folder: "", mode: null }]);
    const run = new RrAuditRun({ root, rules: [assert, off], paths: ["src"], nestedCheckouts: ["src/vendored"] },
      async request => {
        expect(request.policies).toEqual([assert.path]);
        return completed([
          finding("rr-sample-assert", "src/a.py", 0, 0, 5),
          finding("rr-sample-assert", "src/vendored/b.py", 0, 0, 5),
          finding("rr-sample-assert", "tests/c.py", 0, 0, 5),
        ]);
      });
    expect(run.provenance.map(entry => entry.ruleId)).toEqual(["rr-sample-assert"]);
    run.start();
    expect((await run.results()).violations.map(violation => violation.path)).toEqual(["src/a.py"]);
  });

  it("kills its child when stopped while the audit waits for it, and gives the audit no results to record as complete", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    let signal: AbortSignal | undefined;
    const run = new RrAuditRun({ root, rules: [assert], paths: undefined, nestedCheckouts: [] }, request => {
      signal = request.signal;
      const { promise, resolve } = Promise.withResolvers<RrRunOutcome>();
      request.signal?.addEventListener("abort", () => resolve({ status: "killed", reason: "aborted", wallMs: 1, peakRssBytes: 1 }));
      return promise;
    });
    run.start();
    const results = run.results();
    run.abort();
    expect(signal?.aborted).toBe(true);
    await expect(results).rejects.toThrow("its session closed");
  });
});
