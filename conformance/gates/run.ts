import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { join, relative, resolve } from "node:path";

import { ruleKey, readCached, trackedDigest, writeCached, GATE_CODE, GATE_DEPENDENCIES, type SharedKeyInputs } from "./cache.js";
import { upstreamStatusChecks, defaultOn } from "./checks/upstream-status.js";
import { loadAsBundled, loadsGate } from "./checks/loads.js";
import { notOverfitGate, type ClassifiedLiteral } from "./checks/not-overfit.js";
import { canonicalJson, compactGate, NESTING_RUN_LIMIT, REPEATED_SHARE_LIMIT, SIZE_FLAG_BYTES, type SizeMetrics } from "./checks/compact.js";
import { CASES_PER_KIND, differingFiles, MATCHES_REAL_CODE_METHOD, matchesRealCodeGate, type MatchesRealCodeResult, type RealCodeCase } from "./checks/matches-real-code.js";
import { classifyParsers, goldensGate, MATCHES_UPSTREAM_TESTS_METHOD, matchesUpstreamTestsGate, provenGaps, readPath, staysSilentCheck, type CaseResult, type GapCandidate, type GoldenResult, type MatchesUpstreamTestsResult, type UnparsedFile } from "./checks/matches-upstream-tests.js";
import { buildCorpus, buildRerailCorpora, corpusCompleteCheck, CORPUS_TASKS, readRerailCorpora, type CorpusFile, type CorpusTask } from "./corpus.js";
import { engineIdentity, loadEngine, type Engine, type EngineIdentity } from "./engine.js";
import { byText, promote, readGoldens, serializeGoldens, type Golden, type Goldens } from "./goldens.js";
import { GATES_MANIFEST, toolPackDirectory } from "./manifest.js";
import type { CommandRunner } from "./process.js";
import { createRecordedAdapter, readRecordedInputs, staleMessage } from "./recorded-adapter.js";
import { GATES_CORPUS, GATES_RECORDING } from "./recording.js";
import { shardOf, type Shard } from "./shard.js";
import { BLOCKING_GATES, GATES, verdict, type BlockingGateId, type Check, type GateId, type PackAdapter, type PackManifest, type PackRule, type ParserVerdict, type UpstreamCase, type UpstreamFile } from "./types.js";

export const REPORT_FORMAT = "rerail.rule-gates/v1" as const;

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

export interface GateReport {
  pass: boolean;
  checks: Check[];
}

/** How many reported items a list stands for, and the first few of them in order. */
export interface Sample<T> {
  count: number;
  first: T[];
}

/** A report lists this many items of a long list: enough to see why a rule fails. */
export const SAMPLE_SIZE = 5;

function sample<T>(items: T[]): Sample<T> {
  return { count: items.length, first: items.slice(0, SAMPLE_SIZE) };
}

/** Where a document sits: `rules/` ships and blocks; `candidates/**` is reported only. */
export type Location = "shipped" | "candidate";

/** A file the engine could not fully parse, with what independent parsers said, or `not-recorded` when nothing did. */
export interface GapCandidateReport extends Omit<GapCandidate, "verdict"> {
  verdict: GapCandidate["verdict"] | "not-recorded";
}

export interface RuleReport {
  id: string;
  /** Repository-relative path of the document. */
  file: string;
  location: Location;
  /** The upstream rule the document ports, as conformance.json maps it. */
  upstreamRule: string;
  sha256: string;
  bytes: number;
  /** The engine's content digest of the document, when it compiles. */
  policyDigest: string | null;
  /** The rule passes every blocking gate and its tool-pack's checks hold. */
  pass: boolean;
  /** The first blocking gate the rule failed; null when it passed them all. */
  stoppedAt: BlockingGateId | null;
  /** The failing tool-pack checks that fail the rule whatever its gates say (a stale recording, an incomplete corpus), or null. */
  blockedBy: string | null;
  /** upstream-status's call: on when it exists upstream, is stable and is recommended. */
  defaultOn: boolean;
  defaultOnReason: string;
  /** pack.json's `on` for a shipped rule (absent means on); null for a candidate. */
  on: boolean | null;
  gates: {
    "upstream-status": { checks: Check[] };
    loads: GateReport;
    "not-overfit": GateReport & { flagged: Pick<ClassifiedLiteral, "token" | "pointer" | "text">[]; words: Record<string, number>; uncheckedRegexes: string[] };
    compact: GateReport & { metrics: SizeMetrics | null };
    "matches-upstream-tests": GateReport & { totals: MatchesUpstreamTestsResult["totals"] | null; cases: Record<string, number>; differences: CaseResult[]; goldens: { count: number; differing: Sample<GoldenResult>; engineGaps: Sample<GoldenResult> } };
    "matches-real-code": { checks: Check[]; eligible: number; positives: number; cases: RealCodeCase[]; disagreementsElsewhere: Sample<MatchesRealCodeResult["disagreementsElsewhere"][number]> };
  };
}

/** What a rule's cached result holds: its report and everything the tool-pack report gathers from it. */
export interface CachedRule {
  report: RuleReport;
  promoted: Golden[];
  gapUses: Record<string, number>;
  integrity: string[];
  gapCandidates: GapCandidateReport[];
}

interface CachedPack {
  integrity: string[];
  knownGaps: GatesReport["knownGaps"];
  corpus: GatesReport["corpus"];
}

const isRecord = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);

/** Whether a cached value has the shape of one rule's result. */
export function isCachedRule(value: unknown): value is CachedRule {
  if (!isRecord(value) || !isRecord(value.report) || !Array.isArray(value.promoted) || !isRecord(value.gapUses) || !Array.isArray(value.integrity) || !Array.isArray(value.gapCandidates)) return false;
  const report = value.report;
  return typeof report.id === "string" && typeof report.file === "string" && (report.location === "shipped" || report.location === "candidate")
    && typeof report.pass === "boolean" && isRecord(report.gates);
}

/** Whether a cached value has the shape of a tool-pack's shared result. */
export function isCachedPack(value: unknown): value is CachedPack {
  return isRecord(value) && Array.isArray(value.integrity) && Array.isArray(value.knownGaps) && isRecord(value.corpus) && Array.isArray(value.corpus.tasks);
}

export interface GatesReport {
  format: typeof REPORT_FORMAT;
  pack: string;
  tool: string;
  toolPack: string;
  upstream: { name: string; repository: string; commit: string; version: string; license: string; recommended_configs: string[]; runtime: string; identity: Record<string, string> };
  engine: Pick<EngineIdentity, "sha256">;
  gates: { id: GateId; title: string; blocking: boolean }[];
  method: Record<"tool-pack" | GateId, Record<string, string>>;
  /** Tool-pack checks: a failing one fails every shipped rule (the recording is stale). */
  toolPackChecks: GateReport;
  integrity: string[];
  knownGaps: { id: string; proven: boolean; kind: GapCandidateReport["verdict"] | null; excused: number }[];
  gapCandidates: GapCandidateReport[];
  corpus: { tasks: CorpusTask[]; files: number; excluded: number } | null;
  /** Documents with no conformance.json entry for any tool: a shipped one fails. */
  unmapped: { file: string; location: Location }[];
  /** conformance.json entries of this tool whose document does not exist yet. */
  listedWithoutDocument: number;
  cache: { rules: number; hits: number };
  /** Goldens this run promoted, not yet in the committed goldens.tsv (`--write` adds them). */
  uncommittedPromotions: number;
  /** The shard this report judged, and how many rules the whole tool-pack has; absent on a whole or merged report. */
  shard?: Shard & { rules: number };
  summary: {
    shipped: { rules: number; pass: number };
    candidates: { rules: number; pass: number };
    stoppedAt: Record<"shipped" | "candidate", Record<BlockingGateId, number>>;
    failedGate: Record<BlockingGateId, number>;
    endToEnd: { cases: number; mismatches: number; goldens: number; goldensDiffering: number; goldensEngineGaps: number };
  };
  rules: RuleReport[];
}

export interface RunOptions {
  root: string;
  pack: string;
  tool: string;
  cacheRoot: string;
  /** Take a rule's earlier result from the cache when its inputs are unchanged. */
  readCache: boolean;
  /** Save this run's results to the cache. */
  writeCache: boolean;
  runner: CommandRunner;
  /** Judge only this shard's rules (shard.ts); every rule still counts for the tool-pack checks. */
  shard?: Shard;
}

function gate(checks: Check[]): GateReport {
  return { pass: checks.every((check) => check.ok), checks };
}

function notRun(reason: string): GateReport {
  return { pass: false, checks: [{ id: "not-run", ok: false, detail: reason }] };
}

/** The integrity line for a proven known gap that excused nothing. */
export function unusedGapLine(id: string): string {
  return `known gap ${id} excused no mismatch; remove it or correct its fixtures`;
}

/** A tool-pack report's summary, from its rules' reports. */
export function summarizeRules(reports: RuleReport[]): GatesReport["summary"] {
  const shipped = reports.filter((report) => report.location === "shipped");
  const candidates = reports.filter((report) => report.location === "candidate");
  const stopped = (list: RuleReport[]) => Object.fromEntries(BLOCKING_GATES.map((entry) => [entry.id, list.filter((report) => report.stoppedAt === entry.id).length])) as Record<BlockingGateId, number>;
  const goldenCounts = reports.map((report) => report.gates["matches-upstream-tests"].goldens);
  const realCodeCases = reports.flatMap((report) => report.gates["matches-real-code"].cases);
  return {
    shipped: { rules: shipped.length, pass: shipped.filter((entry) => entry.pass).length },
    candidates: { rules: candidates.length, pass: candidates.filter((entry) => entry.pass).length },
    stoppedAt: { shipped: stopped(shipped), candidate: stopped(candidates) },
    failedGate: Object.fromEntries(BLOCKING_GATES.map((entry) => [entry.id, reports.filter((rule) => !rule.gates[entry.id].pass).length])) as Record<BlockingGateId, number>,
    endToEnd: {
      cases: realCodeCases.length,
      mismatches: realCodeCases.filter((testCase) => testCase.status === "mismatch").length,
      goldens: goldenCounts.reduce((sum, entry) => sum + entry.count, 0),
      goldensDiffering: goldenCounts.reduce((sum, entry) => sum + entry.differing.count, 0),
      goldensEngineGaps: goldenCounts.reduce((sum, entry) => sum + entry.engineGaps.count, 0),
    },
  };
}

function countBy<T>(items: T[], keyOf: (item: T) => string): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const item of items) counts[keyOf(item)] = (counts[keyOf(item)] ?? 0) + 1;
  return Object.fromEntries(Object.entries(counts).sort(([a], [b]) => byText(a, b)));
}

interface ConformanceEntry {
  id: string;
  upstream: { tool: string; rule: string };
  policy: string;
  [key: string]: unknown;
}

/** A pack's conformance entries, root and nested candidates/conformance.json, with `policy` resolved to a repository path. */
export function conformanceEntries(root: string, pack: string): (ConformanceEntry & { path: string })[] {
  const entries: (ConformanceEntry & { path: string })[] = [];
  for (const base of [`packs/${pack}`, `packs/${pack}/candidates`]) {
    const manifest = join(root, base, "conformance.json");
    if (!existsSync(manifest)) continue;
    const rules = (JSON.parse(readFileSync(manifest, "utf8")) as { rules?: ConformanceEntry[] }).rules ?? [];
    for (const entry of rules) entries.push({ ...entry, path: `${base}/${entry.policy}` });
  }
  return entries;
}

function walk(root: string, directory: string): string[] {
  const absolute = join(root, directory);
  if (!existsSync(absolute)) return [];
  return readdirSync(absolute, { withFileTypes: true }).flatMap((entry) => {
    const path = `${directory}/${entry.name}`;
    return entry.isDirectory() ? walk(root, path) : [path];
  });
}

/**
 * The pack's rule documents in the working tree: `rules/*.json` (shipped) and every JSON under `candidates/` except
 * hand-labelled probes (`candidates/probes/**`) and conformance manifests (candidate).
 */
export function packDocuments(root: string, pack: string): { path: string; location: Location }[] {
  const shipped = walk(root, `packs/${pack}/rules`).filter((path) => path.endsWith(".json") && path.split("/").length === 4);
  const candidates = walk(root, `packs/${pack}/candidates`)
    .filter((path) => path.endsWith(".json") && !path.startsWith(`packs/${pack}/candidates/probes/`) && !path.endsWith("/conformance.json"));
  return [
    ...shipped.map((path) => ({ path, location: "shipped" as const })),
    ...candidates.map((path) => ({ path, location: "candidate" as const })),
  ].sort((a, b) => byText(a.path, b.path));
}

function readRule(root: string, path: string, rule: string): PackRule {
  const buffer = readFileSync(join(root, path));
  const text = buffer.toString("utf8");
  let id: string | null = null;
  try {
    const parsed: unknown = JSON.parse(text);
    if (parsed !== null && typeof parsed === "object" && typeof (parsed as { id?: unknown }).id === "string") id = (parsed as { id: string }).id;
  } catch {
    id = null;
  }
  return { file: path.slice(path.lastIndexOf("/") + 1), path, rule, text, bytes: buffer.length, sha256: sha256(buffer), id };
}

function packOn(root: string, pack: string): Map<string, boolean> {
  const manifest = JSON.parse(readFileSync(join(root, "packs", pack, "pack.json"), "utf8")) as { rules?: { document: string; on?: boolean }[] };
  return new Map((manifest.rules ?? []).map((rule) => [`packs/${pack}/${rule.document}`, rule.on !== false]));
}

/** Every gated tool-pack under `root`: a `packs/<pack>/upstream/<tool>/gates.yaml`. */
export function gatedToolPacks(root: string): { pack: string; tool: string }[] {
  const found: { pack: string; tool: string }[] = [];
  const packs = join(root, "packs");
  for (const pack of existsSync(packs) ? readdirSync(packs).sort() : []) {
    const upstream = join(packs, pack, "upstream");
    if (!existsSync(upstream)) continue;
    for (const tool of readdirSync(upstream).sort()) {
      if (existsSync(join(upstream, tool, GATES_MANIFEST))) found.push({ pack, tool });
    }
  }
  return found;
}

/** The method strings: what each gate judged on this run. */
function method(manifest: PackManifest): GatesReport["method"] {
  return {
    "tool-pack": {
      pins: "gates.yaml upstream.version, gates.json pins and conformance.json tools[].version agree, and every runtime pin file the recording names (package.json, pyproject.toml, .python-version) has the committed file's sha256",
      license: "the kept licenses/<project>/LICENSE has the sha256 of upstream's LICENSE at the pinned commit, as recorded (rerail's pack checks 5 and 6 cover the rest of the licence contract)",
      effect: "a failing tool-pack check fails every shipped rule of the tool-pack: the recording is stale",
    },
    "upstream-status": {
      rule: `the rule exists upstream with an implementation and tests, its upstream metadata carries no deprecation (stable), and one of the approved configurations (${manifest.upstream.recommended_configs.join(", ")}) turns it on (recommended)`,
      effect: "informational: upstream-status never stops a rule; a rule that passes every blocking gate starts on when every upstream-status check holds, and ships off, as an experimental rule, otherwise",
    },
    loads: {
      engine: "this checkout's engine: its own modules and native binding",
      documents: "read from the working tree: packs/<pack>/rules/*.json ship and block; packs/<pack>/candidates/** (except candidates/probes/**) are reported only; a document written for an earlier rule contract fails current-contract",
      loads: "RERAIL's duplicate-key check, policy schema and engine compile (as written and as the catalog's stable JSON), its bundled-rule loader, no llm clause, and in scope for every file type upstream lints",
    },
    "not-overfit": {
      document: "the document as the engine evaluates it, with its options resolved",
      corpus: "every word in all of upstream's test files (read with the language's scanner) and the option values of its test configurations, as recorded",
      explained: "the rule's own upstream implementation, the modules it imports and the helper functions it calls, the names upstream's own selective regexes match, the language's keywords, and its comment vocabulary in regexes that read comment text",
      regexes: "every regex is run over the text of upstream's test files the recording commits (the case sources); the nightly live run reads every upstream test file, so offline regex judging reads a subset of upstream's tests, while test words stay complete",
    },
    compact: {
      flag: `more than ${SIZE_FLAG_BYTES} bytes as shipped`,
      rewrite: `a flagged rule goes back when ${(REPEATED_SHARE_LIMIT * 100).toFixed(1)}% or more of its clauses is structure written out over and over, or one node kind definitely occurs ${NESTING_RUN_LIMIT} or more times along a single nesting path`,
    },
    "matches-upstream-tests": { ...MATCHES_UPSTREAM_TESTS_METHOD, recorded: "upstream's output on its own test files is read from the tool-pack's gates.json, recorded by the gates recorder at the pinned upstream" },
    "matches-real-code": { ...MATCHES_REAL_CODE_METHOD, cases: MATCHES_REAL_CODE_METHOD.cases.replaceAll("5", String(CASES_PER_KIND)), recorded: "upstream's findings on the corpus are read from gates-corpus.json, joined to each fetched file by task, path and sha256; a fetched file the recording lacks has no ground truth" },
  };
}

async function gapReports(adapter: PackAdapter, files: UnparsedFile[]): Promise<GapCandidateReport[]> {
  if (files.length === 0) return [];
  const verdicts = await adapter.parserVerdicts(files);
  return files.map(({ gate: gateId, file, engine: failure }, index) => {
    const parsers: ParserVerdict[] = verdicts[index] ?? [];
    return { gate: gateId, file, engine: failure, verdict: parsers.length === 0 ? "not-recorded" : classifyParsers(parsers), parsers };
  });
}

/** Evaluate one rule through every gate. */
async function evaluateRule(
  context: { engine: Engine; adapter: PackAdapter; pack: string; scratch: string; goldens: Goldens; upstreamFiles: UpstreamFile[]; corpusFiles: Map<string, UpstreamFile>; cases: Map<string, UpstreamCase[]>; gaps: ReturnType<typeof provenGaps> },
  rule: PackRule,
  location: Location,
  on: boolean | null,
): Promise<CachedRule> {
  const { engine, adapter, goldens, corpusFiles, cases, gaps } = context;
  const name = rule.rule;
  const id = rule.id ?? rule.file;
  const integrity: string[] = [];
  const gapUses: Record<string, number> = {};
  const unparsed = new Map<string, UnparsedFile>();
  let promoted: Golden[] = [];
  const notUpstream = adapter.rule(name) === undefined ? `${name} is not a rule of the pinned upstream` : null;
  const status = upstreamStatusChecks(name, adapter);
  const compiled = loadsGate(engine, rule, context.pack, context.scratch, adapter.inScopeSamples);
  const loads = gate(compiled.checks);
  const document = compiled.resolved ?? compiled.document;

  const notOverfit = document === undefined || notUpstream !== null
    ? { ...notRun(notUpstream ?? "the document is not JSON"), flagged: [], words: {}, uncheckedRegexes: [] }
    : (() => {
        const result = notOverfitGate(document, {
          keywords: adapter.keywords,
          commentVocabulary: adapter.commentVocabulary,
          implementation: adapter.implementationNames(name),
          sharedImplementation: adapter.sharedImplementationNames(name),
          implementationRegexes: adapter.implementationRegexes(name),
          fixtures: adapter.fixtureWords(),
          fixtureTexts: adapter.fixtureTexts(),
          selectedText: (code, selector, language) => engine.selectedText(code, selector, language),
        });
        return {
          ...gate(result.checks),
          flagged: result.literals.filter((literal) => literal.class === "fixture-only").map(({ token, pointer, text }) => ({ token, pointer, text })),
          words: countBy(result.literals, (literal) => literal.class),
          uncheckedRegexes: result.unchecked,
        };
      })();
  const compact = document === undefined
    ? { ...notRun("the document is not JSON"), metrics: null }
    : (() => {
        const result = compactGate(rule.bytes, document);
        return { ...gate(result.checks), metrics: result.metrics };
      })();

  // matches-real-code runs first: each file where the rule and upstream differ becomes a golden this run judges.
  let realCode: RuleReport["gates"]["matches-real-code"];
  if (compiled.compiled === undefined || document === undefined) {
    realCode = { checks: notRun("the document does not compile on this engine").checks, eligible: 0, positives: 0, cases: [], disagreementsElsewhere: sample([]) };
  } else if (notUpstream !== null || adapter.rule(name)!.notOnRealCode !== null) {
    realCode = { checks: notRun(notUpstream ?? adapter.rule(name)!.notOnRealCode!).checks, eligible: 0, positives: 0, cases: [], disagreementsElsewhere: sample([]) };
  } else {
    const result = matchesRealCodeGate(engine, adapter, compiled.compiled, name, document, context.upstreamFiles);
    for (const file of result.unparsed) unparsed.set(`${file.gate}:${file.file}`, file);
    promoted = promote(goldens, id, differingFiles(result).map(({ task, path }) => ({ task, path, sha256: corpusFiles.get(`${task}\0${path}`)!.file.sha256 })));
    realCode = { checks: result.checks, eligible: result.eligible, positives: result.positives, cases: result.cases, disagreementsElsewhere: sample(result.disagreementsElsewhere) };
  }

  let upstreamTests: RuleReport["gates"]["matches-upstream-tests"];
  const emptyGoldens = { count: goldens.get(id)?.length ?? 0, differing: sample<GoldenResult>([]), engineGaps: sample<GoldenResult>([]) };
  if (compiled.compiled === undefined) {
    upstreamTests = { ...notRun("the document does not compile on this engine"), totals: null, cases: {}, differences: [], goldens: emptyGoldens };
  } else if (notUpstream !== null) {
    upstreamTests = { ...notRun(notUpstream), totals: null, cases: {}, differences: [], goldens: emptyGoldens };
  } else {
    const result = matchesUpstreamTestsGate(engine, adapter, name, compiled.compiled, cases.get(name) ?? [], gaps.proven);
    for (const [gapId, uses] of Object.entries(result.gapUses)) gapUses[gapId] = (gapUses[gapId] ?? 0) + uses;
    for (const file of result.unparsed) unparsed.set(`${file.gate}:${file.file}`, file);
    integrity.push(...result.markupDisagreements);
    const ruleGoldens = (goldens.get(id) ?? []).map((golden) => {
      const entry = corpusFiles.get(`${golden.task}\0${golden.path}`);
      const file: CorpusFile | undefined = entry?.file.sha256 === golden.sha256 ? entry.file : undefined;
      if (file === undefined) integrity.push(`golden ${id} ${golden.task}/${golden.path}: the pinned corpus has no such file with sha256 ${golden.sha256}`);
      return { golden, file, expected: entry === undefined || entry.excluded !== null ? null : entry.findings.get(name) ?? [] };
    });
    const onGoldens = goldensGate(engine, adapter, name, compiled.compiled, ruleGoldens);
    for (const file of onGoldens.unparsed) if (!unparsed.has(`matches-real-code:${file.file}`)) unparsed.set(`${file.gate}:${file.file}`, file);
    upstreamTests = {
      ...gate([...result.checks, onGoldens.check, onGoldens.engineCheck, staysSilentCheck(result.cases, onGoldens.results, realCode.cases)]),
      totals: result.totals,
      cases: countBy(result.cases, (testCase) => testCase.status),
      differences: result.cases.filter((testCase) => testCase.status === "mismatch" || testCase.status === "excused"),
      goldens: {
        count: onGoldens.results.length,
        differing: sample(onGoldens.results.filter((golden) => golden.status === "mismatch")),
        engineGaps: sample(onGoldens.results.filter((golden) => golden.status === "engine-gap")),
      },
    };
  }

  const start = defaultOn(status);
  const gates = { "upstream-status": { checks: status }, loads, "not-overfit": notOverfit, compact, "matches-upstream-tests": upstreamTests, "matches-real-code": realCode };
  const unparsedFiles = [...unparsed.values()].sort((a, b) => byText(a.gate, b.gate) || byText(a.file, b.file));
  return {
    report: {
      id,
      file: rule.path,
      location,
      upstreamRule: name,
      sha256: rule.sha256,
      bytes: rule.bytes,
      policyDigest: compiled.compiled?.digest ?? null,
      ...verdict(gates),
      blockedBy: null,
      defaultOn: start.on,
      defaultOnReason: start.reason,
      on,
      gates,
    },
    promoted,
    gapUses,
    integrity,
    gapCandidates: await gapReports(adapter, unparsedFiles),
  };
}

/**
 * Run every gate on every document of one gated tool-pack, from its recordings. Returns the report and the tool-pack's
 * goldens with this run's promotions. With the cache, a rule whose inputs are unchanged takes its cached result; when
 * every rule does, nothing is fetched or evaluated.
 */
export async function runToolPack(options: RunOptions): Promise<{ report: GatesReport; goldens: string; goldensChanged: boolean }> {
  const { root, pack, tool } = options;
  const cacheRoot = resolve(options.cacheRoot);
  const directory = toolPackDirectory(root, pack, tool);
  const entries = conformanceEntries(root, pack);
  const byPath = new Map(entries.map((entry) => [entry.path, entry]));
  const documents = packDocuments(root, pack);
  const unmapped = documents.filter((document) => !byPath.has(document.path)).map(({ path, location }) => ({ file: path, location }));
  const mine = documents
    .filter((document) => byPath.get(document.path)?.upstream.tool === tool)
    .map((document) => ({ ...document, entry: byPath.get(document.path)! }));
  const present = new Set(documents.map((document) => document.path));
  const listedWithoutDocument = entries.filter((entry) => entry.upstream.tool === tool && !present.has(entry.path)).length;
  const names = [...new Set(mine.map((document) => document.entry.upstream.rule))].sort(byText);

  const inputs = readRecordedInputs(root, pack, tool, names);
  const adapter = createRecordedAdapter(inputs, { cacheRoot, runner: options.runner });
  const manifest = inputs.manifest;
  const on = packOn(root, pack);
  const rules = mine.map((document) => ({ ...document, rule: readRule(root, document.path, document.entry.upstream.rule) }));

  const corpora = await readRerailCorpora(root, pack);
  const corpusTasksText = readFileSync(CORPUS_TASKS, "utf8");
  const pinChecks: Check[] = [...adapter.packChecks];
  const recordedPins = adapter.recordedCorpusPins;
  const corpusPinsOk = recordedPins !== null && recordedPins.tasks === sha256(corpusTasksText) && canonicalJson(recordedPins.corpora) === canonicalJson(corpora);
  pinChecks.push({ id: "corpus-pins", ok: corpusPinsOk, detail: corpusPinsOk ? "gates-corpus.json was recorded on this branch's pinned tasks and corpora" : `gates-corpus.json was recorded on other corpus pins: ${staleMessage(pack, tool)}` });

  // bundled-load: rerail loads every document in the pack's rules/ together, whatever its upstream tool, so a conflict
  // between them (a duplicate id) fails here, where each rule's own loads check reads one document. It runs on every
  // run, cached or not.
  const engine = loadEngine(root);
  const shippedDocuments = documents.filter((document) => document.location === "shipped").map((document) => readRule(root, document.path, byPath.get(document.path)?.upstream.rule ?? ""));
  await mkdir(join(cacheRoot, "tmp"), { recursive: true });
  try {
    const loaded = loadAsBundled(engine, join(cacheRoot, "tmp"), pack, shippedDocuments);
    const ok = loaded.ids.length === shippedDocuments.length;
    pinChecks.push({ id: "bundled-load", ok, detail: ok ? `rerail's bundled-rule loader loads all ${shippedDocuments.length} shipped document(s) of the pack together` : `the loader returned ${loaded.ids.length} rule(s) for ${shippedDocuments.length} shipped document(s)` });
  } catch (error) {
    pinChecks.push({ id: "bundled-load", ok: false, detail: `the pack's shipped rules do not load together through rerail's bundled-rule loader: ${error instanceof Error ? error.message : String(error)}` });
  }

  const goldens = await readGoldens(directory);
  const committedGoldens = serializeGoldens(goldens);
  const readOr = (file: string) => (existsSync(join(directory, file)) ? readFileSync(join(directory, file)) : "");
  const shared: SharedKeyInputs = {
    gateCode: trackedDigest(root, GATE_CODE),
    dependencies: trackedDigest(root, GATE_DEPENDENCIES),
    engine: engineIdentity(root).sha256,
    gatesYaml: sha256(readOr(GATES_MANIFEST)),
    gatesJson: sha256(readOr(GATES_RECORDING)),
    gatesCorpus: sha256(readOr(GATES_CORPUS)),
    corpusTasks: sha256(corpusTasksText),
    corpora: JSON.stringify(corpora),
  };
  const keyed = rules.map((entry) => {
    const id = entry.rule.id ?? entry.rule.file;
    const caseFiles = [...new Set((inputs.recording.rules[entry.rule.rule]?.cases ?? []).map((testCase) => testCase.file).filter((file): file is string => file !== null))].sort(byText);
    const key = ruleKey(shared, {
      document: entry.rule.text,
      conformanceEntry: JSON.stringify({ entry: entry.entry, on: entry.location === "shipped" ? (on.get(entry.path) ?? true) : null }),
      goldens: JSON.stringify(goldens.get(id) ?? []),
      caseSources: caseFiles.map((file) => `${file}\0${existsSync(join(directory, file)) ? sha256(readFileSync(join(directory, file))) : "missing"}`),
    });
    return { ...entry, id, key };
  });
  // A shard judges only its own rules; the tool-pack checks and the bundled-rule load above still read every rule.
  const shard = options.shard;
  const judged = shard === undefined ? keyed : keyed.filter((entry) => shardOf(entry.id, shard.count) === shard.index);
  const packKey = sha256(judged.map((entry) => entry.key).sort(byText).join("\n"));

  // Plan: every key first; when all hit, nothing is fetched or evaluated.
  const cached = new Map<string, CachedRule>();
  let packCached: CachedPack | undefined;
  if (options.readCache) {
    for (const entry of judged) {
      const hit = await readCached(cacheRoot, pack, tool, entry.key, isCachedRule, shard);
      // An entry for another document under this key is a miss.
      if (hit !== undefined && hit.report.file === entry.path && hit.report.location === entry.location) cached.set(entry.key, hit);
    }
    packCached = await readCached(cacheRoot, pack, tool, `pack-${packKey}`, isCachedPack, shard);
  }
  const misses = judged.filter((entry) => !cached.has(entry.key));
  const results = new Map<string, CachedRule>(cached);
  let packResult: CachedPack;

  if (misses.length === 0 && packCached !== undefined) {
    packResult = packCached;
    // Promotions are part of a cached result: replay them so --write gives the same goldens.tsv.
    for (const entry of judged) {
      const result = cached.get(entry.key)!;
      promote(goldens, result.report.id, result.promoted);
    }
    pinChecks.push({ id: "corpus-complete", ok: true, detail: "not fetched on this run: every result is cached from a run that fetched every recorded corpus file" });
  } else {
    // not-overfit reads every upstream test text: fetched at the pinned commit only when a rule is evaluated.
    await adapter.loadFixtureTexts();
    const catalog = await buildCorpus(cacheRoot, (path) => adapter.inCorpus(path), options.runner);
    const rerailCorpus = await buildRerailCorpora(corpora, cacheRoot, (path) => adapter.inCorpus(path), options.runner);
    const corpusFiles = [...catalog.files, ...rerailCorpus.files];
    const complete = corpusCompleteCheck(inputs.corpus?.files ?? [], corpusFiles, [...catalog.tasks, ...rerailCorpus.tasks]);
    pinChecks.push(complete);
    // Results judged on part of the corpus are reported (and fail through corpus-complete) but never cached.
    const save = options.writeCache && complete.ok;
    const upstreamFiles = await adapter.lintCorpus(corpusFiles, names);
    const byPlace = new Map<string, UpstreamFile>(upstreamFiles.map((entry) => [`${entry.file.task}\0${entry.file.path}`, entry]));
    const cases = new Map<string, UpstreamCase[]>(names.filter((name) => adapter.rule(name) !== undefined).map((name) => [name, adapter.cases(name)]));
    const gaps = provenGaps(engine, manifest.known_gaps, new Map([...cases.values()].flat().map((testCase) => [testCase.fixture, readPath(testCase)])));
    await mkdir(join(cacheRoot, "tmp"), { recursive: true });
    const scratch = mkdtempSync(join(cacheRoot, "tmp", "gates-"));
    const integrity = [...gaps.problems];
    try {
      // Replay cached promotions first, so evaluated rules see the same goldens a full run would.
      for (const entry of judged) {
        const hit = cached.get(entry.key);
        if (hit !== undefined) promote(goldens, hit.report.id, hit.promoted);
      }
      const context = { engine, adapter, pack, scratch, goldens, upstreamFiles, corpusFiles: byPlace, cases, gaps };
      for (const entry of misses) {
        const result = await evaluateRule(context, entry.rule, entry.location, entry.location === "shipped" ? (on.get(entry.path) ?? true) : null);
        results.set(entry.key, result);
        if (save) await writeCached(cacheRoot, pack, tool, entry.key, result, shard);
      }
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
    const gapUses: Record<string, number> = {};
    for (const result of results.values()) for (const [gapId, uses] of Object.entries(result.gapUses)) gapUses[gapId] = (gapUses[gapId] ?? 0) + uses;
    for (const gap of manifest.known_gaps) {
      if (gaps.proven.has(gap.id) && (gapUses[gap.id] ?? 0) === 0) integrity.push(unusedGapLine(gap.id));
    }
    const probes = [...gaps.proven.values()];
    const probeVerdicts = await adapter.parserVerdicts(probes.map(({ gap, probePath }) => ({ path: probePath, source: gap.probe })));
    const kinds = new Map(probes.map(({ gap }, index) => [gap.id, (probeVerdicts[index]?.length ?? 0) === 0 ? "not-recorded" as const : classifyParsers(probeVerdicts[index]!)]));
    packResult = {
      integrity,
      knownGaps: manifest.known_gaps.map((gap) => ({ id: gap.id, proven: gaps.proven.has(gap.id), kind: kinds.get(gap.id) ?? null, excused: gapUses[gap.id] ?? 0 })),
      corpus: { tasks: [...catalog.tasks, ...rerailCorpus.tasks], files: upstreamFiles.length, excluded: upstreamFiles.filter((entry) => entry.excluded !== null).length },
    };
    if (save) await writeCached(cacheRoot, pack, tool, `pack-${packKey}`, packResult, shard);
  }

  const toolPack = gate(pinChecks);
  const blockedBy = toolPack.pass ? null : toolPack.checks.filter((check) => !check.ok).map((check) => `${check.id}: ${check.detail}`).join("; ");

  const reports = judged.map((entry) => {
    const report = structuredClone(results.get(entry.key)!.report);
    // A failing tool-pack check (a stale recording, an incomplete corpus, shipped rules that do not load together) fails
    // every rule, whatever its gates say: a shipped rule blocks, and a candidate is not ready to promote.
    if (blockedBy !== null) {
      report.pass = false;
      report.blockedBy = blockedBy;
    }
    return report;
  }).sort((a, b) => byText(a.file, b.file));
  const all = judged.map((entry) => results.get(entry.key)!);
  const gapCandidates = new Map<string, GapCandidateReport>();
  for (const result of all) for (const candidate of result.gapCandidates) gapCandidates.set(`${candidate.gate}\0${candidate.file}`, candidate);
  const serialized = serializeGoldens(goldens);

  const report: GatesReport = {
    format: REPORT_FORMAT,
    pack,
    tool,
    toolPack: `${pack}/${tool}`,
    upstream: { ...manifest.upstream, identity: adapter.identity },
    engine: { sha256: shared.engine },
    gates: GATES.map((entry) => ({ ...entry })),
    method: method(manifest),
    toolPackChecks: toolPack,
    integrity: [...new Set([...packResult.integrity, ...all.flatMap((result) => result.integrity)])].sort(byText),
    knownGaps: packResult.knownGaps,
    gapCandidates: [...gapCandidates.values()].sort((a, b) => byText(a.gate, b.gate) || byText(a.file, b.file)),
    corpus: packResult.corpus,
    unmapped,
    listedWithoutDocument,
    cache: { rules: judged.length, hits: cached.size },
    uncommittedPromotions: all.reduce((sum, result) => sum + result.promoted.length, 0),
    ...(shard === undefined ? {} : { shard: { ...shard, rules: keyed.length } }),
    summary: summarizeRules(reports),
    rules: reports,
  };
  return { report, goldens: serialized, goldensChanged: serialized !== committedGoldens };
}

/** Whether a tool-pack report fails CI: a shipped rule fails, or a shipped document is not mapped to an upstream rule. */
export function blockingFailures(report: GatesReport): number {
  return report.rules.filter((rule) => rule.location === "shipped" && !rule.pass).length
    + report.unmapped.filter((entry) => entry.location === "shipped").length;
}

export const relativeTo = (root: string, path: string) => relative(root, path);
