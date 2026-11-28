import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterAll, describe, expect, it, vi } from "vitest";
import type { AuditInput, AuditRecorder, AuditResult, AuditScope } from "../../../src/engine/audit/audit.js";
import type { CodeRulePlacement } from "../../../src/engine/catalog/repository-model.js";
import { rulePackFileSchema } from "../../../src/engine/catalog/rule-packs.js";
import type { CodeRuleEdit, RuleSteerDelivery, SteerMetadata } from "../../../src/engine/code-rule/code-rule.js";
import type { RerailRuleEntry } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { RrAuditRun } from "../../../src/engine/rr/rr-audit-run.js";
import { RerailEditRuns } from "../../../src/engine/rr/rr-edit-runs.js";
import { RrRunQueue } from "../../../src/engine/rr/rr-run-queue.js";
import { RrRunSlots } from "../../../src/engine/rr/rr-run-slots.js";
import type { RrBudgets, RrFinding, RrRun, RrRunOutcome } from "../../../src/engine/rr/rr-runner.js";
import { defaultRerailHome } from "../../../src/engine/index.js";
import { removeFolder } from "../../support/platform/remove-folder.js";
import { installPackDirectory, removeInstalledPacks, uninstallPack, versionOf } from "../../support/installed-packs.js";
import { fixturesPath } from "../../support/oracles.js";

const root = realpathSync(mkdtempSync(join(tmpdir(), "rr-edit-runs-")));
/** The fixture pack whose rules `rule` names: three `.rerail` rules, each with the title its pack.json lists. */
const SAMPLE_PACK = fixturesPath("packs", "rr-sample");

afterAll(async () => {
  removeInstalledPacks();
  await removeFolder(root);
});

function rule(id: string, placement: CodeRulePlacement): RerailRuleEntry {
  return {
    id,
    path: `/packs/rr-sample/rules/${id}.rerail`,
    placement,
    scope: { include: ["**/*.py"], exclude: [], text: [] },
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
const BUDGETS: RrBudgets = { deadlineMs: 30_000, threads: 2, memoryBytes: 1024 ** 3, store: false };

/** A session's `.rerail` runs over fakes: a runner whose runs end when the test says, the audits and steers it records. */
function session(rules: RerailRuleEntry[], scope: AuditScope = "touched-files") {
  const runs: { run: RrRun; end: (outcome: RrRunOutcome) => void }[] = [];
  const runner = (run: RrRun): Promise<RrRunOutcome> => {
    const { promise, resolve } = Promise.withResolvers<RrRunOutcome>();
    runs.push({ run, end: resolve });
    run.signal?.addEventListener("abort", () => resolve({ status: "killed", reason: "aborted", wallMs: 1, peakRssBytes: 1, budgets: BUDGETS }));
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
  const slots = new RrRunSlots(runner);
  const queue = new RrRunQueue(root, { deadlineMs: 30_000, quietMs: 1 }, slots);
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
  return { runs, started, finished, steers, edits, write, slots };
}

const written = (path: string, generatedText = "x = 1\n", previousText?: string): CodeRuleEdit =>
  ({ path, language: "python", generatedText, ...(previousText === undefined ? {} : { previousText }) });

/** Writes `text` to `path` in the checkout, as an edit or another writer leaves it on disk. */
function onDisk(path: string, text: string): void {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), text);
}

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

  it("names a pack's .rerail rule in the steer by the title its pack lists", async ({ onTestFinished }) => {
    // The steer reads the installed packs, which this file's home has not read yet, so the engine reads each of the
    // pack's `.rerail` rules here, with a test run's empty image store. A pack of three small rules keeps that well
    // within the case's limit; one the size of Django's (27 rules) takes more than 5 s on a 16-core Linux machine.
    installPackDirectory(SAMPLE_PACK, "rr-sample", defaultRerailHome(), versionOf(SAMPLE_PACK));
    onTestFinished(() => uninstallPack("rr-sample"));
    const manifest = rulePackFileSchema.parse(JSON.parse(readFileSync(join(SAMPLE_PACK, "pack.json"), "utf8")));
    const listed = manifest.rules.find(entry => entry.id === "rr-sample-assert")?.title;
    expect(listed).toBeDefined();
    const { runs, steers, write } = session([rule("rr-sample-assert", [{ folder: "", mode: "steer" }])]);
    write(1, written("app/a.py"));
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    runs[0]!.end(completed([finding("rr-sample-assert", "app/a.py", 0, 0, 5)]));
    await vi.waitFor(() => expect(steers).toHaveLength(1));
    expect(steers[0]!.metadata?.rules).toEqual([{ id: "rr-sample-assert", title: listed, origin: "From the Sample rules in .rerail rule pack.", state: "steered" }]);
  });

  it("in ai-output scope keeps only the findings on the lines the agent wrote since the file's last run", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    const { runs, finished, write } = session([assert], "ai-output");
    // Two writes before the run: the first one's preimage is what the run measures from.
    write(1, written("a.py", "x = 1\nassert y\n", "x = 1\n"));
    write(2, written("a.py", "x = 1\nassert y\nassert z\n", "x = 1\nassert y\n"));
    onDisk("a.py", "x = 1\nassert y\nassert z\n");
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
    runs[0]!.end({ status: "killed", reason: "deadline", wallMs: 30_000, peakRssBytes: 1, budgets: BUDGETS });
    await vi.waitFor(() => expect(finished).toHaveLength(1));
    expect(finished[0]).toMatchObject({ status: "failed", violations: [], failures: [{ ruleId: "rr-sample-assert", code: "budget-exceeded" }] });
    expect(finished[0]!.failures?.[0]?.message).toContain("RERAIL_RR_DEADLINE_MS");
    expect(steers).toEqual([]);
  });

  it("leaves out a rule whose file changed since the catalog read it, with a failure naming the file, and runs the others", async () => {
    const library = join(root, "changed-lib.rerail");
    writeFileSync(library, "// as approved\n");
    const approved = createHash("sha256").update("// as approved\n").digest("hex");
    const changed = { ...rule("rr-changed", [{ folder: "", mode: "steer" }]), files: [{ path: library, sha256: approved }] };
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    writeFileSync(library, "// edited after approval\n");
    const { runs, finished, write } = session([changed, assert]);
    write(1, written("a.py"));
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    expect(runs[0]!.run.policies).toEqual([assert.path]);
    runs[0]!.end(completed([]));
    await vi.waitFor(() => expect(finished).toHaveLength(1));
    expect(finished[0]).toMatchObject({ status: "failed", failures: [{ ruleId: "rr-changed", message: expect.stringContaining(`${library} changed since Rerail read it`) }] });
  });

  it("checks a rule's files when its run's turn comes: a file edited while the run waits for the slot leaves that rule out", async () => {
    const library = join(root, "waiting-lib.rerail");
    writeFileSync(library, "// as approved\n");
    const approved = createHash("sha256").update("// as approved\n").digest("hex");
    const changed = { ...rule("rr-edited-while-waiting", [{ folder: "", mode: "steer" }]), files: [{ path: library, sha256: approved }] };
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    const { runs, started, finished, write, slots } = session([changed, assert]);
    // Another run holds the process's slot.
    const holding = slots.run({ start: () => ({ root, policies: ["/rules/held.rerail"] }) });
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    write(1, written("a.py"));
    // The edit's run waits for the slot; the rule's library is edited meanwhile.
    writeFileSync(library, "// edited while the run waited\n");
    runs[0]!.end(completed([]));
    await holding;
    await vi.waitFor(() => expect(runs).toHaveLength(2));
    expect(runs[1]!.run.policies).toEqual([assert.path]);
    // Its audit, started as its turn came, carries both rules.
    expect(started[0]!.ruleProvenance?.map(entry => entry.ruleId)).toEqual(["rr-edited-while-waiting", "rr-sample-assert"]);
    runs[1]!.end(completed([]));
    await vi.waitFor(() => expect(finished).toHaveLength(1));
    expect(finished[0]).toMatchObject({ status: "failed", failures: [{ ruleId: "rr-edited-while-waiting", message: expect.stringContaining(`${library} changed since Rerail read it`) }] });
  });

  it("runs no file another writer changed between the edit and the run's turn, until this session writes it again", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    const { runs, started, finished, write, slots } = session([assert], "ai-output");
    const asked = vi.spyOn(slots, "run");
    const holding = slots.run({ start: () => ({ root, policies: ["/rules/held.rerail"] }) });
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    onDisk("shop/views.py", "x = 1\nassert y\n");
    write(5, written("shop/views.py", "x = 1\nassert y\n", "x = 1\n"));
    // Before the run reads the file, another session inserts lines at the top.
    onDisk("shop/views.py", "import os\nimport sys\nx = 1\nassert y\n");
    // The edit's run asks for its turn after its quiet period; a turn asked after it comes only once it has passed.
    await vi.waitFor(() => expect(asked).toHaveBeenCalledTimes(2));
    runs[0]!.end(completed([]));
    await holding;
    expect(await slots.run({ start: () => undefined })).toMatchObject({ status: "completed" });
    expect(runs).toHaveLength(1);
    expect(started).toEqual([]);

    // This session's next write of the file runs it, reporting every line it wrote since the first edit.
    onDisk("shop/views.py", "import os\nimport sys\nx = 1\nassert y\nassert z\n");
    write(6, written("shop/views.py", "import os\nimport sys\nx = 1\nassert y\nassert z\n", "import os\nimport sys\nx = 1\nassert y\n"));
    await vi.waitFor(() => expect(runs).toHaveLength(2));
    runs[1]!.end(completed([finding("rr-sample-assert", "shop/views.py", 2, 21, 26), finding("rr-sample-assert", "shop/views.py", 3, 27, 35), finding("rr-sample-assert", "shop/views.py", 4, 36, 44)]));
    await vi.waitFor(() => expect(finished).toHaveLength(1));
    expect(finished[0]!.violations?.map(violation => violation.span?.startLine)).toEqual([3, 4]);
  });

  it("reports nothing for a file another writer changed while the run went on, and runs it again at this session's next write", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    const { runs, finished, steers, write } = session([assert], "ai-output");
    onDisk("shop/models.py", "x = 1\nassert y\n");
    write(1, written("shop/models.py", "x = 1\nassert y\n", "x = 1\n"));
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    onDisk("shop/models.py", "import os\nx = 1\nassert y\n");
    runs[0]!.end(completed([finding("rr-sample-assert", "shop/models.py", 1, 10, 15)]));
    await vi.waitFor(() => expect(finished).toHaveLength(1));
    expect(finished[0]).toMatchObject({ status: "completed", violations: [] });
    expect(steers).toEqual([]);

    onDisk("shop/models.py", "import os\nx = 1\nassert y\nassert z\n");
    write(2, written("shop/models.py", "import os\nx = 1\nassert y\nassert z\n", "import os\nx = 1\nassert y\n"));
    await vi.waitFor(() => expect(runs).toHaveLength(2));
    runs[1]!.end(completed([finding("rr-sample-assert", "shop/models.py", 2, 16, 24), finding("rr-sample-assert", "shop/models.py", 3, 25, 33)]));
    await vi.waitFor(() => expect(finished).toHaveLength(2));
    // The lines written since the first edit's preimage: `assert y`, and `assert z`.
    expect(finished[1]!.violations?.map(violation => violation.span?.startLine)).toEqual([2, 3]);
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

  it("starts no run for a write outside the rule's `.scope dirs`, and one for a file it reads as text whatever its language", () => {
    // As describe gives `.scope dirs "shop"`, `.scope tests false` and `.scope text "**/*.md"`.
    const shop = { ...rule("rr-shop", [{ folder: "", mode: "steer" }]), scope: { include: ["shop/**"], exclude: ["tests/**", "**/tests/**"], text: ["**/*.md"] } };
    const { edits } = session([shop]);
    expect(edits.takes("shop/views.py")).toBe(true);
    expect(edits.takes("docs/conf.py")).toBe(false);
    expect(edits.takes("shop/tests/test_views.py")).toBe(false);
    expect(edits.takes("shop/README.md")).toBe(true);
    expect(edits.takes("docs/README.md")).toBe(false);
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

  it("leaves the run going when a rule that takes none of its files changes, is turned on or is turned off", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    const templates = { ...rule("rr-templates", [{ folder: "", mode: null }]), scope: { include: ["**/*.html"], exclude: [], text: [] } };
    const rules = [assert, templates];
    const { runs, finished, edits, write } = session(rules);
    write(1, written("a.py"));
    await vi.waitFor(() => expect(runs).toHaveLength(1));
    rules[1] = { ...templates, placement: [{ folder: "", mode: "steer" }] };
    edits.rulesChanged();
    rules.pop();
    edits.rulesChanged();
    expect(runs[0]!.run.signal?.aborted).toBe(false);
    runs[0]!.end(completed([finding("rr-sample-assert", "a.py", 0, 0, 5)]));
    await vi.waitFor(() => expect(finished).toHaveLength(1));
    expect(finished[0]).toMatchObject({ status: "completed", violations: [{ path: "a.py" }] });
  });
});

describe("an audit's .rerail run", () => {
  it("keeps the findings in the files a touched-files audit was given, outside the checkouts nested in it", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    const off = rule("rr-off", [{ folder: "", mode: null }]);
    const run = new RrAuditRun({ root, rules: [assert, off], paths: ["src"], nestedCheckouts: ["src/vendored"] },
      new RrRunSlots(async request => {
        expect(request.policies).toEqual([assert.path]);
        return completed([
          finding("rr-sample-assert", "src/a.py", 0, 0, 5),
          finding("rr-sample-assert", "src/vendored/b.py", 0, 0, 5),
          finding("rr-sample-assert", "tests/c.py", 0, 0, 5),
        ]);
      }));
    expect(run.provenance.map(entry => entry.ruleId)).toEqual(["rr-sample-assert"]);
    run.start();
    expect((await run.results()).violations.map(violation => violation.path)).toEqual(["src/a.py"]);
  });

  it("kills its child when stopped while the audit waits for it, and gives the audit no results to record as complete", async () => {
    const assert = rule("rr-sample-assert", [{ folder: "", mode: "steer" }]);
    let signal: AbortSignal | undefined;
    const run = new RrAuditRun({ root, rules: [assert], paths: undefined, nestedCheckouts: [] }, new RrRunSlots(request => {
      signal = request.signal;
      const { promise, resolve } = Promise.withResolvers<RrRunOutcome>();
      request.signal?.addEventListener("abort", () => resolve({ status: "killed", reason: "aborted", wallMs: 1, peakRssBytes: 1, budgets: BUDGETS }));
      return promise;
    }));
    run.start();
    const results = run.results();
    // Its turn has come and its child started: the audit now waits for it.
    await vi.waitFor(() => expect(signal).toBeDefined());
    run.abort();
    expect(signal?.aborted).toBe(true);
    await expect(results).rejects.toThrow("its session closed");
  });
});
