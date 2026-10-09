/**
 * The known parser gaps ledger (`known-parser-gaps.json`), part of the language conformance layer: source the language
 * accepts that a grammar Rerail pins cannot parse, each with the smallest probe that shows it. A gap is fixed in the
 * grammar, never excused in a rule's validation. `known-parser-gaps.test.ts` proves every probe still fails to parse
 * and every entry's grammar is still the one Rerail builds.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";
import { CODE_RULE_AST_LANGUAGES, LANGUAGE_REGISTRY } from "../../src/code-rule-languages.js";
import { parseJsonWithoutDuplicateKeys } from "../../src/json-duplicate-keys.js";
import { RERAIL_ROOT } from "../languages/pack.js";

export const KNOWN_PARSER_GAPS_PATH = join(RERAIL_ROOT, "conformance", "t1", "known-parser-gaps.json");

const nonEmpty = z.string().min(1);

/** How an entry's upstream test files were tied to its probe, strongest first. */
export const ATTRIBUTIONS = [
  /** A copy of the same upstream case under `packs/` holds the construct and fails to parse there too. */
  "copied-fixture",
  /** An independent parser's error in V1's report names the construct. */
  "parser-message",
  /** V1's report alone (the rule, the dialects its document applies to, the first error recorded), without the case's source. */
  "inferred",
  /** No upstream test file is tied to the probe yet. */
  "unattributed",
] as const;

const sourceSchema = z
  .strictObject({
    /** V1 validation reports (`results/validation/<report>/report.json`) listing every file below as a gap candidate. */
    reports: z.array(nonEmpty),
    /** V1's gap candidates the probe accounts for, as V1 names upstream test files. */
    upstreamTestFiles: z.array(nonEmpty),
    attribution: z.enum(ATTRIBUTIONS),
  })
  .refine(source => (source.attribution === "unattributed") === (source.upstreamTestFiles.length === 0), {
    message: "an unattributed gap lists no upstream test files, and an attributed one lists at least one",
  })
  .refine(source => (source.reports.length === 0) === (source.upstreamTestFiles.length === 0), {
    message: "reports are listed exactly when upstream test files are",
  });

export const knownParserGapSchema = z.strictObject({
  id: z.string().regex(/^[a-z0-9]+(?:-[a-z0-9]+)*$/u),
  /** Every dialect of this language is probed. */
  language: z.enum(CODE_RULE_AST_LANGUAGES),
  /** The upstream grammar and the version Rerail pins (`pinnedGrammar`). */
  grammar: nonEmpty,
  version: nonEmpty,
  construct: nonEmpty,
  /** The smallest source the language accepts and the grammar cannot parse. */
  probe: nonEmpty,
  source: sourceSchema,
});
export type KnownParserGap = z.infer<typeof knownParserGapSchema>;

export const knownParserGapsSchema = z.strictObject({
  /** The V1 rule validation ladder whose reports the entries cite. */
  v1: z.strictObject({ repository: nonEmpty, commit: z.string().regex(/^[0-9a-f]{40}$/u) }),
  gaps: z.array(knownParserGapSchema).superRefine((gaps, context) => {
    const seen = new Set<string>();
    for (const gap of gaps) {
      if (seen.has(gap.id)) context.addIssue({ code: "custom", message: `gap ${gap.id} is listed twice` });
      seen.add(gap.id);
    }
  }),
});
export type KnownParserGaps = z.infer<typeof knownParserGapsSchema>;

export function loadKnownParserGaps(path = KNOWN_PARSER_GAPS_PATH): KnownParserGaps {
  const parsed = knownParserGapsSchema.safeParse(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), "known-parser-gaps.json"));
  if (!parsed.success) throw new Error(`known-parser-gaps.json: ${z.prettifyError(parsed.error)}`);
  return parsed.data;
}

/** The dialects of a language as the registry lists them, each with the suffix its probe file takes. */
export function dialectsOf(language: string): { label: string; suffix: string; grammar: string }[] {
  const entry = LANGUAGE_REGISTRY.find(candidate => candidate.id === language);
  if (entry === undefined) throw new Error(`language ${language} is not in the registry`);
  return entry.dialects.map(dialect => {
    if (!("grammar" in dialect)) throw new Error(`dialect ${dialect.label} of ${language} has no grammar yet`);
    return { label: dialect.label, suffix: dialect.suffixes[0]!, grammar: dialect.grammar };
  });
}

/**
 * The grammar a language's dialects are built from, as Rerail pins it: a vendored grammar by the upstream tag its
 * `native/grammars/tree-sitter-<language>/PATCHES.md` records, an ast-grep one by its crate's version in `Cargo.lock`.
 */
export function pinnedGrammar(language: string): { grammar: string; version: string } {
  const grammar = `tree-sitter-${language}`;
  const kinds = [...new Set(dialectsOf(language).map(dialect => dialect.grammar.split(":")[0]))];
  if (kinds.length === 1 && kinds[0] === "vendored") {
    const patches = readFileSync(join(RERAIL_ROOT, "native", "grammars", grammar, "PATCHES.md"), "utf8");
    const tag = /^Upstream: \S+, tag `v([^`]+)`/mu.exec(patches);
    if (tag === null) throw new Error(`native/grammars/${grammar}/PATCHES.md names no upstream tag`);
    return { grammar, version: tag[1]! };
  }
  if (kinds.length === 1 && kinds[0] === "ast-grep") {
    const lock = readFileSync(join(RERAIL_ROOT, "Cargo.lock"), "utf8");
    const versions = [...lock.matchAll(new RegExp(`^name = "${grammar}"\\nversion = "([^"]+)"$`, "gmu"))].map(match => match[1]!);
    if (versions.length !== 1) throw new Error(`Cargo.lock locks ${versions.length} versions of ${grammar}`);
    return { grammar, version: versions[0]! };
  }
  throw new Error(`no grammar pin is read for ${language}'s dialects (${kinds.join(", ")})`);
}
