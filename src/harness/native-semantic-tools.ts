import type {
  ProjectionSpec,
  RelationSpec,
  SelectionSpec,
  WatermarkChange,
} from "../engine/native/semantic-types.js";

export interface NativeSemanticToolRuntime {
  mark(change: WatermarkChange): Promise<unknown>;
  select(spec: SelectionSpec): Promise<unknown>;
  project(selectionId: string, spec: ProjectionSpec): Promise<unknown>;
  relate(leftId: string, rightId: string, spec: RelationSpec): Promise<unknown>;
}

/** A semantic tool's answer, as the OMP and Pi registrations return it. */
export function semanticToolResult(details: unknown) {
  return {
    content: [{ type: "text" as const, text: JSON.stringify(details, null, 2) }],
    details,
  };
}
