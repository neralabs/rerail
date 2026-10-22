import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, matchesGlob, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { CODE_RULE_LANGUAGE_FAMILIES, compareDeterministicStrings, RULE_PACK_RULE_ID } from "../../src/engine/index.js";

export const HARNESS_DIRECTORY = dirname(fileURLToPath(import.meta.url));
export const RERAIL_ROOT = resolve(HARNESS_DIRECTORY, "..", "..");
/** Rule packs in their own packages (docs/pack-format.md); a ported pack holds its conformance pack as conformance.json. */
export const PACKS_ROOT = join(RERAIL_ROOT, "packs");
/** A conformance pack's manifest: `packs/<id>/conformance.json`, and the same in a pack's `candidates/` and probes. */
export const CONFORMANCE_FILE = "conformance.json";
/** Where `--write` puts reports: `.rerail/` is gitignored, so reports are never committed. */
export const REPORTS_DIRECTORY = join(RERAIL_ROOT, ".rerail", "reports");

/** The T4 bar: a supported pack ports at least this many upstream rules. */
export const MINIMUM_RULES = 12;

const nonEmpty = z.string().min(1);

const corpusSchema = z.strictObject({
  name: nonEmpty,
  repository: nonEmpty,
  commit: nonEmpty,
  /** Directories (or files) under the corpus checkout that are scored; posix, relative. */
  paths: z.array(nonEmpty).min(1),
  /** `path.matchesGlob` patterns over the corpus-relative path; a match is never evaluated nor scored. */
  exclude: z.array(nonEmpty).default([]),
});
export type PackCorpus = z.infer<typeof corpusSchema>;

const toolSchema = z.strictObject({ name: nonEmpty, version: nonEmpty });

const ruleSchema = z.strictObject({
  /** The id of the document it ports (`policy`), in pack.json's style: see `RULE_PACK_RULE_ID`. */
  id: z.string().regex(RULE_PACK_RULE_ID, "rule ids are lowercase kebab-case"),
  upstream: z.strictObject({ tool: nonEmpty, rule: nonEmpty }),
  /** `rules/<id>.json` (shipped, gated) or `candidates/…` (scored, never gated), relative to the pack directory. */
  policy: nonEmpty,
  capabilities: z.array(nonEmpty).min(1),
  match: z.enum(["line", "line+column"]),
  /**
   * `path.matchesGlob` patterns over the tool's fixture paths: the rule is evaluated and scored only on the fixture
   * units they match, the tests its upstream configuration ran on (one rule option variant, say). Default: every
   * unit. Corpora are always scored whole.
   */
  fixtures: z.array(nonEmpty).min(1).optional(),
});
export type PackRule = z.infer<typeof ruleSchema>;

/** Whether `rule` is evaluated and scored on the fixture at `path` (see `fixtures`). */
export function coversFixture(rule: PackRule, path: string): boolean {
  return rule.fixtures === undefined || rule.fixtures.some(pattern => matchesGlob(path, pattern));
}

/** A port still being evaluated (`candidates/`): scored and reported, but its FPs, FNs and engine failures fail nothing. */
export function isCandidate(rule: PackRule): boolean {
  return !documentPath(rule.policy).startsWith("rules/");
}

/**
 * A declared capability no reference linter has a rule for that reads it like-for-like, so T4 cannot port one. It
 * counts as covered for T4's capability bar only with the evidence for that, and with T2 and T3 covering it.
 */
const exemptionSchema = z.strictObject({
  capability: nonEmpty,
  reason: z.string().trim().min(1),
  /** What was checked: each pinned linter's rules for the capability and why none reads it like-for-like. */
  evidence: z.string().trim().min(20),
  /** `T2: <oracle path or test>` and `T3: <benchmark or test>` entries; at least one of each. */
  coveredBy: z
    .array(z.string().regex(/^T[23]: \S/u, "each entry is `T2: <oracle>` or `T3: <benchmark>`"))
    .refine(entries => entries.some(entry => entry.startsWith("T2: ")) && entries.some(entry => entry.startsWith("T3: ")), "needs at least one T2 and one T3 entry"),
});
export type CapabilityExemption = z.infer<typeof exemptionSchema>;

/**
 * A port whose violations need not carry the suppression comment its upstream tool honours (conformance/pack-validation.ts,
 * check 4): `rule` is the port's `id`, `reason` why its document does not model the comment yet.
 */
const suppressionExemptionSchema = z.strictObject({ rule: nonEmpty, reason: z.string().trim().min(20) });
export type SuppressionExemption = z.infer<typeof suppressionExemptionSchema>;

export const packSchema = z
  .strictObject({
    pack: z.string().regex(/^[a-z0-9][a-z0-9-]*$/u),
    family: z.enum(CODE_RULE_LANGUAGE_FAMILIES),
    /** Every capability the pack declares; T4 needs a rule for each, T2 an oracle for each. */
    capabilities: z.array(nonEmpty).min(1),
    /** Declared capabilities T4 covers by exemption instead of a rule. */
    exemptCapabilities: z.array(exemptionSchema).default([]),
    /** Ports exempt from carrying their upstream tool's suppression comment, each with its reason. */
    suppressionExemptions: z.array(suppressionExemptionSchema).default([]),
    corpora: z.array(corpusSchema),
    tools: z.array(toolSchema),
    rules: z.array(ruleSchema),
    /** Recordings directory, relative to the pack directory; default `upstream/` beside conformance.json. */
    oracles: nonEmpty.optional(),
  })
  .superRefine((pack, context) => {
    const issue = (message: string): void => context.addIssue({ code: "custom", message });
    const duplicates = (values: readonly string[]): string[] => values.filter((value, index) => values.indexOf(value) !== index);
    for (const id of duplicates(pack.rules.map(rule => rule.id))) issue(`duplicate rule id ${id}`);
    for (const name of duplicates(pack.corpora.map(corpus => corpus.name))) issue(`duplicate corpus ${name}`);
    for (const name of duplicates(pack.tools.map(tool => tool.name))) issue(`duplicate tool ${name}`);
    for (const capability of duplicates(pack.capabilities)) issue(`duplicate capability ${capability}`);
    for (const capability of duplicates(pack.exemptCapabilities.map(exemption => exemption.capability))) issue(`duplicate exemption for ${capability}`);
    const tools = new Set(pack.tools.map(tool => tool.name));
    const capabilities = new Set(pack.capabilities);
    for (const rule of pack.rules) {
      if (!tools.has(rule.upstream.tool)) issue(`${rule.id}: upstream tool ${rule.upstream.tool} is not in tools`);
      for (const capability of rule.capabilities)
        if (!capabilities.has(capability)) issue(`${rule.id}: capability ${capability} is not declared by the pack`);
    }
    for (const { capability } of pack.exemptCapabilities)
      if (!capabilities.has(capability)) issue(`exemption for ${capability}, which the pack does not declare`);
    for (const rule of duplicates(pack.suppressionExemptions.map(exemption => exemption.rule))) issue(`duplicate suppression exemption for ${rule}`);
    const rules = new Set(pack.rules.map(rule => rule.id));
    for (const { rule } of pack.suppressionExemptions) if (!rules.has(rule)) issue(`suppression exemption for ${rule}, which is not a rule of the pack`);
  });
export type Pack = z.infer<typeof packSchema>;

const findingSchema = z.strictObject({
  rule: nonEmpty,
  /** Posix path relative to the fixtures directory or the corpus checkout. */
  path: nonEmpty,
  /** 1-based. */
  line: z.number().int().positive(),
  /** 1-based, in Unicode code points from the line's start (PHP_CodeSniffer's, ESLint's + 1). */
  column: z.number().int().positive().optional(),
});
export type UpstreamFinding = z.infer<typeof findingSchema>;

/**
 * One analysis unit, evaluated in its own workspace as the upstream rule test analyses it: a file, or several files
 * analysed together (a cross-file fixture). A `package.json` in a unit is no source: it gives the unit's ECMA sources
 * their nearest manifest.
 */
const unitSchema = z.union([nonEmpty, z.array(nonEmpty).min(1)]);

const fixtureSourceSchema = z.strictObject({
  kind: z.literal("fixtures"),
  /** Upstream repository URL `@` commit the fixture files were copied from. */
  origin: nonEmpty,
  /** Every upstream rule the run reported; each is scored over every unit. */
  rules: z.array(nonEmpty).min(1),
  files: z.array(unitSchema).min(1),
});

const corpusSourceSchema = z.strictObject({
  kind: z.literal("corpus"),
  name: nonEmpty,
  repository: nonEmpty,
  commit: nonEmpty,
  rules: z.array(nonEmpty).min(1),
  /** Corpus-relative files the tool could not analyse (`php -l` rejects them, …); never scored. */
  skipped: z.array(nonEmpty).default([]),
  /**
   * Lines of scored files the tool did not analyse, read from its own reports (a function Psalm stops analysing at an
   * undefined closure `use` variable, …): no finding on them, the tool's or Rerail's, is scored for the tool's rules.
   */
  unanalysed: z
    .array(z.strictObject({ path: nonEmpty, startLine: z.number().int().positive(), endLine: z.number().int().positive() }))
    .default([]),
});

export const recordingSchema = z.strictObject({
  tool: nonEmpty,
  version: nonEmpty,
  source: z.discriminatedUnion("kind", [fixtureSourceSchema, corpusSourceSchema]),
  findings: z.array(findingSchema),
});
export type Recording = z.infer<typeof recordingSchema>;
export type FixtureRecording = Recording & { source: z.infer<typeof fixtureSourceSchema> };
export type CorpusRecording = Recording & { source: z.infer<typeof corpusSourceSchema> };

export const DIVERGENCE_KINDS = ["upstream-wrong", "not-comparable", "rerail-unknown"] as const;
const divergenceSchema = z.strictObject({
  /** Pack rule id. */
  rule: nonEmpty,
  /** `fixtures` or a corpus name; omitted, it applies to every source. */
  source: nonEmpty.optional(),
  path: nonEmpty,
  line: z.number().int().positive(),
  kind: z.enum(DIVERGENCE_KINDS),
  /** Why the two disagree, citing what was checked (runtime output, upstream source, issue links). */
  evidence: z.string().min(20),
});
export type Divergence = z.infer<typeof divergenceSchema>;
export const divergencesSchema = z.array(divergenceSchema);

/**
 * The rule pack beside a conformance pack (`pack.json`, docs/pack-format.md), read only for which documents carry
 * `examples`; the pack's own validation (conformance/pack-validation.ts) checks the rest.
 */
const rulePackExamplesSchema = z.object({ rules: z.array(z.object({ document: nonEmpty, examples: z.unknown().optional() })) });

export interface LoadedPack {
  pack: Pack;
  directory: string;
  /** Root of the per-tool recordings: `<oracles>/<tool>/{fixtures,corpus-<name>}.expected.json`. */
  oraclesDirectory: string;
  divergences: Divergence[];
  /**
   * Documents (`rules/<id>.json`) that pack.json lists with `examples`: a caught case the pack checks prove fires, so
   * the port has evidence it catches something even without a fixture TP. Empty without a pack.json.
   */
  exampled: ReadonlySet<string>;
}

/** A rule document's path relative to its pack directory, as pack.json and a port's `policy` both spell it. */
export function documentPath(path: string): string {
  return path.replace(/^\.\//u, "");
}

function parseJson<T>(path: string, schema: z.ZodType<T>): T {
  const parsed = schema.safeParse(JSON.parse(readFileSync(path, "utf8")));
  if (!parsed.success) throw new Error(`${path}: ${z.prettifyError(parsed.error)}`);
  return parsed.data;
}

/** The conformance pack in `directory` (its conformance.json; recordings under `upstream/` unless it sets `oracles`). */
export function loadPack(directory: string): LoadedPack {
  const pack = parseJson(join(directory, CONFORMANCE_FILE), packSchema);
  const divergencesPath = join(directory, "divergences.json");
  const divergences = existsSync(divergencesPath) ? parseJson(divergencesPath, divergencesSchema) : [];
  const ids = new Set(pack.rules.map(rule => rule.id));
  for (const divergence of divergences) {
    if (!ids.has(divergence.rule)) throw new Error(`${divergencesPath}: divergence names unknown rule ${divergence.rule}`);
  }
  const rulePackPath = join(directory, "pack.json");
  const listed = existsSync(rulePackPath) ? parseJson(rulePackPath, rulePackExamplesSchema).rules : [];
  return {
    pack,
    directory,
    oraclesDirectory: resolve(directory, pack.oracles ?? "upstream"),
    divergences,
    exampled: new Set(listed.filter(rule => rule.examples !== undefined).map(rule => documentPath(rule.document))),
  };
}

/** Every pack under packs/ whose rules this runner scores: a directory holding a conformance.json. Their tests are test/engine/catalog/packs.test.ts. */
export function listPortedPacks(): string[] {
  if (!existsSync(PACKS_ROOT)) return [];
  return readdirSync(PACKS_ROOT, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && existsSync(join(PACKS_ROOT, entry.name, CONFORMANCE_FILE)))
    .map(entry => entry.name)
    .sort(compareDeterministicStrings);
}

export function fixturesRecordingPath(loaded: LoadedPack, tool: string): string {
  return join(loaded.oraclesDirectory, tool, "fixtures.expected.json");
}

export function fixturesDirectory(loaded: LoadedPack, tool: string): string {
  return join(loaded.oraclesDirectory, tool, "fixtures");
}

export function corpusRecordingPath(loaded: LoadedPack, tool: string, corpus: string): string {
  return join(loaded.oraclesDirectory, tool, `corpus-${corpus}.expected.json`);
}

export function loadRecording(path: string): Recording {
  return parseJson(path, recordingSchema);
}

/**
 * Pinned corpora are gitignored checkouts under `.rerail/corpora/<pack>/<name>`. A worktree nested inside the main
 * checkout shares its corpora, so the nearest ancestor holding `.rerail/corpora/<pack>` is the default.
 */
export function defaultCorpusRoot(pack: string): string {
  for (let directory = RERAIL_ROOT; ; directory = dirname(directory)) {
    const candidate = join(directory, ".rerail", "corpora", pack);
    if (existsSync(candidate)) return candidate;
    if (dirname(directory) === directory) return join(RERAIL_ROOT, ".rerail", "corpora", pack);
  }
}
