import type { ExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import type {
  ProjectionSpec,
  RelationSpec,
  SelectionSpec,
  WatermarkChange,
} from "../../engine/native/semantic-types.js";
import { internalTesting } from "../../engine/internal-testing.js";
import { semanticToolResult, type NativeSemanticToolRuntime } from "../../harness/native-semantic-tools.js";

/** Internal testing: the semantic tools `rerail_mark`, `rerail_select`, `rerail_project` and `rerail_relate`. */
export function registerOmpSemanticTools(pi: ExtensionAPI, runtime: NativeSemanticToolRuntime): void {
  const z = pi.zod;
  const markParameters = z.object({ change: z.unknown() });
  const selectParameters = z.object({ spec: z.unknown() });
  const projectParameters = z.object({ selectionId: z.string(), spec: z.unknown() });
  const relateParameters = z.object({
    leftProjectionId: z.string(),
    rightProjectionId: z.string(),
    spec: z.unknown(),
  });
  pi.registerTool({
    name: "rerail_mark",
    label: "Rerail Mark",
    description: internalTesting("Attach or detach namespaced watermarks on active-session content."),
    parameters: markParameters,
    approval: "write",
    loadMode: "discoverable",
    strict: true,
    async execute(_id, input) {
      const parsed = markParameters.parse(input);
      return semanticToolResult(await runtime.mark(parsed.change as WatermarkChange));
    },
  });
  pi.registerTool({
    name: "rerail_select",
    label: "Rerail Select",
    description: internalTesting("Select active-session content with source and watermark filters."),
    parameters: selectParameters,
    approval: "read",
    loadMode: "discoverable",
    strict: true,
    async execute(_id, input) {
      const parsed = selectParameters.parse(input);
      return semanticToolResult(await runtime.select(parsed.spec as SelectionSpec));
    },
  });
  pi.registerTool({
    name: "rerail_project",
    label: "Rerail Project",
    description: internalTesting("Render and embed a cached selection."),
    parameters: projectParameters,
    approval: "read",
    loadMode: "discoverable",
    strict: true,
    async execute(_id, input) {
      const parsed = projectParameters.parse(input);
      return semanticToolResult(await runtime.project(parsed.selectionId, parsed.spec as ProjectionSpec));
    },
  });
  pi.registerTool({
    name: "rerail_relate",
    label: "Rerail Relate",
    description: internalTesting("Compare two cached semantic projections."),
    parameters: relateParameters,
    approval: "read",
    loadMode: "discoverable",
    strict: true,
    async execute(_id, input) {
      const parsed = relateParameters.parse(input);
      return semanticToolResult(await runtime.relate(
        parsed.leftProjectionId,
        parsed.rightProjectionId,
        parsed.spec as RelationSpec,
      ));
    },
  });
}
