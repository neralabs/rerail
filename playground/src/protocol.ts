// Browser-safe: zod only. The OpenAPI document lives in ./openapi.ts so the client bundle
// never pulls the generator in.
import { z } from "zod";

export const ruleSnapshotSchema = z
  .object({
    id: z.string().min(1).max(128),
    kind: z.enum(["conversation", "file"]),
    definition: z.string().max(262_144),
    digest: z.string(),
    updatedAt: z.string(),
    /** By Rerail (bundled) or created by you (the rule library). */
    source: z.enum(["rerail", "library"]),
  })
  .strict();
export const scenarioSchema = z
  .object({
    id: z.string().min(1).max(128),
    title: z.string().min(1).max(256),
    expectation: z.enum(["match", "no-match"]),
    explanation: z.string().max(16_384),
    ruleDigest: z.string(),
    ruleKind: z.enum(["conversation", "file"]),
    prompt: z.string().min(1).max(32_768),
    fixtures: z
      .array(z.object({ path: z.string().min(1).max(1_024), content: z.string().max(131_072) }).strict())
      .max(64),
    steps: z
      .array(
        z
          .object({
            text: z.string().max(32_768),
            toolCalls: z
              .array(
                z.object({ name: z.string().min(1).max(128), arguments: z.record(z.string(), z.unknown()) }).strict(),
              )
              .max(16),
          })
          .strict(),
      )
      .max(32),
  })
  .strict();
export const selectionSchema = z
  .object({
    ruleId: z.string().min(1).max(128),
    scenarioId: z.string().max(128),
    mode: z.enum(["preview", "trigger", "agent"]),
    allowCommands: z.boolean(),
  })
  .strict();
export const traceSchema = z
  .object({ at: z.number().finite(), kind: z.string().max(128), text: z.string().max(32_768) })
  .strict();
export const outcomeSchema = z
  .object({
    observed: z.enum(["match", "no-match", "cannot-exercise", "error", "preview"]),
    detail: z.string().max(32_768),
  })
  .strict();
export const runSchema = z
  .object({
    rule: ruleSnapshotSchema,
    scenario: scenarioSchema,
    selection: selectionSchema,
    outcome: outcomeSchema,
    completedAt: z.number(),
    trace: z.array(traceSchema).max(250),
  })
  .strict();
export const experimentSchema = z
  .object({
    ruleId: z.string(),
    selection: selectionSchema,
    scenarios: z.array(scenarioSchema).max(2),
    lastRun: runSchema.optional(),
  })
  .strict();
export const controlSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("start"), selection: selectionSchema }).strict(),
  z.object({ type: z.literal("select"), selection: selectionSchema }).strict(),
  z.object({ type: z.literal("generate"), ruleId: z.string(), revise: z.boolean() }).strict(),
  z.object({ type: z.literal("save-scenario"), ruleId: z.string(), scenario: scenarioSchema }).strict(),
  z.object({ type: z.literal("stop") }).strict(),
  z.object({ type: z.enum(["pause", "resume", "step"]) }).strict(),
  z.object({ type: z.literal("input"), data: z.string().min(1).max(16_384) }).strict(),
  z
    .object({
      type: z.literal("resize"),
      cols: z.number().int().min(20).max(400),
      rows: z.number().int().min(5).max(200),
    })
    .strict(),
]);
export const childEventSchema = z.discriminatedUnion("type", [
  z
    .object({
      type: z.literal("status"),
      state: z.enum(["starting", "generating", "running", "paused", "rebuilding", "stopped", "error"]),
      message: z.string().max(16_384),
    })
    .strict(),
  z.object({ type: z.literal("trace"), entry: traceSchema }).strict(),
  z.object({ type: z.literal("outcome"), outcome: outcomeSchema }).strict(),
]);
export const eventSchema = z.discriminatedUnion("type", [
  z.object({ type: z.literal("experiment"), experiment: experimentSchema, rule: ruleSnapshotSchema }).strict(),
  z.object({ type: z.literal("output"), data: z.string() }).strict(),
  z.object({ type: z.literal("reset") }).strict(),
  ...childEventSchema.options,
]);
export const childCommandSchema = z.discriminatedUnion("type", [
  z
    .object({ type: z.literal("command"), id: z.string().min(1).max(128), command: z.string().min(1).max(16_384) })
    .strict(),
  z.object({ type: z.literal("cancel-command"), id: z.string().min(1).max(128) }).strict(),
]);
export const commandResultSchema = z
  .object({
    type: z.literal("command-result"),
    id: z.string().min(1).max(128),
    ok: z.boolean(),
    output: z.string().max(131_072),
  })
  .strict();
export const childControlSchema = z.union([
  z.object({ type: z.enum(["pause", "resume", "step", "shutdown"]) }).strict(),
  commandResultSchema,
]);
export const runInputSchema = z
  .object({ selection: selectionSchema, rule: ruleSnapshotSchema, scenario: scenarioSchema })
  .strict();
export type PlaygroundSelection = z.infer<typeof selectionSchema>;
export type PlaygroundControl = z.infer<typeof controlSchema>;
export type PlaygroundTrace = z.infer<typeof traceSchema>;
export type PlaygroundEvent = z.infer<typeof eventSchema>;
export type PlaygroundChildControl = z.infer<typeof childControlSchema>;
export type PlaygroundChildEvent = z.infer<typeof childEventSchema>;
export type PlaygroundRuleSnapshot = z.infer<typeof ruleSnapshotSchema>;
export type PlaygroundScenario = z.infer<typeof scenarioSchema>;
export type PlaygroundExperiment = z.infer<typeof experimentSchema>;
export type PlaygroundRun = z.infer<typeof runSchema>;
export type PlaygroundRunInput = z.infer<typeof runInputSchema>;
export type PlaygroundOutcome = z.infer<typeof outcomeSchema>;

export type PlaygroundChildCommand = z.infer<typeof childCommandSchema>;

/** What `GET /__rerail_playground/rules` answers: the daemon's saved rules as playground snapshots. */
export const ruleListSchema = z.object({ rules: z.array(ruleSnapshotSchema) }).strict();
export type PlaygroundRuleList = z.infer<typeof ruleListSchema>;
