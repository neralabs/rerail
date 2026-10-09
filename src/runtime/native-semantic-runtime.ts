import { randomUUID } from "node:crypto";
import { OmpMessageContentSource, OmpWatermarkStore, type OmpSessionEntryLike } from "./omp-semantic.js";
import { NativeSemanticEmbedder, SemanticWorkspace } from "../engine/native/semantic-primitives.js";
import type {
  MarkReceipt,
  ProjectionSpec,
  RelationSpec,
  Selection,
  SelectionSpec,
  SemanticProjection,
  WatermarkChange,
} from "../engine/native/semantic-types.js";

interface NativeSemanticSession {
  sessionId: string;
  entries(): readonly OmpSessionEntryLike[];
  appendEntry(customType: string, data: unknown): void;
}

function publicProjection(projection: SemanticProjection): unknown {
  return {
    projectionId: projection.id,
    model: projection.model,
    lineage: projection.lineage,
    vectors: projection.vectors.map(vector => ({
      id: vector.id,
      members: vector.members,
      groupKey: vector.groupKey,
      renderedText: vector.renderedText,
      renderedHash: vector.renderedHash,
    })),
  };
}

export class NativeSemanticRuntime {
  readonly #embedder: NativeSemanticEmbedder;
  readonly #sourceId: string;
  #workspace: SemanticWorkspace | undefined;
  readonly #selections = new Map<string, Selection>();
  readonly #projections = new Map<string, SemanticProjection>();

  constructor(options: { modelPath: string; sourceId: string }) {
    this.#embedder = new NativeSemanticEmbedder(options.modelPath);
    this.#sourceId = options.sourceId;
  }

  initialize(session: NativeSemanticSession): void {
    const source = new OmpMessageContentSource({
      sessionId: session.sessionId,
      entries: session.entries,
      sourceId: this.#sourceId,
    });
    const watermarks = new OmpWatermarkStore({
      entries: session.entries,
      appendEntry: session.appendEntry,
    });
    this.#workspace = new SemanticWorkspace({ sources: [source], watermarks, embedder: this.#embedder });
    this.#selections.clear();
    this.#projections.clear();
  }

  close(): void {
    this.#workspace = undefined;
    this.#selections.clear();
    this.#projections.clear();
  }

  mark(change: WatermarkChange): Promise<MarkReceipt> {
    return this.#active().mark(change);
  }

  async select(spec: SelectionSpec): Promise<unknown> {
    const selection = await this.#active().select(spec);
    const selectionId = randomUUID();
    this.#selections.set(selectionId, selection);
    return {
      selectionId,
      snapshot: selection.snapshot,
      specificationHash: selection.specificationHash,
      items: selection.items,
    };
  }

  async project(selectionId: string, spec: ProjectionSpec): Promise<unknown> {
    const selection = this.#selections.get(selectionId);
    if (selection === undefined) throw new Error("Unknown selectionId for active session");
    const projection = await this.#active().project(selection, spec);
    this.#projections.set(projection.id, projection);
    return publicProjection(projection);
  }

  relate(leftId: string, rightId: string, spec: RelationSpec): Promise<unknown> {
    const left = this.#projections.get(leftId);
    const right = this.#projections.get(rightId);
    if (left === undefined || right === undefined) throw new Error("Unknown projection id for active session");
    return this.#active().relate(left, right, spec);
  }

  #active(): SemanticWorkspace {
    if (this.#workspace === undefined) throw new Error("semantic tools require an active session");
    return this.#workspace;
  }
}
