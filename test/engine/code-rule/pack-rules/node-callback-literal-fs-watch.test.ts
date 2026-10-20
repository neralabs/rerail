/**
 * Two node pack rules must not steer an agent into breaking working code (NER-1834).
 *
 * n-no-callback-literal (eslint-plugin-n n/no-callback-literal) judges a call by the callee's name alone: any `cb(...)`
 * or `callback(...)` whose first argument is a literal is reported as a literal "in error position". Many callbacks
 * with those names are not error-first: ws's `verifyClient(info, cb)` takes `cb(result, code, message)` and ssh2's
 * `authHandler(methodsLeft, partialSuccess, callback)` takes `callback(nextMethod)`. Passing an error or null first,
 * as upstream's message implies, breaks both. The name cannot tell the two apart, so the rule keeps reporting and its
 * message tells the agent to check the callback's contract first.
 *
 * n-prefer-promises-fs (eslint-plugin-n n/prefer-promises/fs) reports `fs.watch`, but `fs.promises.watch` returns an
 * async iterator of change events rather than an FSWatcher, so it is not a drop-in replacement. The rule no longer
 * reports `watch`; packs/node/divergences.json records the upstream fixture this drops.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../src/engine/native/native.js";
import { loadPackRuleDocuments } from "../../../../src/engine/catalog/rule-documents.js";
import { repositoryPath } from "../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;

const evaluationSchema = z.object({
  violations: z.array(z.object({ path: z.string(), message: z.string(), span: z.object({ startLine: z.number().int().nonnegative() }) })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function compile(ruleId: string): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = loadPackRuleDocuments([repositoryPath("packs", "node", "candidates")]).find(candidate => candidate.rule.id === ruleId);
  if (document === undefined) throw new Error(`${ruleId} is not in packs/node/candidates`);
  return runtime.compile(JSON.stringify(document.rule), "json", capabilities);
}

/**
 * The 1-based lines the rule reports in `source`, each with its message. The file sits in a CommonJS package, as a
 * `.js` file reading `require` does: the rules follow each file's module system.
 */
function findings(compiled: NativeCompiledCodeRuleDsl, source: string): { line: number; message: string }[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [
      { language: "json", path: "package.json", source: '{ "type": "commonjs" }\n' },
      { language: "typescript", path: "src/server.js", source },
    ],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => ({ line: violation.span.startLine + 1, message: violation.message })).sort((a, b) => a.line - b.line);
}

const js = (...lines: string[]) => [...lines, ""].join("\n");

describe("n-no-callback-literal", () => {
  const VERIFY_CLIENT = js(
    "const wss = new WebSocketServer({",
    "  server,",
    "  verifyClient: (info, cb) => {",
    "    if (!authorized(info.req)) return cb(false, 401, 'Unauthorized');",
    "    cb(true);",
    "  },",
    "});",
  );
  const AUTH_HANDLER = js("conn.connect({", "  host,", "  authHandler: (methodsLeft, partialSuccess, callback) => {", "    callback('password');", "  },", "});");

  it("still reports a literal first argument, as upstream does", () => {
    expect(findings(compile("n-no-callback-literal"), js("function done(cb) {", "  cb('failed');", "}")).map(finding => finding.line)).toEqual([2]);
    expect(findings(compile("n-no-callback-literal"), VERIFY_CLIENT).map(finding => finding.line)).toEqual([4, 5]);
    expect(findings(compile("n-no-callback-literal"), AUTH_HANDLER).map(finding => finding.line)).toEqual([4]);
  });

  it("stays silent on an error or null first argument", () => {
    expect(findings(compile("n-no-callback-literal"), js("cb(null, 'ok');", "callback(new Error('failed'));", "cb(err);"))).toEqual([]);
  });

  it("tells the agent to check the callback is error-first before changing the call (ws verifyClient, ssh2 authHandler)", () => {
    const [finding] = findings(compile("n-no-callback-literal"), AUTH_HANDLER);
    expect(finding?.message).toBe(
      "Pass an Error or null as this callback's first argument, unless its API expects a different first argument.",
    );
  });
});

describe("n-prefer-promises-fs", () => {
  it("still reports the fs callback API that has a drop-in promise form", () => {
    const source = js("const fs = require('fs');", "fs.readFile('a', cb);", "fs.stat('a', cb);");
    expect(findings(compile("n-prefer-promises-fs"), source).map(finding => finding.line)).toEqual([2, 3]);
  });

  it.each([
    ["fs.watch on require('fs')", js("const fs = require('fs');", "fs.watch('file', () => {});")],
    ["fs.watch on an ESM namespace import", js("import * as fs from 'node:fs';", "const watcher = fs.watch(dir, { recursive: true }, onChange);", "watcher.close();")],
    ["a destructured watch", js("const { watch } = require('fs');", "watch('file', () => {});")],
    ["an imported watch", js("import { watch } from 'fs';", "watch('file', () => {});")],
  ])("stays silent on %s, whose promise form returns an async iterator", (_name, source) => {
    expect(findings(compile("n-prefer-promises-fs"), source)).toEqual([]);
  });
});
