/**
 * The pack validation contract (docs/pack-format.md, "The validation contract"): checks 1 to 9 over one
 * `packs/<id>/` directory. Every failure is filed under a sub-check (checks.ts); scrub.json covers known failures of
 * existing rules; every rule gets one outcome per rule-level sub-check. Test support, not runtime: test/packs.test.ts
 * and conformance/validate-packs.ts call it, and it reuses the conformance runner for ported rules, so it lives beside
 * that runner and stays out of the published core.
 */
import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { basename, join, matchesGlob, relative, sep } from "node:path";
import { z } from "zod";
import { languageFor } from "../src/code-rule-enforcer.js";
import { languageNotSupportedOf } from "../src/code-rule-language-support.js";
import { CODE_RULE_DIALECTS, codeRuleLanguageDialects, isCodeRuleAstLanguage } from "../src/code-rule-languages.js";
import { codeRuleSchema } from "../src/code-rule-schema.js";
import { findDuplicateJsonKeys, describeDuplicateJsonKey, parseJsonWithoutDuplicateKeys } from "../src/json-duplicate-keys.js";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../src/native.js";
import { rulePackFileSchema, rulePackPackageName, rulePackPackageSchema, type RulePackFile } from "../src/rule-packs.js";
import { compareDeterministicStrings, stableJsonText } from "../src/stable-json.js";
import { PACK_CHECKS, RULE_SUB_CHECK_IDS, SUB_CHECKS, type PackCheckName, type RuleSubCheckId, type SubCheckId, type SubCheckOutcome } from "./checks.js";
import { capabilitiesJson } from "./engine.js";
import { generalityGate, generalityReport, inputsFromSource, readFixtureTexts } from "./gates/generality.js";
import { selectedText } from "./gates/probe.js";
import { ruleProvenanceChecks, toolPins } from "./gates/provenance.js";
import { sizeGate } from "./gates/size.js";
import {
  CONFORMANCE_FILE,
  defaultCorpusRoot,
  fixturesDirectory,
  loadPack,
  RERAIL_ROOT,
  UNSCORED_REASONS,
  UPSTREAM_SOURCE_FILE,
  type LoadedPack,
  type UnscoredReason,
  type UnscoredTest,
  type UpstreamSource,
} from "./languages/pack.js";
import { runPack } from "./languages/runner.js";
import { loadScrub, SCRUB_FILE, type ScrubEntry } from "./scrub.js";
import { GOLDEN_FORMAT, goldenRules, membershipProblems, parseGoldenManifest, type GoldenManifest } from "./tslint/golden.js";
import type { RuleReport, ValidationReport } from "./validation-report.js";

export { PACK_CHECKS, SUB_CHECKS, type PackCheckName, type PackRuleStatus, type SubCheckId, type SubCheckOutcome } from "./checks.js";

export interface PackValidation {
  id: string;
  directory: string;
  /**
   * Each check's failures that scrub.json does not cover, in a stable order: the pack's own (tool- and pack-level)
   * first, then each rule's in pack order. A line is the failure's text, prefixed `<sub-check>: ` unless the sub-check
   * is the check's own bar. An empty list passes the check.
   */
  checks: Record<PackCheckName, string[]>;
  /**
   * Why a check is not judged for some of the pack's rules, such as no source facts recorded for a tool. A check
   * listed here never counts as passed: its test fails on a failure, and is skipped otherwise.
   */
  notJudged: Partial<Record<PackCheckName, string[]>>;
  /** conformance.json's tools; none for a pack another harness scores. */
  tools: ValidationReport["tools"];
  /** Tool-level and pack-level failures: never scrubbed, and no rule's status. */
  packFailures: ValidationReport["packFailures"];
  /** The pack's scrub.json; empty when it has none or it does not load (a `rules` pack failure). */
  scrub: ScrubEntry[];
  /** Every rule pack.json lists, through every rule-level sub-check. */
  rules: RuleReport[];
  /** No pack failure and no failed rule. */
  pass: boolean;
}

export interface ValidatePackOptions {
  /**
   * Check 4 always scores the ported rules on the tools' fixtures. Corpora: `true` scores every pinned corpus (a
   * missing checkout fails), `false` none, and by default those checked out under `corpusRoot`.
   */
  corpora?: boolean;
  /** Corpus checkouts, `<corpusRoot>/<corpus name>`; default the nearest `.rerail/corpora/<pack>`. */
  corpusRoot?: string;
}

/** Validation material: kept in the repository, never published. */
const VALIDATION_MATERIAL = [CONFORMANCE_FILE, "divergences.json", SCRUB_FILE, "upstream", "candidates"] as const;
/** Rerail's own licence for its material in a pack. */
const RERAIL_LICENSE = "Apache-2.0";
const UPSTREAM_LICENSE = /\bLicense: ([A-Za-z0-9][A-Za-z0-9.+-]*)/u;
const UPSTREAM_ROW = /^\|\s*`([^`]+)`/u;
/** The runner's T4 header needs a date; nothing check 4 reads carries it. */
const RUN_DATE = "1970-01-01";
/** The upstream tool of a golden-manifest pack (conformance/tslint/golden.ts). */
const GOLDEN_TOOL = "tslint";
const GOLDEN_NOT_JUDGED = "the TSLint harness's golden gate scores this port against TSLint's own tests (npm run test:conformance)";

let allowlist: ReadonlySet<string> | undefined;
/** The repository's licence allowlist, the one `scripts/check-licenses.mjs` enforces. */
function allowedLicenses(): ReadonlySet<string> {
  allowlist ??= new Set(z.array(z.string()).parse(JSON.parse(readFileSync(join(RERAIL_ROOT, "scripts", "licence-allowlist.json"), "utf8"))));
  return allowlist;
}

const evaluationSchema = z.object({ violations: z.array(z.object({ code: z.string() })).default([]), failures: z.array(z.unknown()).default([]) });

/** A code rule's violation codes and failures on one file, evaluated as a one-file repository snapshot. */
export function evaluateExample(compiled: NativeCompiledCodeRuleDsl, path: string, source: string): { codes: string[]; failures: unknown[] } {
  const language = languageFor(path) ?? "text";
  const input = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: [{ language, path, source }] });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  return { codes: result.violations.map(violation => violation.code), failures: result.failures };
}

const documentSchema = z.object({
  id: z.string(),
  clauses: z.array(z.looseObject({ violation: z.object({ code: z.string() }).optional() })),
});

type Compiled =
  | { kind: "compiled"; compiled: NativeCompiledCodeRuleDsl }
  | { kind: "not-supported-yet"; reason: string }
  | { kind: "failed"; message: string };

const message = (error: unknown): string => (error instanceof Error ? error.message : String(error));
const isRecord = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);

/** `text` compiled by the engine against core's capabilities; `language-not-supported` is not a failure. */
function compileText(text: string): Compiled {
  try {
    return { kind: "compiled", compiled: loadNativeBinding().CodeRuleDslRuntime.compile(text, "json", capabilitiesJson()) };
  } catch (error) {
    const notSupported = languageNotSupportedOf(message(error));
    return notSupported === undefined ? { kind: "failed", message: message(error) } : { kind: "not-supported-yet", reason: notSupported.reason };
  }
}

/** Every AST language a document's clauses name, anywhere (clauses, utilities, definitions). */
function astLanguages(value: unknown, found = new Set<string>()): Set<string> {
  if (Array.isArray(value)) for (const item of value) astLanguages(item, found);
  else if (isRecord(value)) {
    for (const [key, entry] of Object.entries(value)) {
      if (key === "ast" && isRecord(entry) && typeof entry.language === "string") found.add(entry.language);
      astLanguages(entry, found);
    }
  }
  return found;
}

/**
 * `src/example<suffix>` for every suffix of each language's own dialects, sorted. A dialect another language is named
 * for is that language's: `typescript` also reads `javascript` files, but a TypeScript rule (TSLint's) need not.
 */
function inScopeSamples(languages: ReadonlySet<string>): string[] {
  const samples = [...languages]
    .filter(isCodeRuleAstLanguage)
    .flatMap(language =>
      codeRuleLanguageDialects(language)
        .filter(dialect => dialect === language || !isCodeRuleAstLanguage(dialect))
        .flatMap(dialect => CODE_RULE_DIALECTS[dialect].suffixes.map(suffix => `src/example${suffix}`)),
    );
  return [...new Set(samples)].sort(compareDeterministicStrings);
}

/** One line per reason: how many of a rule's upstream tests check 4 does not score, and the first few of them. */
function unscoredLines(unscored: readonly UnscoredTest[]): string[] {
  return (Object.keys(UNSCORED_REASONS) as UnscoredReason[]).flatMap(reason => {
    const tests = unscored.filter(entry => entry.reason === reason).map(entry => (entry.case === null ? entry.test : `${entry.test} ${entry.case}`));
    if (tests.length === 0) return [];
    const named = `${tests.slice(0, 3).join("; ")}${tests.length > 3 ? `; and ${tests.length - 3} more` : ""}`;
    return [`${tests.length} upstream test(s) not among the fixtures check 4 scores, ${UNSCORED_REASONS[reason]}: ${named}`];
  });
}

/** Every file under `directory`, as posix paths relative to `root`, sorted. */
function filesUnder(root: string, directory: string): string[] {
  if (!existsSync(directory)) return [];
  const files: string[] = [];
  const walk = (current: string): void => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.isFile()) files.push(relative(root, path).split(sep).join("/"));
    }
  };
  walk(directory);
  return files.sort(compareDeterministicStrings);
}

/** Whether a `files` entry of package.json publishes `path` (npm publishes a listed directory whole). */
function publishes(entry: string, path: string): boolean {
  const pattern = entry.replace(/^\.\//u, "").replace(/\/+$/u, "");
  return pattern === path || path.startsWith(`${pattern}/`) || matchesGlob(path, pattern) || matchesGlob(`${path}/x`, pattern);
}

/** The SPDX ids an SPDX expression names. */
const spdxIds = (expression: string): string[] =>
  expression
    .split(/\s+(?:AND|OR|WITH)\s+|[()]/u)
    .map(part => part.trim())
    .filter(part => part.length > 0);

interface Licences {
  /** The licence each `licenses/<project>/UPSTREAM.md` records. */
  recorded: Map<string, string>;
  /** Pack-relative paths the UPSTREAM.md tables list. */
  listed: Map<string, string>;
  failures: string[];
}

function readLicences(directory: string): Licences {
  const recorded = new Map<string, string>();
  const listed = new Map<string, string>();
  const failures: string[] = [];
  const licenses = join(directory, "licenses");
  if (!existsSync(licenses)) return { recorded, listed, failures };
  for (const project of readdirSync(licenses, { withFileTypes: true }).filter(entry => entry.isDirectory()).map(entry => entry.name).sort(compareDeterministicStrings)) {
    const record = `licenses/${project}/UPSTREAM.md`;
    const path = join(directory, record);
    if (!existsSync(path)) {
      failures.push(`licenses/${project}/: no UPSTREAM.md`);
      continue;
    }
    if (!readdirSync(join(licenses, project)).some(name => name !== "UPSTREAM.md")) failures.push(`licenses/${project}/: no upstream licence text beside UPSTREAM.md`);
    const text = readFileSync(path, "utf8");
    const licence = UPSTREAM_LICENSE.exec(text)?.[1];
    if (licence === undefined) failures.push(`${record}: no "License: <SPDX id>" line`);
    else recorded.set(project, licence);
    for (const line of text.split("\n")) {
      const file = UPSTREAM_ROW.exec(line)?.[1];
      if (file !== undefined && !listed.has(file)) listed.set(file, record);
    }
  }
  return { recorded, listed, failures };
}

type ListedRule = RulePackFile["rules"][number];

/** One rule on its way through the checks. */
class RuleState {
  readonly results = new Map<RuleSubCheckId, { outcome: SubCheckOutcome; details: string[] }>();
  text: string | undefined;
  /** The document as JSON, when it is JSON. */
  json: unknown;
  /** The document as the catalog stores it (`codeRuleSchema`'s output, options resolved), when the schema accepts it. */
  stored: { clauses?: unknown } | undefined;
  compiled: Compiled | undefined;
  port: RuleReport["port"] = null;
  generality: RuleReport["generality"] = null;
  size: RuleReport["size"] = null;
  ports: RuleReport["ports"] = null;

  constructor(readonly listed: ListedRule) {}

  get id(): string {
    return this.listed.id;
  }

  get notSupported(): string | undefined {
    return this.compiled?.kind === "not-supported-yet" ? this.compiled.reason : undefined;
  }

  fail(subCheck: RuleSubCheckId, detail: string): void {
    const result = this.results.get(subCheck);
    if (result?.outcome === "failed") result.details.push(detail);
    else this.results.set(subCheck, { outcome: "failed", details: [detail] });
  }

  /** `ok`, unless the sub-check already failed. */
  pass(subCheck: RuleSubCheckId): void {
    if (!this.results.has(subCheck)) this.results.set(subCheck, { outcome: "ok", details: [] });
  }

  judge(subCheck: RuleSubCheckId, ok: boolean, detail: string): void {
    if (ok) this.pass(subCheck);
    else this.fail(subCheck, detail);
  }

  mark(subCheck: RuleSubCheckId, outcome: "n/a" | "not-judged" | "not-supported-yet", detail: string): void {
    if (!this.results.has(subCheck)) this.results.set(subCheck, { outcome, details: [detail] });
  }
}

const SUB_CHECK = new Map(SUB_CHECKS.map(subCheck => [subCheck.id as SubCheckId, subCheck]));
const checkOf = (subCheck: SubCheckId): PackCheckName => SUB_CHECK.get(subCheck)!.check;
const numberOf = (check: PackCheckName): number => PACK_CHECKS.find(entry => entry.name === check)!.number;
/** A failure's line in `PackValidation.checks`. */
const failureLine = (subCheck: SubCheckId, detail: string): string => (subCheck === checkOf(subCheck) ? detail : `${subCheck}: ${detail}`);

/** The check 7 to 9 sub-checks a port is judged on. */
const FACT_SUB_CHECKS = SUB_CHECKS.filter(subCheck => subCheck.needsSourceFacts && subCheck.level === "rule").map(subCheck => subCheck.id as RuleSubCheckId);
const PORT_SUB_CHECKS = ["ports:points", "ports:span", "ports:unknown-misses", "ports:evidence"] as const satisfies readonly RuleSubCheckId[];

/**
 * Validates `packs/<id>/` against the contract. Check 4 runs the conformance runner (the engine's production workspace
 * path), so a pack that ports rules takes as long as its fixture tier.
 */
export async function validatePack(directory: string, options: ValidatePackOptions = {}): Promise<PackValidation> {
  const id = basename(directory);
  const packFailures: ValidationReport["packFailures"] = [];
  const packFail = (subCheck: SubCheckId, detail: string, tool: string | null = null): void => {
    packFailures.push({ subCheck, tool, detail });
  };

  // pack.json, and the ported rules by the document they port. A pack's conformance.json is the conformance runner's
  // pack (conformance/languages/pack.ts), or, for TSLint, the golden manifest of the TSLint harness
  // (conformance/tslint/golden.ts), whose upstream tests are fetched at a pinned commit rather than recorded.
  let pack: RulePackFile | undefined;
  const packPath = join(directory, "pack.json");
  if (!existsSync(packPath)) packFail("rules", "pack.json is missing");
  else {
    try {
      const parsed = rulePackFileSchema.safeParse(parseJsonWithoutDuplicateKeys(readFileSync(packPath, "utf8"), "pack.json"));
      if (parsed.success) pack = parsed.data;
      else packFail("rules", `pack.json: ${z.prettifyError(parsed.error)}`);
    } catch (error) {
      packFail("rules", `pack.json: ${message(error)}`);
    }
  }
  const conformancePath = join(directory, CONFORMANCE_FILE);
  let loaded: LoadedPack | undefined;
  let golden: GoldenManifest | undefined;
  if (existsSync(conformancePath)) {
    try {
      const text = readFileSync(conformancePath, "utf8");
      if (z.looseObject({ format: z.literal(GOLDEN_FORMAT) }).safeParse(JSON.parse(text)).success) golden = parseGoldenManifest(text);
      else loaded = loadPack(directory);
    } catch (error) {
      packFail("ports", `${CONFORMANCE_FILE}: ${message(error).replace(`${directory}${sep}`, "")}`);
    }
  }
  // Each tool's source facts, for a tool that declares approved configurations; undefined when source.json is missing or
  // invalid (provenance:pins, reported with checks 7 to 9).
  const pins = (loaded?.pack.tools ?? []).filter(tool => tool.recommended !== undefined).map(tool => ({ tool, ...toolPins(loaded!, { name: tool.name, version: tool.version, recommended: tool.recommended! }) }));
  const sources = new Map<string, UpstreamSource | undefined>(pins.map(({ tool, source }) => [tool.name, source]));
  const portByDocument = new Map((loaded?.pack.rules ?? []).map(port => [port.policy.replace(/^\.\//u, ""), port]));
  const goldenDocuments = new Set(Object.keys(golden?.rules ?? {}).map(rule => `rules/${rule}.json`));
  const isPort = (rule: ListedRule): boolean => portByDocument.has(rule.document) || goldenDocuments.has(rule.document);

  const states = (pack?.rules ?? []).map(rule => new RuleState(rule));
  const stateById = new Map(states.map(state => [state.id, state]));

  // Check 1: rules. pack.json parses and names the pack, every rules/*.json is listed, and every listed document exists,
  // is JSON with no repeated key, carries the listed id, is a valid rerail.policy/v1 code rule, and compiles as written
  // and as the catalog stores it, to the listed id. Its clauses need no model, and a port applies to every file type its
  // tool's approved configurations lint (source.json `extensions`), or without source facts, every file of each
  // language its clauses name.
  if (pack !== undefined) {
    if (pack.id !== id) packFail("rules", `pack.json: id is ${pack.id}; the directory is ${id}`);
    const byDocument = new Map<string, string>();
    const seen = new Set<string>();
    for (const state of states) {
      const { listed } = state;
      if (seen.has(listed.id)) state.fail("rules", `${listed.id}: listed twice`);
      seen.add(listed.id);
      const other = byDocument.get(listed.document);
      if (other !== undefined) state.fail("rules", `${listed.id}: ${listed.document} is also ${other}'s document`);
      byDocument.set(listed.document, listed.id);
    }
    for (const file of filesUnder(directory, join(directory, "rules"))) {
      if (!byDocument.has(file)) packFail("rules", `${file} is not listed in pack.json`);
    }
  }
  for (const state of states) {
    const { listed } = state;
    const notRun = (why: string): string => `${listed.id}: not run: ${why}`;
    const path = join(directory, listed.document);
    if (!existsSync(path)) {
      state.fail("rules", `${listed.id}: ${listed.document} does not exist`);
      state.fail("rules:deterministic", notRun(`${listed.document} does not exist`));
    } else {
      state.text = readFileSync(path, "utf8");
      try {
        state.json = JSON.parse(state.text);
      } catch (error) {
        state.fail("rules", `${listed.id}: ${listed.document} is not JSON: ${message(error)}`);
      }
    }
    const { text, json } = state;
    if (text !== undefined && json !== undefined) {
      const repeated = findDuplicateJsonKeys(text);
      if (repeated.length > 0) state.fail("rules", `${listed.id}: ${listed.document} repeats a key: ${repeated.map(describeDuplicateJsonKey).join("; ")}`);
      const documentId = z.object({ id: z.string() }).safeParse(json);
      if (documentId.success && documentId.data.id !== listed.id) state.fail("rules", `${listed.id}: ${listed.document} has id ${documentId.data.id}`);
      const clauses = isRecord(json) && Array.isArray(json.clauses) ? json.clauses : [];
      const llm = clauses.findIndex(clause => isRecord(clause) && "llm" in clause);
      state.judge("rules:deterministic", llm === -1, `${listed.id}: ${listed.document} clause ${llm} is an llm clause, which needs a model at evaluation time`);
      const valid = codeRuleSchema.safeParse(json);
      if (!valid.success) state.fail("rules", `${listed.id}: ${listed.document} is not a valid code rule (rerail.policy/v1): ${z.prettifyError(valid.error)}`);
      else {
        state.stored = valid.data;
        const written = compileText(text);
        if (written.kind === "failed") state.fail("rules", `${listed.id}: ${listed.document} does not compile: ${written.message}`);
        else if (written.kind === "not-supported-yet") state.compiled = written;
        else {
          // The catalog compiles a stored rule's stable JSON text (src/code-rule-dsl-catalog.ts), and checks its id.
          const stored = stableJsonText(valid.data);
          const catalog = stored === undefined ? ({ kind: "failed", message: "the document is not JSON representable" } as const) : compileText(stored);
          if (catalog.kind !== "compiled") state.fail("rules", `${listed.id}: ${listed.document} does not compile as the catalog stores it: ${catalog.kind === "failed" ? catalog.message : catalog.reason}`);
          else {
            state.compiled = catalog;
            const compiledId = z.object({ id: z.unknown() }).parse(JSON.parse(catalog.compiled.canonicalJson())).id;
            if (compiledId !== listed.id) state.fail("rules", `${listed.id}: ${listed.document} compiles to id ${JSON.stringify(compiledId)}`);
          }
        }
      }
    } else if (text !== undefined) state.fail("rules:deterministic", notRun(`${listed.document} is not JSON`));
    state.pass("rules");

    if (isPort(listed)) {
      const extensions = sources.get(portByDocument.get(listed.document)?.upstream.tool ?? "")?.extensions;
      if (state.notSupported !== undefined) state.mark("rules:in-scope", "not-supported-yet", state.notSupported);
      else if (state.compiled?.kind !== "compiled") state.fail("rules:in-scope", notRun("the document does not compile (check 1)"));
      else {
        const samples = extensions === undefined ? inScopeSamples(astLanguages(json)) : extensions.map(extension => `src/example${extension}`);
        if (samples.length === 0) state.mark("rules:in-scope", "n/a", "its clauses name no AST language");
        else {
          const { compiled } = state.compiled;
          const outside = samples.filter(sample => !compiled.inScope(sample));
          state.judge("rules:in-scope", outside.length === 0, `${listed.id}: ${listed.document} does not apply to ${outside.join(", ")}`);
        }
      }
    }
  }

  // Check 2: proof. Every rule has examples or a port entry.
  // Check 3: examples. A caught example produces a violation of the rule, a passes example none.
  if (pack === undefined) {
    packFail("proof", "pack.json does not parse; see check 1");
    packFail("examples", "pack.json does not parse; see check 1");
  }
  for (const state of states) {
    const { listed } = state;
    state.judge("proof", listed.examples !== undefined || isPort(listed), `${listed.id}: no examples and no port entry in ${CONFORMANCE_FILE}`);
    if (listed.examples === undefined) continue;
    if (state.notSupported !== undefined) continue;
    if (state.compiled?.kind !== "compiled") {
      state.fail("examples", `${listed.id}: not run: the document fails check 1`);
      continue;
    }
    const { compiled } = state.compiled;
    const codes = new Set(documentSchema.parse(state.json).clauses.flatMap(clause => (clause.violation === undefined ? [] : [clause.violation.code])));
    for (const side of ["caught", "passes"] as const) {
      const example = listed.examples[side];
      if (example.path === undefined) {
        state.fail("examples", `${listed.id}: the ${side} example has no path`);
        continue;
      }
      if (!compiled.inScope(example.path)) {
        state.fail("examples", `${listed.id}: the ${side} example's path ${example.path} is outside the rule's scope`);
        continue;
      }
      const evaluation = evaluateExample(compiled, example.path, example.text);
      if (evaluation.failures.length > 0) state.fail("examples", `${listed.id}: the ${side} example (${example.path}) fails in the engine: ${JSON.stringify(evaluation.failures)}`);
      const hits = evaluation.codes.filter(code => codes.has(code));
      if (side === "caught" && hits.length === 0) state.fail("examples", `${listed.id}: the caught example (${example.path}) produces no ${[...codes].join("/")} violation`);
      if (side === "passes" && hits.length > 0) state.fail("examples", `${listed.id}: the passes example (${example.path}) produces ${hits.join(", ")}`);
    }
    state.pass("examples");
  }

  // Check 4: ports. The conformance runner's fixture tier (and corpora) for every port core reads, each port judged on
  // its sub-checks; the pack's capability coverage; a golden manifest lists exactly the pack's rules.
  const stateOfDocument = new Map(states.map(state => [state.listed.document, state]));
  if (loaded !== undefined) {
    const notSupported = new Set<string>();
    for (const port of loaded.pack.rules) {
      const state = stateOfDocument.get(port.policy.replace(/^\.\//u, ""));
      if (state !== undefined) state.port = { id: port.id, tool: port.upstream.tool, rule: port.upstream.rule };
      if (state?.notSupported !== undefined) notSupported.add(port.id);
      else if (state === undefined && existsSync(join(directory, port.policy)) && compileText(readFileSync(join(directory, port.policy), "utf8")).kind === "not-supported-yet") notSupported.add(port.id);
    }
    const corpusRoot = options.corpusRoot ?? defaultCorpusRoot(loaded.pack.pack);
    const corpora = loaded.pack.corpora.filter(corpus => (options.corpora === undefined ? existsSync(join(corpusRoot, corpus.name)) : options.corpora)).map(corpus => corpus.name);
    const run = await runPack(loaded, { rules: loaded.pack.rules.filter(port => !notSupported.has(port.id)).map(port => port.id), fixtures: true, corpora, corpusRoot, date: RUN_DATE });
    for (const port of loaded.pack.rules) {
      const score = run.ports.get(port.id);
      if (score === undefined) continue;
      const state = stateOfDocument.get(port.policy.replace(/^\.\//u, ""));
      if (state === undefined) {
        for (const failure of Object.values(score.failures).flat()) packFail("ports", failure);
        continue;
      }
      for (const subCheck of PORT_SUB_CHECKS) {
        for (const failure of score.failures[subCheck]) state.fail(subCheck, failure);
        state.pass(subCheck);
      }
      state.ports = score.working;
    }
    for (const failure of run.unowned) packFail("ports", failure);
    for (const capability of run.coverage.uncovered) packFail("ports", `capability ${capability}: no ported rule exercises it and it has no exemption`);
  }
  if (golden !== undefined) {
    for (const problem of membershipProblems(golden, goldenRules(join(directory, "rules")))) packFail("ports", problem);
    for (const state of states) {
      if (!goldenDocuments.has(state.listed.document)) continue;
      const name = basename(state.listed.document, ".json");
      state.port = { id: name, tool: GOLDEN_TOOL, rule: name };
      if (state.notSupported !== undefined) continue;
      for (const subCheck of PORT_SUB_CHECKS) state.mark(subCheck, "not-judged", GOLDEN_NOT_JUDGED);
    }
  }

  // Check 5: package. Named for the pack, no dependencies, no validation material published, licence agreeing with
  // what licenses/ records.
  const licences = readLicences(directory);
  const packagePath = join(directory, "package.json");
  if (!existsSync(packagePath)) packFail("package", "package.json is missing");
  else {
    let raw: unknown;
    try {
      raw = parseJsonWithoutDuplicateKeys(readFileSync(packagePath, "utf8"), "package.json");
    } catch (error) {
      packFail("package", `package.json: ${message(error)}`);
    }
    const dependencies = z.looseObject({ dependencies: z.record(z.string(), z.unknown()).optional() }).safeParse(raw);
    const named = dependencies.success ? Object.keys(dependencies.data.dependencies ?? {}) : [];
    if (named.length > 0) packFail("package", `package.json: has dependencies (${named.join(", ")}); a pack has none`);
    const parsed = raw === undefined ? undefined : rulePackPackageSchema.safeParse(raw);
    if (parsed !== undefined && !parsed.success) packFail("package", `package.json: ${z.prettifyError(parsed.error)}`);
    if (parsed?.success === true) {
      const manifest = parsed.data;
      if (manifest.name !== rulePackPackageName(id)) packFail("package", `package.json: name is ${manifest.name}, not ${rulePackPackageName(id)}`);
      for (const material of VALIDATION_MATERIAL) {
        const entry = manifest.files.find(file => publishes(file, material));
        if (entry !== undefined) packFail("package", `package.json: files entry ${entry} publishes ${material}, which is validation material`);
      }
      const required = ["pack.json", "rules", "LICENSE", ...(existsSync(join(directory, "licenses")) ? ["licenses"] : [])];
      for (const path of required) {
        if (!manifest.files.some(file => publishes(file, path))) packFail("package", `package.json: files does not publish ${path}`);
      }
      const spdx = [...new Set(spdxIds(manifest.license))].sort(compareDeterministicStrings);
      const carried = [...new Set([RERAIL_LICENSE, ...licences.recorded.values()])].sort(compareDeterministicStrings);
      if (spdx.join(",") !== carried.join(","))
        packFail("package", `package.json: license ${JSON.stringify(manifest.license)} names ${spdx.join(", ")}; the pack carries ${carried.join(", ")} (${RERAIL_LICENSE} and what licenses/*/UPSTREAM.md records)`);
    }
  }

  // Check 6: licences. Every copied file is listed in an UPSTREAM.md, every listed file exists, and every recorded
  // licence is on the repository allowlist. Under upstream/<tool>/, the recordings and the recorder (files directly in
  // it) are Rerail's; everything in its subdirectories (fixtures/, tests/) is copied from the tool, and source.json
  // holds names, words and regexes extracted from it.
  for (const failure of licences.failures) packFail("licences", failure);
  if (!existsSync(join(directory, "LICENSE"))) packFail("licences", "LICENSE is missing");
  for (const [project, licence] of licences.recorded) {
    if (!allowedLicenses().has(licence)) packFail("licences", `licenses/${project}/UPSTREAM.md: ${licence} is not on the repository licence allowlist`);
  }
  for (const file of filesUnder(directory, join(directory, "upstream"))) {
    const parts = file.split("/");
    if (licences.listed.has(file)) continue;
    if (parts.length > 3) packFail("licences", `${file} is copied but no licenses/*/UPSTREAM.md lists it`);
    else if (parts.length === 3 && parts[2] === UPSTREAM_SOURCE_FILE) packFail("licences", `${file} is extracted from upstream but no licenses/*/UPSTREAM.md lists it`);
  }
  for (const [file, record] of licences.listed) {
    if (!existsSync(join(directory, file))) packFail("licences", `${record} lists ${file}, which does not exist`);
  }

  // Checks 7 to 9: provenance, generality and size, and check 4's coverage, judged for the ports of a tool that
  // declares its approved configurations and so has recorded source facts (upstream/<tool>/source.json). The tool's
  // pins are judged once.
  const tools: ValidationReport["tools"] = [];
  for (const { tool, failures } of pins) {
    for (const failure of failures) packFail("provenance:pins", failure.startsWith(`${tool.name}: `) ? failure.slice(tool.name.length + 2) : failure, tool.name);
  }
  for (const tool of loaded?.pack.tools ?? []) {
    const source = sources.get(tool.name);
    tools.push({
      name: tool.name,
      version: tool.version,
      recommended: tool.recommended ?? null,
      source: source === undefined ? null : { repository: source.repository, commit: source.commit, tag: source.tag },
    });
  }
  const fixtureTexts = new Map<string, string[]>();
  const textsOf = (tool: string): string[] => {
    let texts = fixtureTexts.get(tool);
    if (texts === undefined) {
      const fixtures = fixturesDirectory(loaded!, tool);
      texts = existsSync(fixtures) ? readFixtureTexts(fixtures) : [];
      fixtureTexts.set(tool, texts);
    }
    return texts;
  };
  for (const state of states) {
    const { port, listed } = state;
    if (port === null || state.notSupported !== undefined) continue;
    if (!sources.has(port.tool)) {
      for (const subCheck of FACT_SUB_CHECKS) state.mark(subCheck, "not-judged", `no source facts recorded for ${port.tool}`);
      continue;
    }
    const source = sources.get(port.tool);
    const notRun = (why: string): void => {
      for (const subCheck of FACT_SUB_CHECKS) state.fail(subCheck, `${listed.id}: not run: ${why}`);
    };
    if (source === undefined) {
      notRun(`${port.tool} has no valid source.json (provenance:pins)`);
      continue;
    }
    for (const check of ruleProvenanceChecks(port.rule, source)) state.judge(check.id as RuleSubCheckId, check.ok, `${listed.id}: ${check.detail}`);
    const facts = source.rules[port.rule];
    if (facts === undefined) state.fail("ports:coverage", `${listed.id}: not run: source.json records no ${port.rule} for ${port.tool}`);
    else for (const line of unscoredLines(facts.unscored)) state.fail("ports:coverage", `${listed.id}: ${line}`);
    state.pass("ports:coverage");
    if (state.stored === undefined || state.text === undefined) {
      for (const subCheck of FACT_SUB_CHECKS.filter(subCheck => checkOf(subCheck) === "generality" || checkOf(subCheck) === "size")) state.fail(subCheck, `${listed.id}: not run: the document fails check 1`);
      continue;
    }
    const inputs = inputsFromSource(source, port.rule, textsOf(port.tool), selectedText);
    if (inputs === undefined) {
      for (const subCheck of ["generality:names", "generality:regexes"] as const) state.fail(subCheck, `${listed.id}: not run: source.json records no ${port.rule} for ${port.tool}`);
    } else {
      const generality = generalityGate(state.stored, inputs);
      for (const check of generality.checks) state.judge(check.id as RuleSubCheckId, check.ok, `${listed.id}: ${check.detail}`);
      state.generality = generalityReport(generality);
    }
    const size = sizeGate(Buffer.byteLength(state.text, "utf8"), state.stored);
    for (const check of size.checks) state.judge(check.id as RuleSubCheckId, check.ok, `${listed.id}: ${check.detail}`);
    state.size = size.metrics;
  }
  // Every rule-level sub-check a rule has no result for does not apply to it, or waits for core to read its language.
  for (const state of states) {
    for (const subCheck of RULE_SUB_CHECK_IDS) {
      if (state.results.has(subCheck)) continue;
      const { applies } = SUB_CHECK.get(subCheck)!;
      if (applies === "ports" && state.port === null) state.mark(subCheck, "n/a", "the rule has no port entry; its examples prove it");
      else if (applies === "examples" && state.listed.examples === undefined) state.mark(subCheck, "n/a", "the rule has no examples");
      else if (state.notSupported !== undefined) state.mark(subCheck, "not-supported-yet", state.notSupported);
      else throw new Error(`${id}: ${state.id}: no result for ${subCheck}`);
    }
  }

  // scrub.json: a covered failure is scrubbed; an entry that covers no failure is stale and fails its sub-check.
  let scrub: ScrubEntry[] = [];
  try {
    scrub = loadScrub(directory, new Set(states.map(state => state.id)));
  } catch (error) {
    packFail("rules", message(error));
  }
  for (const entry of scrub) {
    const result = stateById.get(entry.rule)!.results.get(entry.check)!;
    if (result.outcome === "failed") result.outcome = "scrubbed";
    else {
      const was = result.outcome === "ok" ? "passes" : `is ${result.outcome}: ${result.details[0]!}`;
      stateById.get(entry.rule)!.results.set(entry.check, { outcome: "failed", details: [`${entry.rule}: ${was}; remove the ${SCRUB_FILE} entry`] });
    }
  }

  const rules = states.map((state): RuleReport => {
    const results = RULE_SUB_CHECK_IDS.map(subCheck => ({ subCheck, ...state.results.get(subCheck)! }));
    const outcomes = new Set(results.map(result => result.outcome));
    const stops = results.filter(result => result.outcome === "failed" || result.outcome === "scrubbed").map(result => numberOf(checkOf(result.subCheck)));
    const bytes = state.text === undefined ? null : Buffer.from(state.text, "utf8");
    return {
      id: state.id,
      document: state.listed.document,
      sha256: bytes === null ? null : createHash("sha256").update(bytes).digest("hex"),
      bytes: bytes === null ? null : bytes.length,
      port: state.port,
      status: outcomes.has("failed") ? "failed" : outcomes.has("scrubbed") ? "scrubbed" : outcomes.has("not-supported-yet") ? "not-supported-yet" : "ok",
      stoppedAt: stops.length === 0 ? null : Math.min(...stops),
      results,
      generality: state.generality,
      size: state.size,
      ports: state.ports,
    };
  });

  const checks = Object.fromEntries(PACK_CHECKS.map(check => [check.name, [] as string[]])) as Record<PackCheckName, string[]>;
  for (const failure of packFailures) checks[checkOf(failure.subCheck)].push(failureLine(failure.subCheck, failure.tool === null ? failure.detail : `${failure.tool}: ${failure.detail}`));
  const notJudged: Partial<Record<PackCheckName, string[]>> = {};
  for (const rule of rules) {
    for (const result of rule.results) {
      const check = checkOf(result.subCheck);
      if (result.outcome === "failed") checks[check].push(...result.details.map(detail => failureLine(result.subCheck, detail)));
      if (result.outcome === "not-judged" && !(notJudged[check] ?? []).includes(result.details[0]!)) notJudged[check] = [...(notJudged[check] ?? []), result.details[0]!];
    }
  }
  return { id, directory, checks, notJudged, tools, packFailures, scrub, rules, pass: packFailures.length === 0 && rules.every(rule => rule.status !== "failed") };
}
