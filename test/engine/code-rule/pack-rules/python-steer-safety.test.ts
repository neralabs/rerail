/** Safety conditions in native findings are part of the agent-facing guidance contract (NER-2036). */
import { readFile } from "node:fs/promises";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { readRuleDocument } from "../../../../src/engine/catalog/rule-documents.js";
import { loadNativeBinding } from "../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../support/oracles.js";

const capabilities = await readFile(repositoryPath("native", "src", "code_rule_dsl", "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
if (runtime === undefined) throw new Error("Missing native rule runtime");
const evaluationSchema = z.object({
  violations: z.array(z.object({ message: z.string() })).default([]),
  failures: z.array(z.unknown()).default([]),
});

function findings(id: string, source: string): string[] {
  const document = readRuleDocument(repositoryPath("packs", "python", "rules", `${id}.json`));
  const rule = runtime!.compile(JSON.stringify(document.rule), "json", capabilities);
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(JSON.stringify({
    format: "rerail.repository-snapshot/v1", revision: 0, facts: [],
    files: [{ language: "python", path: "src/example.py", source }],
  }), "report")));
  expect(result.failures).toEqual([]);
  return result.violations.map(violation => violation.message);
}

it("F541 preserves runtime text when removing f-prefix brace escapes", () => {
  const messages = findings("ruff-f541-f-string-missing-placeholders", "label = f'{{ready}}'\n");
  expect(messages).toHaveLength(1);
  expect(messages[0]).toMatch(/unescape.*braces/i);
  expect(messages[0]).toMatch(/runtime.*unchanged/i);
  expect(findings("ruff-f541-f-string-missing-placeholders", "label = '{ready}'\n")).toEqual([]);
});

it("B905 offers intentional truncation as well as equal-length checking", () => {
  const id = "ruff-b905-zip-without-explicit-strict";
  const messages = findings(id, "pairs = zip(left, right)\n");
  expect(messages).toHaveLength(1);
  expect(messages[0]).toContain("strict=True");
  expect(messages[0]).toContain("strict=False");
  expect(messages[0]).toMatch(/truncat/i);
  for (const strict of ["True", "False"]) expect(findings(id, `pairs = zip(left, right, strict=${strict})\n`)).toEqual([]);
});

it("ASYNC210 retains a thread option for an existing injected HTTP API", () => {
  const id = "ruff-async210-blocking-http-call-in-async-function";
  const messages = findings(id, "import requests\nasync def fetch(url):\n    return requests.get(url, timeout=5).text\n");
  expect(messages).toHaveLength(1);
  expect(messages[0]).toContain("asyncio.to_thread");
  expect(messages[0]).toMatch(/response handling/i);
  expect(messages[0]).toMatch(/preserve.*API.*options/i);
});

describe("ASYNC222 retains wait arguments and returned status", () => {
  it.each(["os.wait()", "os.waitpid(pid, options)"])("does not assume an async process object for %s", call => {
    const messages = findings("ruff-async222-wait-for-process-in-async-function", `import os\nasync def collect(pid, options):\n    return ${call}\n`);
    expect(messages).toHaveLength(1);
    expect(messages[0]).toContain("asyncio.to_thread");
    expect(messages[0]).toMatch(/arguments.*status/i);
    expect(messages[0]).toMatch(/only.*async process/i);
  });
});

it("SIM112 preserves intentional externally defined lowercase environment keys", () => {
  const messages = findings("ruff-sim112-uncapitalized-environment-variables", "import os\ntoken = os.environ.get('service_token')\n");
  expect(messages).toHaveLength(1);
  expect(messages[0]).toMatch(/preserve intentional lowercase/i);
  expect(messages[0]).toContain("POSIX");
});

describe("SIM300 operand swaps keep comparison meaning", () => {
  it.each([
    ["5 < value", "value > 5"], ["5 > value", "value < 5"],
    ["5 <= value", "value >= 5"], ["5 >= value", "value <= 5"],
    ["5 == value", "value == 5"], ["5 != value", "value != 5"],
  ])("explains the operator mapping for %s", (before, after) => {
    const id = "ruff-sim300-yoda-conditions";
    const messages = findings(id, `result = ${before}\n`);
    expect(messages).toHaveLength(1);
    expect(messages[0]).toMatch(/reverse/);
    expect(messages[0]).toMatch(/equality operators unchanged/);
    expect(findings(id, `result = ${after}\n`)).toEqual([]);
  });
});
