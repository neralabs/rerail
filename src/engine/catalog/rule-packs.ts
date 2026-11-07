import { existsSync } from "node:fs";
import { basename, join } from "node:path";
import { z } from "zod";
import { loadRuleCatalog } from "./rule-catalog.js";
import { parseRuleDocument, readJsonDocument } from "./rule-documents.js";
import {
  DEPENDENCY_ECOSYSTEMS,
  globPattern,
  matchFolder,
  matchReason,
  scanRepository,
  type RulePackMatch,
  type RulePackSignal,
} from "./rule-pack-matcher.js";
import type { RuleDocument } from "./rule-store.js";
import { RULE_MODES, type RuleMode } from "./rule-mode.js";

export type { RulePackMatch, RulePackSignal } from "./rule-pack-matcher.js";

/**
 * Rule packs: named sets of rules, each its own npm package (`@neralabs/rerail-pack-<id>`), installed into the Rerail
 * home and read by `rule-catalog.ts`. A pack's match definition says which repositories it fits and why; a pack
 * without a rule that starts on is never recommended. A pack is a directory (`docs/pack-format.md`): `pack.json` lists
 * each rule with the document in the pack's `rules/` that defines it. A pack's rules run only in repositories that add
 * it (`repository_attachments_set`); this module defines, validates and recommends.
 */
export const RULE_PACK_FORMAT = "rerail.rule-pack/v1" as const;
export const RULE_PACK_CATEGORIES = ["language", "framework", "repository"] as const;
export type RulePackCategory = (typeof RULE_PACK_CATEGORIES)[number];

const nonEmpty = z.string().refine(value => value.trim().length > 0, "must be a non-empty string");

const exampleSchema = z
  .object({
    /** The file a code rule's example is read as; conversation rule examples have none. */
    path: nonEmpty.optional(),
    /** File content for a code rule; the command a coding agent runs for a conversation rule. */
    text: nonEmpty,
  })
  .strict();

/**
 * A pack rule's id: lowercase letters, digits and dashes, one style in every pack (`eslint-no-empty`,
 * `ruff-b011-assert-false`). It is also the name of the rule's document, `rules/<id>.json`, and its port's id in
 * `conformance.json`. Users turn rules on and off by it, so it must not change once the pack is published.
 */
export const RULE_PACK_RULE_ID = /^[a-z0-9][a-z0-9-]*$/u;

/**
 * A rule's code-rule document, relative to the pack directory: `rules/<id>.json`. Only a plain name: a `\` or `..`
 * would reach outside the pack on Windows, where `\` separates folders.
 */
const documentPathSchema = z.string().regex(/^rules\/[a-z0-9][a-z0-9-]*\.json$/u, "must be rules/<id>.json");

const packRuleFields = {
  id: z.string().regex(RULE_PACK_RULE_ID, "must be lowercase letters, digits and dashes"),
  title: nonEmpty,
  group: nonEmpty,
  /** The rule mode the pack starts the rule in; default: the mode its document has. */
  mode: z.enum(RULE_MODES).optional(),
  /** Whether the rule is on when its pack is added; default on. */
  on: z.boolean().optional(),
  /** One example the rule catches and one it lets pass, for the rule panel; tests prove both. */
  examples: z.object({ caught: exampleSchema, passes: exampleSchema }).strict().optional(),
};

const signalSchema: z.ZodType<RulePackSignal> = z.union([
  z.object({ files: z.array(nonEmpty).min(1), exclude: z.array(nonEmpty).optional(), label: nonEmpty }).strict(),
  z.object({ dependencies: z.object({ ecosystem: z.enum(DEPENDENCY_ECOSYSTEMS), names: z.array(nonEmpty).min(1) }).strict() }).strict(),
]);

const matchSchema: z.ZodType<RulePackMatch> = z.object({ any: z.array(signalSchema).min(1) }).strict();

/**
 * `pack.json`: every rule names its document, so the pack is self-contained, and every pack says which repositories
 * it fits. A pack may list no rules while all of its ports are candidates; such a pack is never recommended.
 */
export const rulePackFileSchema = z
  .object({
    format: z.literal(RULE_PACK_FORMAT),
    id: z.string().regex(/^[a-z][a-z0-9-]*$/u, "must be lowercase letters, digits and dashes"),
    name: nonEmpty,
    category: z.enum(RULE_PACK_CATEGORIES),
    covers: nonEmpty,
    reason: nonEmpty,
    /** Which repositories the pack fits: set-up runs it against a repository to recommend the pack. */
    match: matchSchema,
    /** Whether the pack's rules honour their linters' suppression comments (`# noqa`, `eslint-disable`); default yes. */
    honourSuppressionComments: z.boolean().optional(),
    rules: z.array(z.object({ ...packRuleFields, document: documentPathSchema }).strict()),
  })
  .strict();
export type RulePackFile = z.infer<typeof rulePackFileSchema>;

/** The npm package name of pack `<id>`. */
export const rulePackPackageName = (id: string): string => `@neralabs/rerail-pack-${id}`;

/**
 * `package.json`: the pack's npm package. Core's version range is a peer dependency, a declaration and not an import;
 * the pack contract (`docs/pack-format.md`) adds that the name matches the pack, that there are no `dependencies`,
 * that `files` publishes no validation material, and that `license` agrees with `licenses/`.
 */
export const rulePackPackageSchema = z.looseObject({
  name: z.string().regex(/^@neralabs\/rerail-pack-[a-z][a-z0-9-]*$/u, "must be @neralabs/rerail-pack-<id>"),
  version: z.string().regex(/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/u, "must be a semver version"),
  description: nonEmpty.optional(),
  /** An SPDX expression naming every licence the published files carry. */
  license: nonEmpty,
  files: z.array(nonEmpty).min(1),
  peerDependencies: z.looseObject({ "@neralabs/rerail": nonEmpty }),
  publishConfig: z.looseObject({ access: z.enum(["restricted", "public"]) }).optional(),
});
export type RulePackPackage = z.infer<typeof rulePackPackageSchema>;

export interface RuleExample {
  readonly path?: string | undefined;
  readonly text: string;
}

export interface RulePackRule {
  readonly id: string;
  /** A code rule or a conversation rule. */
  readonly kind: RuleDocument["kind"];
  /** Plain-language name, e.g. "Use === and !== in TypeScript". */
  readonly title: string;
  /** The smaller group the rule is filed under inside its pack, e.g. "Complexity" or "api". */
  readonly group: string;
  /** The rule mode it starts in when its pack is added; the seeded document carries it. */
  readonly mode: RuleMode;
  /** Whether the rule is on when its pack is added. */
  readonly on: boolean;
  /** One example the rule catches and one it lets pass, where the pack has them; tests prove both. */
  readonly examples?: { readonly caught: RuleExample; readonly passes: RuleExample };
}

export interface RulePack {
  readonly id: string;
  readonly name: string;
  /** The version of the pack's npm package; null for a pack directory without one. */
  readonly version: string | null;
  readonly category: RulePackCategory;
  /** What the pack covers, in one sentence. */
  readonly covers: string;
  /** What makes the pack fit, in general terms; the reason shown for a repository is built from what matched. */
  readonly reason: string;
  /** Which repositories the pack fits. A `check` signal's path is absolute here. */
  readonly match: RulePackMatch;
  /** Whether the pack's rules honour their linters' suppression comments. When false, a comment silences nothing. */
  readonly honourSuppressionComments: boolean;
  readonly rules: readonly RulePackRule[];
}

/** A pack read from its directory, with everything the catalog takes from it. */
export interface LoadedRulePack {
  readonly pack: RulePack;
  /** The npm package: name, version and core range; undefined for a pack directory without `package.json`. */
  readonly packageJson: RulePackPackage | undefined;
  /** Each listed rule's document, in listing order. */
  readonly documents: readonly RuleDocument[];
  /** Each listed code rule's document as written, by id: its `$option` references unresolved. */
  readonly sources: ReadonlyMap<string, unknown>;
  /** `pack.json` as read, for the catalog's digest. */
  readonly packJson: unknown;
}

export interface RulePackRecommendation {
  readonly pack: RulePack;
  /** The rules on when the pack is added; the rules it lists off are not counted. */
  readonly ruleCount: number;
  /** What was found in this repository, e.g. "matches the react dependency in web/package.json". */
  readonly reason: string;
  /**
   * Where the pack is added, relative to the repository (`''` is the whole repository): the folder holding all of
   * what matched, or the whole repository for a pack of category `repository`, which covers it all.
   */
  readonly folder: string;
}

const CATEGORY_ORDER: Record<RulePackCategory, number> = { language: 0, framework: 1, repository: 2 };

/** Packs listed languages first, then frameworks, then repository packs, each by name. */
export function compareRulePacks(left: RulePack, right: RulePack): number {
  return CATEGORY_ORDER[left.category] - CATEGORY_ORDER[right.category] || left.name.localeCompare(right.name);
}

/** The pack's npm package, checked against `pack.json`'s id; undefined when the directory has no `package.json`. */
function readPackageJson(directory: string, id: string): RulePackPackage | undefined {
  const path = join(directory, "package.json");
  if (!existsSync(path)) {
    if (id !== basename(directory)) throw new Error(`Rule pack ${id} must be stored in packs/${id}/, not ${basename(directory)}/`);
    return undefined;
  }
  const parsed = rulePackPackageSchema.safeParse(readJsonDocument(path));
  if (!parsed.success) throw new Error(`Rule pack ${id}: package.json ${parsed.error.issues[0]?.path.join(".") ?? ""} ${parsed.error.issues[0]?.message ?? "is invalid"}`.replace(/\s+/gu, " "));
  if (parsed.data.name !== rulePackPackageName(id)) throw new Error(`Rule pack ${id} must be the package ${rulePackPackageName(id)}, not ${parsed.data.name}`);
  return parsed.data;
}

/** The pack's match, checked: throws on a malformed glob, naming the pack. */
function checkedMatch(id: string, match: RulePackMatch): RulePackMatch {
  for (const signal of match.any) {
    if (!("files" in signal)) continue;
    try {
      globPattern([...signal.files, ...(signal.exclude ?? [])]);
    } catch (error) {
      throw new Error(`Rule pack ${id}: ${(error as Error).message}`);
    }
  }
  return match;
}

/** One listed rule read and checked: its document is named for its id, exists and carries that id. */
function readPackRule(directory: string, id: string, rule: RulePackFile["rules"][number]): { rule: RulePackRule; document: RuleDocument; source: unknown } {
  const documentPath = join(directory, rule.document);
  if (basename(rule.document, ".json") !== rule.id) throw new Error(`Rule pack ${id} lists ${rule.id}, whose document must be rules/${rule.id}.json, not ${rule.document}`);
  if (!existsSync(documentPath)) throw new Error(`Rule pack ${id} lists ${rule.id}, whose document ${rule.document} does not exist`);
  const source = readJsonDocument(documentPath);
  const document = parseRuleDocument(source, documentPath);
  if (document.rule.id !== rule.id) throw new Error(`Rule pack ${id} lists ${rule.id}, but ${rule.document} has id ${document.rule.id}`);
  const examples = rule.examples;
  if (examples !== undefined && document.kind === "code" && (examples.caught.path === undefined || examples.passes.path === undefined)) {
    throw new Error(`Code rule ${rule.id} in rule pack ${id} needs a path on both examples`);
  }
  if (examples !== undefined && document.kind === "conversation" && (examples.caught.path !== undefined || examples.passes.path !== undefined)) {
    throw new Error(`Conversation rule ${rule.id} in rule pack ${id} takes no example path`);
  }
  // The mode and on state a pack gives its rules take effect only once packs are attached.
  const packRule: RulePackRule = { id: rule.id, kind: document.kind, title: rule.title, group: rule.group, mode: rule.mode ?? document.rule.mode, on: rule.on ?? true, ...(examples === undefined ? {} : { examples }) };
  return { rule: packRule, document, source };
}

/**
 * The pack in `directory`, validated: `pack.json` parses, the npm package (when there is a `package.json`) is the
 * pack's, or else the directory is named for it, every rule's document is named for its id, exists and carries that
 * id, and its globs compile. Throws an error naming the pack on the first problem.
 */
export function readRulePack(directory: string): LoadedRulePack {
  const path = join(directory, "pack.json");
  if (!existsSync(path)) throw new Error(`${directory} has no pack.json`);
  const packJson = readJsonDocument(path);
  const parsed = rulePackFileSchema.safeParse(packJson);
  if (!parsed.success) {
    const issue = parsed.error.issues[0];
    throw new Error(`${path}: ${issue?.path.join(".") ?? ""} ${issue?.message ?? "is invalid"}`.replace(/\s+/gu, " "));
  }
  const file = parsed.data;
  const packageJson = readPackageJson(directory, file.id);
  const match = checkedMatch(file.id, file.match);
  const rules = file.rules.map(rule => readPackRule(directory, file.id, rule));
  const pack: RulePack = {
    id: file.id,
    name: file.name,
    version: packageJson?.version ?? null,
    category: file.category,
    covers: file.covers,
    reason: file.reason,
    match,
    // Like each rule's mode and on state, this takes effect only once packs are attached.
    honourSuppressionComments: file.honourSuppressionComments ?? true,
    rules: rules.map(({ rule }) => rule),
  };
  const sources = new Map(rules.flatMap(({ document, source }) => (document.kind === "code" ? [[document.rule.id, source] as const] : [])));
  return { pack, packageJson, documents: rules.map(({ document }) => document), sources, packJson };
}

/**
 * The packs in `packs` (each a pack directory), every one valid (`readRulePack`) and no rule in two packs; throws on
 * the first problem. Languages first, then frameworks, then repository packs. For the pack tests and tools; the
 * catalog reads installed packs one by one and skips an invalid one (`readRuleCatalog`).
 */
export function readRulePacks(packs: readonly string[]): readonly RulePack[] {
  const owner = new Map<string, string>();
  return packs
    .map(directory => {
      const { pack } = readRulePack(directory);
      for (const rule of pack.rules) {
        const previous = owner.get(rule.id);
        if (previous !== undefined) throw new Error(`Rule ${rule.id} is in both rule packs ${previous} and ${pack.id}`);
        owner.set(rule.id, pack.id);
      }
      return pack;
    })
    .sort(compareRulePacks);
}

/** The installed packs: languages, then frameworks, then repository packs. */
export function loadRulePacks(): readonly RulePack[] {
  return [...loadRuleCatalog().packs.values()];
}

/** The id of the pack a rule belongs to, or null for rules in no pack (created by you). */
export function rulePackIdOf(ruleId: string, packs: readonly RulePack[] = loadRulePacks()): string | null {
  return packs.find(pack => pack.rules.some(rule => rule.id === ruleId))?.id ?? null;
}

/**
 * The packs that fit the repository at `repositoryRoot`, from its files and dependency manifests: no model, no
 * network, no code from the packs, and no list of what does not fit. A pack without a rule that starts on never fits.
 * Languages come first, then frameworks, then repository packs. Each comes with the folder it belongs in (see
 * `RulePackRecommendation.folder`).
 */
export function recommendRulePacks(repositoryRoot: string, packs: readonly RulePack[] = loadRulePacks()): RulePackRecommendation[] {
  const scan = scanRepository(repositoryRoot);
  return packs.flatMap(pack => {
    const ruleCount = pack.rules.filter(rule => rule.on).length;
    if (ruleCount === 0) return [];
    const reason = matchReason(pack.match, scan);
    if (reason === undefined) return [];
    const folder = pack.category === "repository" ? "" : matchFolder(pack.match, scan);
    return [{ pack, ruleCount, reason, folder }];
  });
}

/** How long a daemon reuses a repository's recommendations before scanning it again. */
export const RECOMMENDATION_CACHE_MS = 60_000;
const recommendationCache = new Map<string, { at: number; digest: string; recommendations: RulePackRecommendation[] }>();

/**
 * `recommendRulePacks` over the installed packs, for the daemon, which answers synchronously: the scan blocks other
 * requests while it runs (about 0.3 to 0.5 s on a large repository), so a repository is scanned at most once per
 * `RECOMMENDATION_CACHE_MS` while the installed packs stay the same. A file added or removed within that window shows on
 * the next scan; a pack installed or removed shows at once.
 */
export function cachedRecommendRulePacks(repositoryRoot: string, now = Date.now()): RulePackRecommendation[] {
  const catalog = loadRuleCatalog();
  const cached = recommendationCache.get(repositoryRoot);
  if (cached !== undefined && cached.digest === catalog.digest && now - cached.at < RECOMMENDATION_CACHE_MS) return cached.recommendations;
  const recommendations = recommendRulePacks(repositoryRoot, [...catalog.packs.values()]);
  recommendationCache.set(repositoryRoot, { at: now, digest: catalog.digest, recommendations });
  return recommendations;
}
