import { createHash } from "node:crypto";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { shippedBundledRules } from "./bundled-rule-documents.js";
import type { BundledCatalog } from "./repository-model.js";
import { readRulePacks, rulePacksDirectory } from "./rule-packs.js";
import type { RuleDocument } from "./rule-store.js";
import { compareDeterministicStrings, ruleDocumentDigest } from "./stable-json.js";

/** Changes whenever a bundled document's content or any pack file changes. */
function catalogDigest(documents: readonly RuleDocument[], packsDirectory: string): string {
  const hash = createHash("sha256");
  const documentDigests = documents
    .map(document => [document.rule.id, ruleDocumentDigest(document)] as const)
    .sort(([left], [right]) => compareDeterministicStrings(left, right));
  for (const [id, digest] of documentDigests) hash.update(`rule\0${id}\0${digest}\n`);
  const packFiles = readdirSync(packsDirectory, { withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith(".json"))
    .map(entry => entry.name)
    .sort();
  for (const name of packFiles) hash.update(`pack\0${name}\0`).update(readFileSync(join(packsDirectory, name))).update("\n");
  return `sha256:${hash.digest("hex")}`;
}

let shippedCatalog: BundledCatalog | undefined;

/** The rules and packs that ship with Rerail, loaded once per process. */
export function loadBundledCatalog(): BundledCatalog {
  if (shippedCatalog !== undefined) return shippedCatalog;
  const documents = shippedBundledRules();
  const packsDirectory = rulePacksDirectory();
  const packs = readRulePacks(packsDirectory);
  shippedCatalog = {
    documents: new Map(documents.map(document => [document.rule.id, document])),
    packs: new Map(packs.map(pack => [pack.id, pack])),
    packOf: new Map(packs.flatMap(pack => pack.rules.map(rule => [rule.id, pack.id] as const))),
    digest: catalogDigest(documents, packsDirectory),
  };
  return shippedCatalog;
}
