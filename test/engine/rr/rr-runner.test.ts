import * as childProcess from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });

import { processState } from "../../../src/engine/process-liveness.js";
import { describeRerailPolicies, describeRerailPoliciesSync, runRerailPolicies } from "../../../src/engine/rr/rr-runner.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/** A rule over the core library alone: each call of `shout()`. */
const SHOUT = `.policy "rr-runner-shout" revision 3
.description "Calls of shout()."
.language python
.mode log
.scope include "**/*.py"
.scope exclude "vendor/**"

.violation Shout(at: node) "shout() is called here"
    suppress ruff "T201"
Shout(c) :- kind(c, "call"), field(c, "function", f), text(f, "shout").

.test one
    .fixture "a.py" \`shout(1)\`
    .expect Shout at "a.py":1
.end
`;

const MULTI_BYTE = 'greeting = "héllo wörld 漢字"; shout("ü")\n';
const MULTI_LINE = "def f():\n    return shout(\n        1,\n    )\n";

/**
 * This file's temporary folder, which `os.tmpdir()` reads (TMPDIR, and TEMP and TMP on Windows), and so the runner's
 * children inherit: the runner's temporary directories, and the children's stores, are made there and nowhere else.
 */
let temporary: string;
const TEMPORARY_VARIABLES = ["TMPDIR", "TEMP", "TMP"] as const;
const previous = new Map<string, string | undefined>();
const roots: string[] = [];

beforeAll(() => {
  temporary = realpathSync(mkdtempSync(join(tmpdir(), "rr-runner-test-")));
  for (const name of TEMPORARY_VARIABLES) {
    previous.set(name, process.env[name]);
    process.env[name] = temporary;
  }
});

afterEach(async () => {
  delete process.env.RERAIL_RR_MEMORY_MB;
  delete process.env.RERAIL_RR_DEADLINE_MS;
  vi.clearAllMocks();
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

afterAll(async () => {
  for (const [name, value] of previous) {
    if (value === undefined) delete process.env[name];
    else process.env[name] = value;
  }
  await removeFolder(temporary);
});

/** A checkout holding `files`, and the shout rule beside it. */
function checkout(files: Record<string, string>): { root: string; rule: string } {
  const root = realpathSync(mkdtempSync(join(temporary, "checkout-")));
  roots.push(root);
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), text);
  }
  const rules = mkdtempSync(join(temporary, "rules-"));
  roots.push(rules);
  const rule = join(rules, "shout.rerail");
  writeFileSync(rule, SHOUT);
  return { root, rule };
}

/** The runner's temporary directories left in TMPDIR. */
function leftDirectories(): string[] {
  return readdirSync(temporary).filter(name => name.startsWith("rerail-rr-"));
}

/** Whether every child the runner started is gone. */
function childrenGone(): boolean {
  const started = vi.mocked(childProcess.spawn).mock.results.map(result => result.value as childProcess.ChildProcess);
  return started.length > 0 && started.every(child => child.pid !== undefined && processState(child.pid) === "gone");
}

/** The byte offset of `needle` in `text`, as UTF-8. */
function byteOffset(text: string, needle: string): number {
  return Buffer.byteLength(text.slice(0, text.indexOf(needle)));
}

describe("describeRerailPolicies", () => {
  it("reads what each rule file states, and says why a file that does not read is refused", async () => {
    const { rule } = checkout({});
    const bad = join(rule, "..", "bad.rerail");
    writeFileSync(bad, '.policy "bad" revision 1\nBad(c) :- kind(c, "call"\n');
    const rows = await describeRerailPolicies([rule, bad]);
    expect(rows[0]).toEqual({
      path: rule,
      ok: true,
      id: "rr-runner-shout",
      revision: 3,
      description: "Calls of shout().",
      mode: "log",
      language: "python",
      scope: { include: ["**/*.py"], exclude: ["vendor/**"], toolCalls: false },
      codes: ["Shout"],
      tests: 1,
      suppresses: true,
      fixes: false,
      // The rule file, then what it reads: here `core.rerail`, which every rule includes.
      files: [
        { path: realpathSync(rule), sha256: createHash("sha256").update(SHOUT).digest("hex") },
        { path: expect.stringMatching(/[/\\]docs[/\\]rule-syntax[/\\]lib[/\\]core\.rerail$/u), sha256: expect.stringMatching(/^[0-9a-f]{64}$/u) },
      ],
    });
    expect(rows[1]).toEqual({ path: bad, ok: false, error: expect.stringContaining(bad) });
  });

  it("answers the same without a promise", async () => {
    const { rule } = checkout({});
    expect(describeRerailPoliciesSync([rule])).toEqual(await describeRerailPolicies([rule]));
  });
});

describe("runRerailPolicies", () => {
  it("gives each finding the byte, line and column span of its node, multi-byte text and multi-line nodes included", async () => {
    const { root, rule } = checkout({ "app/text.py": MULTI_BYTE, "app/call.py": MULTI_LINE });
    const outcome = await runRerailPolicies({ root, policies: [rule] });
    if (outcome.status !== "completed") throw new Error(`the run did not complete: ${JSON.stringify(outcome)}`);
    const { findings, failures } = outcome;
    expect(failures).toEqual([]);
    const byPath = Object.fromEntries(findings.map(finding => [finding.path, finding]));
    const shoutAt = byteOffset(MULTI_BYTE, "shout(");
    expect(byPath["app/text.py"]).toMatchObject({
      policy: "rr-runner-shout",
      code: "Shout",
      line: 1,
      message: "shout() is called here",
      span: { startByte: shoutAt, endByte: shoutAt + Buffer.byteLength('shout("ü")'), startLine: 0, startColumn: shoutAt, endLine: 0, endColumn: shoutAt + Buffer.byteLength('shout("ü")') },
    });
    const callAt = byteOffset(MULTI_LINE, "shout(");
    expect(byPath["app/call.py"]).toMatchObject({
      line: 2,
      span: { startByte: callAt, endByte: MULTI_LINE.lastIndexOf(")") + 1, startLine: 1, startColumn: "    return ".length, endLine: 3, endColumn: 5 },
    });
    expect(leftDirectories()).toEqual([]);
  });

  it("runs two checkouts at once, each to its own findings", async () => {
    const first = checkout({ "one.py": "shout(1)\n" });
    const second = checkout({ "two.py": "x = 1\nshout(2)\n" });
    const [one, two] = await Promise.all([
      runRerailPolicies({ root: first.root, policies: [first.rule] }),
      runRerailPolicies({ root: second.root, policies: [second.rule] }),
    ]);
    expect(one.status === "completed" && one.findings.map(finding => [finding.path, finding.line])).toEqual([["one.py", 1]]);
    expect(two.status === "completed" && two.findings.map(finding => [finding.path, finding.line])).toEqual([["two.py", 2]]);
  });

  it("kills a run past its deadline, RERAIL_RR_DEADLINE_MS winning over the caller's", async () => {
    process.env.RERAIL_RR_DEADLINE_MS = "1";
    const { root, rule } = checkout({ "a.py": "shout(1)\n" });
    const outcome = await runRerailPolicies({ root, policies: [rule], deadlineMs: 600_000 });
    expect(outcome).toMatchObject({ status: "killed", reason: "deadline" });
    expect(childrenGone()).toBe(true);
    expect(leftDirectories()).toEqual([]);
  });

  it("kills a run past its memory budget", async () => {
    process.env.RERAIL_RR_MEMORY_MB = "1";
    const { root, rule } = checkout({ "a.py": "shout(1)\n" });
    const outcome = await runRerailPolicies({ root, policies: [rule] });
    if (outcome.status !== "killed") throw new Error(`the run was not killed: ${JSON.stringify(outcome)}`);
    expect(outcome.reason).toBe("memory");
    expect(outcome.peakRssBytes).toBeGreaterThan(1024 ** 2);
    expect(childrenGone()).toBe(true);
    expect(leftDirectories()).toEqual([]);
  });

  it("kills a run its caller aborts", async () => {
    const { root, rule } = checkout({ "a.py": "shout(1)\n" });
    const controller = new AbortController();
    const running = runRerailPolicies({ root, policies: [rule], signal: controller.signal });
    controller.abort();
    expect(await running).toMatchObject({ status: "killed", reason: "aborted" });
    expect(childrenGone()).toBe(true);
    expect(leftDirectories()).toEqual([]);
  });
});
