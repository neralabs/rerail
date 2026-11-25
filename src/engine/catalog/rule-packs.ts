import { existsSync, readFileSync } from "node:fs";
import { basename, extname, join, posix, win32 } from "node:path";
import { z } from "zod";
import { loadRuleCatalog } from "./rule-catalog.js";
import { parseRuleDocument, readJsonDocument } from "./rule-documents.js";
import {
  DEPENDENCY_ECOSYSTEMS,
  fileEndings,
  globPattern,
  matchFolder,
  matchReason,
  nearMiss,
  packLooksFor,
  scanRepository,
  type FileEnding,
  type NearMiss,
  type NotScannedFolder,
  type PackLooksFor,
  type RepositoryScan,
  type RulePackMatch,
  type RulePackSignal,
} from "./rule-pack-matcher.js";
import { describeRerailRules, RERAIL_DOCUMENT_EXTENSION, rerailRuleOf, unsupportedRerailRule, type RerailDescriber } from "./rerail-rules/rerail-rule.js";
import { ruleKindOf, type RuleDocument } from "./rule-store.js";
import type { RuleKind } from "../storage/schema.js";
import type { RrDescription } from "../rr/rr-runner.js";
import { RULE_MODES, type RuleMode } from "./rule-mode.js";

export type { RulePackMatch, RulePackSignal } from "./rule-pack-matcher.js";

/**
 * Rule packs: named sets of rules, each its own npm package (`rerail-pack-<id>` or `@<scope>/rerail-pack-<id>`; Rerail's
 * own are `@neralabs/rerail-pack-<id>`), installed into the Rerail home and read by `rule-catalog.ts`. A pack's match
 * definition says which repositories it fits and why; a pack without a rule that starts on is never recommended. A pack
 * is a directory (`docs/pack-format.md`): `pack.json` lists each rule with the document in the pack's `rules/` that
 * defines it. A pack's rules run only in repositories that add it (`repository_attachments_set`); this module defines,
 * validates and recommends.
 */
export const RULE_PACK_FORMAT = "rerail.rule-pack/v1" as const;
export const RULE_PACK_CATEGORIES = ["language", "framework", "repository"] as const;
export type RulePackCategory = (typeof RULE_PACK_CATEGORIES)[number];

const nonEmpty = z.string().refine(value => value.trim().length > 0, "must be a non-empty string");

/**
 * A pack rule's dimensions (`docs/rule-dimensions.md`): what goes wrong when it is broken (`risk`), where the code it
 * reads lives (`layer`), the worst realistic outcome (`severity`) and, optionally, the hazard it guards against, an id
 * several packs share when they catch the same thing in different stacks (`pg.index-blocks-writes` for SQL, Alembic and
 * Django migrations alike). A pack is one stack; dimensions slice the rules across stacks. The dashboard names the risks
 * in the same order (`RISK_LABELS` in ui/derive.js).
 */
export const RULE_RISKS = ["security", "data-safety", "availability", "correctness", "performance", "maintainability", "test-quality"] as const;
const RULE_LAYERS = ["ui", "server", "data-access", "jobs", "migrations", "tests", "config", "code"] as const;
const RULE_SEVERITIES = ["critical", "high", "medium", "low"] as const;

/** A hazard id: an area, a dot, then the hazard, each lowercase words joined by dashes (`tx.side-effect-before-commit`). */
const HAZARD_ID = /^[a-z0-9]+(?:-[a-z0-9]+)*\.[a-z0-9]+(?:-[a-z0-9]+)*$/u;

/** `dimensions` on a `pack.json` rule: all three axes together, and a hazard when the rule names one. */
export const ruleDimensionsSchema = z
  .object({
    risk: z.enum(RULE_RISKS),
    layer: z.enum(RULE_LAYERS),
    severity: z.enum(RULE_SEVERITIES),
    hazard: z.string().regex(HAZARD_ID, "must be <area>.<hazard>, lowercase words joined by dashes").optional(),
  })
  .strict();
type RuleDimensions = Readonly<z.infer<typeof ruleDimensionsSchema>>;

/** Why `path`, a file of an example's repository, cannot be one: it must be relative and stay inside that repository. */
function examplePathProblem(path: string): string | undefined {
  const normalized = posix.normalize(path.replaceAll("\\", "/"));
  if (posix.isAbsolute(normalized) || win32.isAbsolute(path)) return "must be relative to the example's repository";
  return normalized === ".." || normalized.startsWith("../") ? "must stay inside the example's repository" : undefined;
}

/** Why `path`, a file an example is read with beside its own file `own`, cannot be one (`examplePathProblem`, or it is `own`). */
function exampleFileProblem(path: string, own: string | undefined): string | undefined {
  const problem = examplePathProblem(path);
  if (problem !== undefined || own === undefined) return problem;
  return posix.normalize(path.replaceAll("\\", "/")) === posix.normalize(own.replaceAll("\\", "/")) ? "is the example's own path" : undefined;
}

const exampleSchema = z
  .object({
    /** The file a code rule's example is read as; conversation rule examples have none. */
    path: nonEmpty.optional(),
    /** File content for a code rule; the command a coding agent runs for a conversation rule. */
    text: nonEmpty,
    /**
     * For a rule written in `.rerail` that reads across files: the other files of the one repository the example is
     * read in. Only findings on the example's own `path` count.
     */
    with: z.array(z.object({ path: nonEmpty, text: nonEmpty }).strict()).min(1).optional(),
  })
  .strict()
  .superRefine((example, context) => {
    const own = example.path === undefined ? undefined : examplePathProblem(example.path);
    if (own !== undefined) context.addIssue({ code: "custom", path: ["path"], message: own });
    for (const [at, file] of (example.with ?? []).entries()) {
      const problem = exampleFileProblem(file.path, example.path);
      if (problem !== undefined) context.addIssue({ code: "custom", path: ["with", at, "path"], message: problem });
    }
  });

/** Whether a listed rule's examples carry other files, which only a rule written in `.rerail` reads. */
const examplesWithFiles = (rule: { readonly examples?: { readonly caught: { readonly with?: unknown }; readonly passes: { readonly with?: unknown } } | undefined }): boolean =>
  rule.examples?.caught.with !== undefined || rule.examples?.passes.with !== undefined;

/**
 * A pack's id, its `pack.json` `id`: lowercase letters, digits and dashes, starting with a letter. It is also the end
 * of the pack's package name (`rerail-pack-<id>`), and repositories add the pack by it.
 */
export const RULE_PACK_ID = /^[a-z][a-z0-9-]*$/u;

/**
 * A pack rule's id: lowercase letters, digits and dashes, one style in every pack (`eslint-no-empty`,
 * `ruff-b011-assert-false`). It is also the name of the rule's document, `rules/<id>.json` or `rules/<id>.rerail`, and
 * its port's id in `conformance.json`. Users turn rules on and off by it, so it must not change once the pack is
 * published.
 */
export const RULE_PACK_RULE_ID = /^[a-z0-9][a-z0-9-]*$/u;

/**
 * A rule's document, relative to the pack directory: `rules/<id>.json`, a code or conversation rule in JSON, or
 * `rules/<id>.rerail`, a code rule written in `.rerail`. Only a plain name: a `\` or `..` would reach outside the pack
 * on Windows, where `\` separates folders.
 */
const documentPathSchema = z.string().regex(/^rules\/[a-z0-9][a-z0-9-]*\.(?:json|rerail)$/u, "must be rules/<id>.json or rules/<id>.rerail");

const packRuleFields = {
  id: z.string().regex(RULE_PACK_RULE_ID, "must be lowercase letters, digits and dashes"),
  title: nonEmpty,
  group: nonEmpty,
  /** The rule mode the pack starts the rule in; default: the mode its document has. */
  mode: z.enum(RULE_MODES).optional(),
  /** Whether the rule is on when its pack is added; default on. */
  on: z.boolean().optional(),
  /**
   * Why a reviewer holds the rule off: its steer would do harm though upstream turns it on. Needs `on: false`; set-up
   * never turns a held-off rule on from a repository's lint config.
   */
  heldOff: nonEmpty.optional(),
  /** One example the rule catches and one it lets pass, for the rule panel; tests prove both. */
  examples: z.object({ caught: exampleSchema, passes: exampleSchema }).strict().optional(),
  /** What goes wrong when the rule is broken, where its code lives and how badly: slices rules across packs. */
  dimensions: ruleDimensionsSchema.optional(),
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
    id: z.string().regex(RULE_PACK_ID, "must be lowercase letters, digits and dashes"),
    name: nonEmpty,
    category: z.enum(RULE_PACK_CATEGORIES),
    covers: nonEmpty,
    reason: nonEmpty,
    /** Which repositories the pack fits: set-up runs it against a repository to recommend the pack. */
    match: matchSchema,
    /** Whether the pack's rules honour their linters' suppression comments (`# noqa`, `eslint-disable`); default yes. */
    honourSuppressionComments: z.boolean().optional(),
    rules: z.array(
      z.object({ ...packRuleFields, document: documentPathSchema }).strict()
        .refine(rule => rule.heldOff === undefined || rule.on === false, { message: "needs \"on\": false", path: ["heldOff"] })
        .refine(rule => extname(rule.document) === RERAIL_DOCUMENT_EXTENSION || !examplesWithFiles(rule), {
          message: "with is for a rule written in .rerail, which reads its example with other files",
          path: ["examples"],
        }),
    ),
  })
  .strict();
export type RulePackFile = z.infer<typeof rulePackFileSchema>;

/** The npm scope Rerail publishes its own packs under. */
const RERAIL_PACK_SCOPE = "@neralabs";
/** The publisher of Rerail's own packs, as the catalog and the API name it. */
export const RERAIL_PUBLISHER = "rerail";

/**
 * A pack's npm package name: `rerail-pack-<id>`, or `@<scope>/rerail-pack-<id>` under any scope, where `<id>` is the
 * pack's `pack.json` id. The first group is the scope, the second the id.
 */
export const RULE_PACK_PACKAGE_NAME = /^(?:(@[a-z0-9][a-z0-9._-]*)\/)?rerail-pack-([a-z][a-z0-9-]*)$/u;

/** The npm package name of Rerail's own pack `<id>`. */
export const rulePackPackageName = (id: string): string => `${RERAIL_PACK_SCOPE}/rerail-pack-${id}`;

/** The pack id a pack's package name holds (`@acme/rerail-pack-sql` → `sql`); undefined for any other name. */
export const packIdOfPackage = (packageName: string): string | undefined => RULE_PACK_PACKAGE_NAME.exec(packageName)?.[2];

/**
 * Who publishes the pack in package `packageName`: `rerail` for `@neralabs/*`, else the npm scope (`@acme`), else the
 * package name (`rerail-pack-foo`). A pack directory without a package (null) is a Rerail checkout's `packs/<id>/`.
 */
export function rulePackPublisher(packageName: string | null): string {
  if (packageName === null) return RERAIL_PUBLISHER;
  const scope = RULE_PACK_PACKAGE_NAME.exec(packageName)?.[1];
  return scope === RERAIL_PACK_SCOPE ? RERAIL_PUBLISHER : scope ?? packageName;
}

/**
 * How `rerail packs add` and set-up's `--packs` name the pack in package `packageName`: its id for one of Rerail's own
 * (`sql`), else its package name (`@acme/rerail-pack-sql`). `rulePackPackageOf` reads either back as the package.
 */
export function rulePackShortName(packageName: string): string {
  return rulePackPublisher(packageName) === RERAIL_PUBLISHER ? packIdOfPackage(packageName) ?? packageName : packageName;
}

/**
 * The package a pack's short name names: a pack's package name is itself (`rerail-pack-foo` is one, not an id), and a
 * pack id is Rerail's own package; undefined for anything else.
 */
export function rulePackPackageOf(name: string): string | undefined {
  if (RULE_PACK_PACKAGE_NAME.test(name)) return name;
  return RULE_PACK_ID.test(name) ? rulePackPackageName(name) : undefined;
}

/**
 * `package.json`: the pack's npm package. Core's version range is a peer dependency, a declaration and not an import;
 * the pack contract (`docs/pack-format.md`) adds that the name matches the pack, that there are no `dependencies`,
 * that `files` publishes no validation material, and that `license` agrees with `licenses/`.
 */
export const rulePackPackageSchema = z.looseObject({
  name: z.string().regex(RULE_PACK_PACKAGE_NAME, "must be rerail-pack-<id> or @<scope>/rerail-pack-<id>"),
  version: z.string().regex(/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/u, "must be a semver version"),
  description: nonEmpty.optional(),
  /** An SPDX expression naming every licence the published files carry. */
  license: nonEmpty,
  files: z.array(nonEmpty).min(1),
  peerDependencies: z.looseObject({ "@neralabs/rerail": nonEmpty }),
  publishConfig: z.looseObject({ access: z.enum(["restricted", "public"]) }).optional(),
  /**
   * npm's own flag: `true` means the pack is never published (docs/pack-format.md, "package.json"). The release
   * scripts, the pack list and the conformance checks read it through `isPrivatePack` in scripts/release/packs.mjs.
   */
  private: z.boolean().optional(),
});
export type RulePackPackage = z.infer<typeof rulePackPackageSchema>;

export interface RuleExample {
  readonly path?: string | undefined;
  readonly text: string;
  /** The other files of the example's repository, for a rule written in `.rerail` that reads across files. */
  readonly with?: { path: string; text: string }[] | undefined;
}

export interface RulePackRule {
  readonly id: string;
  /** A code rule or a conversation rule, whatever language its document is written in. */
  readonly kind: RuleKind;
  /** Plain-language name, e.g. "Use === and !== in TypeScript". */
  readonly title: string;
  /** The smaller group the rule is filed under inside its pack, e.g. "Complexity" or "api". */
  readonly group: string;
  /** The rule mode it starts in when its pack is added; the seeded document carries it. */
  readonly mode: RuleMode;
  /** Whether the rule is on when its pack is added. */
  readonly on: boolean;
  /** Why a reviewer holds the rule off (`on` is false); set-up never turns it on from a repository's lint config. */
  readonly heldOff?: string;
  /** One example the rule catches and one it lets pass, where the pack has them; tests prove both. */
  readonly examples?: { readonly caught: RuleExample; readonly passes: RuleExample };
  /** The rule's risk, layer, severity and hazard, where the pack gives them (`docs/rule-dimensions.md`). */
  readonly dimensions?: RuleDimensions;
}

/** Where a pack installed from disk came from: a folder, or a tarball, by its absolute path. */
export interface RulePackLocalSource {
  readonly kind: "folder" | "tarball";
  readonly path: string;
}

export interface RulePack {
  readonly id: string;
  readonly name: string;
  /** The version of the pack's npm package; null for a pack directory without one. */
  readonly version: string | null;
  /** The pack's npm package name, e.g. `@neralabs/rerail-pack-sql`; null for a pack directory without one. */
  readonly packageName: string | null;
  /** Who publishes the pack (`rulePackPublisher`): `rerail` for Rerail's own, else e.g. `@acme` or `rerail-pack-foo`. */
  readonly publisher: string;
  /**
   * The folder or tarball the pack was installed from (`rerail packs add <folder>`), its absolute path, rather than
   * from the npm registry; absent for a pack from the registry or one read where it is. Its `package.json` name is
   * whatever the folder says, published by no one.
   */
  readonly localSource?: RulePackLocalSource;
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
  if (packIdOfPackage(parsed.data.name) !== id) throw new Error(`Rule pack ${id} must be the package rerail-pack-${id} or @<scope>/rerail-pack-${id}, not ${parsed.data.name}`);
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

/** Rule `ruleId`'s examples in pack `id`, checked for its kind: a code rule's both have a path, a conversation rule's none. */
function checkExamples(id: string, ruleId: string, kind: RuleKind, examples: RulePackRule["examples"]): void {
  if (examples === undefined) return;
  const withPath = [examples.caught, examples.passes].filter(example => example.path !== undefined).length;
  if (kind === "code" && withPath < 2) throw new Error(`Code rule ${ruleId} in rule pack ${id} needs a path on both examples`);
  if (kind === "conversation" && withPath > 0) throw new Error(`Conversation rule ${ruleId} in rule pack ${id} takes no example path`);
}

/** Listed rule `rule` with its document, as the catalog holds it. The mode and on state take effect only once packs are attached. */
function packRuleOf(rule: RulePackFile["rules"][number], document: RuleDocument): RulePackRule {
  return {
    id: rule.id, kind: ruleKindOf(document), title: rule.title, group: rule.group, mode: rule.mode ?? document.rule.mode, on: rule.on ?? true,
    ...(rule.heldOff === undefined ? {} : { heldOff: rule.heldOff }),
    ...(rule.examples === undefined ? {} : { examples: rule.examples }),
    ...(rule.dimensions === undefined ? {} : { dimensions: rule.dimensions }),
  };
}

/**
 * The rule document `source`, read from `documentPath`. One the schema refuses is reported like a refused `pack.json`:
 * the pack, the document and its first issue, in a sentence, where the schema's own error is every issue as JSON.
 */
function parsePackDocument(id: string, document: string, source: unknown, documentPath: string): RuleDocument {
  try {
    return parseRuleDocument(source, documentPath);
  } catch (error) {
    if (!(error instanceof z.ZodError)) throw error;
    const issue = error.issues[0];
    throw new Error(`Rule pack ${id}: ${document} ${issue?.path.join(".") ?? ""} ${issue?.message ?? "is invalid"}`.replace(/\s+/gu, " "));
  }
}

/** A listed rule's document as read: the rule, and for a JSON code rule its document as written (`$option` references unresolved). */
interface ReadDocument {
  readonly document: RuleDocument;
  readonly source: unknown;
}

/** The JSON rule document at `documentPath`, listed as `document` in pack `id`. */
function readJsonPackDocument(id: string, document: string, documentPath: string): ReadDocument {
  const source = readJsonDocument(documentPath);
  return { document: parsePackDocument(id, document, source, documentPath), source };
}

/**
 * The `.rerail` rule `rule` lists, from `described`, the engine's reading of its file at `documentPath`: one the engine
 * refuses, or that does what a pack's `.rerail` rule cannot do yet (`unsupportedRerailRule`), fails the pack.
 */
function readRerailPackDocument(id: string, rule: RulePackFile["rules"][number], documentPath: string, described: RrDescription | undefined): ReadDocument {
  if (described === undefined || !described.ok) {
    throw new Error(`Rule pack ${id} lists ${rule.id}, whose document ${rule.document} the engine refuses: ${described?.error ?? "it was not read"}`);
  }
  const unsupported = unsupportedRerailRule(described, rule.mode);
  if (unsupported !== undefined) throw new Error(`Rule pack ${id} lists ${rule.id}: ${unsupported}`);
  return { document: { kind: "rerail", rule: rerailRuleOf(described, readFileSync(documentPath, "utf8")) }, source: undefined };
}

/**
 * One listed rule read and checked: its document is named for its id, exists and carries that id, and its examples fit
 * its kind. `described` holds the engine's reading of each listed `.rerail` document, by its path.
 */
function readPackRule(directory: string, id: string, rule: RulePackFile["rules"][number], described: ReadonlyMap<string, RrDescription>): { rule: RulePackRule } & ReadDocument {
  const documentPath = join(directory, rule.document);
  const extension = extname(rule.document);
  if (basename(rule.document, extension) !== rule.id) throw new Error(`Rule pack ${id} lists ${rule.id}, whose document must be rules/${rule.id}${extension}, not ${rule.document}`);
  if (!existsSync(documentPath)) throw new Error(`Rule pack ${id} lists ${rule.id}, whose document ${rule.document} does not exist`);
  const { document, source } = extension === RERAIL_DOCUMENT_EXTENSION
    ? readRerailPackDocument(id, rule, documentPath, described.get(documentPath))
    : readJsonPackDocument(id, rule.document, documentPath);
  if (document.rule.id !== rule.id) throw new Error(`Rule pack ${id} lists ${rule.id}, but ${rule.document} has id ${document.rule.id}`);
  checkExamples(id, rule.id, ruleKindOf(document), rule.examples);
  return { rule: packRuleOf(rule, document), document, source };
}

/**
 * The engine's reading of every `.rerail` document `file` lists that exists in `directory`, by its path: one call for
 * the pack, and none for a pack of JSON rules.
 */
function describeListed(directory: string, file: RulePackFile, describe: RerailDescriber): ReadonlyMap<string, RrDescription> {
  const paths = file.rules
    .filter(rule => extname(rule.document) === RERAIL_DOCUMENT_EXTENSION)
    .map(rule => join(directory, rule.document))
    .filter(path => existsSync(path));
  if (paths.length === 0) return new Map();
  const rows = describe(paths);
  return new Map(paths.map((path, at) => [path, rows[at]!]));
}

/**
 * The pack in `directory`, validated: `pack.json` parses, the npm package (when there is a `package.json`) is the
 * pack's, or else the directory is named for it, every rule's document is named for its id, exists and carries that
 * id, and its globs compile. The engine reads each `.rerail` document (`describe`), and the pack fails on one it
 * refuses. Throws an error naming the pack on the first problem.
 */
export function readRulePack(directory: string, describe: RerailDescriber = describeRerailRules): LoadedRulePack {
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
  const described = describeListed(directory, file, describe);
  const rules = file.rules.map(rule => readPackRule(directory, file.id, rule, described));
  const pack: RulePack = {
    id: file.id,
    name: file.name,
    version: packageJson?.version ?? null,
    packageName: packageJson?.name ?? null,
    publisher: rulePackPublisher(packageJson?.name ?? null),
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

/** How many of `pack`'s rules start on and run: those this Rerail does not run (`RuleCatalog.notRun`) are left out. */
export function rulesStartingOn(pack: RulePack, notRun: ReadonlyMap<string, unknown>): number {
  return pack.rules.filter(rule => rule.on && !notRun.has(rule.id)).length;
}

/**
 * The packs whose match definition fits the scanned repository, from its files and dependency manifests: no model, no
 * network, no code from the packs. A pack fits whatever its rules; `ruleCount` is how many start on and run, leaving
 * out those of `notRun` (`rulesStartingOn`). Languages come first, then frameworks, then repository packs. Each comes
 * with the folder it belongs in (see `RulePackRecommendation.folder`). `explainNoFit` says why none does.
 */
export function rulePacksThatFit(scan: RepositoryScan, packs: readonly RulePack[], notRun: ReadonlyMap<string, unknown> = new Map()): RulePackRecommendation[] {
  return packs.flatMap(pack => {
    const reason = matchReason(pack.match, scan);
    if (reason === undefined) return [];
    const ruleCount = rulesStartingOn(pack, notRun);
    const folder = pack.category === "repository" ? "" : matchFolder(pack.match, scan);
    return [{ pack, ruleCount, reason, folder }];
  });
}

/** Why none of a set of packs fits a repository: what the scan found and left out, and what each pack looks for. */
export interface NoFitExplanation {
  /** How many files the scan read, with an ending or not. */
  readonly fileCount: number;
  /** The endings of the files scanned, most files first. */
  readonly endings: readonly FileEnding[];
  /** The folders the scan did not enter. */
  readonly notScanned: readonly NotScannedFolder[];
  /** Whether the scan read Git's listing, which leaves out the files Git ignores. */
  readonly git: boolean;
  /** Whether the walk outside Git stopped early, so the rest describes only the files it read. */
  readonly stoppedEarly: boolean;
  /** The packs that look for files the repository has but exclude every one of them, most files first. */
  readonly nearMisses: readonly (NearMiss & { readonly name: string })[];
  /** What each pack looks for, in the order the packs came. */
  readonly packs: readonly (PackLooksFor & { readonly id: string; readonly name: string })[];
}

/** Why none of `packs` fits the scanned repository; the caller knows none does (`rulePacksThatFit` gave none). */
export function explainNoFit(scan: RepositoryScan, packs: readonly RulePack[]): NoFitExplanation {
  const nearMisses = packs.flatMap(pack => {
    const missed = nearMiss(pack.match, scan);
    return missed === undefined ? [] : [{ name: pack.name, ...missed }];
  });
  return {
    fileCount: scan.files.length,
    endings: fileEndings(scan),
    notScanned: scan.notScanned,
    git: scan.git,
    stoppedEarly: scan.stoppedEarly,
    nearMisses: nearMisses.sort((left, right) => right.fileCount - left.fileCount),
    packs: packs.map(pack => ({ id: pack.id, name: pack.name, ...packLooksFor(pack.match) })),
  };
}

/** The packs that fit the repository at `repositoryRoot` (`rulePacksThatFit`) and have a rule that starts on. */
export function recommendRulePacks(
  repositoryRoot: string,
  packs: readonly RulePack[] = loadRulePacks(),
  notRun: ReadonlyMap<string, unknown> = new Map(),
): RulePackRecommendation[] {
  return rulePacksThatFit(scanRepository(repositoryRoot), packs, notRun).filter(found => found.ruleCount > 0);
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
  const recommendations = recommendRulePacks(repositoryRoot, [...catalog.packs.values()], catalog.notRun);
  recommendationCache.set(repositoryRoot, { at: now, digest: catalog.digest, recommendations });
  return recommendations;
}
