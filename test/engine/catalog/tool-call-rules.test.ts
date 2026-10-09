import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import type { NativeSample } from "../../../src/engine/native/native.js";
import { ConversationRuleRegistry } from "../../../src/engine/catalog/registry.js";
import { parseConversationRuleDocument } from "../../../src/engine/catalog/rule-schema.js";
import { stableJsonText } from "../../../src/engine/stable-json.js";

const FORCE_PUSH = "oss:no-force-push";
const SKIP_VERIFY = "oss:no-skip-verify";
const GIT_INTERNALS = "oss:no-git-internals-write";

const document: unknown = JSON.parse(readFileSync(resolve("test/fixtures/tool-call-rules.json"), "utf8"));

/** A tool-call sample as the host runtime builds it: the arguments are both the text and the structured input. */
function call(toolName: string, input: Record<string, unknown>): NativeSample {
  const text = stableJsonText(input)!;
  return { source: `tool:${toolName}`, boundary: "tool-call", text, input: text };
}

/** A tool-result sample: same source and boundary as the call, but no structured input. */
function result(toolName: string, output: string): NativeSample {
  return { source: `tool:${toolName}`, boundary: "tool-call", text: output };
}

function registry(): ConversationRuleRegistry {
  const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
  registry.replace(parseConversationRuleDocument(document));
  return registry;
}

function fired(registry: ConversationRuleRegistry, sample: NativeSample): string[] {
  return registry.match(sample, { respectRepeat: false }).map(match => match.ruleId);
}

describe("tool-call rules from the open-source corpus", () => {
  it("parses the example document without references or embeddings", () => {
    const rules = parseConversationRuleDocument(document);
    expect(rules.map(rule => rule.id)).toEqual([FORCE_PUSH, SKIP_VERIFY, GIT_INTERNALS]);
    expect(rules.every(rule => rule.reference === undefined)).toBe(true);
  });

  it.each([
    ["Bash", { command: "git push --force origin feature/auth" }],
    ["Bash", { command: "git push origin main --force-with-lease" }],
    ["shell", { command: "git push -f origin main" }],
    ["Bash", { command: "git status\ngit push --force" }],
  ])("flags a force push through %s: %j", (tool, input) => {
    expect(fired(registry(), call(tool, input))).toEqual([FORCE_PUSH]);
  });

  it.each([
    ["Bash", { command: "git push origin feature/auth" }],
    ["Bash", { command: "git fetch --force" }],
    ["Write", { file_path: "notes.md", content: "never run git push --force" }],
    ["Read", { command: "git push --force" }],
  ])("passes %s: %j", (tool, input) => {
    expect(fired(registry(), call(tool, input))).toEqual([]);
  });

  it("flags --no-verify only on the Bash tool it is bound to", () => {
    const rules = registry();
    expect(fired(rules, call("Bash", { command: "git commit --no-verify -m 'wip'" }))).toEqual([SKIP_VERIFY]);
    expect(fired(rules, call("Bash", { command: "git commit -m 'wip'" }))).toEqual([]);
    expect(fired(rules, call("Bash", { command: "npm test -- --no-verify" }))).toEqual([]);
    expect(fired(rules, call("shell", { command: "git commit --no-verify -m 'wip'" }))).toEqual([]);
  });

  it("flags writes inside .git whichever argument names the path", () => {
    const rules = registry();
    expect(fired(rules, call("Write", { file_path: ".git/hooks/pre-commit", content: "exit 0" }))).toEqual([GIT_INTERNALS]);
    expect(fired(rules, call("Edit", { path: "repo/.git/config" }))).toEqual([GIT_INTERNALS]);
    expect(fired(rules, call("Write", { file_path: "scripts/hooks/pre-commit", content: "exit 0" }))).toEqual([]);
    expect(fired(rules, call("Write", { file_path: ".github/workflows/ci.yml", content: "" }))).toEqual([]);
  });

  it("never fires on a tool result that merely mentions the command", () => {
    const rules = registry();
    expect(fired(rules, result("Bash", "hint: use 'git push --force' to overwrite"))).toEqual([]);
    expect(fired(rules, result("Write", "wrote .git/hooks/pre-commit"))).toEqual([]);
  });

  it("reports a structural match without similarity facts and delivers the rule's action", () => {
    const [match] = registry().match(call("Bash", { command: "git push --force" }));
    expect(match).toEqual({
      ruleId: FORCE_PUSH,
      kind: "structural",
      mode: "steer",
      action: { content: "Do not force-push: it rewrites published history. Ask the user first." },
    });
  });
});
