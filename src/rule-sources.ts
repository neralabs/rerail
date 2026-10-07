import { existsSync, readFileSync } from "node:fs";
import { isAbsolute, resolve } from "node:path";
import type { CosineRuleLoader } from "./cosine-rule-sync.js";
import { parseJsonWithoutDuplicateKeys } from "./json-duplicate-keys.js";
import { parseCosineRuleDocument, type CosineRule } from "./rule-schema.js";

export interface CosineRuleSourceOptions {
  /** Rules supplied by the embedding host; lowest precedence. */
  rules?: readonly CosineRule[];
  /** JSON rule documents; every listed file must exist. Unset falls back to `defaultFile` when present. */
  ruleFiles?: readonly string[];
}

/** What the project database says about cosine rules: the enabled rows, and every id it holds at all. */
export interface DatabaseCosineRules {
  enabled: readonly CosineRule[];
  /** Ids of every `conversation` row, disabled ones included: a JSON-file rule with one of these ids never shows through. */
  ids: ReadonlySet<string>;
}

/**
 * The cosine rule set for a session. Host-supplied rules, then JSON files, are read once, here, at
 * session start; a later source replaces an earlier rule with the same id. Database rows are layered
 * on top every time the returned loader runs, so the database stays the source of truth: it owns
 * every id it has a row for, and a disabled row means "off", not "fall back to the file copy".
 */
export function cosineRuleLoader(options: CosineRuleSourceOptions, cwd: string, defaultFile: string): CosineRuleLoader {
  const sourced = new Map<string, CosineRule>();
  for (const rule of options.rules ?? []) sourced.set(rule.id, rule);
  for (const configuredPath of options.ruleFiles ?? [defaultFile]) {
    const path = isAbsolute(configuredPath) ? configuredPath : resolve(cwd, configuredPath);
    if (!existsSync(path)) {
      if (options.ruleFiles === undefined) continue;
      throw new Error(`cosine rule file does not exist: ${path}`);
    }
    for (const rule of parseCosineRuleDocument(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path))) {
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
