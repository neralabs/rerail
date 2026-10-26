import { createHash } from "node:crypto";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { extname, join, relative, resolve } from "node:path";

import { dependencyDigest, ruleKey, readCached, toolPackKey, trackedDigest, writeCached, GATE_CODE, type SharedKeyInputs } from "./cache.js";
import { upstreamStatusChecks, defaultOn } from "./checks/upstream-status.js";
import { loadAsBundled, loadsGate } from "./checks/loads.js";
import { packConformance, passesConformanceChecks } from "./checks/passes-conformance.js";
import { notOverfitGate, type ClassifiedLiteral } from "./checks/not-overfit.js";
import { compactGate, NESTING_RUN_LIMIT, REPEATED_SHARE_LIMIT, SIZE_FLAG_BYTES, type SizeMetrics } from "./checks/compact.js";
import { classifyParsers, MATCHES_UPSTREAM_TESTS_METHOD, matchesUpstreamTestsGate, outsideInScope, provenGaps, readPath, type CaseResult, type GapCandidate, type MatchesUpstreamTestsResult, type UnparsedFile } from "./checks/matches-upstream-tests.js";
import { engineKey, loadEngine, policyFamilies, type Engine, type EngineIdentity } from "./engine.js";
import { GATES_MANIFEST, toolPackDirectory } from "./manifest.js";
import type { CommandRunner } from "./process.js";
import { createRecordedAdapter, decodeSource, readRecordedInputs } from "./recorded-adapter.js";
import { adapterSourcesDigest, parsersId, RECORDING_ARCHIVE } from "./recording/archive.js";
import { verdictPath } from "./recording/verdicts.js";
import { ruleTreeDigest } from "./recording/tree.js";
import { shardOf, type Shard } from "./shard.js";
import { BLOCKING_GATES, GATES, verdict, type BlockingGateId, type Check, type GateId, type PackAdapter, type PackManifest, type PackRule, type ParserVerdict, type UpstreamCase } from "./types.js";

export const REPORT_FORMAT = "rerail.port-readiness/v1" as const;

const sha256 = (bytes: string | Buffer): string => createHash("sha256").update(bytes).digest("hex");

export interface GateReport {
  pass: boolean;
  checks: Check[];
}

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

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
  /** The failing tool-pack checks that fail the rule whatever its gates say (a stale recording, shipped rules that do not load together), or null. */
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
    /**
     * `outOfScope`: the cases not scored because their file type is outside the tool-pack's rerail.in_scope, when there
     * are any. `silentCases`: the rule's own upstream cases where upstream reports nothing and the rule agrees, which
     * passes-conformance's stays-silent reads (with the clean samples) when the tool-pack's report is assembled.
     */
    "matches-upstream-tests": GateReport & { totals: MatchesUpstreamTestsResult["totals"] | null; compared?: MatchesUpstreamTestsResult["compared"]; cases: Record<string, number>; silentCases: number; differences: CaseResult[]; outOfScope?: CaseResult[] };
    /** Judged when the tool-pack's report is assembled, from the pack's conformance check; never cached. */
    "passes-conformance": GateReport;
  };
}

/** What a rule's cached result holds: its report and everything the tool-pack report gathers from it. */
export interface CachedRule {
  report: RuleReport;
  gapUses: Record<string, number>;
  integrity: string[];
  gapCandidates: GapCandidateReport[];
}

interface CachedPack {
  integrity: string[];
  knownGaps: GatesReport["knownGaps"];
}

const isRecord = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === "object" && !Array.isArray(value);

/** Whether a cached value has the shape of one rule's result. */
export function isCachedRule(value: unknown): value is CachedRule {
  if (!isRecord(value) || !isRecord(value.report) || !isRecord(value.gapUses) || !Array.isArray(value.integrity) || !Array.isArray(value.gapCandidates)) return false;
  const report = value.report;
  return typeof report.id === "string" && typeof report.file === "string" && (report.location === "shipped" || report.location === "candidate")
    && typeof report.pass === "boolean" && isRecord(report.gates);
}

/** Whether a cached value has the shape of a tool-pack's shared result. */
export function isCachedPack(value: unknown): value is CachedPack {
  return isRecord(value) && Array.isArray(value.integrity) && Array.isArray(value.knownGaps);
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
  /** Documents with no conformance.json entry for any tool: a shipped one fails. */
  unmapped: { file: string; location: Location }[];
  /** conformance.json entries of this tool whose document does not exist yet. */
  listedWithoutDocument: number;
  cache: { rules: number; hits: number };
  /** The shard this report judged, and how many rules the whole tool-pack has; absent on a whole or merged report. */
  shard?: Shard & { rules: number };
  summary: {
    shipped: { rules: number; pass: number };
    candidates: { rules: number; pass: number };
    stoppedAt: Record<"shipped" | "candidate", Record<BlockingGateId, number>>;
    failedGate: Record<BlockingGateId, number>;
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
  return {
    shipped: { rules: shipped.length, pass: shipped.filter((entry) => entry.pass).length },
    candidates: { rules: candidates.length, pass: candidates.filter((entry) => entry.pass).length },
    stoppedAt: { shipped: stopped(shipped), candidate: stopped(candidates) },
    failedGate: Object.fromEntries(BLOCKING_GATES.map((entry) => [entry.id, reports.filter((rule) => !rule.gates[entry.id].pass).length])) as Record<BlockingGateId, number>,
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
      pins: "gates.yaml upstream.version, the recording's pins and conformance.json tools[].version agree, and every runtime pin file the recording names (package.json, pyproject.toml, .python-version, composer.json, .php-version, PHP's extensions.txt, rust-toolchain.toml) has the committed file's sha256",
      license: "the licence kept under licenses/<project>/ (every LICENSE or LICENCE file, in name order) has the sha256 of upstream's licence at the pinned commit, as recorded (rerail's pack checks 5 and 6 cover the rest of the licence contract)",
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
    "matches-upstream-tests": { ...MATCHES_UPSTREAM_TESTS_METHOD, recorded: "upstream's output on its own test files is read from the tool-pack's recording archive (each answer filed under this branch's upstream configuration and the rule, by its case's source and inputs), recorded by the gates recorder at the pinned upstream" },
    "passes-conformance": {
      check: "the pack contract's own check (test:conformance, conformance/languages/): the pack's fixtures and its real-code samples (every tool-pack's samples.lock), over every rule, shipped or candidate, each in its own workspace as the check runs them",
      fails: "an unexplained FP or FN on the fixtures (upstream's cases and Rerail's probes), a missed label or a finding on a clean sample of the real-code samples (judged by line, after the pack's divergences.json), no true positive without pack.json examples, an engine failure, a stale divergence, or no silent evidence (stays-silent: one of the rule's own upstream cases upstream reports nothing on, or a clean sample it is silent on): what the check fails a shipped rule on",
    },
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
  context: { engine: Engine; adapter: PackAdapter; pack: string; scratch: string; cases: Map<string, UpstreamCase[]>; gaps: Awaited<ReturnType<typeof provenGaps>> },
  rule: PackRule,
  location: Location,
  on: boolean | null,
): Promise<CachedRule> {
  const { engine, adapter, cases, gaps } = context;
  const name = rule.rule;
  const id = rule.id ?? rule.file;
  const integrity: string[] = [];
  const gapUses: Record<string, number> = {};
  const unparsed = new Map<string, UnparsedFile>();
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

  let upstreamTests: RuleReport["gates"]["matches-upstream-tests"];
  if (compiled.compiled === undefined) {
    upstreamTests = { ...notRun("the document does not compile on this engine"), totals: null, cases: {}, silentCases: 0, differences: [] };
  } else if (notUpstream !== null) {
    upstreamTests = { ...notRun(notUpstream), totals: null, cases: {}, silentCases: 0, differences: [] };
  } else {
    const result = matchesUpstreamTestsGate(engine, adapter, name, compiled.compiled, cases.get(name) ?? [], gaps.proven);
    for (const [gapId, uses] of Object.entries(result.gapUses)) gapUses[gapId] = (gapUses[gapId] ?? 0) + uses;
    for (const file of result.unparsed) unparsed.set(`${file.gate}:${file.file}`, file);
    integrity.push(...result.markupDisagreements);
    upstreamTests = {
      ...gate(result.checks),
      totals: result.totals,
      compared: result.compared,
      cases: countBy(result.cases, (testCase) => testCase.status),
      silentCases: result.cases.filter((testCase) => testCase.expected === 0 && testCase.status === "agree").length,
      differences: result.cases.filter((testCase) => testCase.status === "mismatch" || testCase.status === "excused"),
      ...(result.cases.some(outsideInScope) ? { outOfScope: result.cases.filter(outsideInScope) } : {}),
    };
  }

  const start = defaultOn(status);
  // passes-conformance reads the whole pack, so runToolPack judges it on every run; this placeholder never decides.
  const gates = { "upstream-status": { checks: status }, loads, "not-overfit": notOverfit, compact, "matches-upstream-tests": upstreamTests, "passes-conformance": gate([]) };
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
    gapUses,
    integrity,
    gapCandidates: await gapReports(adapter, unparsedFiles),
  };
}

/**
 * Run every gate on every document of one gated tool-pack, from its recordings. With the cache, a rule whose inputs are
 * unchanged takes its cached result; when every rule does, nothing is fetched or evaluated.
 */
export async function runToolPack(options: RunOptions): Promise<GatesReport> {
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

  const inputs = readRecordedInputs(root, cacheRoot, pack, tool, names);
  const adapter = createRecordedAdapter(inputs, { cacheRoot, runner: options.runner });
  const manifest = inputs.manifest;
  const on = packOn(root, pack);
  const rules = mine.map((document) => ({ ...document, rule: readRule(root, document.path, document.entry.upstream.rule) }));

  const pinChecks: Check[] = [...adapter.packChecks];

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

  const readOr = (file: string) => (existsSync(join(directory, file)) ? readFileSync(join(directory, file)) : "");
  const committedBytes = new Map<string, Buffer | undefined>();
  const committed = (file: string): Buffer | undefined => {
    if (!committedBytes.has(file)) committedBytes.set(file, existsSync(join(directory, file)) ? readFileSync(join(directory, file)) : undefined);
    return committedBytes.get(file);
  };
  const shared: SharedKeyInputs = {
    gateCode: trackedDigest(root, GATE_CODE),
    dependencies: dependencyDigest(root),
    gatesYaml: sha256(readOr(GATES_MANIFEST)),
    // The recording is not in the tree: the lock's digest is what it is, and the lock is what the commit pins.
    recordingArchive: inputs.lock?.files[RECORDING_ARCHIVE].sha256 ?? "",
    adapterSources: adapterSourcesDigest(manifest.adapter),
  };
  // What every rule reads of the recording beyond its own: the cases of any rule a known gap names, with their
  // committed sources, and the parsers' verdicts on each gap's probe, which provenGaps proves the gaps on.
  const verdictsId = parsersId(manifest.adapter, inputs.recording.parsers);
  const recordedTree = inputs.tree;
  const verdictDigest = (path: string) => sha256(`${path}\0${recordedTree.text(path) ?? "missing"}`);
  const gapCases = Object.entries(inputs.recording.rules).flatMap(([name, rule]) => rule.cases
    .filter((testCase) => manifest.known_gaps.some((gap) => gap.fixtures.includes(testCase.fixture)))
    .map((testCase) => ({ name, ...testCase, committed: testCase.file === null ? null : sha256(committed(testCase.file) ?? "missing") })));
  const gapInputs = JSON.stringify({
    cases: gapCases,
    probes: manifest.known_gaps.flatMap((gap) => {
      const named = gapCases.filter((testCase) => gap.fixtures.includes(testCase.fixture));
      const types = [...new Set(named.map((testCase) => extname(readPath(testCase))))];
      const script = named.every((testCase) => testCase.script === true);
      return types.map((type) => verdictDigest(verdictPath(verdictsId, { path: `probe${type}`, sha256: sha256(gap.probe), script })));
    }),
  });
  const keyed = rules.map((entry) => {
    const id = entry.rule.id ?? entry.rule.file;
    const recordedCases = inputs.recording.rules[entry.rule.rule]?.cases ?? [];
    const caseFiles = [...new Set(recordedCases.map((testCase) => testCase.file).filter((file): file is string => file !== null))].sort(byText);
    const families = policyFamilies(entry.rule.text);
    const caseSources = caseFiles.map((file) => `${file}\0${sha256(committed(file) ?? "missing")}`);
    // The verdicts the rule's unparsed cases would be looked up under, read as the recorded adapter reads them.
    const verdictPaths = recordedCases.flatMap((testCase) => {
      const bytes = testCase.file === null ? undefined : committed(testCase.file);
      return bytes === undefined ? [] : [verdictPath(verdictsId, { path: readPath(testCase), sha256: sha256(decodeSource(bytes)), script: testCase.script === true })];
    });
    const key = ruleKey(shared, {
      // The shared engine and the families this rule's document names; the whole engine when it names none the
      // language registry knows, so a rule the gates cannot place is judged again on any engine change.
      engine: engineKey(engine.identity, families),
      document: entry.rule.text,
      conformanceEntry: JSON.stringify({ entry: entry.entry, on: entry.location === "shipped" ? (on.get(entry.path) ?? true) : null }),
      caseSources,
      recording: `${ruleTreeDigest(recordedTree, inputs.config, inputs.recording, entry.rule.rule, verdictPaths)}\0${sha256(gapInputs)}`,
    });
    return { ...entry, id, key, families, caseSources };
  });
  // A shard judges only its own rules; the tool-pack checks and the bundled-rule load above still read every rule, so
  // their key covers what they read of every rule, whichever shard it is in.
  const shard = options.shard;
  const judged = shard === undefined ? keyed : keyed.filter((entry) => shardOf(entry.id, shard.count) === shard.index);
  const packKey = toolPackKey(shared, {
    engine: engineKey(engine.identity, keyed.some((entry) => entry.families.length === 0) ? [] : keyed.flatMap((entry) => entry.families)),
    names,
    caseSources: [...new Set(keyed.flatMap((entry) => entry.caseSources))],
  }, judged.map((entry) => entry.key));

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
  } else {
    // not-overfit reads every upstream test text: fetched at the pinned commit only when a rule is evaluated.
    await adapter.loadFixtureTexts();
    const cases = new Map<string, UpstreamCase[]>(names.filter((name) => adapter.rule(name) !== undefined).map((name) => [name, adapter.cases(name)]));
    const gaps = await provenGaps(engine, manifest.known_gaps, [...cases.values()].flat(), (files) => adapter.parserVerdicts(files));
    await mkdir(join(cacheRoot, "tmp"), { recursive: true });
    const scratch = mkdtempSync(join(cacheRoot, "tmp", "gates-"));
    const integrity = [...gaps.problems];
    try {
      const context = { engine, adapter, pack, scratch, cases, gaps };
      for (const entry of misses) {
        const result = await evaluateRule(context, entry.rule, entry.location, entry.location === "shipped" ? (on.get(entry.path) ?? true) : null);
        results.set(entry.key, result);
        if (options.writeCache) await writeCached(cacheRoot, pack, tool, entry.key, result, shard);
      }
    } finally {
      rmSync(scratch, { recursive: true, force: true });
    }
    const gapUses: Record<string, number> = {};
    for (const result of results.values()) for (const [gapId, uses] of Object.entries(result.gapUses)) gapUses[gapId] = (gapUses[gapId] ?? 0) + uses;
    for (const gap of manifest.known_gaps) {
      if (gaps.proven.has(gap.id) && (gapUses[gap.id] ?? 0) === 0) integrity.push(unusedGapLine(gap.id));
    }
    packResult = {
      integrity,
      knownGaps: manifest.known_gaps.map((gap) => ({ id: gap.id, proven: gaps.proven.has(gap.id), kind: gaps.kinds.get(gap.id) ?? null, excused: gapUses[gap.id] ?? 0 })),
    };
    if (options.writeCache) await writeCached(cacheRoot, pack, tool, `pack-${packKey}`, packResult, shard);
  }

  // passes-conformance: the pack's own conformance check reads every rule of the pack, so it runs on every run.
  const conformance = await packConformance(root, pack);
  pinChecks.push({ id: "conformance-runs", ok: conformance.errors.length === 0, detail: conformance.errors.length === 0 ? "the pack's conformance check runs without errors" : conformance.errors.slice(0, 3).join("; ") });
  const toolPack = gate(pinChecks);
  const blockedBy = toolPack.pass ? null : toolPack.checks.filter((check) => !check.ok).map((check) => `${check.id}: ${check.detail}`).join("; ");

  const reports = judged.map((entry) => {
    const report = structuredClone(results.get(entry.key)!.report);
    // stays-silent's own-case evidence comes from this rule's matches-upstream-tests result (`silentCases`).
    report.gates["passes-conformance"] = gate(passesConformanceChecks(conformance, report.id, report.gates["matches-upstream-tests"].silentCases));
    Object.assign(report, verdict(report.gates));
    // A failing tool-pack check (a stale recording, shipped rules that do not load together) fails
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

  const report: GatesReport = {
    format: REPORT_FORMAT,
    pack,
    tool,
    toolPack: `${pack}/${tool}`,
    upstream: { ...manifest.upstream, identity: adapter.identity },
    // The whole engine, every family's files included: what the run judged on, not what any one rule keyed on.
    engine: { sha256: engine.identity.sha256 },
    gates: GATES.map((entry) => ({ ...entry })),
    method: method(manifest),
    toolPackChecks: toolPack,
    integrity: [...new Set([...packResult.integrity, ...all.flatMap((result) => result.integrity)])].sort(byText),
    knownGaps: packResult.knownGaps,
    gapCandidates: [...gapCandidates.values()].sort((a, b) => byText(a.gate, b.gate) || byText(a.file, b.file)),
    unmapped,
    listedWithoutDocument,
    cache: { rules: judged.length, hits: cached.size },
    ...(shard === undefined ? {} : { shard: { ...shard, rules: keyed.length } }),
    summary: summarizeRules(reports),
    rules: reports,
  };
  return report;
}

/** Whether a tool-pack report fails CI: a shipped rule fails, or a shipped document is not mapped to an upstream rule. */
export function blockingFailures(report: GatesReport): number {
  return report.rules.filter((rule) => rule.location === "shipped" && !rule.pass).length
    + report.unmapped.filter((entry) => entry.location === "shipped").length;
}

export const relativeTo = (root: string, path: string) => relative(root, path);
