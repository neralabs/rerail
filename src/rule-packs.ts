import { existsSync, readFileSync } from "node:fs";
import { basename, join } from "node:path";
import { z } from "zod";
import { parseJsonWithoutDuplicateKeys } from "./json-duplicate-keys.js";
import { corePackDirectories, readRuleDocument } from "./bundled-rule-documents.js";
import { loadBundledCatalog } from "./bundled-rules.js";
import { globPattern, matchFolder, matchReason, scanRepository, type RulePackMatch, type RulePackSignal } from "./rule-pack-matcher.js";
import type { RuleDocument } from "./rule-store.js";
import { RULE_MODES, type RuleMode } from "./rule-mode.js";

export type { RulePackMatch, RulePackSignal } from "./rule-pack-matcher.js";

/**
 * Rule packs: named sets of rules that come with Rerail. A pack's optional match definition says which repositories
 * it fits and why; a pack without one is never recommended and is chosen by hand. A pack is a `packs/<id>/` directory
 * (`docs/pack-format.md`): `pack.json` lists each rule with the document in the pack's `rules/` that defines it. Core
 * ships the packs in `CORE_RULE_PACKS` inside its own package. A pack's rules run only in repositories that add it
 * (`repository_attachments_set`); this module defines, validates and recommends.
 */
export const RULE_PACK_FORMAT = "rerail.rule-pack/v1" as const;
export const RULE_PACK_CATEGORIES = ["language", "framework", "repository"] as const;
export type RulePackCategory = (typeof RULE_PACK_CATEGORIES)[number];

const nonEmpty = z.string().refine(value => value.trim().length > 0, "must be a non-empty string");

const exampleSchema = z
  .object({
    /** The file a file rule's example is read as; conversation rule examples have none. */
    path: nonEmpty.optional(),
    /** File content for a file rule; the command a coding agent runs for a conversation rule. */
    text: nonEmpty,
  })
  .strict();

/** A rule's code-rule document, relative to the pack directory: `rules/<name>.json`. */
const documentPathSchema = z.string().regex(/^rules\/[^/]+\.json$/u, "must be rules/<name>.json");

const packRuleFields = {
  id: nonEmpty,
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
  z.object({ npmDependencies: z.array(nonEmpty).min(1) }).strict(),
]);

const matchSchema: z.ZodType<RulePackMatch> = z.object({ any: z.array(signalSchema).min(1) }).strict();

/** `packs/<id>/pack.json`: every rule names its document, so the pack is self-contained. */
export const rulePackFileSchema = z
  .object({
    format: z.literal(RULE_PACK_FORMAT),
    id: z.string().regex(/^[a-z][a-z0-9-]*$/u, "must be lowercase letters, digits and dashes"),
    name: nonEmpty,
    category: z.enum(RULE_PACK_CATEGORIES),
    covers: nonEmpty,
    reason: nonEmpty,
    /** Which repositories the pack fits; without it the pack is never recommended. */
    match: matchSchema.optional(),
    /** Whether the pack's rules honour their linters' suppression comments (`# noqa`, `eslint-disable`); default yes. */
    honourSuppressionComments: z.boolean().optional(),
    rules: z.array(z.object({ ...packRuleFields, document: documentPathSchema }).strict()).min(1),
  })
  .strict();
export type RulePackFile = z.infer<typeof rulePackFileSchema>;

/** The npm package name of pack `<id>`. */
export const rulePackPackageName = (id: string): string => `@neralabs/rerail-pack-${id}`;

/**
 * `packs/<id>/package.json`: the pack's npm package. Core's version range is a peer dependency, a declaration and not
 * an import; the pack contract (`docs/pack-format.md`) adds that the name matches the pack, that there are no
 * `dependencies`, that `files` publishes no validation material, and that `license` agrees with `licenses/`.
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
  /** A file rule or a conversation rule. */
  readonly kind: RuleDocument["kind"];
  /** Plain-language name, e.g. "Use === and !== in TypeScript". */
  readonly title: string;
  /** The smaller group the rule is filed under inside its pack, e.g. "TSLint" or "api". */
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
  readonly category: RulePackCategory;
  /** What the pack covers, in one sentence. */
  readonly covers: string;
  /** What makes the pack fit, in general terms; the reason shown for a repository is built from what matched. */
  readonly reason: string;
  /** Which repositories the pack fits; a pack without one is never recommended. */
  readonly match?: RulePackMatch;
  /** Whether the pack's rules honour their linters' suppression comments. When false, a comment silences nothing. */
  readonly honourSuppressionComments: boolean;
  readonly rules: readonly RulePackRule[];
}

export interface RulePackRecommendation {
  readonly pack: RulePack;
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

/**
 * The packs in `packs` (each a `packs/<id>/` directory), validated: `pack.json` parses and names its directory, every
 * rule's document exists and carries its id, and no rule is in two packs.
 */
export function readRulePacks(packs: readonly string[] = corePackDirectories()): readonly RulePack[] {
  const owner = new Map<string, string>();
  return packs
    .map(directory => {
      const path = join(directory, "pack.json");
      const file = rulePackFileSchema.parse(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path));
      if (file.id !== basename(directory)) throw new Error(`Rule pack ${file.id} must be stored in packs/${file.id}/, not ${basename(directory)}/`);
      for (const signal of file.match?.any ?? []) {
        if (!("files" in signal)) continue;
        try {
          globPattern([...signal.files, ...(signal.exclude ?? [])]);
        } catch (error) {
          throw new Error(`Rule pack ${file.id}: ${(error as Error).message}`);
        }
      }
      // Like each rule's mode and on state, this takes effect only once packs are attached.
      const honourSuppressionComments = file.honourSuppressionComments ?? true;
      const rules = file.rules.map((rule): RulePackRule => {
        const documentPath = join(directory, rule.document);
        if (!existsSync(documentPath)) throw new Error(`Rule pack ${file.id} lists ${rule.id}, whose document ${rule.document} does not exist`);
        const document: RuleDocument = readRuleDocument(documentPath);
        if (document.rule.id !== rule.id) throw new Error(`Rule pack ${file.id} lists ${rule.id}, but ${rule.document} has id ${document.rule.id}`);
        const previous = owner.get(rule.id);
        if (previous !== undefined) throw new Error(`Rule ${rule.id} is in both rule packs ${previous} and ${file.id}`);
        const examples = rule.examples;
        if (examples !== undefined && document.kind === "file" && (examples.caught.path === undefined || examples.passes.path === undefined)) {
          throw new Error(`File rule ${rule.id} in rule pack ${file.id} needs a path on both examples`);
        }
        if (examples !== undefined && document.kind === "conversation" && (examples.caught.path !== undefined || examples.passes.path !== undefined)) {
          throw new Error(`Conversation rule ${rule.id} in rule pack ${file.id} takes no example path`);
        }
        owner.set(rule.id, file.id);
        // The mode and on state a pack gives its rules take effect only once packs are attached.
        return { id: rule.id, kind: document.kind, title: rule.title, group: rule.group, mode: rule.mode ?? document.rule.mode, on: rule.on ?? true, ...(examples === undefined ? {} : { examples }) };
      });
      return { id: file.id, name: file.name, category: file.category, covers: file.covers, reason: file.reason, ...(file.match === undefined ? {} : { match: file.match }), honourSuppressionComments, rules };
    })
    .sort((left, right) => CATEGORY_ORDER[left.category] - CATEGORY_ORDER[right.category] || left.name.localeCompare(right.name));
}

let shipped: readonly RulePack[] | undefined;

/** The packs that come with Rerail: languages, then frameworks, then repository packs. Read once per process. */
export function loadRulePacks(): readonly RulePack[] {
  shipped ??= [...loadBundledCatalog().packs.values()];
  return shipped;
}

/** The id of the pack a rule belongs to, or null for rules in no pack (created by you, or older bundled rules). */
export function rulePackIdOf(ruleId: string, packs: readonly RulePack[] = loadRulePacks()): string | null {
  return packs.find(pack => pack.rules.some(rule => rule.id === ruleId))?.id ?? null;
}

/**
 * The packs that fit the repository at `repositoryRoot`, from its files and `package.json` dependencies alone:
 * no model, no network, and no list of what does not fit. A pack without a match definition never fits.
 * Languages come first, then frameworks, then repository packs. Each comes with the folder it belongs in (see
 * `RulePackRecommendation.folder`).
 */
export function recommendRulePacks(repositoryRoot: string, packs: readonly RulePack[] = loadRulePacks()): RulePackRecommendation[] {
  const scan = scanRepository(repositoryRoot);
  return packs.flatMap(pack => {
    if (pack.match === undefined) return [];
    const reason = matchReason(pack.match, scan);
    if (reason === undefined) return [];
    const folder = pack.category === "repository" ? "" : matchFolder(pack.match, scan);
    return [{ pack, ruleCount: pack.rules.length, reason, folder }];
  });
}

/** How long a daemon reuses a repository's recommendations before scanning it again. */
export const RECOMMENDATION_CACHE_MS = 60_000;
const recommendationCache = new Map<string, { at: number; recommendations: RulePackRecommendation[] }>();

/**
 * `recommendRulePacks` for the daemon, which answers synchronously: the scan blocks other requests while it runs
 * (about 0.3 to 0.5 s on a large repository), so a repository is scanned at most once per `RECOMMENDATION_CACHE_MS`.
 * A file added or removed within that window shows on the next scan.
 */
export function cachedRecommendRulePacks(repositoryRoot: string, now = Date.now()): RulePackRecommendation[] {
  const cached = recommendationCache.get(repositoryRoot);
  if (cached !== undefined && now - cached.at < RECOMMENDATION_CACHE_MS) return cached.recommendations;
  const recommendations = recommendRulePacks(repositoryRoot);
  recommendationCache.set(repositoryRoot, { at: now, recommendations });
  return recommendations;
}
