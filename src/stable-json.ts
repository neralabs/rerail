import { createHash } from "node:crypto";
import type { RuleDocument } from "./rule-store.js";

/** A JSON-representable value. */
export type Json =
  | null
  | boolean
  | number
  | string
  | readonly Json[]
  | { readonly [key: string]: Json };

function compareKeys(left: string, right: string): number {
  let leftIndex = 0;
  let rightIndex = 0;
  while (leftIndex < left.length && rightIndex < right.length) {
    const leftCodePoint = left.codePointAt(leftIndex)!;
    const rightCodePoint = right.codePointAt(rightIndex)!;
    if (leftCodePoint !== rightCodePoint) return leftCodePoint - rightCodePoint;
    leftIndex += leftCodePoint > 0xffff ? 2 : 1;
    rightIndex += rightCodePoint > 0xffff ? 2 : 1;
  }
  if (leftIndex !== left.length) return 1;
  return rightIndex === right.length ? 0 : -1;
}

/** Code-unit ordering: stable across hosts and locales, unlike `String.prototype.localeCompare`. */
export function compareDeterministicStrings(left: string, right: string): number {
  if (left === right) return 0;
  return left < right ? -1 : 1;
}

function sortJsonValue(value: unknown, ancestors: Set<object>): unknown {
  if (value === null || typeof value !== "object") return value;
  if (ancestors.has(value)) throw new TypeError("Cannot serialize a cyclic value");

  ancestors.add(value);
  try {
    if (Array.isArray(value)) return value.map(entry => sortJsonValue(entry, ancestors));
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => compareKeys(left, right))
        .map(([key, entry]) => [key, sortJsonValue(entry, ancestors)]),
    );
  } finally {
    ancestors.delete(value);
  }
}

/** JSON.stringify with recursively key-sorted objects; undefined means the value is not representable as JSON. */
export function stableJsonText(value: unknown): string | undefined {
  try {
    const json = JSON.stringify(value);
    if (json === undefined) return undefined;
    return JSON.stringify(sortJsonValue(JSON.parse(json), new Set()));
  } catch {
    return undefined;
  }
}

/** Content digest of a rule document; equal digests mean the kernel would compile identical rules. */
export function ruleDocumentDigest(document: RuleDocument): string {
  const canonical = stableJsonText({ kind: document.kind, rule: document.rule });
  if (canonical === undefined) throw new TypeError(`rule ${document.rule.id} is not JSON representable`);
  return `sha256:${createHash("sha256").update(canonical).digest("hex")}`;
}
