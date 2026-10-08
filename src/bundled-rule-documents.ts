/**
 * The rule documents that ship with Rerail, read from the package. A leaf: the schema upgrade reads the bundled ids
 * through it, so it imports nothing that reaches the database or the runtime.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { parseJsonWithoutDuplicateKeys } from "./json-duplicate-keys.js";
import { codeRuleSchema, CODE_RULE_FORMAT } from "./code-rule-schema.js";
import { conversationRuleSchema } from "./rule-schema.js";
import type { RuleDocument } from "./rule-store.js";

/** `<package>/code-rule-dsl/rules`: the documents that ship with Rerail. */
export function bundledRulesDirectory(): string {
  return resolve(dirname(fileURLToPath(import.meta.url)), "..", "code-rule-dsl", "rules");
}

/** A file rule (`format` is the code rule format) or a conversation rule; `label` names the source in errors. */
export function parseRuleDocument(value: unknown, label: string): RuleDocument {
  const isCodeRule = typeof value === "object" && value !== null && "format" in value && value.format === CODE_RULE_FORMAT;
  if (isCodeRule) return { kind: "file", rule: codeRuleSchema.parse(value) };
  const conversationRule = conversationRuleSchema.safeParse(value);
  if (conversationRule.success) return { kind: "conversation", rule: conversationRule.data };
  throw new Error(`${label} is neither a ${CODE_RULE_FORMAT} document nor a conversation rule: ${conversationRule.error.issues[0]?.message ?? "invalid"}`);
}

/**
 * Subdirectory of the bundled rules holding opt-in packs, one folder per pack (`opt-in/<pack>/*.json`, e.g. the TSLint
 * ports in `opt-in/tslint/`). Their documents are ordinary bundled rules; only the pack files decide where they apply.
 */
export const OPT_IN_RULES_DIRECTORY = "opt-in";

function readRuleDirectory(directory: string): { path: string; document: RuleDocument }[] {
  return readdirSync(directory, { withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith(".json"))
    .map(entry => entry.name)
    .sort()
    .map(name => {
      const path = join(directory, name);
      return { path, document: parseRuleDocument(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path), path) };
    });
}

/**
 * Every opt-in pack's documents, by pack name then file name. Hidden entries (`.DS_Store`, `.hidden-pack/`) and non-JSON
 * files directly in `opt-in/` (a README) are ignored; a `*.json` file there belongs to no pack and is an error.
 */
function readOptInPacks(directory: string): { path: string; document: RuleDocument }[] {
  if (!existsSync(directory)) return [];
  const entries = readdirSync(directory, { withFileTypes: true }).filter(entry => !entry.name.startsWith("."));
  const loose = entries
    .filter(entry => !entry.isDirectory() && entry.name.endsWith(".json"))
    .map(entry => join(directory, entry.name));
  if (loose.length > 0) throw new Error(`Opt-in rules must live in a pack folder (${OPT_IN_RULES_DIRECTORY}/<pack>/*.json): ${loose.join(", ")}`);
  return entries
    .filter(entry => entry.isDirectory())
    .map(entry => entry.name)
    .sort()
    .flatMap(pack => readRuleDirectory(join(directory, pack)));
}

/**
 * Every `*.json` rule document in a directory, sorted by file name, then those in each `opt-in/<pack>/` folder
 * (by pack, then file name); ids must be unique across all of them. Reads the files on every call;
 * {@link shippedBundledRules} is the cached shipped set.
 */
export function loadBundledRules(directory = bundledRulesDirectory()): RuleDocument[] {
  const entries = [...readRuleDirectory(directory), ...readOptInPacks(join(directory, OPT_IN_RULES_DIRECTORY))];
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
