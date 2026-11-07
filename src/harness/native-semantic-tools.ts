import type { ExtensionAPI as OmpExtensionAPI } from "@oh-my-pi/pi-coding-agent";
import type { ExtensionAPI as PiExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";
import type {
  ProjectionSpec,
  RelationSpec,
  SelectionSpec,
  WatermarkChange,
} from "../engine/native/semantic-types.js";
import { internalTesting } from "../engine/internal-testing.js";

export interface NativeSemanticToolRuntime {
  mark(change: WatermarkChange): Promise<unknown>;
  select(spec: SelectionSpec): Promise<unknown>;
  project(selectionId: string, spec: ProjectionSpec): Promise<unknown>;
  relate(leftId: string, rightId: string, spec: RelationSpec): Promise<unknown>;
}


function result(details: unknown) {
  return {
    content: [{ type: "text" as const, text: JSON.stringify(details, null, 2) }],
    details,
  };
}

/** Internal testing: the semantic tools `rerail_mark`, `rerail_select`, `rerail_project` and `rerail_relate`. */
export function registerOmpSemanticTools(pi: OmpExtensionAPI, runtime: NativeSemanticToolRuntime): void {
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
      return result(await runtime.mark(parsed.change as WatermarkChange));
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
      return result(await runtime.select(parsed.spec as SelectionSpec));
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
      return result(await runtime.project(parsed.selectionId, parsed.spec as ProjectionSpec));
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
      return result(await runtime.relate(
        parsed.leftProjectionId,
        parsed.rightProjectionId,
        parsed.spec as RelationSpec,
      ));
    },
  });
}

/** Internal testing: the semantic tools, as {@link registerOmpSemanticTools}. */
export function registerPiSemanticTools(pi: PiExtensionAPI, runtime: NativeSemanticToolRuntime): void {
  pi.registerTool({
    name: "rerail_mark",
    label: "Rerail Mark",
    description: internalTesting("Attach or detach namespaced watermarks on active-session content."),
    parameters: Type.Object({ change: Type.Unknown() }),
    async execute(_id, input) {
      return result(await runtime.mark(input.change as WatermarkChange));
    },
  });
  pi.registerTool({
    name: "rerail_select",
    label: "Rerail Select",
    description: internalTesting("Select active-session content with source and watermark filters."),
    parameters: Type.Object({ spec: Type.Unknown() }),
    async execute(_id, input) {
      return result(await runtime.select(input.spec as SelectionSpec));
    },
  });
  pi.registerTool({
    name: "rerail_project",
    label: "Rerail Project",
    description: internalTesting("Render and embed a cached selection."),
    parameters: Type.Object({ selectionId: Type.String(), spec: Type.Unknown() }),
    async execute(_id, input) {
      return result(await runtime.project(input.selectionId, input.spec as ProjectionSpec));
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
      return result(await runtime.relate(input.leftProjectionId, input.rightProjectionId, input.spec as RelationSpec));
    },
  });
}
