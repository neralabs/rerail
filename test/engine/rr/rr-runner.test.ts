import * as childProcess from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, realpathSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { afterAll, afterEach, beforeAll, describe, expect, it, vi } from "vitest";

vi.mock("node:child_process", { spy: true });

import { PACKAGE_ROOT } from "../../../src/engine/package-root.js";
import { processState } from "../../../src/engine/process-liveness.js";
import { describeRerailPolicies, describeRerailPoliciesSync, runRerailPolicies } from "../../../src/engine/rr/rr-runner.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/** A rule file's own library file, in a folder beside it, as a pack keeps one under `rules/lib/`. */
const SHOUT_CALLS = `.decl shout_call(c: node)
shout_call(c) :- kind(c, "call"), field(c, "function", f), text(f, "shout").
`;

/** A rule that reads its own library file: each call of `shout()`. */
const SHOUT = `.policy "rr-runner-shout" revision 3
.description "Calls of shout()."
.language python
.mode log
.scope include "**/*.py"
.scope exclude "vendor/**"

#include "lib/shout-calls.rerail"

.violation Shout(at: node) "shout() is called here"
    suppress ruff "T201"
Shout(c) :- shout_call(c).

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
  delete process.env.RERAIL_RR_STORE;
  delete process.env.RERAIL_RR_THREADS;
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

/** A checkout holding `files`, and the shout rule, with its library file, in a folder of its own. */
function checkout(files: Record<string, string>): { root: string; rule: string } {
  const root = realpathSync(mkdtempSync(join(temporary, "checkout-")));
  roots.push(root);
  for (const [path, text] of Object.entries(files)) {
    mkdirSync(join(root, path, ".."), { recursive: true });
    writeFileSync(join(root, path), text);
  }
  const rules = realpathSync(mkdtempSync(join(temporary, "rules-")));
  roots.push(rules);
  mkdirSync(join(rules, "lib"));
  writeFileSync(join(rules, "lib", "shout-calls.rerail"), SHOUT_CALLS);
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

function sha256(text: string): string {
  return createHash("sha256").update(text).digest("hex");
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
      scope: { include: ["**/*.py"], exclude: ["vendor/**"], text: [], toolCalls: false },
      codes: ["Shout"],
      tests: 1,
      suppresses: true,
      fixes: false,
      // The rule file first, then what it reads, as the engine read it: `core.rerail`, then its own library file.
      files: [
        { path: rule, sha256: sha256(SHOUT) },
        { path: expect.stringMatching(/[/\\]docs[/\\]rule-syntax[/\\]lib[/\\]core\.rerail$/u), sha256: expect.stringMatching(/^[0-9a-f]{64}$/u) },
        { path: join(rule, "..", "lib", "shout-calls.rerail"), sha256: sha256(SHOUT_CALLS) },
      ],
    });
    expect(rows[1]).toEqual({ path: bad, ok: false, error: expect.stringContaining(bad) });
  });

  it("answers the same without a promise", async () => {
    const { rule } = checkout({});
    expect(describeRerailPoliciesSync([rule])).toEqual(await describeRerailPolicies([rule]));
  });

  it("reads the library `RERAIL_RR_LIB` names, as the engine's own tools do", async () => {
    const { rule } = checkout({});
    const library = join(temporary, "library-copy");
    cpSync(join(PACKAGE_ROOT, "docs", "rule-syntax", "lib"), library, { recursive: true });
    vi.stubEnv("RERAIL_RR_LIB", library);
    try {
      const [row] = await describeRerailPolicies([rule]);
      expect(row).toMatchObject({ ok: true, files: [{ path: rule }, { path: join(library, "core.rerail") }, { path: join(rule, "..", "lib", "shout-calls.rerail") }] });
    } finally {
      vi.unstubAllEnvs();
    }
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

  it("keeps no store unless RERAIL_RR_STORE=checkout asks for the checkout's, and refuses any other value", async () => {
    const { root, rule } = checkout({ "a.py": "shout(1)\n" });
    const stores = (): string[] => readdirSync(temporary).filter(name => name === "rerail-stores");
    expect(await runRerailPolicies({ root, policies: [rule] })).toMatchObject({ status: "completed" });
    expect(stores()).toEqual([]);
    process.env.RERAIL_RR_STORE = "checkout";
    expect(await runRerailPolicies({ root, policies: [rule] })).toMatchObject({ status: "completed" });
    expect(stores()).toEqual(["rerail-stores"]);
    process.env.RERAIL_RR_STORE = "on";
    expect(await runRerailPolicies({ root, policies: [rule] })).toEqual({ status: "failed", message: 'RERAIL_RR_STORE is "on": it takes "off" or "checkout"' });
  });

  it("refuses a deadline override past the longest timer Node keeps, which would end every run at once", async () => {
    process.env.RERAIL_RR_DEADLINE_MS = "9999999999";
    const { root, rule } = checkout({ "a.py": "shout(1)\n" });
    expect(await runRerailPolicies({ root, policies: [rule] })).toEqual({
      status: "failed",
      message: 'RERAIL_RR_DEADLINE_MS is "9999999999": it takes a whole number from 1 to 2147483647',
    });
  });

  it("completes a run whose child has 2 threads, as on a 4-core runner or a small laptop, with no store", async () => {
    process.env.RERAIL_RR_THREADS = "2";
    const { root, rule } = checkout({ "app/one.py": "shout(1)\n", "app/two.py": "def f():\n    return shout(2)\n" });
    const outcome = await runRerailPolicies({ root, policies: [rule], deadlineMs: 60_000 });
    if (outcome.status !== "completed") throw new Error(`the run did not complete: ${JSON.stringify(outcome)}`);
    expect(outcome.findings.map(finding => [finding.path, finding.line]).sort()).toEqual([["app/one.py", 1], ["app/two.py", 2]]);
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

describe("the runner's child", () => {
  // A FIFO as the request file holds the child's main thread in its read, as a long synchronous `describeRr` holds it:
  // only the child's watch thread can see its parent go. POSIX only: Windows has no FIFO in its file system.
  it.skipIf(process.platform === "win32")("ends itself, and removes its directory, once its parent is killed while its main thread is held", async () => {
    const directory = realpathSync(mkdtempSync(join(temporary, "held-")));
    const request = join(directory, "request.json");
    childProcess.execFileSync("mkfifo", [request]);
    const tsx = pathToFileURL(createRequire(import.meta.url).resolve("tsx")).href;
    const args = ["--import", tsx, join(PACKAGE_ROOT, "src", "engine", "rr", "rr-child.ts"), request, join(directory, "result.json")];
    // A parent that starts the child with no IPC channel, as the synchronous describe does, and waits.
    const starter = `const c = require("node:child_process").spawn(process.execPath, ${JSON.stringify(args)}, { stdio: "ignore" }); console.log(c.pid); setInterval(() => {}, 1000);`;
    const parent = childProcess.spawn(process.execPath, ["-e", starter], { stdio: ["ignore", "pipe", "inherit"] });
    const [line] = (await once(parent.stdout!, "data")) as [Buffer];
    const child = Number(line.toString().trim());
    parent.kill("SIGKILL");
    await once(parent, "exit");
    const until = Date.now() + 15_000;
    while ((processState(child) !== "gone" || existsSync(directory)) && Date.now() < until) await new Promise(resolve => setTimeout(resolve, 100));
    expect(processState(child)).toBe("gone");
    expect(existsSync(directory)).toBe(false);
  });
});
