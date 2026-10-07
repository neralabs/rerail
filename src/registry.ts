import {
  loadNativeBinding,
  type ModelFingerprint,
  type NativeCosineKernel,
  type NativeRuleInput,
  type NativeSample,
} from "./native.js";
import { isSimilarityMatch, parseCosineRule, type CosineRule, type MatchOperator } from "./rule-schema.js";

export interface CosineRuleRegistryOptions {
  modelPath: string;
  maxInputTokens?: number;
  maxRules?: number;
}

/** Similarity facts are present only on a `similarity` match; a structural match computes none. */
export interface RuleMatch {
  ruleId: string;
  kind: "similarity" | "structural";
  similarity?: number;
  threshold?: number;
  operator?: MatchOperator;
  tokenCount?: number;
  action: CosineRule["action"];
}

export interface RuleInspection {
  rule: CosineRule;
  lastDeliveredTurn?: number;
}

export interface MatchOptions {
  turn?: number;
  respectRepeat?: boolean;
}

function toNative(rule: CosineRule): NativeRuleInput {
  const base = { id: rule.id, source: rule.sample.source, boundary: rule.sample.boundary, minimumTokens: rule.sample.minimumTokens ?? 1 };
  if (!isSimilarityMatch(rule.match)) return { ...base, conditions: JSON.stringify(rule.match) };
  return { ...base, reference: rule.reference, operator: rule.match.operator, threshold: rule.match.threshold };
}


export class CosineRuleRegistry {
  readonly #kernel: NativeCosineKernel;
  readonly #maxRules: number;
  #rules = new Map<string, CosineRule>();
  readonly #lastDeliveredTurn = new Map<string, number>();

  constructor(options: CosineRuleRegistryOptions) {
    const maxInputTokens = options.maxInputTokens ?? 256;
    this.#maxRules = options.maxRules ?? 4_096;
    if (!Number.isInteger(this.#maxRules) || this.#maxRules <= 0) {
      throw new RangeError("maxRules must be a positive integer");
    }
    this.#kernel = loadNativeBinding().CosineKernel.load(options.modelPath, maxInputTokens);
  }

  fingerprint(): ModelFingerprint {
    return this.#kernel.fingerprint();
  }

  get count(): number {
    return this.#rules.size;
  }

  replace(input: readonly unknown[]): void {
    if (input.length > this.#maxRules) {
      throw new RangeError(`rule count exceeds configured maximum ${this.#maxRules}`);
    }
    const parsed = input.map((value, index) => parseCosineRule(value, `rules[${index}]`));
    const next = new Map<string, CosineRule>();
    for (const rule of parsed) {
      if (next.has(rule.id)) throw new TypeError(`duplicate rule id: ${rule.id}`);
      next.set(rule.id, rule);
    }

    this.#kernel.replaceRules(parsed.map(toNative));
    this.#rules = next;
    for (const id of this.#lastDeliveredTurn.keys()) {
      if (!next.has(id)) this.#lastDeliveredTurn.delete(id);
    }
  }

  upsert(input: unknown, options: { replace?: boolean } = {}): CosineRule {
    const rule = parseCosineRule(input);
    if (this.#rules.has(rule.id) && options.replace !== true) {
      throw new Error(`rule ${rule.id} already exists; pass replace: true`);
    }
    if (!this.#rules.has(rule.id) && this.#rules.size >= this.#maxRules) {
      throw new RangeError(`rule count exceeds configured maximum ${this.#maxRules}`);
    }

    this.#kernel.upsertRules([toNative(rule)]);
    this.#rules.set(rule.id, rule);
    return structuredClone(rule);
  }

  /**
   * Drop rules from matching. A rule that is merely switched off (and may come back)
   * keeps its repeat state with `forgetDelivery: false`, so re-enabling a `once` rule
   * that already fired does not fire it again.
   */
  remove(ids: readonly string[], options: { forgetDelivery?: boolean } = {}): number {
    const uniqueIds = [...new Set(ids)];
    const removed = this.#kernel.removeRules(uniqueIds);
    for (const id of uniqueIds) {
      this.#rules.delete(id);
      if (options.forgetDelivery !== false) this.#lastDeliveredTurn.delete(id);
    }
    return removed;
  }

  match(sample: NativeSample, options: MatchOptions = {}): RuleMatch[] {
    const turn = options.turn ?? 0;
    const respectRepeat = options.respectRepeat ?? true;
    return this.#kernel
      .matchText(sample)
      .filter(match => {
        const rule = this.#rules.get(match.ruleId);
        return rule !== undefined && (!respectRepeat || this.#eligible(rule, turn));
      })
      .map(match => {
        const rule = this.#rules.get(match.ruleId);
        if (rule === undefined) throw new Error(`native matcher returned unknown rule ${match.ruleId}`);
        return {
          ...match,
          action: { ...rule.action },
        };
      });
  }

  markDelivered(ruleIds: readonly string[], turn: number): void {
    if (!Number.isInteger(turn) || turn < 0) throw new RangeError("turn must be a non-negative integer");
    for (const id of ruleIds) {
      if (!this.#rules.has(id)) throw new Error(`cannot deliver unknown rule ${id}`);
      this.#lastDeliveredTurn.set(id, turn);
    }
  }

  restoreDelivery(ruleId: string, turn: number): void {
    if (this.#rules.has(ruleId)) this.#lastDeliveredTurn.set(ruleId, turn);
  }

  inspect(): RuleInspection[] {
    return [...this.#rules.values()]
      .sort((left, right) => left.id.localeCompare(right.id))
      .map(rule => {
        const lastDeliveredTurn = this.#lastDeliveredTurn.get(rule.id);
        return {
          rule: structuredClone(rule),
          ...(lastDeliveredTurn === undefined ? {} : { lastDeliveredTurn }),
        };
      });
  }

  #eligible(rule: CosineRule, turn: number): boolean {
    const repeat = rule.repeat ?? { mode: "once" as const };
    if (repeat.mode === "always") return true;
    const previous = this.#lastDeliveredTurn.get(rule.id);
    if (previous === undefined) return true;
    if (repeat.mode === "once") return false;
    return turn - previous >= repeat.gapTurns;
  }
}
