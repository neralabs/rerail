/**
 * The engine `loads` judges a rule document on: this checkout's own, its TypeScript modules imported from
 * src/engine/index.ts and its native binding loaded the way the rest of conformance/ loads it. `loads` therefore means
 * "compiles and loads on this commit's engine"; nothing is pinned, fetched or built here.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import {
  bundledCodeRuleDslDirectory,
  codeRuleSchema,
  loadPackRuleDocuments,
  loadNativeBinding,
  parseJsonWithoutDuplicateKeys,
  stableJsonText,
} from "../../../src/engine/index.js";

interface ZodLike {
  safeParse(value: unknown): { success: true; data: unknown } | { success: false; error: { issues: readonly { path: readonly PropertyKey[]; message: string }[] } };
}

/** The rerail modules `loads` calls, from src/engine/index.ts. */
export interface RerailModules {
  findDuplicateJsonKeys(text: string): { pointer: string; key: string }[];
  codeRuleSchema: ZodLike;
  stableJsonText(value: unknown): string | undefined;
  /** Each pack directory's `rules/*.json`, read as rerail reads the packs it loads. */
  loadPackRuleDocuments(packs: readonly string[]): { kind: string; rule: { id: string } }[];
}

export interface CompiledPolicy {
  canonicalJson: string;
  inScope(path: string): boolean;
}

export interface Engine {
  rerail: RerailModules;
  compile(text: string): CompiledPolicy;
}

/**
 * Every repeated object key in a JSON text, as src/engine's `findDuplicateJsonKeys` reports it: read back from the
 * error `parseJsonWithoutDuplicateKeys` (exported by src/engine/index.ts) names them in, until index.ts exports
 * `findDuplicateJsonKeys` itself. Only the first repeat is read back, which is all `loads` reports; text that is not
 * JSON throws, as there.
 */
function findDuplicateJsonKeys(text: string): { pointer: string; key: string }[] {
  const label = "document";
  try {
    parseJsonWithoutDuplicateKeys(text, label);
    return [];
  } catch (error) {
    if (!(error instanceof Error) || error instanceof SyntaxError || !error.message.startsWith(`${label}: duplicate key \``)) throw error;
    const first = error.message.slice(`${label}: `.length).split("; duplicate key `")[0]!;
    const match = /^duplicate key `([\s\S]*)` at ([\s\S]*)$/u.exec(first);
    return [match === null ? { pointer: "", key: first } : { key: match[1]!, pointer: match[2] === "/" ? "" : match[2]! }];
  }
}

/** This checkout's engine, the one `loads` judges every rule on. */
export function loadEngine(): Engine {
  const runtime = loadNativeBinding().CodeRuleDslRuntime;
  const capabilities = readFileSync(join(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
  return {
    rerail: { findDuplicateJsonKeys, codeRuleSchema, stableJsonText, loadPackRuleDocuments },
    compile(text) {
      const compiled = runtime.compile(text, "json", capabilities);
      return { canonicalJson: compiled.canonicalJson(), inScope: (path) => compiled.inScope(path) };
    },
  };
}
