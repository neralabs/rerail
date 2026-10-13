import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { NativeSample } from "../../../src/engine/native/native.js";
import { ConversationRuleRegistry } from "../../../src/engine/catalog/registry.js";
import { parseConversationRule } from "../../../src/engine/catalog/rule-schema.js";
import { readRulePacks } from "../../../src/engine/catalog/rule-packs.js";
import { stableJsonText } from "../../../src/engine/stable-json.js";

const RULE = "guardrails-no-new-lint-suppression";
const rule = parseConversationRule(JSON.parse(readFileSync(resolve("packs/guardrails/rules/guardrails-no-new-lint-suppression.json"), "utf8")));

/** A tool-call sample as the host runtime builds it: the arguments are both the text and the structured input. */
function call(toolName: string, input: Record<string, unknown>): NativeSample {
  const text = stableJsonText(input)!;
  return { source: `tool:${toolName}`, boundary: "tool-call", text, input: text };
}

function fired(toolName: string, input: Record<string, unknown>): string[] {
  const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
  registry.replace([rule]);
  return registry.match(call(toolName, input), { respectRepeat: false }).map(match => match.ruleId);
}

describe("guardrails-no-new-lint-suppression", () => {
  it.each<[string, Record<string, unknown>]>([
    ["Write", { file_path: "src/log.ts", content: "export function log(user: string) {\n  // eslint-disable-next-line no-console\n  console.log(user);\n}\n" }],
    ["Edit", { file_path: "app.py", old_string: "if x == None:", new_string: "if x == None:  # noqa: E711" }],
    ["apply_patch", { command: "*** Begin Patch\n*** Update File: app.py\n@@\n-    x = 1\n+    x = 1  # nosec\n*** End Patch" }],
    ["MultiEdit", { file_path: "src/a.js", edits: [{ old_string: "let a = 1;", new_string: "let a = 1;" }, { old_string: "run();", new_string: "/* oxlint-disable */\nrun();" }] }],
    ["write", { path: "src/a.ts", content: "// @ts-ignore\nconst a: number = 'x';\n" }],
    ["edit", { input: "[src/a.php#1A2B]\nPUT 3.=3:\n+/** @phpstan-ignore argument.type */\n" }],
    ["Edit", { file_path: "a.py", old_string: "import os", new_string: "import os  # NOQA" }],
    ["Edit", { file_path: "a.py", old_string: "x: int = f()", new_string: "x: int = f()  # type: ignore[assignment]" }],
    ["Edit", { file_path: "a.py", old_string: "def f():", new_string: "def f():  # pylint: disable=missing-docstring" }],
    ["Edit", { file_path: "a.php", old_string: "$a = 1;", new_string: "$a = 1; // phpcs:ignore Generic.Files" }],
    ["Edit", { file_path: "a.php", old_string: "$a = 1;", new_string: "/** @psalm-suppress MixedAssignment */\n$a = 1;" }],
    ["Edit", { file_path: "a.ts", old_string: "a == b", new_string: "// biome-ignore lint/suspicious/noDoubleEquals: legacy\na == b" }],
    ["Edit", { file_path: "a.ts", old_string: "eval(x)", new_string: "/* eslint no-eval: off */\neval(x)" }],
  ])("fires when %s adds a suppression: %j", (tool, input) => {
    expect(fired(tool, input)).toEqual([RULE]);
  });

  it.each<[string, Record<string, unknown>]>([
    ["Edit", { file_path: "src/log.ts", old_string: "  // eslint-disable-next-line no-console\n  console.log(user);", new_string: "  logger.debug(user);" }],
    ["MultiEdit", { file_path: "app.py", edits: [{ old_string: "x = 1  # noqa: F841", new_string: "" }] }],
    ["apply_patch", { command: "*** Begin Patch\n*** Update File: app.py\n@@\n-    x = 1  # nosec\n+    x = 1\n*** End Patch" }],
    ["Bash", { command: "grep -rn eslint-disable ." }],
    ["Read", { file_path: "src/log.ts", content: "// eslint-disable-next-line no-console" }],
    ["Write", { file_path: "src/log.ts", content: "export const log = (user: string) => logger.debug(user);\n" }],
    ["Edit", { file_path: "a.py", old_string: "#", new_string: "# noqalike comment about nosecurity" }],
  ])("does not fire on %s: %j", (tool, input) => {
    expect(fired(tool, input)).toEqual([]);
  });

  it("steers with the rule's action every time", () => {
    const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
    registry.replace([rule]);
    const sample = call("Write", { file_path: "a.py", content: "import os  # noqa\n" });
    expect(registry.match(sample).map(match => match.mode)).toEqual(["steer"]);
    expect(registry.match(sample).map(match => match.mode)).toEqual(["steer"]);
    expect(rule.action.content).toMatch(/fix the code/u);
  });

  it("catches the pack's caught example and lets its passing example pass, written as a file's content", () => {
    const listed = readRulePacks([resolve("packs/guardrails")])[0]!.rules.find(candidate => candidate.id === RULE)!;
    const { caught, passes } = listed.examples!;
    expect(fired("Write", { file_path: "src/log.ts", content: caught.text })).toEqual([RULE]);
    expect(fired("Write", { file_path: "src/log.ts", content: passes.text })).toEqual([]);
  });
});
