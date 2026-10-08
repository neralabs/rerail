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

/** A rule file in the shape written before rule modes: valid then, so it is skipped rather than fatal. */
export class RetiredRuleFileError extends Error {}

/** A rule document whose rules still carry `action.delivery` and no `mode`. */
function predatesRuleModes(document: unknown): boolean {
  const rules = typeof document === "object" && document !== null ? (document as { rules?: unknown }).rules : undefined;
  return Array.isArray(rules) && rules.some(rule => {
    if (typeof rule !== "object" || rule === null || "mode" in rule) return false;
    const action = (rule as { action?: unknown }).action;
    return typeof action === "object" && action !== null && "delivery" in action;
  });
}

/** Parse a JSON conversation rule file; one written before rule modes throws {@link RetiredRuleFileError}. */
export function parseConversationRuleFile(text: string, path: string): ConversationRule[] {
  const document = parseJsonWithoutDuplicateKeys(text, path);
  if (predatesRuleModes(document)) {
    throw new RetiredRuleFileError("it was written before rule modes; give each rule a mode (log, steer, or block) instead of action.delivery");
  }
  return parseConversationRuleDocument(document);
}

export interface LegacyConversationRuleSource {
  path: string;
  /** Reads the rules on call, so a source the catalog already imported is never parsed again. */
  rules: () => readonly ConversationRule[];
}

/** Resolve legacy host and JSON inputs so callers can import each source once. */
export function loadLegacyConversationRuleSources(
  options: ConversationRuleSourceOptions,
  cwd: string,
  defaultFile: string,
): LegacyConversationRuleSource[] {
  const sources: LegacyConversationRuleSource[] = [];
  if (options.rules !== undefined) {
    const embedded = options.rules;
    sources.push({ path: resolve(cwd, ".rerail", "embedded-cosine-rules"), rules: () => embedded });
  }
  const configuredPaths = options.ruleFiles ?? [defaultFile];
  for (const configuredPath of configuredPaths) {
    const path = isAbsolute(configuredPath) ? configuredPath : resolve(cwd, configuredPath);
    if (!existsSync(path)) {
      if (options.ruleFiles === undefined) continue;
      throw new Error(`cosine rule file does not exist: ${path}`);
    }
    sources.push({ path, rules: () => parseConversationRuleFile(readFileSync(path, "utf8"), path) });
  }
  return sources;
}

/** What the project database says about conversation rules: the enabled rows, and every id it holds at all. */
export interface DatabaseConversationRules {
  enabled: readonly ConversationRule[];
  /** Ids of every `conversation` row, disabled ones included: a JSON-file rule with one of these ids never shows through. */
  ids: ReadonlySet<string>;
}

/**
 * Assemble the conversation rule set for a session: host-supplied rules, then JSON
 * files, then database rows. Later sources replace earlier ones by id so the
 * database stays the source of truth while legacy JSON files keep working. The
 * database owns every id it has a row for: a disabled row means "off", not
 * "fall back to the file copy".
 */
export function loadConversationRules(
  options: ConversationRuleSourceOptions,
  cwd: string,
  defaultFile: string,
  database: DatabaseConversationRules,
): ConversationRule[] {
  const merged = new Map<string, ConversationRule>();
  for (const source of loadLegacyConversationRuleSources(options, cwd, defaultFile)) {
    for (const rule of source.rules()) merged.set(rule.id, rule);
  }
  for (const id of database.ids) merged.delete(id);
  for (const rule of database.enabled) merged.set(rule.id, rule);
  return [...merged.values()];
}
