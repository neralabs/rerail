/**
 * The not-gated and pending checks (conformance/languages/checks/ungated.ts): the frozen list of shipped rules no gate
 * judges, the pending tool-packs, and the guard that both lists only shrink against the base commit. Repositories are
 * hand-made; git is a stand-in runner answering for the base commit.
 */
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import { PENDING, UNGATED_RULES, ungatedChecks } from "../../conformance/languages/checks/ungated.js";
import type { CommandResult, CommandRunner } from "../../conformance/process.js";

const BASE = "0123456789abcdef0123456789abcdef01234567";

let root: string;

beforeEach(() => {
  root = mkdtempSync(join(tmpdir(), "rerail-ungated-"));
});

afterEach(() => rmSync(root, { recursive: true, force: true }));

const write = (path: string, value: unknown): void => {
  mkdirSync(dirname(join(root, path)), { recursive: true });
  writeFileSync(join(root, path), typeof value === "string" ? value : JSON.stringify(value));
};

/** A pack shipping `rules` (id → upstream tool), with `tools` in its conformance.json and a gates.yaml for each of `gated`. */
function pack(name: string, tools: string[], rules: Record<string, string>, gated: string[] = []): void {
  write(`packs/${name}/pack.json`, { rules: Object.keys(rules).map((id) => ({ id })) });
  write(`packs/${name}/conformance.json`, { tools: tools.map((tool) => ({ name: tool })), rules: Object.entries(rules).map(([id, tool]) => ({ id, upstream: { tool } })) });
  for (const tool of gated) write(`packs/${name}/upstream/${tool}/gates.yaml`, "adapter: x\n");
}

function lists(frozen: Record<string, string[]>, pending: Record<string, string> = {}): void {
  write(UNGATED_RULES, frozen);
  write(PENDING, pending);
}

const ok = (stdout = ""): CommandResult => ({ exitCode: 0, stdout, stderr: "" });
const failed = (stderr: string): CommandResult => ({ exitCode: 1, stdout: "", stderr });

/** git at the base commit: `files` maps a path to its text there (absent: not in the base tree). */
function baseGit(files: Record<string, string>, options: { present?: boolean; fetch?: CommandResult } = {}): CommandRunner & { calls: string[][] } {
  const calls: string[][] = [];
  return {
    calls,
    async run(argv) {
      calls.push([...argv]);
      const [, command] = argv;
      if (command === "cat-file") return options.present === false ? failed("missing") : ok();
      if (command === "fetch") return options.fetch ?? ok();
      if (command === "ls-tree") {
        const path = argv.at(-1)!;
        return ok(path in files ? `${path}\n` : "");
      }
      if (command === "show") return ok(files[argv.at(-1)!.slice(BASE.length + 1)] ?? "");
      throw new Error(`unexpected git ${argv.join(" ")}`);
    },
  };
}

const NO_GIT: CommandRunner = { run: async (argv) => { throw new Error(`git ran without --base: ${argv.join(" ")}`); } };

const byId = async (options: Parameters<typeof ungatedChecks>[1] = { runner: NO_GIT }) => {
  const result = await ungatedChecks(root, options);
  return { result, check: (id: string) => result.checks.find((entry) => entry.id === id) };
};

describe("not-gated", () => {
  it("passes when every shipped rule of a tool with no gates.yaml is on the frozen list", async () => {
    pack("plain", ["lintx"], { a: "lintx", b: "lintx" });
    lists({ plain: ["a", "b"] });
    const { result, check } = await byId();
    expect(check("not-gated:plain")).toMatchObject({ ok: true });
    expect(result.notGated).toEqual([{ pack: "plain", rules: 2 }]);
  });

  it("fails a new shipped rule no gate judges and the frozen list does not name", async () => {
    pack("plain", ["lintx"], { a: "lintx", b: "lintx" });
    lists({ plain: ["a"] });
    const { check } = await byId();
    expect(check("not-gated:plain")).toMatchObject({ ok: false, detail: expect.stringMatching(/^1 new shipped rule\(s\) \(b of lintx\)/u) });
  });

  it("holds a gated pack's rule of another, ungated tool to the frozen list", async () => {
    pack("mixed", ["gatedlint", "otherlint"], { judged: "gatedlint", loose: "otherlint" }, ["gatedlint"]);
    lists({});
    const { check } = await byId();
    expect(check("not-gated:mixed")).toMatchObject({ ok: false, detail: expect.stringContaining("loose of otherlint") });
  });

  it("is not reported for a pack whose every shipped rule is gated", async () => {
    pack("gated", ["gatedlint"], { judged: "gatedlint" }, ["gatedlint"]);
    lists({});
    const { result } = await byId();
    expect(result.checks).toEqual([]);
    expect(result.notGated).toEqual([]);
  });
});

describe("frozen-list", () => {
  it("fails a listed rule that no longer ships, or that is now gated", async () => {
    pack("plain", ["lintx"], { a: "lintx" });
    pack("gated", ["gatedlint"], { judged: "gatedlint" }, ["gatedlint"]);
    lists({ plain: ["a", "gone"], gated: ["judged"] });
    const { check } = await byId();
    expect(check("frozen-list:plain")).toMatchObject({ ok: false, detail: expect.stringContaining("(gone)") });
    expect(check("frozen-list:gated")).toMatchObject({ ok: false, detail: expect.stringContaining("(judged)") });
  });

  it("passes when every listed rule still ships ungated", async () => {
    pack("plain", ["lintx"], { a: "lintx" });
    lists({ plain: ["a"] });
    const { check } = await byId();
    expect(check("frozen-list:plain")).toEqual({ id: "frozen-list:plain", ok: true, detail: "1 listed rule(s), all still shipped and not gated" });
  });
});

describe("pending", () => {
  it("takes a pending tool's shipped rules off the frozen list, and counts them", async () => {
    pack("plain", ["lintx"], { a: "lintx", b: "lintx" });
    lists({}, { "plain/lintx": "validated by its own suite" });
    const { result, check } = await byId();
    expect(check("pending:plain/lintx")).toEqual({ id: "pending:plain/lintx", ok: true, detail: "validated by its own suite" });
    expect(result.pending).toEqual([{ toolPack: "plain/lintx", reason: "validated by its own suite", rules: 2 }]);
    expect(result.notGated).toEqual([]);
  });

  it("fails an entry for a tool that has a gates.yaml now", async () => {
    pack("gated", ["gatedlint"], { judged: "gatedlint" }, ["gatedlint"]);
    lists({}, { "gated/gatedlint": "old reason" });
    const { check } = await byId();
    expect(check("pending:gated/gatedlint")).toMatchObject({ ok: false, detail: "it has a gates.yaml now: remove it from pending.json" });
  });

  it("fails an entry naming no tool of the pack", async () => {
    pack("plain", ["lintx"], { a: "lintx" });
    lists({ plain: ["a"] }, { "plain/nolint": "typo" });
    const { check } = await byId();
    expect(check("pending:plain/nolint")).toMatchObject({ ok: false, detail: "no such tool in the pack's conformance.json: remove it from pending.json" });
  });
});

describe("shrinks", () => {
  beforeEach(() => pack("plain", ["lintx"], { a: "lintx", b: "lintx" }));

  it("passes when both lists only shrink against the base", async () => {
    lists({ plain: ["a", "b"] });
    const runner = baseGit({ [UNGATED_RULES]: JSON.stringify({ plain: ["a", "b", "c"] }), [PENDING]: JSON.stringify({ "plain/old": "x" }) });
    const { check } = await byId({ base: BASE, runner });
    expect(check(`shrinks:${UNGATED_RULES}`)).toMatchObject({ ok: true });
    expect(check(`shrinks:${PENDING}`)).toMatchObject({ ok: true });
  });

  it("fails a frozen-list or pending entry the base lacks", async () => {
    lists({ plain: ["a", "b"] }, { "plain/lintx": "new" });
    const runner = baseGit({ [UNGATED_RULES]: JSON.stringify({ plain: ["a"] }), [PENDING]: "{}" });
    const { check } = await byId({ base: BASE, runner });
    expect(check(`shrinks:${UNGATED_RULES}`)).toMatchObject({ ok: false, detail: expect.stringContaining("(plain/b)") });
    expect(check(`shrinks:${PENDING}`)).toMatchObject({ ok: false, detail: expect.stringContaining("(plain/lintx)") });
  });

  it("is skipped when the base commit has no list", async () => {
    lists({ plain: ["a", "b"] });
    const { check } = await byId({ base: BASE, runner: baseGit({}) });
    expect(check(`shrinks:${UNGATED_RULES}`)).toMatchObject({ ok: true, detail: expect.stringContaining("the guard is skipped") });
  });

  it("fails closed on a base list that is not a JSON object, or not JSON", async () => {
    lists({ plain: ["a", "b"] });
    const { check } = await byId({ base: BASE, runner: baseGit({ [UNGATED_RULES]: "[]", [PENDING]: "not json" }) });
    expect(check(`shrinks:${UNGATED_RULES}`)).toMatchObject({ ok: false, detail: expect.stringContaining("is not a JSON object") });
    expect(check(`shrinks:${PENDING}`)).toMatchObject({ ok: false, detail: expect.stringContaining("is not JSON") });
  });

  it("fetches a base commit that is not present, and fails closed when the fetch fails", async () => {
    lists({ plain: ["a", "b"] });
    const runner = baseGit({}, { present: false, fetch: failed("fatal: no such commit") });
    const { check } = await byId({ base: BASE, runner });
    expect(runner.calls).toContainEqual(["git", "fetch", "--depth=1", "origin", BASE]);
    expect(check(`shrinks:${UNGATED_RULES}`)).toMatchObject({ ok: false, detail: expect.stringContaining("git fetch failed: fatal: no such commit") });
  });

  it("fails closed on a --base that is not a commit sha, without running git", async () => {
    lists({ plain: ["a", "b"] });
    const { check } = await byId({ base: "main; rm -rf /", runner: NO_GIT });
    expect(check(`shrinks:${UNGATED_RULES}`)).toMatchObject({ ok: false, detail: expect.stringContaining("is not a commit sha") });
  });
});
