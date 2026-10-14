/**
 * The recording contract between the gates recorder (conformance/recorders/gates/, run on a maintainer machine and
 * nightly) and the gates (conformance/gates/, run in PR CI): what a live adapter answers, written into the tool-pack's
 * directory `packs/<pack>/upstream/<tool>/` so the gates can read it back through a recorded adapter without running
 * upstream.
 *
 * - `gates.json` (`rerail.gates-recording/v1`) holds every PackAdapter answer the gates read except upstream's answers
 *   on the corpus: identity, tool-pack checks, the license digest, the words and names `not-overfit` reads, every
 *   upstream rule's facts, name sources, span parts and cases, and independent parsers' verdicts on every case source.
 * - `gates-corpus.json` (`rerail.gates-corpus/v1`) holds `lintCorpus`'s answers: upstream's findings on every corpus
 *   file, and independent parsers' verdicts on each.
 *
 * Each PackAdapter member and where its recorded answer is:
 *   identity → `identity`; packChecks → `packChecks`; licenseText → `licenseSha256` (the text itself is the kept
 *   `licenses/<project>/LICENSE`, checked against this digest); rule(name) → `rules[name].facts`, undefined for a name
 *   in `absent`; implementationNames / sharedImplementationNames / implementationRegexes → `rules[name].names` with
 *   `nameSets`; fixtureWords → `fixtureWords`; fixtureTexts → `fixtureTexts`: upstream's files fetched at the
 *   pinned commit (each checked against its sha256) and the inline texts, or, in a recording without it, the
 *   committed source of every case (`cases[].file`); keywords → `keywords`;
 *   commentVocabulary → `commentVocabulary`; inScopeSamples → `inScopeSamples`; cases(name) → `rules[name].cases`,
 *   each source read from its `file`; toSpan(name) → span-units/<adapter>.ts with `rules[name].spanParts`; inCorpus →
 *   `corpus`; lintCorpus → gates-corpus.json `files`; parserVerdicts → `parserVerdicts` here and gates-corpus.json
 *   `files[].verdicts`, looked up by path, script and the source's sha256.
 *
 * Both files are deterministic: every list the schema calls sorted is sorted in code-unit order, objects are written
 * with sorted keys (`recordingText`), and nothing in them names a machine path, a date or a cache directory.
 */
import { z } from "zod";

import { ADAPTERS } from "./manifest.js";
import type { Span } from "./types.js";

export const GATES_RECORDING_FORMAT = "rerail.gates-recording/v1";
export const GATES_CORPUS_FORMAT = "rerail.gates-corpus/v1";

/** The recording's file names in a tool-pack's directory. */
export const GATES_RECORDING = "gates.json";
export const GATES_CORPUS = "gates-corpus.json";

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

const nonEmpty = z.string().min(1);
const sha256 = z.string().regex(/^[0-9a-f]{64}$/u, "a sha256 in lowercase hex");
const commit = z.string().regex(/^[0-9a-f]{40}$/u, "a full commit sha");
const line = z.number().int().nonnegative();
/** A repository-relative or tool-pack-relative path: `/`-separated, no `.` or `..` segment, not absolute. */
const relativePath = z.string().regex(/^(?!\/)(?!.*(?:^|\/)\.\.?(?:\/|$))[^\0\\]+$/u, "a relative /-separated path without . or .. segments");

/** Strings in code-unit order, each once. */
function sortedStrings<T extends z.ZodString>(item: T) {
  return z.array(item).refine((items) => items.every((value, index) => index === 0 || byText(items[index - 1]!, value) < 0), "sorted in code-unit order, each once");
}

/**
 * A finding's position in upstream's native units, as `[startLine, startCol, endLine, endCol]`: 0-based lines,
 * columns in the tool's unit (Ruff code points, Bandit UTF-8 bytes, ESLint UTF-16 code units), null for a part upstream
 * does not report. It is the gates' `Span` written compactly; `spanOf` and `spanTuple` convert.
 */
export const recordedSpanSchema = z.tuple([line, line.nullable(), line.nullable(), line.nullable()]);
export type RecordedSpan = z.infer<typeof recordedSpanSchema>;

export function spanOf([startLine, startCol, endLine, endCol]: RecordedSpan): Span {
  return { startLine, startCol, endLine, endCol };
}

export function spanTuple(span: Span): RecordedSpan {
  return [span.startLine, span.startCol, span.endLine, span.endCol];
}

const checkSchema = z.strictObject({
  /** The check's id, e.g. `upstream-commit`, `runtime-version`, `declared-license`. */
  id: nonEmpty,
  /** Whether it held when the recorder ran. */
  ok: z.boolean(),
  /** What it compared, as the live adapter wrote it. */
  detail: z.string(),
});

/** A regex upstream matches names with, translated to JavaScript (`new RegExp(source, flags)` rebuilds it). */
const regexSchema = z.strictObject({
  /** `RegExp.prototype.source` of the translated regex. */
  source: z.string(),
  /** `RegExp.prototype.flags` of the translated regex, in the order JavaScript writes them. */
  flags: z.string().regex(/^d?g?i?m?s?u?v?y?$/u, "RegExp flags in JavaScript's order"),
}).refine(({ source, flags }) => {
  try {
    new RegExp(source, flags);
    return true;
  } catch {
    return false;
  }
}, "a regex JavaScript compiles");

/** Names and regexes several rules share (one upstream source file, a helper module, a whole crate): stored once. */
const nameSetSchema = z.strictObject({
  /** Every name the set's upstream code checks, sorted. */
  names: sortedStrings(nonEmpty),
  /** The regexes the set's upstream code matches names with, in the order the live adapter read them. */
  regexes: z.array(regexSchema),
});

/** One part of upstream's code a rule's names come from: the shared sets it consists of, and what is its own. */
const nameSourceSchema = z.strictObject({
  /** Ids of `nameSets`, in the order the live adapter read their code; each id once. */
  sets: z.array(nonEmpty).refine((ids) => new Set(ids).size === ids.length, "names each set once"),
  /** Names of this part beyond its sets' (Bandit's blacklist qualnames for one test id), sorted. */
  names: sortedStrings(nonEmpty),
  /** Regexes of this part beyond its sets', in the order the live adapter read them. */
  regexes: z.array(regexSchema),
});

const parserVerdictSchema = z.strictObject({
  /** The parser and its version, e.g. `cpython 3.14.8`, `@babel/parser 7.28.5`. */
  parser: nonEmpty,
  /** Errors that stopped it building a tree. */
  syntax: z.array(z.string()),
  /** What the language forbids in a tree it built; null for a parser that makes no grammar checks (libcst). */
  grammar: z.array(z.string()).nullable(),
});

/** Independent parsers' verdicts on one source text as the gates would ask for them. */
const verdictEntrySchema = z.strictObject({
  /** The path the source is read as: a case's path with its `parsedAs` file type, or a known gap's `probe<ext>`. */
  path: relativePath,
  /** Whether it is read as an ECMAScript script, as upstream read it (false for every other language). */
  script: z.boolean(),
  /** sha256 of the source text, UTF-8. */
  sha256,
  /** Every independent parser's verdict, in the live adapter's order. */
  verdicts: z.array(parserVerdictSchema).min(1),
});

const caseSchema = z.strictObject({
  /** UpstreamCase.fixture: the upstream test file, relative to upstream's repository. */
  fixture: nonEmpty,
  /** UpstreamCase.variant: the upstream test configuration (test names for Ruff, `<file>#<run>/<kind>[i]` for ESLint). */
  variant: z.string(),
  /** UpstreamCase.path: the path the engine evaluates the case at. */
  path: relativePath,
  /** UpstreamCase.parsedAs: the file type the engine reads the case as, when upstream's parser read another language's. */
  parsedAs: z.string().regex(/^\.[A-Za-z0-9]+$/u).optional(),
  /** UpstreamCase.script: read as an ECMAScript script (ESLint's `sourceType: "script"`); absent where the adapter says nothing. */
  script: z.boolean().optional(),
  /**
   * UpstreamCase.skipped: why upstream did not lint the case, `not-copied` when its source cannot be committed (Ruff's
   * GPL-derived fixtures), or null; a skipped case is not scored.
   */
  skipped: nonEmpty.nullable(),
  /** UpstreamCase.expected: upstream's findings on the case, sorted by position. */
  expected: z.array(recordedSpanSchema),
  /** UpstreamCase.markup: upstream's written expectation, or null. */
  markup: z.array(recordedSpanSchema).nullable(),
  /**
   * The case's committed source, relative to the tool-pack directory (`fixtures/pyflakes/F401_0.py`), byte-identical to
   * upstream's; null only for a skipped case, whose source no gate reads.
   */
  file: relativePath.nullable(),
}).refine((testCase) => testCase.file !== null || testCase.skipped !== null, "only a skipped case has no committed source");

const ruleSchema = z.strictObject({
  /** rule(name): what upstream says about the rule at the pinned version. */
  facts: z.strictObject({
    /** UpstreamRule.name: upstream's own name for the rule (`B101`, `@typescript-eslint/no-explicit-any`). */
    name: nonEmpty,
    /** UpstreamRule.implementation: the rule's implementation files, relative to upstream's repository. */
    implementation: z.array(nonEmpty),
    /** UpstreamRule.tests: the upstream test files that test the rule. */
    tests: z.array(nonEmpty),
    /** UpstreamRule.deprecation: upstream's deprecation message, or null. */
    deprecation: z.string().nullable(),
    /** UpstreamRule.recommended: the first approved configuration that turns the rule on, with its options; or null. */
    recommended: z.strictObject({ config: nonEmpty, options: z.array(z.json()) }).nullable(),
    /** UpstreamRule.notOnRealCode: why upstream cannot lint real code for the rule (type-checked rules), or null. */
    notOnRealCode: z.string().nullable(),
  }),
  /**
   * Where the rule's names come from. implementationNames = `implementation.names` and its sets' names;
   * sharedImplementationNames = the same of `shared`; implementationRegexes = `implementation.regexes`, its sets'
   * regexes, then `shared.regexes` and its sets' regexes.
   */
  names: z.strictObject({ implementation: nameSourceSchema, shared: nameSourceSchema }),
  /** The end parts upstream reports for the rule, which toSpan keeps (types.ts `SpanParts`). */
  spanParts: z.strictObject({ endLine: z.boolean(), endCol: z.boolean() }),
  /** cases(name): upstream's output on its own test files for the rule, in the live adapter's order. */
  cases: z.array(caseSchema),
});

export type RecordedRule = z.infer<typeof ruleSchema>;
export type RecordedCase = z.infer<typeof caseSchema>;

export const gatesRecordingSchema = z.strictObject({
  format: z.literal(GATES_RECORDING_FORMAT),
  /** The tool: `tools[].name` in the pack's conformance.json, and the directory under the pack's upstream/. */
  tool: nonEmpty,
  /** gates.yaml's adapter: whose live adapter recorded this, and whose span units (span-units/<adapter>.ts) toSpan uses. */
  adapter: z.enum(ADAPTERS),
  /** The upstream pins the recording was made at; the pins-agree check compares them with gates.yaml and conformance.json. */
  pins: z.strictObject({
    /** gates.yaml `upstream.repository`. */
    repository: nonEmpty,
    /** gates.yaml `upstream.commit`. */
    commit,
    /** gates.yaml `upstream.version`, conformance.json `tools[].version`. */
    version: nonEmpty,
    /** gates.yaml `upstream.runtime`: the directory under conformance/recorders/gates/runtimes/ upstream ran from. */
    runtime: nonEmpty,
  }),
  /**
   * Every runtime the recorder ran (upstream's, an ESLint environment's, the independent parsers', the Python
   * toolkit's), by its directory under conformance/recorders/gates/runtimes/: each lockfile's sha256 by file name. The
   * pins-agree check compares them with the committed lockfiles.
   */
  runtimes: z.record(z.string().regex(/^[a-z0-9._-]+$/u), z.record(z.string().regex(/^[A-Za-z0-9._-]+$/u), sha256)),
  /**
   * identity: how upstream ran and how its positions are read, as the report states it. No machine paths; a value that
   * names the machine's toolchain (ESLint's `node <version>`) makes `gates:record --check` differ across machines.
   */
  identity: z.record(nonEmpty, z.string()),
  /** packChecks: the tool-pack checks only the live adapter can make, as they stood when the recorder ran. */
  packChecks: z.array(checkSchema),
  /** licenseText's sha256 (UTF-8): upstream's LICENSE at the pinned commit; the kept licenses/<project>/LICENSE must match it. */
  licenseSha256: sha256,
  /** inScopeSamples: one path per file type upstream lints, which a ported rule must be in scope for. */
  inScopeSamples: z.array(relativePath).min(1),
  /** inCorpus as data: the file extensions upstream lints in a corpus, and whether they are matched case-sensitively. */
  corpus: z.strictObject({
    /** Extensions with their dot (`.py`, `.pyi`), sorted. */
    extensions: sortedStrings(z.string().regex(/^\.[A-Za-z0-9]+$/u)),
    /** True when a path's extension must be written as listed (Ruff, Bandit); false when any letter case matches (ESLint). */
    caseSensitive: z.boolean(),
  }),
  /** keywords: the upstream language's keywords, sorted. */
  keywords: sortedStrings(nonEmpty),
  /** commentVocabulary: words the language and the linter give meaning to inside comments, sorted. */
  commentVocabulary: sortedStrings(nonEmpty),
  /** fixtureWords: every word of all of upstream's test files and test configurations (not only the copied ones), sorted. */
  fixtureWords: sortedStrings(nonEmpty),
  /**
   * fixtureTexts, by origin: every upstream file the live adapter read a text from, by its path in upstream's checkout at
   * the pinned commit, the sha256 (UTF-8) of the text as the adapter produced it and the transform that produced it
   * (`notebook-code-cells`: a notebook's code cells joined), sorted by path; and every text that is no file (Ruff's
   * test-module string literals), sorted, each as often as the adapter read it. The gates fetch the files at the pinned
   * commit and check each digest. A text of Ruff's GPL-derived fixtures is never inline. Absent in an older recording,
   * whose fixtureTexts are the committed case sources.
   */
  fixtureTexts: z.strictObject({
    files: z.array(z.strictObject({ path: relativePath, sha256, transform: z.literal("notebook-code-cells").optional() }))
      .refine((files) => files.every((file, index) => index === 0 || byText(files[index - 1]!.path, file.path) < 0), "sorted by path, each path once"),
    inline: z.array(z.strictObject({ text: z.string() }))
      .refine((texts) => texts.every((entry, index) => index === 0 || byText(texts[index - 1]!.text, entry.text) <= 0), "sorted in code-unit order"),
  }).optional(),
  /** Name sets several rules share, by an id the recorder chooses (an upstream file path, `crates:ruff_python_ast`). */
  nameSets: z.record(nonEmpty, nameSetSchema),
  /** Every upstream rule the pack's conformance.json names for the tool that upstream has, by that name. */
  rules: z.record(nonEmpty, ruleSchema),
  /**
   * The names the recorder was asked for that the pinned upstream lacks (rule() is undefined), sorted. A name in neither
   * `rules` nor `absent` was never recorded: the recording is stale.
   */
  absent: sortedStrings(nonEmpty),
  /**
   * Independent parsers' verdicts on every committed case source as its cases read it, and on every known gap's probe,
   * sorted by path, script and sha256; each combination once.
   */
  parserVerdicts: z.array(verdictEntrySchema),
}).superRefine((recording, context) => {
  const issue = (path: (string | number)[], message: string) => context.addIssue({ code: "custom", path, message });
  for (const [name, rule] of Object.entries(recording.rules)) {
    for (const part of ["implementation", "shared"] as const) {
      for (const id of rule.names[part].sets) if (!(id in recording.nameSets)) issue(["rules", name, "names", part, "sets"], `names the set ${id}, which nameSets lacks`);
    }
    if (recording.absent.includes(name)) issue(["absent"], `${name} is recorded as a rule and as absent`);
  }
  const keys = recording.parserVerdicts.map((entry) => `${entry.path}\0${entry.script}\0${entry.sha256}`);
  if (!keys.every((key, index) => index === 0 || byText(keys[index - 1]!, key) < 0)) issue(["parserVerdicts"], "sorted by path, script and sha256, each combination once");
});

export type GatesRecording = z.infer<typeof gatesRecordingSchema>;

const corpusFileSchema = z.strictObject({
  /** The catalog task the file comes from, or `rerail:<corpus>` for a corpus the pack pins (CorpusFile.task). */
  task: nonEmpty,
  /** The file's path in its repository (CorpusFile.path). */
  path: relativePath,
  /** sha256 of the file's text, UTF-8 (CorpusFile.sha256); the gates join a fetched file to its answer by it. */
  sha256,
  /** Why upstream gives no ground truth for the file (its own parser rejects it), or null (UpstreamFile.excluded). */
  excluded: z.string().nullable(),
  /**
   * Upstream's findings on the file for each rule of `rules` that has any, each sorted by position; a rule of `rules`
   * absent here has none. Empty for an excluded file.
   */
  findings: z.record(nonEmpty, z.array(recordedSpanSchema).min(1)),
  /** Independent parsers' verdicts on the file, read at its path as a module. */
  verdicts: z.array(parserVerdictSchema).min(1),
});

export type RecordedCorpusFile = z.infer<typeof corpusFileSchema>;

export const gatesCorpusSchema = z.strictObject({
  format: z.literal(GATES_CORPUS_FORMAT),
  /** The tool, as in gates.json. */
  tool: nonEmpty,
  /** The pins of the corpus upstream was run on; a fetched corpus with other pins is not this recording's. */
  pins: z.strictObject({
    /** sha256 of conformance/gates/corpus-tasks.json's bytes: the pinned catalog tasks. */
    tasks: sha256,
    /** The corpora the pack pins in its conformance manifests, sorted by name, as readRerailCorpora reads them. */
    corpora: z.array(z.strictObject({
      name: nonEmpty,
      repository: nonEmpty,
      commit,
      paths: sortedStrings(nonEmpty),
      exclude: sortedStrings(nonEmpty),
    })),
  }),
  /** The rules upstream was asked about on every file (lintCorpus's names), sorted. */
  rules: sortedStrings(nonEmpty),
  /** Every corpus file of a type upstream lints, sorted by task then path. */
  files: z.array(corpusFileSchema),
}).superRefine((recording, context) => {
  const issue = (path: (string | number)[], message: string) => context.addIssue({ code: "custom", path, message });
  const places = recording.files.map((file) => `${file.task}\0${file.path}`);
  if (!places.every((place, index) => index === 0 || byText(places[index - 1]!, place) < 0)) issue(["files"], "sorted by task then path, each file once");
  const rules = new Set(recording.rules);
  recording.files.forEach((file, index) => {
    for (const rule of Object.keys(file.findings)) if (!rules.has(rule)) issue(["files", index, "findings"], `has findings of ${rule}, which rules lacks`);
    if (file.excluded !== null && Object.keys(file.findings).length > 0) issue(["files", index, "findings"], "an excluded file has no findings");
  });
});

export type GatesCorpusRecording = z.infer<typeof gatesCorpusSchema>;

/**
 * A recording's text, the same on every machine: objects with their keys in code-unit order, two-space indentation, a
 * span tuple on one line, and a final newline.
 */
export function recordingText(value: GatesRecording | GatesCorpusRecording): string {
  const write = (node: unknown, indent: string): string => {
    if (node === null || typeof node !== "object") return JSON.stringify(node);
    const inner = `${indent}  `;
    if (Array.isArray(node)) {
      if (node.length === 0) return "[]";
      if (node.every((item) => item === null || typeof item === "number")) return `[${node.map((item) => JSON.stringify(item)).join(", ")}]`;
      return `[\n${node.map((item) => `${inner}${write(item, inner)}`).join(",\n")}\n${indent}]`;
    }
    const keys = Object.keys(node).sort(byText);
    if (keys.length === 0) return "{}";
    return `{\n${keys.map((key) => `${inner}${JSON.stringify(key)}: ${write((node as Record<string, unknown>)[key], inner)}`).join(",\n")}\n${indent}}`;
  };
  return `${write(value, "")}\n`;
}
