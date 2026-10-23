/**
 * Rule documents read from a pack directory. A leaf: the schema upgrade reads documents through it, so it imports
 * nothing that reaches the database or the runtime. Core ships no packs and no rules: every rule comes from an
 * installed pack (`rule-catalog.ts`).
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { parseJsonWithoutDuplicateKeys } from "../json-duplicate-keys.js";
import { codeRuleSchema, CODE_RULE_FORMAT } from "../code-rule/code-rule-schema.js";
import { conversationRuleSchema, reportedIssue } from "./rule-schema.js";
import type { RuleDocument } from "./rule-store.js";

/** A code rule (`format` is the code rule format) or a conversation rule; `label` names the source in errors. */
export function parseRuleDocument(value: unknown, label: string): RuleDocument {
  const isCodeRule = typeof value === "object" && value !== null && "format" in value && value.format === CODE_RULE_FORMAT;
  if (isCodeRule) return { kind: "code", rule: codeRuleSchema.parse(value) };
  const conversationRule = conversationRuleSchema.safeParse(value);
  if (conversationRule.success) return { kind: "conversation", rule: conversationRule.data };
  const issue = conversationRule.error.issues[0];
  throw new Error(`${label} is neither a ${CODE_RULE_FORMAT} document nor a conversation rule: ${issue === undefined ? "invalid" : reportedIssue(issue).message}`);
}

/** The JSON at `path`, without duplicate keys. */
export function readJsonDocument(path: string): unknown {
  return parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path);
}

/** The rule document at `path`: a code rule or a conversation rule, JSON without duplicate keys. */
export function readRuleDocument(path: string): RuleDocument {
  return parseRuleDocument(readJsonDocument(path), path);
}

/**
 * Every `*.json` rule document in each pack's `rules/` (in `packs` order, then by file name); ids must be unique across
 * all of them. The catalog reads only the documents a pack lists (`readRulePack`); this reads the directory, for the
 * pack tests and for tools that look at a pack checkout.
 */
export function loadPackRuleDocuments(packs: readonly string[]): RuleDocument[] {
  const seen = new Set<string>();
  return packs.flatMap(pack => {
    const directory = join(pack, "rules");
    if (!existsSync(directory)) return [];
    return readdirSync(directory, { withFileTypes: true })
      .filter(entry => entry.isFile() && entry.name.endsWith(".json"))
      .map(entry => entry.name)
      .sort()
      .map(name => {
        const path = join(directory, name);
        const document = readRuleDocument(path);
        if (seen.has(document.rule.id)) throw new Error(`Duplicate rule id: ${document.rule.id} (${path})`);
        seen.add(document.rule.id);
        return document;
      });
  });
}
