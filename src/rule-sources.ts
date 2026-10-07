import { existsSync, readFileSync } from "node:fs";
import { isAbsolute, resolve } from "node:path";
import { parseJsonWithoutDuplicateKeys } from "./json-duplicate-keys.js";
import { parseCosineRuleDocument, type CosineRule } from "./rule-schema.js";

export interface CosineRuleSourceOptions {
  /** Rules supplied by the embedding host; lowest precedence. */
  rules?: readonly CosineRule[];
  /** JSON rule documents; every listed file must exist. Unset falls back to `defaultFile` when present. */
  ruleFiles?: readonly string[];
}

export interface LegacyCosineRuleSource {
  path: string;
  rules: readonly CosineRule[];
}

/** Resolve legacy host and JSON inputs so callers can import each source once. */
export function loadLegacyCosineRuleSources(
  options: CosineRuleSourceOptions,
  cwd: string,
  defaultFile: string,
): LegacyCosineRuleSource[] {
  const sources: LegacyCosineRuleSource[] = [];
  if (options.rules !== undefined) {
    sources.push({ path: resolve(cwd, ".rerail", "embedded-cosine-rules"), rules: options.rules });
  }
  const configuredPaths = options.ruleFiles ?? [defaultFile];
  for (const configuredPath of configuredPaths) {
    const path = isAbsolute(configuredPath) ? configuredPath : resolve(cwd, configuredPath);
    if (!existsSync(path)) {
      if (options.ruleFiles === undefined) continue;
      throw new Error(`cosine rule file does not exist: ${path}`);
    }
    sources.push({ path, rules: parseCosineRuleDocument(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path)) });
  }
  return sources;
}

/** What the project database says about cosine rules: the enabled rows, and every id it holds at all. */
export interface DatabaseCosineRules {
  enabled: readonly CosineRule[];
  /** Ids of every `cosine` row, disabled ones included: a file rule with one of these ids never shows through. */
  ids: ReadonlySet<string>;
}

/**
 * Assemble the cosine rule set for a session: host-supplied rules, then JSON
 * files, then database rows. Later sources replace earlier ones by id so the
 * database stays the source of truth while legacy JSON files keep working. The
 * database owns every id it has a row for: a disabled row means "off", not
 * "fall back to the file copy".
 */
export function loadCosineRules(
  options: CosineRuleSourceOptions,
  cwd: string,
  defaultFile: string,
  database: DatabaseCosineRules,
): CosineRule[] {
  const merged = new Map<string, CosineRule>();
  for (const source of loadLegacyCosineRuleSources(options, cwd, defaultFile)) {
    for (const rule of source.rules) merged.set(rule.id, rule);
  }
  for (const id of database.ids) merged.delete(id);
  for (const rule of database.enabled) merged.set(rule.id, rule);
  return [...merged.values()];
}
