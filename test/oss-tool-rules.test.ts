import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import type { NativeSample } from "../src/native.js";
import { CosineRuleRegistry } from "../src/registry.js";
import { importScopedRules } from "../src/rule-bootstrap.js";
import { parseCosineRule, type CosineRule, type MatchCondition } from "../src/rule-schema.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import { stableJsonText } from "../src/stable-json.js";

interface OssToolRule {
  id: string;
  surface: "tool.command" | "tool.path";
  gate: "block" | "warning";
  pattern: string;
  flag: string;
  pass: string;
  hit_examples: string[];
  rule: unknown;
}

const PACK_DIRECTORY = resolve("candidate-rules/oss-tool-rules");
const pack: OssToolRule[] = JSON.parse(readFileSync(join(PACK_DIRECTORY, "rules.json"), "utf8")).rules;

/** The tools and argument names each surface is read from, across hosts. */
const SURFACE_CALLS: Record<OssToolRule["surface"], Array<[tool: string, argument: string]>> = {
  "tool.command": [["Bash", "command"], ["bash", "command"], ["shell", "command"], ["run_terminal_cmd", "command"], ["exec_command", "cmd"]],
  "tool.path": [["Write", "file_path"], ["Edit", "path"], ["MultiEdit", "filePath"], ["write", "path"]],
};
/** A tool of the wrong kind carrying the same text: reading a lockfile is not editing it. */
const WRONG_TOOL: Record<OssToolRule["surface"], [tool: string, argument: string]> = {
  "tool.command": ["Read", "command"],
  "tool.path": ["Read", "file_path"],
};

function call(tool: string, argument: string, value: string): NativeSample {
  const text = stableJsonText({ [argument]: value })!;
  return { source: `tool:${tool}`, boundary: "tool-call", text, input: text };
}

function regexes(condition: MatchCondition): string[] {
  if ("regex" in condition) return [condition.regex];
  if ("all" in condition) return condition.all.flatMap(regexes);
  if ("any" in condition) return condition.any.flatMap(regexes);
  if ("not" in condition) return regexes(condition.not);
  return [];
}

describe("oss-tool-rules pack", () => {
  let registry: CosineRuleRegistry;
  const rules = new Map<string, CosineRule>();
  const fired = (sample: NativeSample) => registry.match(sample, { respectRepeat: false }).map(match => match.ruleId);

  beforeAll(() => {
    for (const record of pack) rules.set(record.id, parseCosineRule(record.rule, `rules[${record.id}]`));
    registry = new CosineRuleRegistry({ modelPath: resolve("model") });
    registry.replace([...rules.values()]);
  });

  it("carries a structural rule for every entry and no __tool__ path hack", () => {
    expect(pack).toHaveLength(32);
    expect(registry.count).toBe(32);
    expect(readFileSync(join(PACK_DIRECTORY, "rules.json"), "utf8")).not.toContain("__tool__");
    for (const record of pack) {
      const rule = rules.get(record.id)!;
      expect(rule.id).toBe(`oss:${record.id}`);
      expect(rule.reference).toBeUndefined();
      expect(rule.sample).toEqual({ source: "tool:*", boundary: "tool-call" });
      expect(rule.action.content.startsWith(`${record.id.toUpperCase().replaceAll("-", "_")}: `)).toBe(true);
    }
  });

  it.each(pack.map(record => [record.id, record] as const))("%s keeps its reviewed pattern in every argument it reads", (_id, record) => {
    const rule = rules.get(record.id)!;
    const [toolName, ...argumentPatterns] = regexes(rule.match as MatchCondition);
    expect(toolName).toMatch(/^\(\?i\)/u);
    expect(argumentPatterns.length).toBeGreaterThan(1);
    expect(new Set(argumentPatterns)).toEqual(new Set([record.pattern]));
  });

  it.each(pack.map(record => [record.id, record] as const))("%s flags its fixture from every host's tool", (_id, record) => {
    for (const [tool, argument] of SURFACE_CALLS[record.surface]) {
      expect(fired(call(tool, argument, record.flag)), `${tool}.${argument}`).toContain(`oss:${record.id}`);
    }
  });

  it.each(pack.map(record => [record.id, record] as const))("%s passes its near-miss fixture", (_id, record) => {
    const [tool, argument] = SURFACE_CALLS[record.surface][0]!;
    expect(fired(call(tool, argument, record.pass))).not.toContain(`oss:${record.id}`);
  });

  it.each(pack.filter(record => record.hit_examples.length > 0).map(record => [record.id, record] as const))(
    "%s still flags the sanctioned commands its pattern matched",
    (_id, record) => {
      const [tool, argument] = SURFACE_CALLS[record.surface][0]!;
      for (const example of record.hit_examples) {
        expect(fired(call(tool, argument, example)), example).toContain(`oss:${record.id}`);
      }
    },
  );

  it.each(pack.map(record => [record.id, record] as const))("%s ignores the wrong kind of tool and tool results", (_id, record) => {
    const [tool, argument] = WRONG_TOOL[record.surface];
    expect(fired(call(tool, argument, record.flag))).not.toContain(`oss:${record.id}`);
    const [resultTool] = SURFACE_CALLS[record.surface][0]!;
    expect(fired({ source: `tool:${resultTool}`, boundary: "tool-call", text: record.flag })).toEqual([]);
  });
});

describe("oss-tool-rules import bundle", () => {
  let directory: string;
  beforeAll(async () => {
    directory = await mkdtemp(join(tmpdir(), "rerail-oss-tool-rules-"));
  });
  afterAll(async () => {
    await rm(directory, { recursive: true, force: true });
  });

  it("imports every rule as a conversation-kind structural rule, enabled per enabled.txt", () => {
    const bundle = join(directory, "import-bundle.json");
    execFileSync("python3", [join(PACK_DIRECTORY, "make_bundle.py"), "--out", bundle], { stdio: "pipe" });
    const enabled = readFileSync(join(PACK_DIRECTORY, "enabled.txt"), "utf8")
      .split("\n").map(line => line.trim()).filter(line => line !== "" && !line.startsWith("#"));

    const catalog = ScopedRuleCatalog.open(join(directory, "catalog.sqlite"), { repositoryId: "repo", workspaceRoot: "/repo" });
    try {
      expect(importScopedRules(catalog, bundle)).toBe(pack.length);
      expect(catalog.list().map(stored => stored.kind)).toEqual(pack.map(() => "conversation"));
      expect(catalog.cosineRules().map(rule => rule.id).sort()).toEqual(enabled.map(id => `oss:${id}`).sort());
    } finally {
      catalog.close();
    }
  });
});
