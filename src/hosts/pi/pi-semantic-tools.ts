import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import { internalTesting } from "../../engine/internal-testing.js";
import type { ProjectionSpec, RelationSpec, SelectionSpec, WatermarkChange } from "../../engine/native/semantic-types.js";
import { semanticToolResult, type NativeSemanticToolRuntime } from "../../harness/native-semantic-tools.js";

/** Internal testing: the semantic tools, as `registerOmpSemanticTools` registers them in OMP. */
export function registerPiSemanticTools(pi: ExtensionAPI, runtime: NativeSemanticToolRuntime): void {
  pi.registerTool({
    name: "rerail_mark",
    label: "Rerail Mark",
    description: internalTesting("Attach or detach namespaced watermarks on active-session content."),
    parameters: Type.Object({ change: Type.Unknown() }),
    async execute(_id, input) {
      return semanticToolResult(await runtime.mark(input.change as WatermarkChange));
    },
  });
  pi.registerTool({
    name: "rerail_select",
    label: "Rerail Select",
    description: internalTesting("Select active-session content with source and watermark filters."),
    parameters: Type.Object({ spec: Type.Unknown() }),
    async execute(_id, input) {
      return semanticToolResult(await runtime.select(input.spec as SelectionSpec));
    },
  });
  pi.registerTool({
    name: "rerail_project",
    label: "Rerail Project",
    description: internalTesting("Render and embed a cached selection."),
    parameters: Type.Object({ selectionId: Type.String(), spec: Type.Unknown() }),
    async execute(_id, input) {
      return semanticToolResult(await runtime.project(input.selectionId, input.spec as ProjectionSpec));
    },
  });
  pi.registerTool({
    name: "rerail_relate",
    label: "Rerail Relate",
    description: internalTesting("Compare two cached semantic projections."),
    parameters: Type.Object({
      leftProjectionId: Type.String(),
      rightProjectionId: Type.String(),
      spec: Type.Unknown(),
    }),
    async execute(_id, input) {
      return semanticToolResult(await runtime.relate(input.leftProjectionId, input.rightProjectionId, input.spec as RelationSpec));
    },
  });
}
