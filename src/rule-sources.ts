import { existsSync, readFileSync } from "node:fs";
import { isAbsolute, resolve } from "node:path";
import { parseJsonWithoutDuplicateKeys } from "./json-duplicate-keys.js";
import { parseConversationRuleDocument, type ConversationRule } from "./rule-schema.js";

export interface ConversationRuleSourceOptions {
  /** Rules supplied by the embedding host; lowest precedence. */
  rules?: readonly ConversationRule[];
  /** JSON rule documents; every listed file must exist. Unset falls back to `defaultFile` when present. */
  ruleFiles?: readonly string[];
}

/** The conversation rules a checkout's configuration runs, and every conversation rule id it configures at all. */
export interface DatabaseConversationRules {
  enabled: readonly ConversationRule[];
  /** Ids of every configured conversation rule, off ones included: a JSON-file rule with one of these ids never shows through. */
  ids: ReadonlySet<string>;
}

/** Merge a checkout's configured conversation rules with whatever else the host supplies (JSON files, options). */
export type ConversationRuleLoader = (configured: DatabaseConversationRules) => readonly ConversationRule[];

/**
 * The conversation rule set for a session. Host-supplied rules, then JSON files, are read once, here, at
 * session start; a later source replaces an earlier rule with the same id. The checkout's configured rules
 * are layered on top every time the returned loader runs, so the configuration stays the source of truth:
 * it owns every id it configures, and a rule that is off means "off", not "fall back to the file copy".
 */
export function conversationRuleLoader(options: ConversationRuleSourceOptions, cwd: string, defaultFile: string): ConversationRuleLoader {
  const sourced = new Map<string, ConversationRule>();
  for (const rule of options.rules ?? []) sourced.set(rule.id, rule);
  for (const configuredPath of options.ruleFiles ?? [defaultFile]) {
    const path = isAbsolute(configuredPath) ? configuredPath : resolve(cwd, configuredPath);
    if (!existsSync(path)) {
      if (options.ruleFiles === undefined) continue;
      throw new Error(`cosine rule file does not exist: ${path}`);
    }
    for (const rule of parseConversationRuleDocument(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path))) {
      sourced.set(rule.id, rule);
    }
  }
  return database => {
    const merged = new Map(sourced);
    for (const id of database.ids) merged.delete(id);
    for (const rule of database.enabled) merged.set(rule.id, rule);
    return [...merged.values()];
  };
}
