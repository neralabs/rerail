import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import type { AuditInput, AuditRecorder, AuditResult } from "../../../../src/engine/audit/audit.js";
import { bundledCodeRuleDslDirectory } from "../../../../src/engine/code-rule/code-rule-compile.js";
import { NativeCodeRuleEnforcer } from "../../../../src/engine/code-rule/code-rule-enforcer.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import { repositoryPath } from "../../../support/oracles.js";
import { testCodeRuleCatalog } from "../../../support/repository-rules.js";

// The Node pack's ports of eslint-plugin-n rules that read CommonJS's wrapper bindings (require, module, exports,
// __dirname, __filename) pin no sourceType, so each file keeps its own module system (packs/node/README.md), as the
// plugin's flat/mixed-esm-and-cjs config gives a CommonJS file flat/recommended-script and an ES module
// flat/recommended-module, which turns those names off.

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

async function directory(prefix: string): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

/**
 * The pack's documents that pinned sourceType commonjs before each file kept its own module system. All are candidates
 * until eslint-plugin-n is gated (NER-1898).
 */
const CODE_RULES = await Promise.all(
  [
    "n-exports-style", "n-global-require", "n-no-exports-assign", "n-no-path-concat", "n-prefer-global-buffer", "n-prefer-global-console",
    "n-prefer-global-process", "n-prefer-global-text-decoder", "n-prefer-global-text-encoder", "n-prefer-global-url-search-params",
    "n-prefer-global-url", "n-prefer-promises-dns", "n-prefer-promises-fs", "n-no-deprecated-api",
  ].map(async id => codeRuleSchema.parse(JSON.parse(await readFile(repositoryPath("packs", "node", "candidates", "rules", `${id}.json`), "utf8")))),
);

/** CommonJS code: each line is one rule's finding there. */
const COMMONJS_CODE = [
  "const { Buffer } = require(\"buffer\");",
  "const fs = require(\"fs\");",
  "fs.readFile(\"a\", () => {});",
  "function load() { return require(\"x\"); }",
  "exports.a = new Buffer(1);",
  "exports = {};",
  "module.exports = __dirname + \"/a\";",
  "",
].join("\n");

/**
 * The same code three times: under a package whose `type` is module (an ES module), under one whose `type` is
 * commonjs, and as a `.cjs` file inside the module package. And an ES module's own path, `import.meta.dirname`.
 */
const FILES = {
  "esm/package.json": "{ \"type\": \"module\" }\n",
  "esm/index.js": COMMONJS_CODE,
  "esm/tool.cjs": COMMONJS_CODE,
  "esm/meta.js": "export const path = import.meta.dirname + \"/a\";\n",
  "cjs/package.json": "{ \"type\": \"commonjs\" }\n",
  "cjs/index.js": COMMONJS_CODE,
};

async function repository(git: boolean): Promise<string> {
  const root = await directory("rerail-node-module-system-");
  for (const [path, text] of Object.entries(FILES)) {
    await mkdir(dirname(join(root, path)), { recursive: true });
    await writeFile(join(root, path), text);
  }
  if (git) {
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    execFileSync("git", ["add", "--force", "."], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
  }
  return root;
}

/** One log-only repository audit of `workspaceRoot`: each file's violations as `rule/clause:line`, and the failures. */
async function audit(workspaceRoot: string) {
  const home = await directory("rerail-node-module-system-catalog-");
  const capabilitiesPath = join(home, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json")));
  const audits = new Map<string, AuditInput & { result?: AuditResult }>();
  const recorder: AuditRecorder = {
    start(input) { const auditId = `run-${audits.size + 1}`; audits.set(auditId, { ...input }); return auditId; },
    finish(auditId, result) { audits.get(auditId)!.result = result; },
    progress(auditId, result) { audits.get(auditId)!.result = { status: "completed", ...result }; },
    interrupt() {},
  };
  const enforcer = new NativeCodeRuleEnforcer(testCodeRuleCatalog(CODE_RULES, capabilitiesPath), {
    workspaceRoot, steerDelivery: { requestSteer: async () => undefined }, audits: recorder,
  });
  try {
    await enforcer.openWorkspace("node");
    const [auditId] = await enforcer.audit("node", { scope: "repository", mode: "log" });
    const result = audits.get(auditId!)?.result;
    const reported: Record<string, string[]> = {};
    for (const violation of result?.violations ?? []) {
      (reported[violation.path ?? ""] ??= []).push(`${violation.ruleId}/${violation.clauseId}:${(violation.span?.startLine ?? -1) + 1}`);
    }
    for (const path of Object.keys(reported)) reported[path]!.sort();
    return { status: result?.status, reported, failures: (result?.failures ?? []).map(failure => `${failure.path ?? ""} ${failure.code}`) };
  } finally {
    await enforcer.close();
  }
}

/** What a CommonJS file running COMMONJS_CODE gets. */
const COMMONJS_FINDINGS = [
  "n-exports-style/upstream:5",
  "n-exports-style/upstream:6",
  "n-global-require/upstream:4",
  "n-no-deprecated-api/buffer:5",
  "n-no-exports-assign/upstream:6",
  "n-no-path-concat/binary-commonjs:7",
  "n-prefer-global-buffer/destructured:1",
  "n-prefer-promises-fs/call:3",
];

describe("the Node pack's CommonJS-binding rules", () => {
  it.each([
    ["the native repository index (Git checkout)", true],
    ["the host's directory walk", false],
  ])("read require, exports and __dirname only in CommonJS files, through %s", async (_name, git) => {
    expect(await audit(await repository(git))).toEqual({
      status: "completed",
      failures: [],
      reported: {
        // An ES module has no require, exports or __dirname: only global-require, which upstream reads by name, reports.
        "esm/index.js": ["n-global-require/upstream:4"],
        "esm/meta.js": ["n-no-path-concat/binary:1"],
        "esm/tool.cjs": COMMONJS_FINDINGS,
        "cjs/index.js": COMMONJS_FINDINGS,
      },
    });
  });
});
