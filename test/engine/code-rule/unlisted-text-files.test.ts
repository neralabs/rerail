import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, open, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AuditInput, AuditRecorder, AuditResult } from "../../../src/engine/audit/audit.js";
import { CodeRuleBridge } from "../../../src/engine/code-rule/code-rule-bridge.js";
import { bundledCodeRuleDslDirectory } from "../../../src/engine/code-rule/code-rule-compile.js";
import { languageFor, NativeCodeRuleEnforcer } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { type CodeRule, codeRuleSchema } from "../../../src/engine/code-rule/code-rule-schema.js";
import { testCodeRuleCatalog } from "../../support/repository-rules.js";

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

/** A regex rule on every file: it reads text, so the files no language lists are read for it. */
const REGEX_ANYWHERE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:regex-anywhere", revision: 1, mode: "block",
  scope: { include: ["**/*"], exclude: [] },
  clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { regex: { pattern: "NEEDLE", source: "file.content" } } }],
});

/** An AST rule on every file, scoped as the shipped packs scope theirs: it reads no file no language lists. */
const AST_ANYWHERE = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:ast-anywhere", revision: 1, mode: "block",
  scope: { include: ["**/*"], exclude: [] },
  clauses: [{ id: "needle", violation: { code: "NEEDLE", anchor: "witness.span", message: "No NEEDLE" }, forbid: { ast: { language: "typescript", rule: { pattern: "NEEDLE($ARG)" } } } }],
});

/** Text files no language lists, a binary one (a NUL byte), and a TypeScript file. */
const TREE: Readonly<Record<string, string>> = {
  "Justfile": "build:\n\techo NEEDLE\n",
  "LICENSE": "NEEDLE\n",
  "deps/yarn.lock": "NEEDLE\n",
  "assets/logo.png": "\u0089PNG\0NEEDLE\n",
  "web/a.ts": "NEEDLE(1);\n",
};

async function workspace(git: boolean, codeRules: readonly CodeRule[], tree: Readonly<Record<string, string>> = TREE) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-unlisted-")));
  const home = await mkdtemp(join(tmpdir(), "rerail-unlisted-catalog-"));
  roots.push(root, home);
  for (const [path, content] of Object.entries(tree)) {
    await mkdir(dirname(join(root, path)), { recursive: true });
    await writeFile(join(root, path), content);
  }
  if (git) {
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    execFileSync("git", ["add", "."], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
  }
  const capabilitiesPath = join(home, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json")));
  const audits = new Map<string, AuditInput & { result?: AuditResult }>();
  const recorder: AuditRecorder = {
    start(input) { const auditId = `run-${audits.size + 1}`; audits.set(auditId, { ...input }); return auditId; },
    finish(auditId, result) { audits.get(auditId)!.result = result; },
    progress(auditId, result) { audits.get(auditId)!.result = { status: "completed", ...result }; },
    interrupt() {},
  };
  const lines: string[] = [];
  const enforcer = new NativeCodeRuleEnforcer(testCodeRuleCatalog(codeRules, capabilitiesPath), {
    workspaceRoot: root, steerDelivery: { requestSteer: vi.fn(async () => undefined) }, audits: recorder,
    scope: "ai-output", timingLog: line => lines.push(line),
  });
  /** One log-only audit: its status, and the sorted paths and failures it reports (a directory walk follows the file system's entry order). */
  const audit = async (scope: "repository" | "touched-files", paths?: string[]) => {
    const [auditId] = await enforcer.audit("unlisted", { scope, mode: "log", ...(paths === undefined ? {} : { paths }) });
    const result = audits.get(auditId!)?.result;
    return {
      status: result?.status,
      paths: (result?.violations ?? []).map(violation => violation.path).toSorted(),
      failures: (result?.failures ?? []).map(failure => `${failure.path ?? ""} ${failure.code}`).toSorted(),
    };
  };
  return { root, enforcer, audit, lines };
}

async function settledViolationPaths(enforcer: NativeCodeRuleEnforcer): Promise<string[]> {
  await expect.poll(() => enforcer.snapshotStatus().pending, { timeout: 15_000, interval: 10 }).toBe(false);
  return enforcer.snapshotStatus().violations.map(violation => violation.path ?? "").toSorted();
}

/** A sparse file past the 32 MB per-file byte budget that starts with `head`, so writing it stays fast. */
async function oversizeFile(root: string, path: string, head: string): Promise<void> {
  await mkdir(dirname(join(root, path)), { recursive: true });
  const handle = await open(join(root, path), "w");
  try {
    await handle.write(head, 0);
    await handle.write("\n", 33_000_000);
  } finally { await handle.close(); }
}

const WALKS = [
  ["the native repository index (Git checkout)", true],
  ["the host's directory walk", false],
] as const;

describe("files no language lists", () => {
  it.each(WALKS)("are audited as text for a regex rule on every file through %s, a binary one skipped without a failure", async (_name, git) => {
    const { enforcer, audit } = await workspace(git, [REGEX_ANYWHERE]);
    try {
      await enforcer.openWorkspace("unlisted");
      expect(enforcer.acceptsPath("Justfile")).toBe(true);
      await expect(audit("repository")).resolves.toEqual({
        status: "completed", paths: ["Justfile", "LICENSE", "deps/yarn.lock", "web/a.ts"], failures: [],
      });
    } finally { await enforcer.close(); }
  });

  it("are read by a touched-files audit for a regex rule, a binary one skipped without a failure", async () => {
    const { enforcer, audit } = await workspace(true, [REGEX_ANYWHERE]);
    try {
      await enforcer.openWorkspace("unlisted");
      await expect(audit("touched-files", ["Justfile", "assets/logo.png"])).resolves.toEqual({ status: "completed", paths: ["Justfile"], failures: [] });
    } finally { await enforcer.close(); }
  });

  it.each(WALKS)("are never read for an AST rule on every file through %s", async (_name, git) => {
    const { enforcer, audit, lines } = await workspace(git, [AST_ANYWHERE]);
    try {
      await enforcer.openWorkspace("unlisted");
      expect(enforcer.acceptsPath("Justfile")).toBe(false);
      expect(enforcer.acceptsPath("web/a.ts")).toBe(true);
      await expect(audit("repository")).resolves.toEqual({ status: "completed", paths: ["web/a.ts"], failures: [] });
      expect(lines.filter(line => line.startsWith("rerail: repository audit"))).toEqual([expect.stringContaining(": 1 files,")]);
    } finally { await enforcer.close(); }
  });

  it("skip an oversize binary one through the host's directory walk, and keep the budget failure for an oversize text or listed one", async () => {
    const { root, enforcer, audit } = await workspace(false, [REGEX_ANYWHERE]);
    await oversizeFile(root, "assets/weights.bin", "\0NEEDLE");
    await oversizeFile(root, "data/dump.raw", "NEEDLE\n".repeat(2000));
    await oversizeFile(root, "web/big.ts", "\0NEEDLE");
    expect([languageFor("assets/weights.bin"), languageFor("data/dump.raw")]).toEqual([undefined, undefined]);
    try {
      await enforcer.openWorkspace("unlisted");
      await expect(audit("repository")).resolves.toEqual({
        status: "failed", paths: ["Justfile", "LICENSE", "deps/yarn.lock", "web/a.ts"],
        failures: ["data/dump.raw budget-exceeded", "web/big.ts budget-exceeded"],
      });
    } finally { await enforcer.close(); }
  });

  it("reach a regex rule when a session edits one, and a binary one is skipped without a failure", async () => {
    const { root, enforcer } = await workspace(true, [REGEX_ANYWHERE]);
    const bridge = new CodeRuleBridge(root, enforcer);
    try {
      await bridge.openWorkspace("unlisted");
      await bridge.begin("text", "write", { path: "Justfile" });
      await writeFile(join(root, "Justfile"), "build:\n\techo NEEDLE\nNEEDLE\n");
      expect(await bridge.finish("text")).toEqual([expect.objectContaining({ failures: [] })]);
      expect(await settledViolationPaths(enforcer)).toEqual(["Justfile"]);

      await bridge.begin("binary", "write", { path: "assets/logo.png" });
      await writeFile(join(root, "assets", "logo.png"), "\u0089PNG\0NEEDLE again\n");
      expect(await bridge.finish("binary")).toEqual([]);
      await bridge.begin("shell", "bash", { command: "cp assets/logo.png assets/copy.png" });
      await writeFile(join(root, "assets", "copy.png"), "\u0089PNG\0NEEDLE\n");
      expect((await bridge.finish("shell")).flatMap(response => response.failures)).toEqual([]);
      expect((await bridge.checkpoint("final")).failures).toEqual([]);
    } finally { await bridge.close(); }
  });

  it("are not edits an AST rule on every file reads", async () => {
    const { root, enforcer } = await workspace(true, [AST_ANYWHERE]);
    const bridge = new CodeRuleBridge(root, enforcer);
    try {
      await bridge.openWorkspace("unlisted");
      await bridge.begin("text", "write", { path: "Justfile" });
      await writeFile(join(root, "Justfile"), "build:\n\techo NEEDLE\nNEEDLE\n");
      expect(await bridge.finish("text")).toEqual([]);
    } finally { await bridge.close(); }
  });

  it("past the session's byte limit are reported as not checked at one boundary, not at every one", async () => {
    const { root, enforcer } = await workspace(true, [REGEX_ANYWHERE]);
    await mkdir(join(root, "data"));
    await writeFile(join(root, "data", "dump.raw"), "NEEDLE\n".repeat(200_000));
    const bridge = new CodeRuleBridge(root, enforcer);
    const unchecked = { clauseId: "source-discovery", code: "source-incomplete", message: "data/dump.raw: Source exceeds 1000000 byte policy limit", stage: "scan" };
    try {
      await bridge.openWorkspace("oversize");
      expect((await bridge.checkpoint("turn")).failures).toEqual([unchecked]);
      expect((await bridge.checkpoint("turn")).failures).toEqual([]);
      expect((await bridge.checkpoint("final")).failures).toEqual([]);

      await writeFile(join(root, "data", "dump.raw"), "NEEDLE\n".repeat(200_001));
      expect((await bridge.checkpoint("turn")).failures).toEqual([unchecked]);
      expect((await bridge.checkpoint("final")).failures).toEqual([]);
    } finally { await bridge.close(); }
  });
});

/**
 * 22 paths in path order, the byte order Git sorts paths in: an upper-case name before a lower-case one, and folder `a`
 * after its siblings `a-b.ts` and `a.ts` but before `a0.ts`, as if its name ended in "/".
 */
const IN_PATH_ORDER = [
  "B/x.ts", "a-b.ts", "a.ts", "a/b-c.ts", "a/b.ts", "a/b/c.ts", "a/b0.ts", "a/c.ts", "a0.ts", "b.ts",
  "c/1.ts", "c/10.ts", "c/11.ts", "c/12.ts", "c/2.ts", "c/3.ts", "c/4.ts", "c/5.ts", "c/6.ts", "c/7.ts", "c/8.ts", "c/9.ts",
];

describe("a repository audit's failure list", () => {
  it.each(WALKS)("names the first 20 failures in path order through %s, whatever order a folder lists its entries in", async (_name, git) => {
    const { root, enforcer, audit } = await workspace(git, [REGEX_ANYWHERE]);
    // A TypeScript file holding a NUL byte cannot be read as source: one failure each. They are made in an order that
    // is neither path order nor its reverse (7 and 22 share no factor), so a filesystem that lists a folder in the
    // order its entries were made, or in the reverse, lists them in neither.
    for (const index of IN_PATH_ORDER.keys()) {
      const path = IN_PATH_ORDER[(index * 7) % IN_PATH_ORDER.length]!;
      await mkdir(dirname(join(root, path)), { recursive: true });
      await writeFile(join(root, path), "NEEDLE(1);\0\n");
    }
    try {
      await enforcer.openWorkspace("unlisted");
      await expect(audit("repository")).resolves.toEqual({
        status: "completed", paths: ["Justfile", "LICENSE", "deps/yarn.lock", "web/a.ts"],
        failures: IN_PATH_ORDER.slice(0, 20).map(path => `${path} source-incomplete`),
      });
    } finally { await enforcer.close(); }
  });
});

/** Files named like ones holding secrets: unlisted ones, and a listed one (`.yaml`) a rule still reads. */
const SECRETS: Readonly<Record<string, string>> = {
  ".env": "TOKEN=NEEDLE\n",
  ".npmrc": "//registry.npmjs.org/:_authToken=NEEDLE\n",
  "secrets/id_rsa": "NEEDLE\n",
  "keys/server.pem": "NEEDLE\n",
  "infra/terraform.tfstate": "{\"NEEDLE\": 1}\n",
  "config/secrets.yaml": "NEEDLE: 1\n",
};
const UNLISTED_SECRETS = [".env", ".npmrc", "secrets/id_rsa", "keys/server.pem", "infra/terraform.tfstate"];

describe("unlisted files named like ones holding secrets", () => {
  it.each(WALKS)("are never read through %s, without a failure, while a listed one is", async (_name, git) => {
    const { enforcer, audit } = await workspace(git, [REGEX_ANYWHERE], { ...TREE, ...SECRETS });
    try {
      await enforcer.openWorkspace("unlisted");
      expect(UNLISTED_SECRETS.filter(path => enforcer.acceptsPath(path))).toEqual([]);
      expect(enforcer.acceptsPath("config/secrets.yaml")).toBe(true);
      await expect(audit("repository")).resolves.toEqual({
        status: "completed", paths: ["Justfile", "LICENSE", "config/secrets.yaml", "deps/yarn.lock", "web/a.ts"], failures: [],
      });
    } finally { await enforcer.close(); }
  });

  it("are never read by a touched-files audit, without a failure, while a listed one is", async () => {
    const { enforcer, audit } = await workspace(true, [REGEX_ANYWHERE], { ...TREE, ...SECRETS });
    try {
      await enforcer.openWorkspace("unlisted");
      await expect(audit("touched-files", Object.keys(SECRETS))).resolves.toEqual({ status: "completed", paths: ["config/secrets.yaml"], failures: [] });
    } finally { await enforcer.close(); }
  });

  it("are never read when a session edits them, while a listed one is", async () => {
    const { root, enforcer } = await workspace(true, [REGEX_ANYWHERE], { ...TREE, ...SECRETS });
    const bridge = new CodeRuleBridge(root, enforcer);
    try {
      await bridge.openWorkspace("secrets");
      for (const path of UNLISTED_SECRETS) {
        await bridge.begin(path, "write", { path });
        await writeFile(join(root, path), "NEEDLE again\n");
        expect(await bridge.finish(path)).toEqual([]);
      }
      await bridge.begin("shell", "bash", { command: "cp keys/server.pem keys/client.pem" });
      await writeFile(join(root, "keys", "client.pem"), "NEEDLE\n");
      expect((await bridge.finish("shell")).flatMap(response => response.failures)).toEqual([]);
      expect((await bridge.checkpoint("final")).failures).toEqual([]);
      expect(await settledViolationPaths(enforcer)).toEqual([]);

      await bridge.begin("listed", "write", { path: "config/secrets.yaml" });
      await writeFile(join(root, "config", "secrets.yaml"), "NEEDLE: 2\n");
      expect(await bridge.finish("listed")).toEqual([expect.objectContaining({ failures: [] })]);
      expect(await settledViolationPaths(enforcer)).toEqual(["config/secrets.yaml"]);
    } finally { await bridge.close(); }
  });
});

/** A `file` query: every `pack.json` but one at the top of a pack folder should not exist. */
const MISPLACED_PACK_JSON = codeRuleSchema.parse({
  format: "rerail.policy/v1", id: "test:pack-json-placement", revision: 1, mode: "block",
  scope: { include: ["**/pack.json"], exclude: ["packs/*/pack.json"] },
  clauses: [{ id: "misplaced", violation: { code: "MISPLACED", anchor: "witness.span", message: "Move it" }, forbid: { file: {} } }],
});

describe("a violation about a whole file in a session", () => {
  it("is the edit's that creates the file, not one that changes a file already there", async () => {
    const { root, enforcer } = await workspace(true, [MISPLACED_PACK_JSON]);
    await mkdir(join(root, "old"));
    await writeFile(join(root, "old", "pack.json"), "{}\n");
    const bridge = new CodeRuleBridge(root, enforcer);
    try {
      await bridge.openWorkspace("whole-file");
      await bridge.begin("change", "write", { path: "old/pack.json" });
      await writeFile(join(root, "old", "pack.json"), "{ \"a\": 1 }\n");
      await bridge.finish("change");
      expect(await settledViolationPaths(enforcer)).toEqual([]);

      await bridge.begin("create", "write", { path: "packs/a/rules/pack.json" });
      await mkdir(join(root, "packs", "a", "rules"), { recursive: true });
      await writeFile(join(root, "packs", "a", "rules", "pack.json"), "{}\n");
      await bridge.finish("create");
      expect(await settledViolationPaths(enforcer)).toEqual(["packs/a/rules/pack.json"]);
      expect(enforcer.snapshotStatus().violations[0]).not.toHaveProperty("span");
    } finally { await bridge.close(); }
  });
});
