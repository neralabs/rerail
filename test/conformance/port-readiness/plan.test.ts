// conformance/port-readiness/plan.mjs: the legs port-readiness.yml runs, every one or the ones a change selects.
import assert from "node:assert/strict";
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";

import { afterAll, describe, it } from "vitest";
import { parse } from "yaml";

import { changedPaths, gatedToolPacks, legsOf, plan, REPORT_ONLY, ROOT, ROOTS, SHARDS, writeOutputs, type Leg } from "../../../conformance/port-readiness/plan.mjs";
import { GATE_CODE } from "../../../conformance/port-readiness/cache.js";
import { gatedToolPacks as runGatedToolPacks } from "../../../conformance/port-readiness/run.js";

const temporary: string[] = [];
afterAll(() => {
  for (const directory of temporary.splice(0)) rmSync(directory, { recursive: true, force: true });
});

function tempRoot(): string {
  const root = mkdtempSync(join(tmpdir(), "rerail-gates-plan-"));
  temporary.push(root);
  return root;
}

function write(root: string, path: string, text = "\n"): void {
  mkdirSync(join(root, dirname(path)), { recursive: true });
  writeFileSync(join(root, path), text);
}

const workflow = (name: string): Record<string, any> => parse(readFileSync(join(ROOT, ".github", "workflows", name), "utf8"));

// The legs port-readiness.yml listed by hand before the plan built them, in discovery order (packs, then tools,
// sorted). The order only orders the jobs in the run's page; names, artifacts and cache keys come from the fields.
const EVERY_LEG: Leg[] = [
  { pack: "javascript", tool: "eslint", shard: 1, shards: 4 },
  { pack: "javascript", tool: "eslint", shard: 2, shards: 4 },
  { pack: "javascript", tool: "eslint", shard: 3, shards: 4 },
  { pack: "javascript", tool: "eslint", shard: 4, shards: 4 },
  { pack: "javascript", tool: "eslint-plugin-unicorn" },
  { pack: "next", tool: "eslint-plugin-next" },
  { pack: "python", tool: "bandit" },
  { pack: "python", tool: "ruff" },
  { pack: "react", tool: "eslint-plugin-react" },
  { pack: "react", tool: "eslint-plugin-react-hooks" },
  { pack: "typescript", tool: "eslint-plugin-vitest" },
  { pack: "typescript", tool: "typescript-eslint" },
];

const TOOL_PACKS = gatedToolPacks();
const names = (entries: readonly { pack: string; tool: string }[]): string[] => entries.map(({ pack, tool }) => `${pack}/${tool}`).sort();

describe("every leg", () => {
  it("is one leg per gated tool-pack, sharded as SHARDS says", () => {
    assert.deepEqual(plan(TOOL_PACKS).legs, EVERY_LEG);
    assert.deepEqual(plan(TOOL_PACKS).toolPacks, names(TOOL_PACKS));
    for (const toolPack of Object.keys(SHARDS)) assert.ok(names(TOOL_PACKS).includes(toolPack), `SHARDS names ${toolPack}, which is not gated`);
  });

  it("comes from the tool-packs run.ts gates", () => {
    assert.deepEqual(TOOL_PACKS, runGatedToolPacks(ROOT));
  });

  it("covers what ci.yml's port-readiness filter calls the workflow for, glob for glob", () => {
    const filter = workflow("ci.yml").jobs.changes.steps.find((step: { id?: string }) => step.id === "filter");
    assert.deepEqual(ROOTS, parse(filter.with.filters)["port-readiness"]);
  });

  // A report-only path selects no leg, so it must also be out of every rule's key (cache.ts GATE_CODE): otherwise a
  // pull request that edits only it would skip the legs whose keys it moved.
  it("leaves out only files the results key leaves out too", () => {
    const tracked = (pathspec: readonly string[]): string[] =>
      execFileSync("git", ["ls-files", "-z", "--", ...pathspec], { cwd: ROOT, encoding: "utf8" }).split("\0").filter((path) => path !== "");
    const keyed = new Set(tracked(GATE_CODE));
    for (const path of REPORT_ONLY) {
      assert.deepEqual(tracked([path]), [path], `REPORT_ONLY names ${path}, which is not tracked`);
      assert.ok(!keyed.has(path), `REPORT_ONLY names ${path}, which is part of every rule's key`);
    }
  });

  it("is what the recorder and the nightly's drift check list by hand", () => {
    const steps = Object.values(workflow("port-readiness-record.yml").jobs).flatMap((job: any) => job.steps ?? []);
    const recorded = steps.find((step: any) => step.env?.TOOL_PACKS !== undefined).env.TOOL_PACKS;
    assert.deepEqual(names(JSON.parse(recorded)), names(TOOL_PACKS));
    assert.deepEqual(names(workflow("port-readiness-nightly.yml").jobs["record-check"].strategy.matrix.include), names(TOOL_PACKS));
  });
});

// port-readiness-record.yml's publish job, read the way the runner reads it.
describe("the record workflow", () => {
  const publish = (): { name: string; if?: string; run?: string; env?: Record<string, string> }[] => workflow("port-readiness-record.yml").jobs.publish.steps;
  const push = () => publish().find((step) => step.name === "Push the commit")!;

  // A run with no pull request opens none from the branch it pushes; a person does, from the link in the run's summary.
  it("opens no pull request: a run with none pushes its branch and links to open one by hand", () => {
    assert.deepEqual(publish().filter((step) => /\bgh pr create\b/u.test(step.run ?? "")).map((step) => step.name), []);
    assert.match(push().run!, /\/compare\/\$BASE\.\.\.\$branch\?expand=1/u);
    // And the job goes on: the arm for no pull request ends by saying there is none to comment on, and fails nothing.
    const arm = /if \[ -z "\$PR" \]; then\n([\s\S]*?)\n\s*elif /u.exec(push().run!)?.[1];
    assert.ok(arm !== undefined, "Push the commit has no arm for a run with no pull request");
    assert.doesNotMatch(arm, /\bexit [1-9]/u);
    assert.match(arm, /echo "pr=" >> "\$GITHUB_OUTPUT"$/u);
    // What the run's summary says is built before the push, so a step that fails after it cannot leave a branch
    // pushed with no notice and no summary saying where it is.
    const pushed = arm.indexOf("git push origin");
    assert.ok(pushed > 0, "the arm pushes nothing");
    for (const built of ["compare=", "prefilled=", '} > "$body"']) assert.ok(arm.indexOf(built) >= 0 && arm.indexOf(built) < pushed, `${built} comes after the push`);
    assert.ok(arm.indexOf("::notice::") > pushed, "the notice says the branch was pushed before it was");
  });

  it("links the pull request it leaves to open to the run attempt that pushed its branch", () => {
    // The branch is named for the attempt, and a re-run's page would show another attempt's summary.
    assert.equal(push().env!.RUN, "${{ github.run_id }}-${{ github.run_attempt }}");
    const arm = /if \[ -z "\$PR" \]; then\n([\s\S]*?)\n\s*elif /u.exec(push().run!)![1]!;
    assert.match(arm, /branch="port-readiness-recordings\/run-\$RUN"/u);
    assert.match(arm, /\/actions\/runs\/\$GITHUB_RUN_ID\/attempts\/\$GITHUB_RUN_ATTEMPT\b/u);
    assert.doesNotMatch(arm, /\/actions\/runs\/\$GITHUB_RUN_ID(?!\/attempts)/u);
  });

  it("says what to do when there is nothing to commit and port readiness still fails", () => {
    // The lock it would write is the one committed, so a recording of this commit changes nothing the gates read.
    const none = /if \[ -z "\$COMMIT" \]; then\n([\s\S]*?)\n\s*fi\n/u.exec(push().run!)?.[1];
    assert.ok(none !== undefined, "Push the commit has no arm for a run with nothing to commit");
    assert.match(none, /If port readiness still fails on this commit, recording it again changes nothing: the failing check says what to do instead\./u);
  });

  it("puts what a run with no pull request recorded, and what moved per rule, in the run's summary", () => {
    const summary = publish().find((step) => step.if === "steps.push.outputs.pr == ''");
    assert.ok(summary?.run !== undefined, "no step writes the summary of a run with no pull request to comment on");
    assert.match(summary.run, /cat "\$RUNNER_TEMP\/comment\.md" >> "\$GITHUB_STEP_SUMMARY"/u);
    assert.match(summary.run, /\/changes\.md"/u);
  });
});

describe("the legs a changed path selects", () => {
  const ruff = EVERY_LEG.filter((leg) => leg.tool === "ruff");
  const bandit = EVERY_LEG.filter((leg) => leg.tool === "bandit");
  const cases: [string, Leg[]][] = [
    ["packs/node/README.md", []],
    ["packs/README.md", []],
    ["packs/python/candidates/rules/x.json", [...bandit, ...ruff]],
    ["conformance/port-readiness/run.ts", EVERY_LEG],
    ["conformance/port-readiness/plan.mjs", EVERY_LEG],
    ["native/src/x.rs", EVERY_LEG],
    ["package-lock.json", EVERY_LEG],
    ["conformance/recorders/port-readiness/runtimes/ruff-0.16.9/pyproject.toml", EVERY_LEG],
    ["conformance/languages/runner.ts", EVERY_LEG],
    ["conformance/port-readiness/ungated-rules.json", []],
    ["conformance/port-readiness/ungated.ts", []],
    // Out of the key, but the report job's merge and summary should run on real reports.
    ["conformance/port-readiness/summary.ts", EVERY_LEG],
    ["conformance/port-readiness/merge.ts", EVERY_LEG],
    ["docs/port-readiness.md", []],
  ];
  for (const [path, legs] of cases) {
    it(`${path}: ${legs.length === EVERY_LEG.length ? "every leg" : `${legs.length} leg(s)`}`, () => {
      assert.deepEqual(plan(TOOL_PACKS, [path]).legs, legs);
    });
  }

  it("a pack that gains a gates.yaml gets one unsharded leg", () => {
    const root = tempRoot();
    write(root, "packs/node/upstream/eslint-plugin-n/gates.yaml");
    write(root, "packs/node/rules/no-sync.json", "{}\n");
    const planned = plan(gatedToolPacks(root), ["packs/node/rules/no-sync.json"]);
    assert.deepEqual(planned.legs, [{ pack: "node", tool: "eslint-plugin-n" }]);
    assert.deepEqual(planned.lines, ["node/eslint-plugin-n (1 leg): packs/node/rules/no-sync.json changed"]);
  });

  it("writes exactly `legs=` and `tool-packs=` lines, empty lists included", () => {
    const out = join(tempRoot(), "github-output");
    writeOutputs(out, plan(TOOL_PACKS, ["docs/port-readiness.md"]));
    writeOutputs(out, { legs: legsOf([{ pack: "python", tool: "bandit" }]), toolPacks: ["python/bandit"] });
    assert.equal(readFileSync(out, "utf8"), 'legs=[]\ntool-packs=[]\nlegs=[{"pack":"python","tool":"bandit"}]\ntool-packs=["python/bandit"]\n');
  });
});

describe("the command", () => {
  // A checkout of its own with a copy of plan.mjs, whose root is that checkout: two gated packs and a rule document,
  // then the rule moved from one pack to the other, then a change no leg reads.
  const root = tempRoot();
  const git = (...args: string[]): string =>
    execFileSync("git", ["-c", "user.name=gates", "-c", "user.email=gates@example.test", "-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null", ...args], { cwd: root, encoding: "utf8" });
  git("init", "-q", "-b", "main");
  write(root, "packs/python/upstream/bandit/gates.yaml");
  write(root, "packs/javascript/upstream/eslint-plugin-unicorn/gates.yaml");
  write(root, "packs/python/candidates/rules/bandit-b101.json", '{ "id": "bandit-b101", "rule": "a document long enough to be a rename" }\n');
  mkdirSync(join(root, "conformance", "port-readiness"), { recursive: true });
  copyFileSync(join(ROOT, "conformance", "port-readiness", "plan.mjs"), join(root, "conformance", "port-readiness", "plan.mjs"));
  git("add", "-A");
  git("commit", "-q", "-m", "base");
  mkdirSync(join(root, "packs", "javascript", "candidates", "rules"), { recursive: true });
  git("mv", "packs/python/candidates/rules/bandit-b101.json", "packs/javascript/candidates/rules/bandit-b101.json");
  git("commit", "-q", "-m", "move");
  write(root, "docs/port-readiness.md", "# Port readiness\n");
  git("add", "-A");
  git("commit", "-q", "-m", "docs");

  function run(...args: string[]): { stdout: string; output: string } {
    const output = join(tempRoot(), "github-output");
    const stdout = execFileSync(process.execPath, [join(root, "conformance", "port-readiness", "plan.mjs"), ...args], { cwd: root, encoding: "utf8", env: { ...process.env, GITHUB_OUTPUT: output } });
    return { stdout, output: readFileSync(output, "utf8") };
  }

  it("lists a moved rule at both paths, so both packs are judged", () => {
    assert.deepEqual(git("diff", "-M", "--name-only", "HEAD~2", "HEAD~1").trim().split("\n"), ["packs/javascript/candidates/rules/bandit-b101.json"]);
    assert.deepEqual(changedPaths("HEAD~2", "HEAD~1", root), ["packs/javascript/candidates/rules/bandit-b101.json", "packs/python/candidates/rules/bandit-b101.json"]);
    const { stdout, output } = run("--diff", "HEAD~2", "HEAD~1");
    assert.equal(output, 'legs=[{"pack":"javascript","tool":"eslint-plugin-unicorn"},{"pack":"python","tool":"bandit"}]\ntool-packs=["javascript/eslint-plugin-unicorn","python/bandit"]\n');
    assert.match(stdout, /^python\/bandit \(1 leg\): packs\/python\/candidates\/rules\/bandit-b101\.json changed$/mu);
  });

  it("plans no leg for a change no leg reads, and says so", () => {
    const { stdout, output } = run("--diff", "HEAD~1", "HEAD");
    assert.equal(output, "legs=[]\ntool-packs=[]\n");
    assert.match(stdout, /no leg runs/u);
  });

  it("plans every leg when git cannot diff, with a warning", () => {
    const { stdout, output } = run("--diff", "no-such-commit", "HEAD");
    assert.match(stdout, /^::warning title=Port readiness plan::git diff no-such-commit HEAD failed, so every leg runs: /mu);
    assert.equal(output, 'legs=[{"pack":"javascript","tool":"eslint-plugin-unicorn"},{"pack":"python","tool":"bandit"}]\ntool-packs=["javascript/eslint-plugin-unicorn","python/bandit"]\n');
  });
});
