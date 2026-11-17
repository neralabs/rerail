/**
 * A flat config's values as JSON, read where they are literal: through same-file `const` bindings, TypeScript's type
 * wrappers and spreads, as JavaScript builds them; anything else is a value Rerail does not run, and it says why. A
 * budget shared by the whole reading of one file bounds the work a small file can make it do.
 */
import type * as t from "@babel/types";
import { LintConfigFormatError } from "../formats/lint-config-formats.js";
import { dataProperty, MAX_DEPTH, type FlatConfigFile } from "./eslint-flat-config-file.js";

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
type Unresolved = { readonly ok: false; readonly why: string };
export type Resolved = { readonly ok: true; readonly value: Json } | Unresolved;

/**
 * How much work reading one file may take: the reader steps into a value (`ReadBudget.step`) and builds JSON values
 * (`ReadBudget.built`). A const named twice (`const b = [a, a]`), a choice with it on both sides, or an array spread
 * twice doubles both at each level, so 40 such levels in a small file would take 2^40 steps. A config written by hand
 * takes far less: 1,000 rules set to `["error", { ... }]` with ten options each take 13,000 steps and build 12,000
 * values.
 */
const MAX_STEPS = 100_000;
const MAX_VALUES = 100_000;

/** How much work reading one file has taken. */
export class ReadBudget {
  #steps = 0;
  #values = 0;

  /** One step into a value; throws once the file takes more than `MAX_STEPS`. */
  step(): void {
    this.#steps += 1;
    if (this.#steps > MAX_STEPS) throw new LintConfigFormatError(`it takes more than ${MAX_STEPS} steps to read`);
  }

  /** `count` more JSON values built; throws once the file builds more than `MAX_VALUES`. */
  built(count: number): void {
    this.#values += count;
    if (this.#values > MAX_VALUES) throw new LintConfigFormatError(`it builds more than ${MAX_VALUES} values`);
  }
}

/** Reads a file's values as JSON, each step and value built counted against the reading's budget. */
export class FlatConfigValues {
  readonly #file: FlatConfigFile;
  readonly #budget: ReadBudget;

  constructor(file: FlatConfigFile, budget: ReadBudget) {
    this.#file = file;
    this.#budget = budget;
  }

  /** `node` as JSON where it is literal, through same-file `const` bindings and spreads; why not otherwise. */
  resolve(node: t.Expression, depth: number): Resolved {
    this.#budget.step();
    if (depth > MAX_DEPTH) return { ok: false, why: "it nests too deep" };
    const value = this.#file.unwrap(node, depth);
    switch (value.type) {
      case "StringLiteral":
      case "NumericLiteral":
      case "BooleanLiteral":
        return { ok: true, value: value.value };
      case "NullLiteral":
        return { ok: true, value: null };
      case "TemplateLiteral":
        return this.#resolveTemplate(value);
      case "UnaryExpression":
        return this.#resolveUnary(value);
      case "ArrayExpression":
        return this.#resolveArray(value, depth);
      case "ObjectExpression":
        return this.#resolveObject(value, depth);
      default:
        return this.#unreadValue(value);
    }
  }

  #unreadValue(value: t.Expression): Unresolved {
    return { ok: false, why: `it is ${this.#file.source(value)}, which Rerail does not run` };
  }

  /** A template literal without `${…}` is its one string; an untagged template's escapes are valid, so it has one. */
  #resolveTemplate(value: t.TemplateLiteral): Resolved {
    return value.expressions.length === 0 ? { ok: true, value: value.quasis[0]!.value.cooked! } : this.#unreadValue(value);
  }

  #resolveUnary(value: t.UnaryExpression): Resolved {
    if ((value.operator !== "-" && value.operator !== "+") || value.argument.type !== "NumericLiteral") return this.#unreadValue(value);
    return { ok: true, value: value.operator === "-" ? -value.argument.value : value.argument.value };
  }

  #resolveArray(value: t.ArrayExpression, depth: number): Resolved {
    const items: Json[] = [];
    for (const element of value.elements) {
      const added = this.#elementItems(value, element, depth);
      if (!added.ok) return added;
      this.#budget.built(added.value.length);
      for (const item of added.value) items.push(item);
    }
    return { ok: true, value: items };
  }

  /** What one element adds to an array: its value, or the items of a spread array. */
  #elementItems(array: t.ArrayExpression, element: t.ArrayExpression["elements"][number], depth: number): { readonly ok: true; readonly value: Json[] } | Unresolved {
    if (element === null) return this.#unreadValue(array);
    if (element.type !== "SpreadElement") {
      const inner = this.resolve(element, depth + 1);
      return inner.ok ? { ok: true, value: [inner.value] } : inner;
    }
    const inner = this.resolve(element.argument, depth + 1);
    if (!inner.ok) return inner;
    return Array.isArray(inner.value) ? { ok: true, value: inner.value } : this.#unreadValue(array);
  }

  #resolveObject(value: t.ObjectExpression, depth: number): Resolved {
    const object: { [key: string]: Json } = {};
    for (const property of value.properties) {
      const unresolved = property.type === "SpreadElement"
        ? this.#spreadInto(object, value, property, depth)
        : this.#propertyInto(object, property, depth);
      if (unresolved !== undefined) return unresolved;
    }
    return { ok: true, value: object };
  }

  #spreadInto(object: { [key: string]: Json }, value: t.ObjectExpression, property: t.SpreadElement, depth: number): Unresolved | undefined {
    const inner = this.resolve(property.argument, depth + 1);
    if (!inner.ok) return inner;
    if (inner.value === null || typeof inner.value !== "object" || Array.isArray(inner.value)) return this.#unreadValue(value);
    this.#budget.built(Object.keys(inner.value).length);
    Object.assign(object, inner.value);
    return undefined;
  }

  #propertyInto(object: { [key: string]: Json }, property: t.ObjectProperty | t.ObjectMethod, depth: number): Unresolved | undefined {
    const [key, node] = dataProperty(property);
    const inner = this.resolve(node, depth + 1);
    if (!inner.ok) return inner;
    this.#budget.built(1);
    Object.defineProperty(object, key, { value: inner.value, enumerable: true, writable: true, configurable: true });
    return undefined;
  }
}
