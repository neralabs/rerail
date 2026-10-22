import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { validatePack } from "../../../conformance/pack-validation.js";
import { ConversationRuleRegistry, parseConversationRule } from "../../../src/engine/index.js";
import type { NativeSample } from "../../../src/engine/native/native.js";
import { fixturesPath, repositoryPath } from "../../support/oracles.js";

const rule = parseConversationRule(JSON.parse(await readFile(repositoryPath("packs", "workflow", "candidates", "rules", "workflow-check-unfinished-task.json"), "utf8")));
const cases: { caught: NativeSample[]; passed: NativeSample[] } = JSON.parse(await readFile(fixturesPath("host", "task-completion", "workflow-check-unfinished-task.cases.json"), "utf8"));
const temporary: string[] = [];

afterEach(async () => {
  await Promise.all(temporary.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function candidatePack(candidate: unknown): Promise<string> {
  const parent = await mkdtemp(join(tmpdir(), "rerail-workflow-candidate-"));
  temporary.push(parent);
  const pack = join(parent, "empty");
  await cp(fixturesPath("packs", "empty"), pack, { recursive: true });
  const rules = join(pack, "candidates", "rules");
  await mkdir(rules, { recursive: true });
  await writeFile(join(rules, "workflow-check-unfinished-task.json"), JSON.stringify(candidate));
  return pack;
}

function registry(): ConversationRuleRegistry {
  const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
  registry.replace([rule]);
  return registry;
}

describe("unfinished-task observation rule", () => {
  it("catches admitted unfinished work and passes completion, blocker, and boundary cases", () => {
    const rules = registry();
    for (const expected of ["caught", "passed"] as const) {
      for (const sample of cases[expected]) {
        expect(rules.match(sample, { respectRepeat: false }).length > 0, sample.text).toBe(expected === "caught");
      }
    }
  });

  it("keeps matches in Log mode and respects delivery once per session", () => {
    const rules = registry();
    const sample = { source: "assistant-text" as const, boundary: "turn" as const, text: "I have not completed the task." };
    const matches = rules.match(sample, { turn: 1 });
    expect(matches).toMatchObject([{ ruleId: "workflow-check-unfinished-task", mode: "log" }]);
    rules.markDelivered(matches.map(match => match.ruleId), 1);
    expect(rules.match(sample, { turn: 2 })).toEqual([]);
  });

  it("validates a conversation candidate without shipping it", async () => {
    const result = await validatePack(await candidatePack(rule), { corpora: false });
    expect(result.checks).toEqual(expect.objectContaining({ rules: [], text: [] }));
    expect(result.pass).toBe(true);
    expect(result.rules).toEqual([]);
  });

  it("rejects a malformed conversation candidate", async () => {
    const result = await validatePack(await candidatePack({ id: rule.id, mode: "log" }), { corpora: false });
    expect(result.checks.rules).not.toEqual([]);
  });
});
