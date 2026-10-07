import { readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { parseJsonWithoutDuplicateKeys } from "./json-duplicate-keys.js";
import { bundledRulesDirectory, loadBundledRules } from "./rule-bootstrap.js";
import { globPattern, matchReason, scanRepository, type RulePackMatch, type RulePackSignal } from "./rule-pack-matcher.js";
import type { RuleDocument } from "./rule-store.js";
import { RULE_MODES, type RuleMode } from "./rule-mode.js";

export type { RulePackMatch, RulePackSignal } from "./rule-pack-matcher.js";

/**
 * Rule packs: named sets of rules that come with Rerail, each with a match definition that says which
 * repositories it fits and why. Pack definitions live apart from rule documents
 * (`policies/rule-packs/<id>.json`); a pack lists bundled rules by id and adds no rules of its own. Attaching packs to
 * repositories, and so seeding their rules, is not built yet (NER-1745); this module defines, validates and recommends.
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

const packRuleSchema = z
  .object({
    id: nonEmpty,
    title: nonEmpty,
    group: nonEmpty,
    /** The rule mode the pack starts the rule in; default: the mode its document has. */
    mode: z.enum(RULE_MODES).optional(),
    /** Whether the rule is on when its pack is added; default on. */
    on: z.boolean().optional(),
    /** One example the rule catches and one it lets pass, for the rule panel; tests prove both. */
    examples: z.object({ caught: exampleSchema, passes: exampleSchema }).strict().optional(),
  })
  .strict();

const signalSchema: z.ZodType<RulePackSignal> = z.union([
  z.object({ files: z.array(nonEmpty).min(1), exclude: z.array(nonEmpty).optional(), label: nonEmpty }).strict(),
  z.object({ npmDependencies: z.array(nonEmpty).min(1) }).strict(),
]);

const matchSchema: z.ZodType<RulePackMatch> = z.object({ any: z.array(signalSchema).min(1) }).strict();

const packFileSchema = z
  .object({
    format: z.literal(RULE_PACK_FORMAT),
    id: z.string().regex(/^[a-z][a-z0-9-]*$/u, "must be lowercase letters, digits and dashes"),
    name: nonEmpty,
    category: z.enum(RULE_PACK_CATEGORIES),
    covers: nonEmpty,
    reason: nonEmpty,
    match: matchSchema,
    /** Whether the pack's rules honour their linters' suppression comments (`# noqa`, `eslint-disable`); default yes. */
    honourSuppressionComments: z.boolean().optional(),
    rules: z.array(packRuleSchema).min(1),
  })
  .strict();

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
  /** The smaller group the rule is filed under inside its pack, e.g. "TSLint" or "nera-api". */
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
  readonly match: RulePackMatch;
  /** Whether the pack's rules honour their linters' suppression comments. When false, a comment silences nothing. */
  readonly honourSuppressionComments: boolean;
  readonly rules: readonly RulePackRule[];
}

export interface RulePackRecommendation {
  readonly pack: RulePack;
  readonly ruleCount: number;
  /** What was found in this repository, e.g. "matches the react dependency in web/package.json". */
  readonly reason: string;
}

const CATEGORY_ORDER: Record<RulePackCategory, number> = { language: 0, framework: 1, repository: 2 };

/** `<package>/policies/rule-packs`: one `<id>.json` definition per pack. */
export function rulePacksDirectory(): string {
  return resolve(dirname(fileURLToPath(import.meta.url)), "..", "policies", "rule-packs");
}

/** The packs in `directory`, validated against the bundled rules: every rule resolves and belongs to one pack. */
export function readRulePacks(directory = rulePacksDirectory(), bundledDirectory = bundledRulesDirectory()): readonly RulePack[] {
  const pool = new Map<string, RuleDocument>();
  for (const document of loadBundledRules(bundledDirectory)) {
    pool.set(document.rule.id, document);
  }
  const owner = new Map<string, string>();
  return readdirSync(directory, { withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith(".json"))
    .map(entry => {
      const path = join(directory, entry.name);
      const file = packFileSchema.parse(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path));
      if (`${file.id}.json` !== entry.name) throw new Error(`Rule pack ${file.id} must be stored as ${file.id}.json, not ${entry.name}`);
      for (const signal of file.match.any) {
        if (!("files" in signal)) continue;
        try {
          globPattern([...signal.files, ...(signal.exclude ?? [])]);
        } catch (error) {
          throw new Error(`Rule pack ${file.id}: ${(error as Error).message}`);
        }
      }
      // Like each rule's mode and on state, this takes effect only once packs are attached (NER-1745).
      const honourSuppressionComments = file.honourSuppressionComments ?? true;
      const rules = file.rules.map((rule): RulePackRule => {
        const document = pool.get(rule.id);
        if (document === undefined) throw new Error(`Rule pack ${file.id} lists ${rule.id}, which is not a bundled rule`);
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
        // The mode and on state a pack gives its rules take effect only once packs are attached (NER-1745).
        return { id: rule.id, kind: document.kind, title: rule.title, group: rule.group, mode: rule.mode ?? document.rule.mode, on: rule.on ?? true, ...(examples === undefined ? {} : { examples }) };
      });
      return { id: file.id, name: file.name, category: file.category, covers: file.covers, reason: file.reason, match: file.match, honourSuppressionComments, rules };
    })
    .sort((left, right) => CATEGORY_ORDER[left.category] - CATEGORY_ORDER[right.category] || left.name.localeCompare(right.name));
}

let shipped: readonly RulePack[] | undefined;

/** The packs that come with Rerail: languages, then frameworks, then repository packs. Read once per process. */
export function loadRulePacks(): readonly RulePack[] {
  shipped ??= readRulePacks();
  return shipped;
}

/** The id of the pack a rule belongs to, or null for rules in no pack (created by you, or older bundled rules). */
export function rulePackIdOf(ruleId: string, packs: readonly RulePack[] = loadRulePacks()): string | null {
  return packs.find(pack => pack.rules.some(rule => rule.id === ruleId))?.id ?? null;
}

/**
 * The packs that fit the repository at `repositoryRoot`, from its files and `package.json` dependencies alone:
 * no model, no network, and no list of what does not fit. Languages come first, then frameworks, then repository packs.
 */
export function recommendRulePacks(repositoryRoot: string, packs: readonly RulePack[] = loadRulePacks()): RulePackRecommendation[] {
  const scan = scanRepository(repositoryRoot);
  return packs.flatMap(pack => {
    const reason = matchReason(pack.match, scan);
    return reason === undefined ? [] : [{ pack, ruleCount: pack.rules.length, reason }];
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
