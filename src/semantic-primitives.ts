import { createHash } from "node:crypto";
import { loadNativeBinding } from "./native.js";
import type { NativeCosineKernel } from "./native.js";
import type {
  ContentItem,
  ContentSource,
  ContentRef,
  ContentSnapshot,
  MarkReceipt,
  ProjectedVector,
  ProjectionRender,
  ProjectionSpec,
  RelationSpec,
  RelationReduction,
  SelectedContent,
  Selection,
  SelectionSpec,
  SemanticEmbedder,
  SemanticPrimitives,
  SemanticProjection,
  SemanticRelation,
  SemanticRelationReduction,
  SemanticRelationSet,
  Watermark,
  WatermarkAssignment,
  WatermarkChange,
  WatermarkExpression,
  WatermarkSnapshot,
  WatermarkStore,
} from "./semantic-types.js";

function stableValue(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(stableValue);
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, entry]) => [key, stableValue(entry)]),
    );
  }
  return value;
}

function stableJson(value: unknown): string {
  return JSON.stringify(stableValue(value));
}

function hash(value: unknown): string {
  return createHash("sha256").update(typeof value === "string" ? value : stableJson(value)).digest("hex");
}

function contentRefKey(ref: ContentRef): string {
  return stableJson(ref);
}

function validateWatermark(watermark: Watermark): void {
  if (watermark.namespace.trim() === "" || watermark.key.trim() === "") {
    throw new Error("Watermark namespace and key must be non-empty");
  }
  if (!Number.isFinite(watermark.createdAt)) throw new Error("Watermark createdAt must be finite");
  if (watermark.expiresAt !== undefined && watermark.expiresAt <= watermark.createdAt) {
    throw new Error("Watermark expiresAt must be later than createdAt");
  }
  if (watermark.weight !== undefined && !Number.isFinite(watermark.weight)) {
    throw new Error("Watermark weight must be finite");
  }
}

function validateRef(ref: ContentRef): void {
  if (ref.source.trim() === "" || ref.id.trim() === "") {
    throw new Error("Content reference source and id must be non-empty");
  }
  if (
    ref.span !== undefined &&
    (!Number.isInteger(ref.span.start) ||
      !Number.isInteger(ref.span.end) ||
      ref.span.start < 0 ||
      ref.span.end <= ref.span.start)
  ) {
    throw new Error("Content reference span must be a non-empty, non-negative range");
  }
}

export class InMemoryWatermarkStore implements WatermarkStore {
  readonly #assignments = new Map<string, WatermarkAssignment>();
  #revision = 0;
  #nextId = 1;

  async apply(change: WatermarkChange): Promise<MarkReceipt> {
    const changed = change.kind === "attach"
      ? this.#attach(change.targets, change.watermarks)
      : this.#detach(change.assignmentIds);
    if (changed.length > 0) this.#revision += 1;
    return { revision: String(this.#revision), assignmentIds: changed };
  }

  #attach(targets: readonly ContentRef[], watermarks: readonly Watermark[]): string[] {
    for (const target of targets) validateRef(target);
    for (const watermark of watermarks) validateWatermark(watermark);
    const changed: string[] = [];
    for (const target of targets) {
      for (const watermark of watermarks) {
        const id = `watermark-${this.#nextId++}`;
        this.#assignments.set(id, {
          id,
          target: structuredClone(target),
          watermark: structuredClone(watermark),
        });
        changed.push(id);
      }
    }
    return changed;
  }

  #detach(assignmentIds: readonly string[]): string[] {
    return assignmentIds.filter(id => this.#assignments.delete(id));
  }

  async read(): Promise<WatermarkSnapshot> {
    return {
      revision: String(this.#revision),
      assignments: [...this.#assignments.values()].map(assignment => structuredClone(assignment)),
    };
  }
}
export class NativeSemanticEmbedder implements SemanticEmbedder {
  readonly #kernel: NativeCosineKernel;

  constructor(modelPath: string, maxInputTokens = 256) {
    this.#kernel = loadNativeBinding().CosineKernel.load(modelPath, maxInputTokens);
  }

  fingerprint() {
    return this.#kernel.fingerprint();
  }

  async embed(texts: readonly string[]): Promise<readonly Float32Array[]> {
    return this.#kernel.embedTexts([...texts]);
  }
  async fitToTokenLimit(
    texts: readonly string[],
    maximumTokens: number,
    overflow: "truncate-start" | "truncate-end" | "reject",
  ): Promise<readonly string[]> {
    return this.#kernel.fitTexts([...texts], maximumTokens, overflow);
  }
}


function isActive(assignment: WatermarkAssignment, now: number): boolean {
  const { createdAt, expiresAt } = assignment.watermark;
  return createdAt <= now && (expiresAt === undefined || expiresAt > now);
}

function watermarkMatches(assignment: WatermarkAssignment, expression: Extract<WatermarkExpression, { mark: unknown }>): boolean {
  const expected = expression.mark;
  const actual = assignment.watermark;
  const weight = actual.weight;
  return (
    (expected.namespace === undefined || actual.namespace === expected.namespace) &&
    actual.key === expected.key &&
    (!("value" in expected) || actual.value === expected.value) &&
    (expected.owner === undefined || actual.owner === expected.owner) &&
    (expected.visibility === undefined || actual.visibility === expected.visibility) &&
    (expected.provenance === undefined || actual.provenance === expected.provenance) &&
    (expected.minimumWeight === undefined || (weight !== undefined && weight >= expected.minimumWeight)) &&
    (expected.maximumWeight === undefined || (weight !== undefined && weight <= expected.maximumWeight))
  );
}

function expressionMatches(assignments: readonly WatermarkAssignment[], expression: WatermarkExpression): boolean {
  if ("mark" in expression) return assignments.some(assignment => watermarkMatches(assignment, expression));
  if ("and" in expression) return expression.and.every(child => expressionMatches(assignments, child));
  if ("or" in expression) return expression.or.some(child => expressionMatches(assignments, child));
  return !expressionMatches(assignments, expression.not);
}
function validateMarkExpression(expression: Extract<WatermarkExpression, { mark: unknown }>): void {
  if (expression.mark.key.trim() === "") throw new Error("Watermark expression key must be non-empty");
  const { minimumWeight, maximumWeight } = expression.mark;
  if (minimumWeight !== undefined && !Number.isFinite(minimumWeight)) {
    throw new Error("Watermark expression minimumWeight must be finite");
  }
  if (maximumWeight !== undefined && !Number.isFinite(maximumWeight)) {
    throw new Error("Watermark expression maximumWeight must be finite");
  }
  if (minimumWeight !== undefined && maximumWeight !== undefined && minimumWeight > maximumWeight) {
    throw new Error("Watermark expression minimumWeight must not exceed maximumWeight");
  }
}

function validateExpression(expression: WatermarkExpression): void {
  if ("mark" in expression) {
    validateMarkExpression(expression);
    return;
  }
  if ("and" in expression) {
    for (const child of expression.and) validateExpression(child);
    return;
  }
  if ("or" in expression) {
    for (const child of expression.or) validateExpression(child);
    return;
  }
  validateExpression(expression.not);
}


function matchesSourceFilters(item: ContentItem, spec: SelectionSpec): boolean {
  const from = spec.from;
  if (from === undefined) return true;
  if (from.sources !== undefined && !from.sources.includes(item.ref.source)) return false;
  if (from.sessions !== undefined && (item.metadata.session === undefined || !from.sessions.includes(item.metadata.session))) return false;
  if (from.owners !== undefined && (item.metadata.owner === undefined || !from.owners.includes(item.metadata.owner))) return false;
  if (from.after !== undefined && (item.metadata.timestamp === undefined || item.metadata.timestamp < from.after)) return false;
  if (from.before !== undefined && (item.metadata.timestamp === undefined || item.metadata.timestamp > from.before)) return false;
  return true;
}

function resolveCandidate(ref: ContentRef, items: ReadonlyMap<string, ContentItem>): ContentItem | undefined {
  const base = items.get(`${ref.source}\0${ref.id}`);
  if (base === undefined || (ref.revision !== undefined && base.ref.revision !== ref.revision)) return undefined;
  if (ref.span === undefined) return { ...base, ref };
  if (ref.span.end > base.text.length) return undefined;
  return { ...base, ref, text: base.text.slice(ref.span.start, ref.span.end) };
}

function compareSourceOrder(left: SelectedContent, right: SelectedContent, order: ReadonlyMap<string, number>): number {
  return (order.get(contentRefKey(left.ref)) ?? Number.MAX_SAFE_INTEGER) -
    (order.get(contentRefKey(right.ref)) ?? Number.MAX_SAFE_INTEGER);
}

function sortSelection(items: SelectedContent[], orderBy: SelectionSpec["order"], sourceOrder: ReadonlyMap<string, number>): void {
  if (orderBy === "time-ascending" || orderBy === "time-descending") {
    const direction = orderBy === "time-ascending" ? 1 : -1;
    items.sort((left, right) => {
      const timestamp = ((left.metadata.timestamp ?? 0) - (right.metadata.timestamp ?? 0)) * direction;
      return timestamp || contentRefKey(left.ref).localeCompare(contentRefKey(right.ref));
    });
    return;
  }
  items.sort((left, right) => compareSourceOrder(left, right, sourceOrder));
}

function renderValue(item: SelectedContent, field: string): string {
  if (field === "text") return item.text;
  if (field === "source") return item.ref.source;
  if (field === "id") return item.ref.id;
  if (field === "owner") return item.metadata.owner ?? "";
  if (field === "session") return item.metadata.session ?? "";
  if (field === "timestamp") return item.metadata.timestamp?.toString() ?? "";
  return item.metadata.fields?.[field]?.toString() ?? "";
}

function renderItem(item: SelectedContent, render: ProjectionRender): string {
  if (render.kind === "text-only") return item.text;
  if (render.kind === "labeled") {
    const labels = render.fields.map(field => `${field}=${renderValue(item, field)}`).join(" ");
    return labels === "" ? item.text : `${labels}\n${item.text}`;
  }
  return render.template.replace(/\{\{([a-zA-Z0-9_.-]+)\}\}/g, (_match, field: string) => renderValue(item, field));
}

interface ProjectionGroup {
  key: string;
  items: SelectedContent[];
}

function windowGroups(
  items: readonly SelectedContent[],
  unit: Extract<ProjectionSpec["unit"], { kind: "window" }>,
): ProjectionGroup[] {
  const stride = unit.stride ?? unit.size;
  if (!Number.isInteger(unit.size) || unit.size < 1) {
    throw new Error("Projection window size must be a positive integer");
  }
  if (!Number.isInteger(stride) || stride < 1) {
    throw new Error("Projection window stride must be a positive integer");
  }
  const windows: ProjectionGroup[] = [];
  for (let start = 0; start < items.length; start += stride) {
    const windowItems = items.slice(start, start + unit.size);
    if (windowItems.length < unit.size && unit.includePartial !== true) break;
    windows.push({ key: `window:${start}:${start + windowItems.length}`, items: windowItems });
  }
  return windows;
}

function groupValue(
  item: SelectedContent,
  by: Extract<ProjectionSpec["unit"], { kind: "group" }>["by"],
): unknown {
  if (by === "owner") return item.metadata.owner;
  if (by === "session") return item.metadata.session;
  if ("field" in by) return item.metadata.fields?.[by.field];
  const assignment = item.watermarks.find(candidate =>
    candidate.watermark.key === by.watermark.key
    && (by.watermark.namespace === undefined || candidate.watermark.namespace === by.watermark.namespace)
  );
  return assignment === undefined
    ? { present: false }
    : { present: true, value: assignment.watermark.value };
}

function groupedItems(
  items: readonly SelectedContent[],
  by: Extract<ProjectionSpec["unit"], { kind: "group" }>["by"],
): ProjectionGroup[] {
  const groups = new Map<string, SelectedContent[]>();
  for (const item of items) {
    const value = groupValue(item, by);
    const key = stableJson(value === undefined ? { present: false } : { present: true, value });
    const group = groups.get(key);
    if (group === undefined) groups.set(key, [item]);
    else group.push(item);
  }
  return [...groups].map(([key, groupItems]) => ({ key, items: groupItems }));
}

function groupSelection(selection: Selection, spec: ProjectionSpec): ProjectionGroup[] {
  if (spec.unit.kind === "item") {
    return selection.items.map(item => ({ key: contentRefKey(item.ref), items: [item] }));
  }
  if (spec.unit.kind === "whole-selection") {
    return selection.items.length === 0 ? [] : [{ key: "whole", items: [...selection.items] }];
  }
  if (spec.unit.kind === "window") return windowGroups(selection.items, spec.unit);
  return groupedItems(selection.items, spec.unit.by);
}

function applyCharacterLimit(text: string, spec: ProjectionSpec): string {
  if (spec.limit === undefined || !("maximumCharacters" in spec.limit)) return text;
  const { maximumCharacters, overflow } = spec.limit;
  if (!Number.isInteger(maximumCharacters) || maximumCharacters < 1) {
    throw new Error("Projection maximumCharacters must be a positive integer");
  }
  if (text.length <= maximumCharacters) return text;
  if (overflow === "reject") throw new Error(`Projection text exceeds ${maximumCharacters} characters`);
  return overflow === "truncate-start" ? text.slice(-maximumCharacters) : text.slice(0, maximumCharacters);
}

function sameModel(left: SemanticProjection["model"], right: SemanticProjection["model"]): boolean {
  return stableJson(left) === stableJson(right);
}

export function cosineSimilarity(left: Float32Array, right: Float32Array): number {
  if (left.length !== right.length) throw new Error("Cannot relate vectors with different dimensions");
  let dot = 0;
  let leftNorm = 0;
  let rightNorm = 0;
  for (let index = 0; index < left.length; index += 1) {
    const leftValue = left[index] ?? 0;
    const rightValue = right[index] ?? 0;
    dot += leftValue * rightValue;
    leftNorm += leftValue * leftValue;
    rightNorm += rightValue * rightValue;
  }
  if (leftNorm === 0 || rightNorm === 0) return 0;
  return dot / Math.sqrt(leftNorm * rightNorm);
}
function relationGroupKey(relation: SemanticRelation, groupBy: RelationReduction["groupBy"]): string {
  if (groupBy === "all") return "all";
  return groupBy === "left" ? relation.left : relation.right;
}

function reducedSimilarity(group: readonly SemanticRelation[], reduction: RelationReduction): number {
  if (reduction.kind === "max") return Math.max(...group.map(relation => relation.similarity));
  if (reduction.kind === "min") return Math.min(...group.map(relation => relation.similarity));
  if (reduction.kind === "mean") {
    return group.reduce((sum, relation) => sum + relation.similarity, 0) / group.length;
  }
  if (!("weight" in reduction)) throw new Error(`Unsupported relation reduction: ${reduction.kind}`);
  let weightedSimilarity = 0;
  let totalWeight = 0;
  for (const relation of group) {
    const weight = reduction.weight(relation);
    if (!Number.isFinite(weight) || weight < 0) {
      throw new Error("Relation reduction weights must be finite and non-negative");
    }
    weightedSimilarity += relation.similarity * weight;
    totalWeight += weight;
  }
  if (totalWeight === 0) throw new Error("Relation weighted-mean requires a positive total weight");
  return weightedSimilarity / totalWeight;
}

function reduceRelations(
  relations: readonly SemanticRelation[],
  reduction: RelationReduction | undefined,
): SemanticRelationReduction[] {
  if (reduction === undefined) return [];
  const groups = new Map<string, SemanticRelation[]>();
  for (const relation of relations) {
    const key = relationGroupKey(relation, reduction.groupBy);
    const group = groups.get(key);
    if (group === undefined) groups.set(key, [relation]);
    else group.push(relation);
  }
  return [...groups].map(([key, group]) => ({
    key,
    similarity: reducedSimilarity(group, reduction),
    count: group.length,
  }));
}

interface SelectionIndex {
  baseItems: Map<string, ContentItem>;
  candidateRefs: Map<string, ContentRef>;
  sourceOrder: Map<string, number>;
}

function indexSources(sourceSnapshots: readonly ContentSnapshot[]): SelectionIndex {
  const index: SelectionIndex = {
    baseItems: new Map(),
    candidateRefs: new Map(),
    sourceOrder: new Map(),
  };
  for (const snapshot of sourceSnapshots) {
    for (const item of snapshot.items) {
      validateRef(item.ref);
      if (item.ref.span !== undefined) throw new Error(`Content source ${item.ref.source} returned a partial item`);
      const baseKey = `${item.ref.source}\0${item.ref.id}`;
      if (index.baseItems.has(baseKey)) {
        throw new Error(`Duplicate content reference: ${item.ref.source}:${item.ref.id}`);
      }
      index.baseItems.set(baseKey, item);
      const key = contentRefKey(item.ref);
      index.candidateRefs.set(key, item.ref);
      index.sourceOrder.set(key, index.sourceOrder.size);
    }
  }
  return index;
}

function indexAssignments(
  index: SelectionIndex,
  assignments: readonly WatermarkAssignment[],
): Map<string, WatermarkAssignment[]> {
  const assignmentsByTarget = new Map<string, WatermarkAssignment[]>();
  for (const assignment of assignments) {
    const key = contentRefKey(assignment.target);
    if (!index.candidateRefs.has(key)) {
      index.candidateRefs.set(key, assignment.target);
      index.sourceOrder.set(key, index.sourceOrder.size);
    }
    const targetAssignments = assignmentsByTarget.get(key);
    if (targetAssignments === undefined) assignmentsByTarget.set(key, [assignment]);
    else targetAssignments.push(assignment);
  }
  return assignmentsByTarget;
}

function selectedItems(
  spec: SelectionSpec,
  index: SelectionIndex,
  assignmentsByTarget: ReadonlyMap<string, readonly WatermarkAssignment[]>,
): SelectedContent[] {
  const items: SelectedContent[] = [];
  for (const [key, ref] of index.candidateRefs) {
    const item = resolveCandidate(ref, index.baseItems);
    if (item === undefined || !matchesSourceFilters(item, spec)) continue;
    const assignments = assignmentsByTarget.get(key) ?? [];
    if (spec.where !== undefined && !expressionMatches(assignments, spec.where)) continue;
    if (spec.exclude !== undefined && expressionMatches(assignments, spec.exclude)) continue;
    items.push({ ...item, watermarks: assignments });
  }
  return items;
}


export class SemanticWorkspace implements SemanticPrimitives {
  readonly #sources;
  readonly #watermarks;
  readonly #embedder;
  readonly #now;
  constructor(options: {
    sources: readonly ContentSource[];
    watermarks: WatermarkStore;
    embedder: SemanticEmbedder;
    now?: () => number;
  }) {
    const sourceIds = new Set<string>();
    for (const source of options.sources) {
      if (sourceIds.has(source.id)) throw new Error(`Duplicate content source: ${source.id}`);
      sourceIds.add(source.id);
    }
    this.#sources = [...options.sources];
    this.#watermarks = options.watermarks;
    this.#embedder = options.embedder;
    this.#now = options.now ?? Date.now;
  }

  mark(change: WatermarkChange): Promise<MarkReceipt> {
    if (change.kind === "attach") {
      for (const target of change.targets) validateRef(target);
      for (const watermark of change.watermarks) validateWatermark(watermark);
    } else if (change.assignmentIds.some(id => id.trim() === "")) {
      throw new Error("Watermark assignment ids must be non-empty");
    }
    return this.#watermarks.apply(change);
  }

  async select(spec: SelectionSpec): Promise<Selection> {
    if (spec.limit !== undefined && (!Number.isInteger(spec.limit) || spec.limit < 0)) {
      throw new Error("Selection limit must be a non-negative integer");
    }
    if (spec.where !== undefined) validateExpression(spec.where);
    if (spec.exclude !== undefined) validateExpression(spec.exclude);
    const [sourceSnapshots, watermarkSnapshot] = await Promise.all([
      Promise.all(this.#sources.map(source => source.read())),
      this.#watermarks.read(),
    ]);
    return this.#selectFromSnapshots(spec, sourceSnapshots, watermarkSnapshot);
  }

  #selectFromSnapshots(
    spec: SelectionSpec,
    sourceSnapshots: readonly ContentSnapshot[],
    watermarkSnapshot: WatermarkSnapshot,
  ): Selection {
    const index = indexSources(sourceSnapshots);
    const selectionTime = this.#now();
    if (!Number.isFinite(selectionTime)) throw new Error("Selection time must be finite");
    const activeAssignments = watermarkSnapshot.assignments.filter(assignment => isActive(assignment, selectionTime));
    const assignmentsByTarget = indexAssignments(index, activeAssignments);
    const items = selectedItems(spec, index, assignmentsByTarget);
    sortSelection(items, spec.order ?? "source", index.sourceOrder);
    const limitedItems = spec.limit === undefined ? items : items.slice(0, spec.limit);
    const snapshot = stableJson({
      sources: sourceSnapshots.map(source => source.id),
      watermarks: watermarkSnapshot.revision,
    });
    return {
      snapshot,
      specificationHash: hash(spec),
      items: limitedItems,
    };
  }

  async project(selection: Selection, spec: ProjectionSpec): Promise<SemanticProjection> {
    if (
      spec.limit !== undefined &&
      "maximumCharacters" in spec.limit &&
      (!Number.isInteger(spec.limit.maximumCharacters) || spec.limit.maximumCharacters < 1)
    ) {
      throw new Error("Projection maximumCharacters must be a positive integer");
    }
    const groups = groupSelection(selection, spec);
    const unbounded = groups.map(group =>
      group.items.map(item => renderItem(item, spec.render)).join("\n"),
    );
    if (unbounded.some(text => text.trim() === "")) {
      throw new Error("Projection rendered an empty text");
    }
    let rendered = unbounded.map(text => applyCharacterLimit(text, spec));
    if (spec.limit !== undefined && "maximumTokens" in spec.limit) {
      const { maximumTokens, overflow } = spec.limit;
      if (!Number.isInteger(maximumTokens) || maximumTokens < 1) {
        throw new Error("Projection maximumTokens must be a positive integer");
      }
      if (this.#embedder.fitToTokenLimit === undefined) {
        throw new Error("The configured embedder does not support token limits");
      }
      rendered = [...await this.#embedder.fitToTokenLimit(rendered, maximumTokens, overflow)];
      if (rendered.length !== groups.length) {
        throw new Error(`Embedder fitted ${rendered.length} texts for ${groups.length} projection groups`);
      }
    }
    if (rendered.some(text => text.trim() === "")) {
      throw new Error("Projection token limit produced an empty text");
    }
    const model = this.#embedder.fingerprint();
    if (!Number.isInteger(model.dimensions) || model.dimensions < 1) {
      throw new Error("Embedder model dimensions must be a positive integer");
    }
    const embedded = await this.#embedder.embed(rendered);
    if (!sameModel(model, this.#embedder.fingerprint())) {
      throw new Error("Embedder model changed while projecting");
    }
    if (embedded.length !== rendered.length) {
      throw new Error(`Embedder returned ${embedded.length} vectors for ${rendered.length} texts`);
    }
    const projectionSpecificationHash = hash(spec);
    const projectionId = hash({
      selection: selection.snapshot,
      selectionSpecificationHash: selection.specificationHash,
      projectionSpecificationHash,
      model,
    });
    const vectors: ProjectedVector[] = embedded.map((values, index) => {
      if (values.length !== model.dimensions) {
        throw new Error(`Embedder returned ${values.length} dimensions; expected ${model.dimensions}`);
      }
      if (values.some(value => !Number.isFinite(value))) {
        throw new Error("Embedder returned a non-finite vector value");
      }
      const renderedText = rendered[index] ?? "";
      const group = groups[index];
      if (group === undefined) throw new Error("Projection group was missing");
      return {
        id: `${projectionId}:${index}`,
        values,
        members: group.items.map(item => item.ref),
        groupKey: group.key,
        renderedText,
        renderedHash: hash(renderedText),
      };
    });
    return {
      id: projectionId,
      vectors,
      model,
      lineage: {
        selectionSnapshot: selection.snapshot,
        selectionSpecificationHash: selection.specificationHash,
        projectionSpecificationHash,
      },
    };
  }

  async relate(
    left: SemanticProjection,
    right: SemanticProjection,
    spec: RelationSpec,
  ): Promise<SemanticRelationSet> {
    if (!sameModel(left.model, right.model)) throw new Error("Cannot relate projections from different models");
    if (spec.pairing.kind === "top-k" && (!Number.isInteger(spec.pairing.k) || spec.pairing.k < 1)) {
      throw new Error("Relation top-k must be a positive integer");
    }
    if (spec.minimumSimilarity !== undefined && (spec.minimumSimilarity < -1 || spec.minimumSimilarity > 1)) {
      throw new Error("Relation minimumSimilarity must be between -1 and 1");
    }

    const pairs: SemanticRelation[] = [];
    for (const leftVector of left.vectors) {
      const candidates = right.vectors
        .filter(rightVector =>
          (spec.pairing.kind !== "corresponding-groups" || leftVector.groupKey === rightVector.groupKey) &&
          (!spec.excludeSelf || leftVector.id !== rightVector.id),
        )
        .map(rightVector => ({
          left: leftVector.id,
          right: rightVector.id,
          similarity: Math.max(-1, Math.min(1, cosineSimilarity(leftVector.values, rightVector.values))),
          leftMembers: leftVector.members,
          rightMembers: rightVector.members,
          model: left.model,
        }))
        .filter(relation => spec.minimumSimilarity === undefined || relation.similarity >= spec.minimumSimilarity)
        .sort((first, second) => second.similarity - first.similarity || first.right.localeCompare(second.right));
      pairs.push(...(spec.pairing.kind === "top-k" ? candidates.slice(0, spec.pairing.k) : candidates));
    }
    return { pairs, reductions: reduceRelations(pairs, spec.reduction) };
  }
}
