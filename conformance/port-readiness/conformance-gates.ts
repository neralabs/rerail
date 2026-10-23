/**
 * The conformance gates (`loads`, `compact`, `passes-conformance`; types.ts CONFORMANCE_GATES) over every rule document
 * no gated tool-pack judges: the packs with no upstream recordings (complexity, and node, whose tool has no gates.yaml
 * yet), a gated pack's documents of another tool, and the documents no conformance.json entry maps. A gated tool-pack's
 * rules meet `loads` and `compact` in its own run (run.ts), so together the two cover every rule of every pack; the
 * summary gives each rule one verdict, its conformance gates before any port step.
 *
 * `passes-conformance` is the pack's conformance check, which scores every rule of the pack at once, so it runs here,
 * once per pack with a conformance.json, for every rule that file maps: the tool-pack runs in CI leave it to this job
 * (`--conformance-in-own-job`), and the summary takes each of their rules' results from `passesConformance`. It does
 * not apply to a rule conformance.json does not map (complexity has no conformance.json at all).
 *
 * Before the gates, a rule must declare its origin (conformance/origins.ts): a port, by its conformance.json entry or in
 * origins.json, or an original, in origins.json. One declared nowhere stops at `origin`; a gated tool-pack's rules are
 * declared by the entries that map them. Each pack's `origins` check fails on an origins.json that is wrong.
 *
 * Nothing here reads a recording or upstream's code. `loads` has no upstream to name the files a rule must apply to, so
 * its `in-scope` check is not made. Nothing is cached: the pack checks take about a minute together.
 */
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { join } from "node:path";

import { packOrigins, type Origin } from "../origins.js";
import { compactGate, type SizeMetrics } from "./checks/compact.js";
import { loadAsBundled, loadsGate } from "./checks/loads.js";
import { packConformance, passesConformanceChecks, type PackConformance } from "./checks/passes-conformance.js";
import { DEFAULT_CACHE_ROOT } from "./corpus.js";
import { loadEngine, type Engine, type EngineIdentity } from "./engine.js";
import { conformanceEntries, gatedToolPacks, packDocuments, readRule, type GateReport, type Location } from "./run.js";
import { CONFORMANCE_GATES, type Check, type ConformanceGateId } from "./types.js";

export const CONFORMANCE_REPORT_FORMAT = "rerail.conformance-gates/v2" as const;
/** The report's file name beside the tool-pack reports; readReports never takes it for one (`<pack>--<tool>.json`). */
export const CONFORMANCE_REPORT_FILE = "conformance-gates.json";

const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

/** Every pack under `root` (a directory with a pack.json), sorted, with the upstream tools its conformance.json names. */
function packsOf(root: string): { pack: string; tools: string[] }[] {
  const packs = join(root, "packs");
  return readdirSync(packs, { withFileTypes: true })
    .filter((entry) => entry.isDirectory() && existsSync(join(packs, entry.name, "pack.json")))
    .map((entry) => entry.name)
    .sort(byText)
    .map((pack) => {
      const manifest = join(packs, pack, "conformance.json");
      const tools = existsSync(manifest) ? ((JSON.parse(readFileSync(manifest, "utf8")) as { tools?: { name: string }[] }).tools ?? []).map((tool) => tool.name) : [];
      return { pack, tools };
    });
}

export interface ConformanceRuleReport {
  id: string;
  /** Repository-relative path of the document. */
  file: string;
  pack: string;
  location: Location;
  /** Where the rule comes from, as its pack declares it (conformance/origins.ts), or null when nothing declares it. */
  origin: Origin | null;
  /** The rule declares its origin and passes every conformance gate that applies to it. */
  pass: boolean;
  /** `origin` when it declares none, else the first conformance gate it fails. */
  stoppedAt: "origin" | ConformanceGateId | null;
  gates: {
    loads: GateReport;
    compact: GateReport & { metrics: SizeMetrics | null };
    /** `applies` is false when conformance.json does not map the rule: the check has nothing to score it against. */
    "passes-conformance": GateReport & { applies: boolean };
  };
}

export interface ConformanceReport {
  format: typeof CONFORMANCE_REPORT_FORMAT;
  engine: Pick<EngineIdentity, "sha256">;
  /**
   * Per pack: its checks (`bundled-load`: every shipped document loads together; `origins`: its origins.json parses,
   * names only its rule documents and the tools its conformance.json lists, and declares no document twice;
   * `conformance-runs` when the pack's conformance check ran here), and how many of its documents a gated tool-pack
   * judges instead.
   */
  packs: { pack: string; checks: GateReport; judgedByToolPacks: number }[];
  rules: ConformanceRuleReport[];
  /** `passes-conformance` of each rule a gated tool-pack judges, by its document, from the pack's one conformance check. */
  passesConformance: { file: string; location: Location; gate: GateReport }[];
}

const gate = (checks: Check[]): GateReport => ({ pass: checks.every((check) => check.ok), checks });
const notRun = (reason: string): GateReport => ({ pass: false, checks: [{ id: "not-run", ok: false, detail: reason }] });
const notApplicable = (reason: string): GateReport & { applies: false } => ({ pass: true, applies: false, checks: [{ id: "not-applicable", ok: true, detail: reason }] });

/**
 * One rule on the conformance gates. `mapped` is the upstream rule a conformance.json entry maps it to: only then does
 * the pack's conformance check score it.
 */
function judgeRule(root: string, engine: Engine, scratch: string, pack: string, path: string, location: Location, origin: Origin | null, mapped: { tool: string; rule: string } | null, conformance: PackConformance | undefined, hasManifest: boolean): ConformanceRuleReport {
  const rule = readRule(root, path, mapped?.rule ?? "");
  const id = rule.id ?? rule.file;
  const compiled = loadsGate(engine, rule, pack, scratch, null);
  const document = compiled.resolved ?? compiled.document;
  const compact = document === undefined ? { ...notRun("the document is not JSON"), metrics: null } : (() => {
    const result = compactGate(rule.bytes, document);
    return { ...gate(result.checks), metrics: result.metrics };
  })();
  const passesConformance = conformance !== undefined && mapped !== null
    ? { ...gate(passesConformanceChecks(conformance, id)), applies: true }
    : notApplicable(hasManifest ? "no conformance.json entry maps it to an upstream rule, so the pack's conformance check has nothing to score it against" : `packs/${pack} has no conformance.json, so the pack's conformance check does not run on it`);
  const gates = { loads: gate(compiled.checks), compact, "passes-conformance": passesConformance };
  const stoppedAt = origin === null ? "origin" : (CONFORMANCE_GATES.find((entry) => !gates[entry.id].pass)?.id ?? null);
  return { id, file: path, pack, location, origin, pass: stoppedAt === null, stoppedAt, gates };
}

/**
 * Judge every rule document of every pack under `root` that no gated tool-pack judges, on the conformance gates, with
 * the engine of `root` unless `options.engine` is given; scratch files go under `options.cacheRoot`.
 */
export async function runConformanceGates(root: string, options: { cacheRoot?: string; engine?: Engine; packConformance?: typeof packConformance } = {}): Promise<ConformanceReport> {
  const engine = options.engine ?? loadEngine(root);
  const cacheRoot = options.cacheRoot ?? DEFAULT_CACHE_ROOT;
  const gated = new Set(gatedToolPacks(root).map(({ pack, tool }) => `${pack}/${tool}`));
  mkdirSync(join(cacheRoot, "tmp"), { recursive: true });
  const scratch = mkdtempSync(join(cacheRoot, "tmp", "conformance-"));
  const packs: ConformanceReport["packs"] = [];
  const rules: ConformanceRuleReport[] = [];
  const passesConformance: ConformanceReport["passesConformance"] = [];
  const conformanceOf = options.packConformance ?? packConformance;
  try {
    for (const { pack, tools } of packsOf(root)) {
      const entries = new Map(conformanceEntries(root, pack).map((entry) => [entry.path, entry]));
      const documents = packDocuments(root, pack);
      const mine = documents.filter((document) => {
        const entry = entries.get(document.path);
        return entry === undefined || !gated.has(`${pack}/${entry.upstream.tool}`);
      });
      const prefix = `packs/${pack}/`;
      const origins = packOrigins(join(root, "packs", pack), documents.map((document) => document.path.slice(prefix.length)));
      const checks: Check[] = [];
      const shipped = documents.filter((document) => document.location === "shipped").map((document) => readRule(root, document.path, ""));
      try {
        const loaded = loadAsBundled(engine, scratch, pack, shipped);
        const ok = loaded.ids.length === shipped.length;
        checks.push({ id: "bundled-load", ok, detail: ok ? `rerail's bundled-rule loader loads all ${shipped.length} shipped document(s) of the pack together` : `the loader returned ${loaded.ids.length} rule(s) for ${shipped.length} shipped document(s)` });
      } catch (error) {
        checks.push({ id: "bundled-load", ok: false, detail: `the pack's shipped rules do not load together through rerail's bundled-rule loader: ${error instanceof Error ? error.message : String(error)}` });
      }
      checks.push({ id: "origins", ok: origins.failures.length === 0, detail: origins.failures.length === 0 ? "origins.json declares only the pack's rule documents, each once" : origins.failures.join("; ") });
      // The pack's conformance check scores every rule conformance.json maps, this job's and the tool-pack runs' alike.
      const hasManifest = tools.length > 0 || entries.size > 0;
      let conformance: PackConformance | undefined;
      if (documents.some((document) => entries.has(document.path))) {
        try {
          conformance = await conformanceOf(root, pack);
          checks.push({ id: "conformance-runs", ok: conformance.errors.length === 0, detail: conformance.errors.length === 0 ? "the pack's conformance check runs without errors" : conformance.errors.slice(0, 3).join("; ") });
        } catch (error) {
          checks.push({ id: "conformance-runs", ok: false, detail: `the pack's conformance check failed to run: ${error instanceof Error ? error.message : String(error)}` });
        }
      }
      for (const document of documents.filter((entry) => !mine.includes(entry))) {
        const entry = entries.get(document.path)!;
        passesConformance.push({ file: document.path, location: document.location, gate: conformance === undefined ? notRun("the pack's conformance check did not run (see the pack's checks)") : gate(passesConformanceChecks(conformance, entry.id)) });
      }
      packs.push({ pack, checks: gate(checks), judgedByToolPacks: documents.length - mine.length });
      for (const document of mine) {
        const entry = entries.get(document.path);
        const origin = origins.origins.get(document.path.slice(prefix.length)) ?? null;
        rules.push(judgeRule(root, engine, scratch, pack, document.path, document.location, origin, entry === undefined ? null : { tool: entry.upstream.tool, rule: entry.upstream.rule }, conformance, hasManifest));
      }
    }
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
  return { format: CONFORMANCE_REPORT_FORMAT, engine: { sha256: engine.identity.sha256 }, packs, rules: rules.sort((a, b) => byText(a.file, b.file)), passesConformance: passesConformance.sort((a, b) => byText(a.file, b.file)) };
}

/** Whether the report fails CI: a shipped rule declares no origin or fails a conformance gate, or a pack check fails. */
export function conformanceBlockingFailures(report: ConformanceReport): number {
  return report.rules.filter((rule) => rule.location === "shipped" && !rule.pass).length
    + report.passesConformance.filter((entry) => entry.location === "shipped" && !entry.gate.pass).length
    + report.packs.filter((entry) => !entry.checks.pass).length;
}
