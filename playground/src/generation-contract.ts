import { z } from "zod";
import { validateScenarioFixtures } from "./experiments.js";
import { ruleSnapshotSchema, runSchema, scenarioSchema, type PlaygroundRuleSnapshot, type PlaygroundScenario } from "./protocol.js";

export const generationInputSchema = z.object({
  rule: ruleSnapshotSchema,
  previous: runSchema.optional(),
  checkoutRoot: z.string().min(1),
}).strict();
export const generatedScenariosSchema = z.array(
  scenarioSchema.omit({ ruleDigest: true, ruleKind: true }).extend({
    explanation: z.string().min(1).max(16_384),
    // Models may omit text-only tool lists, but supplied malformed lists stay invalid.
    steps: z.array(scenarioSchema.shape.steps.element.extend({
      toolCalls: scenarioSchema.shape.steps.element.shape.toolCalls.default([]),
    })).max(32),
  }),
).length(2);
export function parseGeneratedScenarios(value: unknown, rule: PlaygroundRuleSnapshot): PlaygroundScenario[] {
  const pair = generatedScenariosSchema.parse(value);
  if (new Set(pair.map(scenario => scenario.expectation)).size !== 2)
    throw new Error("Generation must return one match and one no-match scenario; expectations are never repaired automatically");
  if (pair[0]!.id === pair[1]!.id) throw new Error("Generated scenarios must have distinct ids");
  if (JSON.stringify([pair[0]!.prompt, pair[0]!.fixtures, pair[0]!.steps]) ===
      JSON.stringify([pair[1]!.prompt, pair[1]!.fixtures, pair[1]!.steps]))
    throw new Error("Generated scenarios must contrast actual inputs, not merely expectation labels");
  return pair.map(scenario => {
    const pinned = { ...scenario, ruleDigest: rule.digest, ruleKind: rule.kind };
    validateScenarioFixtures(pinned);
    return pinned;
  });
}
