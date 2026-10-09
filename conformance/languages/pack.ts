import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { CODE_RULE_LANGUAGE_FAMILIES } from "../../src/code-rule-languages.js";
import { parseJsonWithoutDuplicateKeys } from "../../src/json-duplicate-keys.js";
import { compareDeterministicStrings } from "../../src/stable-json.js";

export const HARNESS_DIRECTORY = dirname(fileURLToPath(import.meta.url));
export const RERAIL_ROOT = resolve(HARNESS_DIRECTORY, "..", "..");
/** Rule packs in their own packages (docs/pack-format.md); a ported pack holds its conformance pack as conformance.json. */
export const PACKS_ROOT = join(RERAIL_ROOT, "packs");
/** A pack moved to `packs/<id>/` names its conformance pack this; one still under conformance/languages/, `pack.json`. */
export const CONFORMANCE_FILE = "conformance.json";
/** The conformance fixtures: `RERAIL_ORACLES_DIR` (relative to the repository root) or the repository's `fixtures/`. */
export const ORACLES_ROOT = resolve(RERAIL_ROOT, process.env.RERAIL_ORACLES_DIR || "fixtures");
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

/** No value twice. */
const distinct = (values: readonly string[]): boolean => new Set(values).size === values.length;

const toolSchema = z.strictObject({
  name: nonEmpty,
  version: nonEmpty,
  /**
   * The tool's approved configurations, in order of preference, named as its recorder names them (Ruff's `default`,
   * ESLint's `recommended`). A port is recorded with the options of the first one that turns its rule on, and with
   * upstream's defaults when none does. Declaring them declares the tool's source facts: the recorder writes
   * `upstream/<tool>/source.json`, and checks 7 to 9 judge the tool's ports. Without them, those checks are not judged.
   */
  recommended: z.array(nonEmpty).min(1).refine(distinct, "recommended configurations are listed once each").optional(),
});

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
    /** Recordings directory, relative to the pack directory; default `upstream/` beside conformance.json, else `<fixtures>/<pack>/upstream`. */
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

const position = z.number().int().positive();

/**
 * A finding carries the parts of its position the tool reports: `line`; `line, column`; `line, column, endColumn`
 * (Bandit, whose end column lies on a line it does not report); or `line, column, endLine, endColumn`.
 */
const findingSchema = z
  .strictObject({
    rule: nonEmpty,
    /** Posix path relative to the fixtures directory or the corpus checkout. */
    path: nonEmpty,
    /** 1-based. */
    line: position,
    /** 1-based, in Unicode code points from the line's start (PHP_CodeSniffer's, ESLint's + 1). */
    column: position.optional(),
    /** 1-based line of the finding's end, the position just past its last character. */
    endLine: position.optional(),
    /**
     * That end's column: 1-based, in Unicode code points, exclusive (an empty finding's equals its `column`). Ruff's
     * `end_location.column`, ESLint's `endColumn` converted to code points. Without `endLine`, the end lies on a line
     * the tool does not report (Bandit's `end_col_offset`, converted from UTF-8 bytes).
     */
    endColumn: position.optional(),
  })
  .superRefine((finding, context) => {
    const issue = (message: string): void => context.addIssue({ code: "custom", message });
    if (finding.endColumn !== undefined && finding.column === undefined) issue("endColumn needs column");
    if (finding.endLine !== undefined && finding.endColumn === undefined) issue("endLine needs endColumn");
    if (finding.endLine !== undefined && finding.endLine < finding.line) issue(`endLine ${finding.endLine} is before line ${finding.line}`);
    if (finding.endLine === finding.line && finding.column !== undefined && finding.endColumn !== undefined && finding.endColumn < finding.column)
      issue(`endColumn ${finding.endColumn} is before column ${finding.column} on line ${finding.line}`);
  });
export type UpstreamFinding = z.infer<typeof findingSchema>;

/** The position parts a finding carries beyond its line. */
export const FINDING_PARTS = ["column", "endLine", "endColumn"] as const;
const partsOf = (finding: UpstreamFinding): string => ["line", ...FINDING_PARTS.filter(part => finding[part] !== undefined)].join(", ");

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

/** Every finding of one rule carries the same parts, which are the parts the rule's findings are compared on. */
export const recordingSchema = z
  .strictObject({
    tool: nonEmpty,
    version: nonEmpty,
    source: z.discriminatedUnion("kind", [fixtureSourceSchema, corpusSourceSchema]),
    findings: z.array(findingSchema),
  })
  .superRefine((recording, context) => {
    const parts = new Map<string, string>();
    const reported = new Set<string>();
    for (const finding of recording.findings) {
      const carried = partsOf(finding);
      const first = parts.get(finding.rule);
      if (first === undefined) parts.set(finding.rule, carried);
      else if (first !== carried && !reported.has(finding.rule)) {
        reported.add(finding.rule);
        context.addIssue({ code: "custom", message: `${finding.rule}: findings carry different parts (${first} / ${carried})` });
      }
    }
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

/** A set of strings, written in `compareDeterministicStrings` order with no repeats, so a recorder writes it byte-stably. */
const sortedSet = z.array(nonEmpty).superRefine((values, context) => {
  const index = values.findIndex((value, at) => at > 0 && compareDeterministicStrings(values[at - 1]!, value) >= 0);
  if (index !== -1) context.addIssue({ code: "custom", message: `${JSON.stringify(values[index])} is repeated or out of order` });
});

/** A regex upstream's code matches names with, as the recorder translated it to a JavaScript regex. */
const upstreamRegexSchema = z
  .strictObject({ source: nonEmpty, flags: z.string().regex(/^[dgimsuvy]*$/u) })
  .refine(({ source, flags }) => {
    try {
      new RegExp(source, flags);
      return true;
    } catch {
      return false;
    }
  }, "not a JavaScript regex");

/**
 * Why check 4 does not score one of a rule's upstream tests. A test upstream itself skips, and a case whose code another
 * scored case repeats under the same parser, is no test lost and has no entry.
 */
export const UNSCORED_REASONS = {
  notebook: "a Jupyter notebook, which no dialect reads",
  settings: "upstream's tests lint it only with settings other than the approved configuration's",
  "not-copied": "upstream reports the rule there under the approved configuration, but the recorder does not copy it",
  "custom-parser": "the case names a parser the pack's configuration does not lint with",
  "type-information": "the case needs type information, which the pack's configuration does not build",
  syntax: "the case is written in a syntax only another parser reads (Flow)",
  unparsable: "the pack's configuration cannot parse the case's code",
} as const;
export type UnscoredReason = keyof typeof UNSCORED_REASONS;

/** One upstream test of a rule that check 4 does not score: a test file, or one RuleTester case of it. */
const unscoredTestSchema = z.strictObject({
  /** The test file's upstream path. */
  test: nonEmpty,
  /** The RuleTester case (`invalid[5]`, `run 2 valid[0]`), or null for a whole file. */
  case: nonEmpty.nullable(),
  reason: z.enum(Object.keys(UNSCORED_REASONS) as [UnscoredReason, ...UnscoredReason[]]),
});
export type UnscoredTest = z.infer<typeof unscoredTestSchema>;
const unscoredKey = (entry: UnscoredTest): string => `${entry.test}\u0000${entry.case ?? ""}`;

/** What one upstream rule's source says about it at the pinned commit. */
const upstreamRuleSchema = z.strictObject({
  /** Upstream paths of the rule's implementation: its own module, and the files that raise its violation. */
  implementation: sortedSet,
  /** Upstream paths of the rule's own tests. */
  tests: sortedSet,
  /** The deprecation upstream's own metadata reports for the rule, or null. */
  deprecated: nonEmpty.nullable(),
  /** The first approved configuration that turns the rule on, and the options it sets; null when none does. */
  enabledBy: z.strictObject({ configuration: nonEmpty, options: z.array(z.unknown()) }).nullable(),
  names: z.strictObject({
    /** Names the rule's own implementation checks. */
    implementation: sortedSet,
    /** Names the helpers only this rule delegates to check; `sharedNames` holds the helpers every rule shares. */
    shared: sortedSet,
  }),
  /** The regexes the implementation and the helpers it delegates to match names with. */
  regexes: z.array(upstreamRegexSchema),
  /** The rule's upstream tests the recorded fixtures leave out, by test file and case, each once. */
  unscored: z.array(unscoredTestSchema).superRefine((entries, context) => {
    const index = entries.findIndex((entry, at) => at > 0 && compareDeterministicStrings(unscoredKey(entries[at - 1]!), unscoredKey(entry)) >= 0);
    const entry = entries[index];
    if (entry !== undefined) context.addIssue({ code: "custom", message: `${entry.test}${entry.case === null ? "" : ` ${entry.case}`} is repeated or out of order` });
  }),
});
export type UpstreamRule = z.infer<typeof upstreamRuleSchema>;

/**
 * `upstream/<tool>/source.json`: the facts checks 7 and 8 read about a tool's source, written by the tool's recorder
 * from the pinned checkout and runtime, so the checks need neither. The recorder refuses a checkout that is not
 * `commit`, a source tree that differs from `tag`'s, or a runtime that is not `version`.
 */
export const upstreamSourceSchema = z
  .strictObject({
    tool: nonEmpty,
    version: nonEmpty,
    repository: z.url(),
    commit: z.string().regex(/^[0-9a-f]{40}$/u, "a full lowercase commit sha"),
    /** The release tag the runtime was published from. */
    tag: nonEmpty,
    /** The approved configurations the facts were recorded under: conformance.json's `tools[].recommended`. */
    configurations: z.array(nonEmpty).min(1).refine(distinct, "configurations are listed once each"),
    /** Keywords of the language the tool lints: syntax wherever a rule spells them. */
    keywords: sortedSet,
    /** Words the language or the tool gives meaning inside comments (directives, pragmas, documentation tags). */
    commentVocabulary: sortedSet,
    /** The file extensions the approved configurations lint (`.py`, `.mjs`): check 1's in-scope probe. */
    extensions: sortedSet.min(1),
    /** Every word of upstream's test code (and expected outputs), for all of the tool's rules: what check 8 runs regexes over. */
    testWords: sortedSet,
    /** The words of upstream's test configurations and other test data that its test code does not hold. */
    configurationWords: sortedSet,
    /** Names the helpers every rule of the tool delegates to check (shared crates or modules). */
    sharedNames: sortedSet,
    /** Per upstream rule name, as conformance.json's `upstream.rule` spells it: every port of the tool has one. */
    rules: z.record(nonEmpty, upstreamRuleSchema),
  })
  .superRefine((source, context) => {
    const issue = (message: string): void => context.addIssue({ code: "custom", message });
    const names = Object.keys(source.rules);
    if (names.length === 0) issue("rules is empty");
    const unsorted = names.findIndex((name, at) => at > 0 && compareDeterministicStrings(names[at - 1]!, name) >= 0);
    if (unsorted !== -1) issue(`rules: ${names[unsorted]} is out of order`);
    for (const [name, rule] of Object.entries(source.rules)) {
      if (rule.enabledBy !== null && !source.configurations.includes(rule.enabledBy.configuration))
        issue(`rules.${name}: enabledBy names ${rule.enabledBy.configuration}, which is not in configurations`);
    }
  });
export type UpstreamSource = z.infer<typeof upstreamSourceSchema>;
export const UPSTREAM_SOURCE_FILE = "source.json";

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

/** `packs/<name>/` once the pack has moved there (it holds conformance.json), else `conformance/languages/<name>/`. */
export function packDirectory(name: string): string {
  const moved = join(PACKS_ROOT, name);
  return existsSync(join(moved, CONFORMANCE_FILE)) ? moved : join(HARNESS_DIRECTORY, name);
}

/**
 * The conformance pack in `directory`: `packs/<id>/` (conformance.json, recordings under `upstream/` by default) or
 * `conformance/languages/<name>/` (pack.json, recordings under `<fixtures>/<pack>/upstream` by default).
 */
export function loadPack(directory: string): LoadedPack {
  const moved = existsSync(join(directory, CONFORMANCE_FILE));
  const pack = parseJson(join(directory, moved ? CONFORMANCE_FILE : "pack.json"), packSchema);
  const divergencesPath = join(directory, "divergences.json");
  const divergences = existsSync(divergencesPath) ? parseJson(divergencesPath, divergencesSchema) : [];
  const ids = new Set(pack.rules.map(rule => rule.id));
  for (const divergence of divergences) {
    if (!ids.has(divergence.rule)) throw new Error(`${divergencesPath}: divergence names unknown rule ${divergence.rule}`);
  }
  const defaultOracles = moved ? join(directory, "upstream") : join(ORACLES_ROOT, pack.pack, "upstream");
  return {
    pack,
    directory,
    oraclesDirectory: pack.oracles === undefined ? defaultOracles : resolve(directory, pack.oracles),
    divergences,
  };
}

/** Every pack still under conformance/languages/: a directory holding a pack.json. */
export function listPacks(): string[] {
  return readdirSync(HARNESS_DIRECTORY, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && existsSync(join(HARNESS_DIRECTORY, entry.name, "pack.json")))
    .map(entry => entry.name)
    .sort(compareDeterministicStrings);
}

/**
 * Every pack under packs/ whose rules this runner scores: a directory holding a conformance.json in this runner's
 * schema. A conformance.json with a `format` is another harness's (the TSLint pack's golden manifest,
 * conformance/tslint/golden.ts). Their tests are test/packs.test.ts.
 */
export function listPortedPacks(): string[] {
  if (!existsSync(PACKS_ROOT)) return [];
  return readdirSync(PACKS_ROOT, { withFileTypes: true })
    .filter(entry => {
      const path = join(PACKS_ROOT, entry.name, CONFORMANCE_FILE);
      return entry.isDirectory() && existsSync(path) && !z.looseObject({ format: z.string() }).safeParse(JSON.parse(readFileSync(path, "utf8"))).success;
    })
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

export function upstreamSourcePath(loaded: LoadedPack, tool: string): string {
  return join(loaded.oraclesDirectory, tool, UPSTREAM_SOURCE_FILE);
}

/** A tool's source facts; throws naming the file when it is not JSON, repeats a key or fails the schema. */
export function loadUpstreamSource(path: string): UpstreamSource {
  const parsed = upstreamSourceSchema.safeParse(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path));
  if (!parsed.success) throw new Error(`${path}: ${z.prettifyError(parsed.error)}`);
  return parsed.data;
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
