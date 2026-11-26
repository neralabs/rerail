import { createHash } from "node:crypto";
import type { RuleDocument } from "./catalog/rule-store.js";

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

/**
 * What a rule document's digest covers: a JSON rule's body, and for a `.rerail` rule its id, its text and the bytes of
 * every file it includes (their SHA-256 each), wherever it is installed, so that a change to an included file, core's
 * library after an update among them, moves the digest.
 */
function digested(document: RuleDocument): unknown {
  if (document.kind !== "rerail") return { kind: document.kind, rule: document.rule };
  const { id, source, includes } = document.rule;
  return { kind: document.kind, rule: { id, source, includes: includes.map(file => file.sha256).sort(compareDeterministicStrings) } };
}

/**
 * Each rule document's digest, by the document: a document is never changed once read, and the daemon's one rule
 * catalog hands the same pack documents to every checkout's catalog, which digests each at every build.
 */
const documentDigests = new WeakMap<RuleDocument, string>();

/** Content digest of a rule document; equal digests mean the engine would compile identical rules. */
export function ruleDocumentDigest(document: RuleDocument): string {
  const known = documentDigests.get(document);
  if (known !== undefined) return known;
  const canonical = stableJsonText(digested(document));
  if (canonical === undefined) throw new TypeError(`rule ${document.rule.id} is not JSON representable`);
  const digest = `sha256:${createHash("sha256").update(canonical).digest("hex")}`;
  documentDigests.set(document, digest);
  return digest;
}

/**
 * The stable JSON text of a rule (`stableJsonText`) and its digest, by the rule: a rule is never changed once read, and
 * every checkout running a rule at one mode is handed one object for it (`runningDocument`), so every checkout's catalog
 * builds from the same rules.
 */
const ruleSources = new WeakMap<object, { readonly text: string; readonly digest: string }>();

/** {@link stableJsonText} of `rule` and the sha256 of that text, worked out once per rule; undefined when not JSON. */
export function ruleSourceText(rule: object): { readonly text: string; readonly digest: string } | undefined {
  const known = ruleSources.get(rule);
  if (known !== undefined) return known;
  const text = stableJsonText(rule);
  if (text === undefined) return undefined;
  const source = { text, digest: `sha256:${createHash("sha256").update(text).digest("hex")}` };
  ruleSources.set(rule, source);
  return source;
}
