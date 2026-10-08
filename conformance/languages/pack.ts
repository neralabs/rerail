import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { CODE_RULE_LANGUAGE_FAMILIES } from "../../src/code-rule-languages.js";
import { compareDeterministicStrings } from "../../src/stable-json.js";

export const HARNESS_DIRECTORY = dirname(fileURLToPath(import.meta.url));
export const RERAIL_ROOT = resolve(HARNESS_DIRECTORY, "..", "..");
/** The conformance fixtures: `RERAIL_FIXTURES_DIR` (relative to the repository root) or the repository's `fixtures/`. */
export const ORACLES_ROOT = resolve(RERAIL_ROOT, process.env.RERAIL_FIXTURES_DIR || "fixtures");
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
  id: z.string().regex(/^[a-z0-9][a-z0-9-]*$/u, "rule ids are lowercase kebab-case"),
  upstream: z.strictObject({ tool: nonEmpty, rule: nonEmpty }),
  /** `rules/<id>.json`, relative to the pack directory. */
  policy: nonEmpty,
  capabilities: z.array(nonEmpty).min(1),
  match: z.enum(["line", "line+column"]),
});
export type PackRule = z.infer<typeof ruleSchema>;

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

export const packSchema = z
  .strictObject({
    pack: z.string().regex(/^[a-z0-9][a-z0-9-]*$/u),
    family: z.enum(CODE_RULE_LANGUAGE_FAMILIES),
    /** Every capability the pack declares; T4 needs a rule for each, T2 an oracle for each. */
    capabilities: z.array(nonEmpty).min(1),
    /** Declared capabilities T4 covers by exemption instead of a rule. */
    exemptCapabilities: z.array(exemptionSchema).default([]),
    corpora: z.array(corpusSchema),
    tools: z.array(toolSchema),
    rules: z.array(ruleSchema),
    /** Recordings directory, relative to the pack directory; default `<fixtures>/<pack>/rules`. */
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
 * analysed together (a cross-file fixture).
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

export interface LoadedPack {
  pack: Pack;
  directory: string;
  /** Root of the per-tool recordings: `<oracles>/<tool>/{fixtures,corpus-<name>}.expected.json`. */
  oraclesDirectory: string;
  divergences: Divergence[];
}

function parseJson<T>(path: string, schema: z.ZodType<T>): T {
  const parsed = schema.safeParse(JSON.parse(readFileSync(path, "utf8")));
  if (!parsed.success) throw new Error(`${path}: ${z.prettifyError(parsed.error)}`);
  return parsed.data;
}

export function packDirectory(name: string): string {
  return join(HARNESS_DIRECTORY, name);
}

export function loadPack(directory: string): LoadedPack {
  const pack = parseJson(join(directory, "pack.json"), packSchema);
  const divergencesPath = join(directory, "divergences.json");
  const divergences = existsSync(divergencesPath) ? parseJson(divergencesPath, divergencesSchema) : [];
  const ids = new Set(pack.rules.map(rule => rule.id));
  for (const divergence of divergences) {
    if (!ids.has(divergence.rule)) throw new Error(`${divergencesPath}: divergence names unknown rule ${divergence.rule}`);
  }
  return {
    pack,
    directory,
    oraclesDirectory: pack.oracles === undefined ? join(ORACLES_ROOT, pack.pack, "rules") : resolve(directory, pack.oracles),
    divergences,
  };
}

/** Every pack under conformance/languages/: a directory holding a pack.json. */
export function listPacks(): string[] {
  return readdirSync(HARNESS_DIRECTORY, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && existsSync(join(HARNESS_DIRECTORY, entry.name, "pack.json")))
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
