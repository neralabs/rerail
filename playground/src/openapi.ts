import { OpenApiGeneratorV31, zodToOpenAPIRegistry } from "@asteasolutions/zod-to-openapi";
import type { z } from "zod";
import {
  controlSchema,
  eventSchema,
  experimentSchema,
  outcomeSchema,
  ruleListSchema,
  ruleSnapshotSchema,
  runSchema,
  scenarioSchema,
  selectionSchema,
  traceSchema,
} from "./protocol.js";

const components: ReadonlyArray<[z.ZodType, string]> = [
  [ruleSnapshotSchema, "PlaygroundRuleSnapshot"],
  [ruleListSchema, "PlaygroundRuleList"],
  [scenarioSchema, "PlaygroundScenario"],
  [selectionSchema, "PlaygroundSelection"],
  [traceSchema, "PlaygroundTrace"],
  [outcomeSchema, "PlaygroundOutcome"],
  [runSchema, "PlaygroundRun"],
  [experimentSchema, "PlaygroundExperiment"],
  [controlSchema, "PlaygroundControl"],
  [eventSchema, "PlaygroundEvent"],
];
for (const [schema, refId] of components) zodToOpenAPIRegistry.add(schema, { _internal: { refId } });

export const playgroundOpenApiDocument = new OpenApiGeneratorV31(
  components.map(([schema]) => ({ type: "schema", schema })),
).generateDocument({
  openapi: "3.1.0",
  info: {
    title: "Rerail Rule Playground",
    version: "3.0.0",
    description:
      "Development-only rule experiments. The client is served from this origin at /; commands and events "
      + "use the /__rerail_playground WebSocket; GET /__rerail_playground/rules lists the daemon's saved rules.",
  },
});
