import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { CODE_RULE_FORMAT } from "../code-rule/code-rule-schema.js";
import { parseJsonWithoutDuplicateKeys } from "../json-duplicate-keys.js";
import { corePackDirectories, loadBundledRules, shippedBundledRules } from "./bundled-rule-documents.js";
import type { BundledCatalog } from "./repository-model.js";
import { readRulePacks } from "./rule-packs.js";
import type { RuleDocument } from "./rule-store.js";
import { compareDeterministicStrings, ruleDocumentDigest } from "../stable-json.js";

/** Changes whenever a bundled document's content or any pack's `pack.json` changes. */
function catalogDigest(documents: readonly RuleDocument[], packDirectories: readonly string[]): string {
  const hash = createHash("sha256");
  const documentDigests = documents
    .map(document => [document.rule.id, ruleDocumentDigest(document)] as const)
    .sort(([left], [right]) => compareDeterministicStrings(left, right));
  for (const [id, digest] of documentDigests) hash.update(`rule\0${id}\0${digest}\n`);
  for (const directory of packDirectories) {
    hash.update(`pack\0${basename(directory)}\0`).update(readFileSync(join(directory, "pack.json"))).update("\n");
  }
  return `sha256:${hash.digest("hex")}`;
}

/**
 * Each code rule document in the packs' `rules/*.json`, by id, as written: its `$option` references unresolved.
 * The documents were validated when they were loaded; this reads them again as plain JSON.
 */
function ruleSources(packDirectories: readonly string[]): Map<string, unknown> {
  const sources = new Map<string, unknown>();
  for (const directory of packDirectories.map(pack => join(pack, "rules")).filter(existsSync)) {
    for (const name of readdirSync(directory).filter(file => file.endsWith(".json"))) {
      const path = join(directory, name);
      const value = parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path) as { format?: unknown; id?: unknown } | null;
      if (value?.format === CODE_RULE_FORMAT && typeof value.id === "string") sources.set(value.id, value);
    }
  }
  return sources;
}

/** The catalog of the packs in `packDirectories` (each a `packs/<id>/` directory), read from disk on every call. */
export function readBundledCatalog(packDirectories: readonly string[], documents: readonly RuleDocument[] = loadBundledRules(packDirectories)): BundledCatalog {
  const packs = readRulePacks(packDirectories);
  return {
    documents: new Map(documents.map(document => [document.rule.id, document])),
    packs: new Map(packs.map(pack => [pack.id, pack])),
    packOf: new Map(packs.flatMap(pack => pack.rules.map(rule => [rule.id, pack.id] as const))),
    sources: ruleSources(packDirectories),
    digest: catalogDigest(documents, packDirectories),
  };
}

let shippedCatalog: BundledCatalog | undefined;

/** The rules and packs that ship with Rerail, loaded once per process. */
export function loadBundledCatalog(): BundledCatalog {
  shippedCatalog ??= readBundledCatalog(corePackDirectories(), shippedBundledRules());
  return shippedCatalog;
}
