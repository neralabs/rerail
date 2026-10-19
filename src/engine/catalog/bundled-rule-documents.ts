/**
 * The rule documents that ship with Rerail, read from the package. A leaf: the schema upgrade reads the bundled ids
 * through it, so it imports nothing that reaches the database or the runtime.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { parseJsonWithoutDuplicateKeys } from "../json-duplicate-keys.js";
import { codeRuleSchema, CODE_RULE_FORMAT } from "../code-rule/code-rule-schema.js";
import { PACKAGE_ROOT } from "../package-root.js";
import { conversationRuleSchema } from "./rule-schema.js";
import type { RuleDocument } from "./rule-store.js";

/**
 * The rule packs core ships inside its own package (`package.json` `files`) until it loads installed packs: the
 * complexity pack alone. Each is a `packs/<id>/` directory in the pack layout (`docs/pack-format.md`): its `rules/`
 * documents are By Rerail rules, and its `pack.json` is read by `src/engine/catalog/rule-packs.ts`. Core has no rules
 * of its own: every rule it ships is in a pack.
 */
// NER-1979 study build only: every pack in the repository loads, as it will once installed packs load (NER-1753, #140).
export const CORE_RULE_PACKS = ["complexity", "blade", "javascript", "next", "node", "php", "python", "react", "typescript"] as const;

/** `<package>/packs/<id>` for each pack core ships. */
export function corePackDirectories(): string[] {
  return CORE_RULE_PACKS.map(id => join(PACKAGE_ROOT, "packs", id));
}

/** A code rule (`format` is the code rule format) or a conversation rule; `label` names the source in errors. */
export function parseRuleDocument(value: unknown, label: string): RuleDocument {
  const isCodeRule = typeof value === "object" && value !== null && "format" in value && value.format === CODE_RULE_FORMAT;
  if (isCodeRule) return { kind: "code", rule: codeRuleSchema.parse(value) };
  const conversationRule = conversationRuleSchema.safeParse(value);
  if (conversationRule.success) return { kind: "conversation", rule: conversationRule.data };
  throw new Error(`${label} is neither a ${CODE_RULE_FORMAT} document nor a conversation rule: ${conversationRule.error.issues[0]?.message ?? "invalid"}`);
}

/** The rule document at `path`: a code rule or a conversation rule, JSON without duplicate keys. */
export function readRuleDocument(path: string): RuleDocument {
  return parseRuleDocument(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path), path);
}

/** Every `*.json` rule document directly in `directory`, sorted by file name; none when it does not exist. */
function readRuleDirectory(directory: string): { path: string; document: RuleDocument }[] {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith(".json"))
    .map(entry => entry.name)
    .sort()
    .map(name => ({ path: join(directory, name), document: readRuleDocument(join(directory, name)) }));
}

/**
 * Each pack's `rules/*.json` (in `packs` order, then by file name); ids must be unique across all of them. Reads the
 * files on every call; {@link shippedBundledRules} is the cached shipped set.
 */
export function loadBundledRules(packs: readonly string[] = corePackDirectories()): RuleDocument[] {
  const entries = packs.flatMap(pack => readRuleDirectory(join(pack, "rules")));
  const seen = new Set<string>();
  return entries.map(({ path, document }) => {
    if (seen.has(document.rule.id)) throw new Error(`Duplicate bundled rule id: ${document.rule.id} (${path})`);
    seen.add(document.rule.id);
    return document;
  });
}

let shippedDocuments: readonly RuleDocument[] | undefined;

/** The shipped bundled documents, read once per process. */
export function shippedBundledRules(): readonly RuleDocument[] {
  shippedDocuments ??= Object.freeze(loadBundledRules());
  return shippedDocuments;
}
