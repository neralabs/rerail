import type { ModelFingerprint } from "./native.js";

export type WatermarkValue = string | number | boolean | null;
export type WatermarkVisibility = "private" | "team" | "project";

export interface ContentRef {
  source: string;
  id: string;
  span?: { start: number; end: number };
  revision?: string;
}

export interface ContentMetadata {
  owner?: string;
  session?: string;
  timestamp?: number;
  fields?: Readonly<Record<string, WatermarkValue>>;
}

export interface ContentItem {
  ref: ContentRef;
  text: string;
  metadata: ContentMetadata;
}

export interface ContentSnapshot {
  id: string;
  items: readonly ContentItem[];
}

export interface ContentSource {
  readonly id: string;
  read(): Promise<ContentSnapshot>;
}

export interface Watermark {
  namespace: string;
  key: string;
  value?: WatermarkValue;
  weight?: number;
  owner: string;
  visibility: WatermarkVisibility;
  createdAt: number;
  expiresAt?: number;
  provenance?: string;
}

export interface WatermarkAssignment {
  id: string;
  target: ContentRef;
  watermark: Watermark;
}

export type WatermarkChange =
  | {
      kind: "attach";
      targets: readonly ContentRef[];
      watermarks: readonly Watermark[];
    }
  | {
      kind: "detach";
      assignmentIds: readonly string[];
    };

export interface MarkReceipt {
  revision: string;
  assignmentIds: readonly string[];
}

export interface WatermarkSnapshot {
  revision: string;
  assignments: readonly WatermarkAssignment[];
}

export interface WatermarkStore {
  apply(change: WatermarkChange): Promise<MarkReceipt>;
  read(): Promise<WatermarkSnapshot>;
}

export type WatermarkExpression =
  | {
      mark: {
        namespace?: string;
        key: string;
        value?: WatermarkValue;
        owner?: string;
        visibility?: WatermarkVisibility;
        provenance?: string;
        minimumWeight?: number;
        maximumWeight?: number;
      };
    }
  | { and: readonly WatermarkExpression[] }
  | { or: readonly WatermarkExpression[] }
  | { not: WatermarkExpression };

export interface SelectionSpec {
  from?: {
    sources?: readonly string[];
    sessions?: readonly string[];
    owners?: readonly string[];
    after?: number;
    before?: number;
  };
  where?: WatermarkExpression;
  exclude?: WatermarkExpression;
  order?: "source" | "time-ascending" | "time-descending";
  limit?: number;
}

export interface SelectedContent extends ContentItem {
  watermarks: readonly WatermarkAssignment[];
}

export interface Selection {
  snapshot: string;
  specificationHash: string;
  items: readonly SelectedContent[];
}

export type OverflowStrategy = "truncate-start" | "truncate-end" | "reject";

export interface SemanticEmbedder {
  fingerprint(): ModelFingerprint;
  embed(texts: readonly string[]): Promise<readonly Float32Array[]>;
  fitToTokenLimit?(
    texts: readonly string[],
    maximumTokens: number,
    overflow: OverflowStrategy,
  ): Promise<readonly string[]>;
}

export type ProjectionUnit =
  | { kind: "item" }
  | { kind: "whole-selection" }
  | {
      kind: "group";
      by:
        | "owner"
        | "session"
        | { field: string }
        | { watermark: { namespace?: string; key: string } };
    }
  | {
      kind: "window";
      size: number;
      stride?: number;
      includePartial?: boolean;
    };

export type ProjectionRender =
  | { kind: "text-only" }
  | { kind: "labeled"; fields: readonly string[] }
  | { kind: "template"; template: string };

export interface ProjectionSpec {
  unit: ProjectionUnit;
  render: ProjectionRender;
  limit?:
    | {
        maximumCharacters: number;
        overflow: OverflowStrategy;
      }
    | {
        maximumTokens: number;
        overflow: OverflowStrategy;
      };
}

export interface ProjectedVector {
  id: string;
  values: Float32Array;
  members: readonly ContentRef[];
  groupKey: string;
  renderedText: string;
  renderedHash: string;
}

export interface SemanticProjection {
  id: string;
  vectors: readonly ProjectedVector[];
  model: ModelFingerprint;
  lineage: {
    selectionSnapshot: string;
    selectionSpecificationHash: string;
    projectionSpecificationHash: string;
  };
}

export type RelationReduction =
  | {
      kind: "max" | "min" | "mean";
      groupBy: "all" | "left" | "right";
    }
  | {
      kind: "weighted-mean";
      groupBy: "all" | "left" | "right";
      weight: (relation: SemanticRelation) => number;
    };

export type RelationSpec = {
  pairing:
    | { kind: "all-pairs" }
    | { kind: "top-k"; k: number }
    | { kind: "corresponding-groups" };
  minimumSimilarity?: number;
  excludeSelf?: boolean;
  reduction?: RelationReduction;
};

export interface SemanticRelation {
  left: string;
  right: string;
  similarity: number;
  leftMembers: readonly ContentRef[];
  rightMembers: readonly ContentRef[];
  model: ModelFingerprint;
}

export interface SemanticRelationReduction {
  key: string;
  similarity: number;
  count: number;
}

export interface SemanticRelationSet {
  pairs: readonly SemanticRelation[];
  reductions: readonly SemanticRelationReduction[];
}

export interface SemanticPrimitives {
  mark(change: WatermarkChange): Promise<MarkReceipt>;
  select(spec: SelectionSpec): Promise<Selection>;
  project(selection: Selection, spec: ProjectionSpec): Promise<SemanticProjection>;
  relate(
    left: SemanticProjection,
    right: SemanticProjection,
    spec: RelationSpec,
  ): Promise<SemanticRelationSet>;
}
