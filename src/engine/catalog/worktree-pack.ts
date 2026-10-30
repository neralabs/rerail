/**
 * A worktree's own rules: the rule documents in `<checkout>/.rerail/rules/`, read as one rule pack, `worktree`, that
 * only that checkout's catalog holds and that runs there for the whole checkout without being added
 * (docs/rule-packs.md, "A worktree's own rules"). Every other checkout, the same repository's included, never sees it.
 * The folder is read again whenever a file in it is added, removed or changed, so an edit applies on the next event.
 */
import { createHash } from "node:crypto";
import { lstatSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { ZodError } from "zod";
import { CodeRuleCompileError, EngineUnavailableError, storableCodeRule } from "../code-rule/code-rule-compile.js";
import { compareDeterministicStrings } from "../stable-json.js";
import type { RuleCatalog, SkippedRulePack } from "./repository-model.js";
import { catalogDigest } from "./rule-catalog.js";
import { parseRuleDocument, readJsonDocument } from "./rule-documents.js";
import { compareRulePacks, type LoadedRulePack, type RulePack, type RulePackRule } from "./rule-packs.js";
import type { RuleDocument } from "./rule-store.js";

/** Where a checkout keeps its own rules, relative to its top level. */
export const WORKTREE_RULES_FOLDER = ".rerail/rules";
/** The id of the pack a checkout's own rules form; settings name it as they name any pack. */
export const WORKTREE_PACK_ID = "worktree";
/**
 * The publisher of a checkout's own rules. No npm package can have it: a package's publisher is `rerail`, an `@scope`
 * or a name starting `rerail-pack-` (`rulePackPublisher`).
 */
export const WORKTREE_PUBLISHER = "worktree";
/** A rule document larger than this is left out unread, as a dependency manifest that large is when matching packs. */
const MAX_RULE_DOCUMENT_BYTES = 1024 * 1024;

/** Whether `pack` is a checkout's own rules rather than an installed pack. */
export function isWorktreePack(pack: Pick<RulePack, "publisher"> | undefined): boolean {
  return pack?.publisher === WORKTREE_PUBLISHER;
}

/** The folder holding the checkout's own rules. */
export function worktreeRulesDirectory(checkoutRoot: string): string {
  return join(checkoutRoot, WORKTREE_RULES_FOLDER);
}

interface RuleFile {
  readonly name: string;
  readonly path: string;
  /** Changes when the file is edited, replaced or becomes something other than a regular file. */
  readonly stamp: string;
  readonly regular: boolean;
  readonly size: number;
}

/**
 * The `.json` entries directly in the checkout's rules folder, by name; undefined when there is no such folder. A
 * symbolic link is never followed, for the folder or a file, so the rules a checkout runs are files inside it.
 */
function ruleFiles(checkoutRoot: string): readonly RuleFile[] | undefined {
  const parent = join(checkoutRoot, ".rerail");
  const directory = worktreeRulesDirectory(checkoutRoot);
  try {
    if (!lstatSync(parent).isDirectory() || !lstatSync(directory).isDirectory()) return undefined;
    return readdirSync(directory)
      .filter(name => name.endsWith(".json"))
      .sort(compareDeterministicStrings)
      .map(name => {
        const path = join(directory, name);
        const info = lstatSync(path);
        return { name, path, stamp: `${info.mtimeMs}:${info.size}:${info.mode}`, regular: info.isFile(), size: info.size };
      });
  } catch {
    return undefined;
  }
}

/** Why a document could not be read or parsed, in one line. */
function documentProblem(error: unknown): string {
  if (error instanceof ZodError) {
    const issue = error.issues[0];
    return `${issue?.path.join(".") ?? ""} ${issue?.message ?? "is not a rule document"}`.replace(/\s+/gu, " ").trim();
  }
  return error instanceof Error ? error.message : String(error);
}

/** The pack's entry for one of its rules: titled by its description, else its id; on, in its document's mode. */
function packRuleOf(document: RuleDocument): RulePackRule {
  const title = document.kind === "code" ? document.rule.description?.trim() || document.rule.id : document.rule.id;
  return { id: document.rule.id, kind: document.kind, title, group: "This worktree", mode: document.rule.mode, on: true };
}

/**
 * The checkout's rules folder read as a pack, beside the installed packs of `installed`. Each document is checked on
 * its own and one that fails is left out with why, while the others load: it must be a regular file of at most 1 MiB
 * holding a code or conversation rule, its id no other document in the folder (the first by file name keeps it) and
 * no installed pack has, and a code rule must compile, unless the engine refuses it only as `language-not-supported`.
 * No pack when no rule loads, or when an installed pack holds the id `worktree`.
 */
function readWorktreePack(installed: RuleCatalog, directory: string, files: readonly RuleFile[]): { loaded: LoadedRulePack | undefined; skipped: SkippedRulePack[] } {
  const skipped: SkippedRulePack[] = [];
  const leaveOut = (path: string, reason: string): void => {
    skipped.push({ path, packageName: null, version: null, reason });
  };
  if (files.length === 0) return { loaded: undefined, skipped };
  const holder = installed.packs.get(WORKTREE_PACK_ID);
  if (holder !== undefined) {
    leaveOut(directory, `pack id ${WORKTREE_PACK_ID} is taken by ${holder.packageName ?? holder.id}`);
    return { loaded: undefined, skipped };
  }
  const documents: RuleDocument[] = [];
  const sources = new Map<string, unknown>();
  const fileOf = new Map<string, string>();
  let compile = true;
  for (const file of files) {
    if (!file.regular) {
      leaveOut(file.path, "it is not a regular file");
      continue;
    }
    if (file.size > MAX_RULE_DOCUMENT_BYTES) {
      leaveOut(file.path, "it is larger than 1 MiB");
      continue;
    }
    let source: unknown;
    let document: RuleDocument;
    try {
      source = readJsonDocument(file.path);
      document = parseRuleDocument(source, file.path);
    } catch (error) {
      leaveOut(file.path, documentProblem(error));
      continue;
    }
    const id = document.rule.id;
    const earlier = fileOf.get(id);
    if (earlier !== undefined) {
      leaveOut(file.path, `rule ${id} is already in ${earlier}`);
      continue;
    }
    const packId = installed.packOf.get(id);
    if (installed.documents.has(id)) {
      const pack = packId === undefined ? undefined : installed.packs.get(packId);
      leaveOut(file.path, `rule ${id} is already in the installed pack ${pack?.packageName ?? packId ?? "of another rule"}`);
      continue;
    }
    if (compile && document.kind === "code") {
      try {
        storableCodeRule(document.rule);
      } catch (error) {
        if (error instanceof EngineUnavailableError) {
          compile = false;
        } else if (error instanceof CodeRuleCompileError) {
          leaveOut(file.path, `this Rerail cannot run rule ${id}: ${error.reason}`);
          continue;
        } else {
          throw error;
        }
      }
    }
    fileOf.set(id, file.name);
    documents.push(document);
    if (document.kind === "code") sources.set(id, source);
  }
  if (documents.length === 0) return { loaded: undefined, skipped };
  const pack: RulePack = {
    id: WORKTREE_PACK_ID,
    name: "This worktree",
    version: null,
    packageName: null,
    publisher: WORKTREE_PUBLISHER,
    category: "repository",
    covers: `The rules in ${WORKTREE_RULES_FOLDER} of this worktree, which run here only.`,
    reason: `has rules in ${WORKTREE_RULES_FOLDER}`,
    match: { any: [{ files: [`${WORKTREE_RULES_FOLDER}/*.json`], label: WORKTREE_RULES_FOLDER }] },
    honourSuppressionComments: true,
    rules: documents.map(packRuleOf),
  };
  return { loaded: { pack, packageJson: undefined, documents, sources, packJson: { ...pack, rules: [...pack.rules] } }, skipped };
}

/** `installed` with the checkout's own pack added and the rule documents it left out reported in `skipped`. */
function withWorktreePack(installed: RuleCatalog, loaded: LoadedRulePack | undefined, skipped: readonly SkippedRulePack[]): RuleCatalog {
  if (loaded === undefined) return skipped.length === 0 ? installed : { ...installed, skipped: [...installed.skipped, ...skipped] };
  const ids = loaded.documents.map(document => document.rule.id);
  return {
    documents: new Map([...installed.documents, ...loaded.documents.map(document => [document.rule.id, document] as const)]),
    packs: new Map([...installed.packs.values(), loaded.pack].sort(compareRulePacks).map(pack => [pack.id, pack] as const)),
    packOf: new Map([...installed.packOf, ...ids.map(id => [id, WORKTREE_PACK_ID] as const)]),
    sources: new Map([...installed.sources, ...loaded.sources]),
    digest: `sha256:${createHash("sha256").update(`${installed.digest}\0${catalogDigest([loaded])}`).digest("hex")}`,
    skipped: [...installed.skipped, ...skipped],
  };
}

/** Each checkout's catalog as last built, with the installed catalog and the folder stamp it was built from. */
const worktreeCatalogs = new Map<string, { readonly installed: RuleCatalog; readonly stamp: string; readonly catalog: RuleCatalog }>();

/**
 * The catalog of the checkout whose top level is `checkoutRoot`: `installed` plus the checkout's own rules
 * (`.rerail/rules`), or `installed` itself when the checkout has no such folder. The same object is returned until
 * `installed` or a file in the folder changes, so what is derived from a catalog (configured rules) is reused.
 */
export function worktreeRuleCatalog(installed: RuleCatalog, checkoutRoot: string): RuleCatalog {
  const files = ruleFiles(checkoutRoot);
  if (files === undefined) {
    worktreeCatalogs.delete(checkoutRoot);
    return installed;
  }
  const stamp = files.map(file => `${file.name}\0${file.stamp}`).join("\n");
  const cached = worktreeCatalogs.get(checkoutRoot);
  if (cached !== undefined && cached.installed === installed && cached.stamp === stamp) return cached.catalog;
  const { loaded, skipped } = readWorktreePack(installed, worktreeRulesDirectory(checkoutRoot), files);
  const catalog = withWorktreePack(installed, loaded, skipped);
  worktreeCatalogs.set(checkoutRoot, { installed, stamp, catalog });
  return catalog;
}
