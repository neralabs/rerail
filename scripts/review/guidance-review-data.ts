/**
 * The data behind the guidance review (`just rerail-guidance-review`): every rule of every rule pack under packs/,
 * shipped and candidate, with its details, the cases it was run on, and the steer each case gives the coding agent.
 *
 * Cases come from the rule's `examples` in pack.json and from its upstream fixtures, run through the native engine
 * the way the conformance runner runs them (conformance/languages/runner.ts). Each caught case's guidance is
 * `renderSteer`'s output for that file's violations: the block the coding agent reads after an edit, verbatim.
 *
 *   node --import tsx scripts/review/guidance-review-data.ts [--pack <id>]... [--out <file>]
 */
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, relative } from "node:path";
import { parseArgs } from "node:util";
import { z } from "zod";
import {
  compareDeterministicStrings,
  encodeCodeRuleWorkspaceRequest,
  languageFor,
  loadNativeBinding,
  renderSteer,
  type CodeRuleSourceLanguage,
  type CodeRuleViolation,
  type NativeCompiledCodeRuleDsl,
} from "../../src/engine/index.js";
import { capabilitiesJson } from "../../conformance/engine.js";
import {
  coversFixture,
  documentPath,
  fixturesDirectory,
  fixturesExpectedPath,
  isCandidate,
  loadPack,
  loadExpectedFindings,
  PACKS_ROOT,
  RERAIL_ROOT,
  type FixtureExpectedFindings,
  type LoadedPack,
  type PackRule,
} from "../../conformance/languages/pack.js";

export const DEFAULT_OUT = join(RERAIL_ROOT, ".rerail", "guidance-review", "data.json");

/** Cases kept per rule and kind: enough to judge the guidance without reading every fixture. */
const CAUGHT_CASES = 3;
const OTHER_CASES = 1;
/** Lines of context kept around a case's first finding; a shorter file is kept whole. */
const CONTEXT_LINES = 6;
const WHOLE_FILE_LINES = 40;
/** Fixture cases remembered per rule, kind and origin before the best are picked. */
const CANDIDATES_KEPT = 4;

interface SourceFile {
  path: string;
  language: CodeRuleSourceLanguage;
  source: string;
}

const spanSchema = z.object({
  startByte: z.number(),
  endByte: z.number(),
  startLine: z.number(),
  startColumn: z.number(),
  endLine: z.number(),
  endColumn: z.number(),
});
const pageSchema = z.object({
  cursor: z.number(),
  done: z.boolean(),
  violations: z.array(
    z.object({
      id: z.string(),
      ruleId: z.string(),
      clauseId: z.string(),
      code: z.string(),
      mode: z.enum(["log", "steer", "block"]),
      message: z.string(),
      path: z.string().optional(),
      span: spanSchema.optional(),
      captures: z.record(z.string(), z.array(spanSchema)).default({}),
      variables: z.record(z.string(), z.union([z.boolean(), z.number(), z.string()])).default({}),
      fix: z.object({ span: spanSchema, text: z.string() }).optional(),
    }),
  ),
  failures: z.array(z.object({ ruleId: z.string().optional(), code: z.string(), message: z.string(), path: z.string().optional() })),
});

/** One run of the engine over `files` with `rules`, as the repository audit runs it. */
async function evaluate(rules: readonly NativeCompiledCodeRuleDsl[], files: readonly SourceFile[]): Promise<z.infer<typeof pageSchema>> {
  const workspace = new (loadNativeBinding().CodeRuleDslWorkspace)();
  try {
    for (const rule of rules) workspace.addCodeRule(rule);
    await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: 1, files: [...files], removed: [], deferGlobal: true, quiet: true }));
    await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: 2, files: [], removed: [], quiet: true }));
    const result: z.infer<typeof pageSchema> = { cursor: 0, done: true, violations: [], failures: [] };
    for (let cursor = 0; ; ) {
      const page = pageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, 8 * 1024 * 1024)));
      result.violations.push(...page.violations);
      result.failures.push(...page.failures);
      if (page.done || page.cursor <= cursor) break;
      cursor = page.cursor;
    }
    return result;
  } finally {
    workspace.close();
  }
}

export interface ReviewCase {
  id: string;
  /** `example`: pack.json's caught example; `caught`: Rerail and upstream both flag it; `extra`: only Rerail does; `missed`: only upstream does; `passes`: neither does. */
  kind: "example" | "caught" | "extra" | "missed" | "passes";
  path: string;
  language: string;
  /** First line number of `snippet`, 1-based. */
  firstLine: number;
  snippet: string;
  /** 1-based lines Rerail flagged, and lines the upstream tool flagged. */
  flagged: number[];
  upstream: number[];
  /** The steer the coding agent reads for this file; empty when Rerail flags nothing. */
  guidance: string;
  failures: string[];
}

export interface ReviewRule {
  id: string;
  pack: string;
  title: string;
  description: string;
  candidate: boolean;
  on: boolean;
  mode: string;
  group: string | null;
  document: string;
  upstream: { tool: string; rule: string } | null;
  scope: { include: string[]; exclude: string[] };
  messages: string[];
  counts: { caught: number; extra: number; missed: number; units: number };
  compileError: string | null;
  cases: ReviewCase[];
  /** Changes when the rule's wording or rendered guidance changes, so a review can be marked out of date. */
  digest: string;
}

export interface ReviewPack {
  id: string;
  name: string;
  covers: string;
  reason: string;
  rules: ReviewRule[];
}

export interface ReviewData {
  generatedAt: string;
  commit: string;
  packs: ReviewPack[];
}

interface PackJsonRule {
  id: string;
  document: string;
  title: string;
  group?: string;
  on?: boolean;
  mode?: string;
  examples?: { caught: { path?: string; text: string }; passes: { path?: string; text: string } };
}

interface RuleDocument {
  id: string;
  mode?: string;
  description?: string;
  scope?: { include?: string[]; exclude?: string[] };
}

function readJson<T>(path: string): T {
  return JSON.parse(readFileSync(path, "utf8")) as T;
}

/** Every violation message in a rule document, in order, including each case of a message chosen by an option. */
function messagesOf(document: unknown): string[] {
  const found: string[] = [];
  const texts = (value: unknown): void => {
    if (typeof value === "string") {
      if (!found.includes(value)) found.push(value);
    } else if (value !== null && typeof value === "object") {
      for (const [key, inner] of Object.entries(value)) if (key !== "$option") texts(inner);
    }
  };
  const walk = (value: unknown): void => {
    if (Array.isArray(value)) value.forEach(walk);
    else if (value !== null && typeof value === "object") {
      for (const [key, inner] of Object.entries(value)) {
        if (key === "message") texts(inner);
        else walk(inner);
      }
    }
  };
  walk(document);
  return found;
}

/** The lines around a case's findings, or the whole file when it is short. */
function snippetOf(source: string, lines: readonly number[]): { firstLine: number; snippet: string } {
  const all = source.replace(/\n$/u, "").split("\n");
  if (all.length <= WHOLE_FILE_LINES || lines.length === 0) {
    return { firstLine: 1, snippet: all.slice(0, Math.max(WHOLE_FILE_LINES, 1)).join("\n") };
  }
  const first = Math.max(1, Math.min(...lines) - CONTEXT_LINES);
  const last = Math.min(all.length, Math.max(...lines) + CONTEXT_LINES, first + WHOLE_FILE_LINES - 1);
  return { firstLine: first, snippet: all.slice(first - 1, last).join("\n") };
}

function violationLine(violation: CodeRuleViolation): number | undefined {
  return violation.span === undefined ? undefined : violation.span.startLine + 1;
}

/** A candidate has no pack.json title: its upstream tool and rule stand in, as shipped titles begin. */
function candidateTitle(rule: PackRule | undefined): string {
  return rule === undefined ? "" : `${rule.upstream.tool} ${rule.upstream.rule.replace(/^@[^/]+\//u, "")}`;
}

function digestOf(rule: Omit<ReviewRule, "digest">): string {
  const hash = createHash("sha256");
  hash.update(JSON.stringify([rule.title, rule.description, rule.mode, rule.messages, rule.cases.map(item => item.guidance)]));
  return hash.digest("hex").slice(0, 16);
}

interface Compiled {
  rule: PackRule | undefined;
  packRule: PackJsonRule | undefined;
  documentPath: string;
  document: RuleDocument;
  text: string;
  compiled: NativeCompiledCodeRuleDsl | undefined;
  compileError: string | null;
}

interface PendingCase {
  kind: "caught" | "extra" | "missed" | "passes";
  own: boolean;
  file: SourceFile;
  mine: CodeRuleViolation[];
  theirs: number[];
  failures: string[];
}

/** Whether a fixture was written for `upstreamRule`: its path starts with the rule's name, as upstream test suites lay them out. */
function ownFixture(upstreamRule: string, path: string): boolean {
  const name = upstreamRule.split("/").pop() ?? upstreamRule;
  return path.split("/").some(part => part === name || part.replace(/\.[^.]+$/u, "") === name);
}

async function packData(directory: string): Promise<ReviewPack | undefined> {
  const packJsonPath = join(directory, "pack.json");
  if (!existsSync(packJsonPath)) return undefined;
  const packJson = readJson<{ id: string; name: string; covers?: string; reason?: string; rules: PackJsonRule[] }>(packJsonPath);
  const loaded: LoadedPack | undefined = existsSync(join(directory, "conformance.json")) ? loadPack(directory) : undefined;
  const runtime = loadNativeBinding().CodeRuleDslRuntime;
  const byDocument = new Map(packJson.rules.map(rule => [documentPath(rule.document), rule]));

  const entries: Compiled[] = [];
  const seen = new Set<string>();
  const add = (rule: PackRule | undefined, path: string): void => {
    const document = documentPath(path);
    if (seen.has(document)) return;
    seen.add(document);
    const text = readFileSync(join(directory, document), "utf8");
    let compiled: NativeCompiledCodeRuleDsl | undefined;
    let compileError: string | null = null;
    try {
      compiled = runtime.compile(text, "json", capabilitiesJson());
    } catch (error) {
      compileError = error instanceof Error ? error.message : String(error);
    }
    entries.push({ rule, packRule: byDocument.get(document), documentPath: document, document: JSON.parse(text) as RuleDocument, text, compiled, compileError });
  };
  for (const rule of loaded?.pack.rules ?? []) add(rule, rule.policy);
  for (const rule of packJson.rules) add(undefined, rule.document);
  // Candidates not yet recorded in conformance.json (no upstream, no fixture cases) are reviewed too.
  const candidates = join(directory, "candidates", "rules");
  if (existsSync(candidates)) {
    for (const name of readdirSync(candidates).filter(name => name.endsWith(".json")).sort(compareDeterministicStrings)) add(undefined, `candidates/rules/${name}`);
  }

  const cases = new Map<string, ReviewCase[]>(entries.map(entry => [entry.document.id, []]));
  const counts = new Map(entries.map(entry => [entry.document.id, { caught: 0, extra: 0, missed: 0, units: 0 }]));
  const kept = (id: string, kind: ReviewCase["kind"]): number => (cases.get(id) ?? []).filter(item => item.kind === kind).length;

  const caseOf = (id: string, kind: ReviewCase["kind"], file: SourceFile, violations: CodeRuleViolation[], upstream: number[], failures: string[]): ReviewCase => {
    const flagged = [...new Set(violations.map(violationLine).filter((line): line is number => line !== undefined))].sort((a, b) => a - b);
    const { firstLine, snippet } = snippetOf(file.source, [...flagged, ...upstream]);
    return {
      id: `${id}:${kind}:${file.path}`,
      kind,
      path: file.path,
      language: file.language,
      firstLine,
      snippet,
      flagged,
      upstream,
      guidance: violations.length === 0 ? "" : renderSteer(violations, { kind: "edit" }),
      failures,
    };
  };

  const pending = new Map<string, PendingCase[]>();

  // pack.json examples: the caught case and the case that passes, each run alone.
  for (const entry of entries) {
    const examples = entry.packRule?.examples;
    if (examples === undefined || entry.compiled === undefined) continue;
    for (const [kind, example] of [["example", examples.caught], ["passes", examples.passes]] as const) {
      const path = example.path ?? "example";
      const language = languageFor(path);
      if (language === undefined) continue;
      const file = { path, language, source: example.text };
      const result = await evaluate([entry.compiled], [file]);
      const violations = result.violations.filter(item => item.ruleId === entry.document.id) as CodeRuleViolation[];
      const failures = result.failures.map(item => `${item.code}: ${item.message}`);
      cases.get(entry.document.id)?.push(caseOf(entry.document.id, kind, file, violations, [], failures));
    }
  }

  // Upstream fixtures: every unit a rule covers, all of the tool's rules at once.
  if (loaded !== undefined) {
    const tools = [...new Set(entries.flatMap(entry => (entry.rule === undefined ? [] : [entry.rule.upstream.tool])))].sort(compareDeterministicStrings);
    for (const tool of tools) {
      const expectedPath = fixturesExpectedPath(loaded, tool);
      if (!existsSync(expectedPath)) continue;
      const toolExpected = loadExpectedFindings(expectedPath) as FixtureExpectedFindings;
      if (toolExpected.source.kind !== "fixtures") continue;
      const toolEntries = entries.filter(entry => entry.rule?.upstream.tool === tool && entry.compiled !== undefined);
      const expected = new Map<string, Map<string, number[]>>();
      for (const finding of toolExpected.findings) {
        const perPath = expected.get(finding.rule) ?? new Map<string, number[]>();
        perPath.set(finding.path, [...(perPath.get(finding.path) ?? []), finding.line]);
        expected.set(finding.rule, perPath);
      }
      for (const unit of toolExpected.source.files) {
        const paths = typeof unit === "string" ? [unit] : unit;
        const unitEntries = toolEntries.filter(entry => entry.rule !== undefined && paths.some(path => coversFixture(entry.rule as PackRule, path)));
        for (const entry of unitEntries) {
          const total = counts.get(entry.document.id);
          if (total !== undefined) total.units += 1;
        }
        if (unitEntries.length === 0) continue;
        const files: SourceFile[] = [];
        for (const path of paths) {
          const language = languageFor(path);
          if (language === undefined) continue;
          files.push({ path, language, source: readFileSync(join(fixturesDirectory(loaded, tool), path), "utf8") });
        }
        if (files.length === 0) continue;
        const shipped = unitEntries.filter(entry => entry.rule !== undefined && !isCandidate(entry.rule));
        const candidates = unitEntries.filter(entry => entry.rule !== undefined && isCandidate(entry.rule));
        const violations: CodeRuleViolation[] = [];
        const failures: z.infer<typeof pageSchema>["failures"] = [];
        for (const group of [shipped, candidates]) {
          if (group.length === 0) continue;
          const result = await evaluate(group.map(entry => entry.compiled as NativeCompiledCodeRuleDsl), files);
          violations.push(...(result.violations as CodeRuleViolation[]));
          failures.push(...result.failures);
        }
        for (const entry of unitEntries) {
          const id = entry.document.id;
          const upstreamRule = entry.rule?.upstream.rule ?? "";
          const total = counts.get(id);
          for (const file of files) {
            const mine = violations.filter(item => item.ruleId === id && item.path === file.path);
            const theirs = [...new Set(expected.get(upstreamRule)?.get(file.path) ?? [])].sort((a, b) => a - b);
            const myLines = new Set(mine.map(violationLine));
            const both = theirs.filter(line => myLines.has(line)).length;
            const onlyMine = [...myLines].filter(line => line !== undefined && !theirs.includes(line)).length;
            const onlyTheirs = theirs.length - both;
            if (total !== undefined) {
              total.caught += both;
              total.extra += onlyMine;
              total.missed += onlyTheirs;
            }
            const fileFailures = failures.filter(item => (item.ruleId === undefined || item.ruleId === id) && (item.path === undefined || item.path === file.path)).map(item => `${item.code}: ${item.message}`);
            const kind: ReviewCase["kind"] | undefined =
              both > 0 ? "caught" : onlyMine > 0 ? "extra" : onlyTheirs > 0 ? "missed" : mine.length === 0 ? "passes" : undefined;
            if (kind === undefined) continue;
            const own = ownFixture(upstreamRule, file.path);
            const list = pending.get(id) ?? [];
            if (list.filter(item => item.kind === kind && item.own === own).length < CANDIDATES_KEPT) list.push({ kind, own, file, mine, theirs, failures: fileFailures });
            pending.set(id, list);
          }
        }
      }
    }
  }

  // Each rule's cases: its own fixtures (written for this upstream rule) before ones written for another rule.
  for (const [id, list] of pending) {
    for (const kind of ["caught", "extra", "missed", "passes"] as const) {
      const limit = kind === "caught" ? CAUGHT_CASES : OTHER_CASES;
      const picked = list.filter(item => item.kind === kind).sort((a, b) => Number(b.own) - Number(a.own)).slice(0, Math.max(0, limit - kept(id, kind)));
      for (const item of picked) cases.get(id)?.push(caseOf(id, kind, item.file, item.mine, item.theirs, item.failures));
    }
  }

  const order: Record<ReviewCase["kind"], number> = { example: 0, caught: 1, extra: 2, missed: 3, passes: 4 };
  const rules: ReviewRule[] = entries.map(entry => {
    const id = entry.document.id;
    const packRule = entry.packRule;
    const candidate = entry.rule === undefined ? entry.documentPath.startsWith("candidates/") : isCandidate(entry.rule);
    const rule: Omit<ReviewRule, "digest"> = {
      id,
      pack: packJson.id,
      title: packRule?.title ?? (candidateTitle(entry.rule) || id),
      description: entry.document.description ?? "",
      candidate,
      on: packRule?.on ?? !candidate,
      mode: packRule?.mode ?? entry.document.mode ?? "steer",
      group: packRule?.group ?? null,
      document: relative(RERAIL_ROOT, join(directory, entry.documentPath)),
      upstream: entry.rule === undefined ? null : entry.rule.upstream,
      scope: { include: entry.document.scope?.include ?? [], exclude: entry.document.scope?.exclude ?? [] },
      messages: messagesOf(JSON.parse(entry.text)),
      counts: counts.get(id) ?? { caught: 0, extra: 0, missed: 0, units: 0 },
      compileError: entry.compileError,
      cases: (cases.get(id) ?? []).sort((a, b) => order[a.kind] - order[b.kind]),
    };
    return { ...rule, digest: digestOf(rule) };
  });
  rules.sort((a, b) => Number(a.candidate) - Number(b.candidate) || compareDeterministicStrings(a.id, b.id));
  return { id: packJson.id, name: packJson.name, covers: packJson.covers ?? "", reason: packJson.reason ?? "", rules };
}

export async function reviewData(packIds: readonly string[] = []): Promise<ReviewData> {
  const ids = readdirSync(PACKS_ROOT, { withFileTypes: true })
    .filter(entry => entry.isDirectory() && (packIds.length === 0 || packIds.includes(entry.name)))
    .map(entry => entry.name)
    .sort(compareDeterministicStrings);
  const packs: ReviewPack[] = [];
  for (const id of ids) {
    const started = performance.now();
    const pack = await packData(join(PACKS_ROOT, id));
    if (pack === undefined) continue;
    packs.push(pack);
    process.stderr.write(`${id}: ${pack.rules.length} rules, ${pack.rules.reduce((total, rule) => total + rule.cases.length, 0)} cases (${Math.round(performance.now() - started)} ms)\n`);
  }
  const commit = execFileSync("git", ["rev-parse", "--short", "HEAD"], { cwd: RERAIL_ROOT, encoding: "utf8" }).trim();
  return { generatedAt: new Date().toISOString(), commit, packs };
}

async function main(): Promise<void> {
  const { values } = parseArgs({ options: { pack: { type: "string", multiple: true, default: [] }, out: { type: "string", default: DEFAULT_OUT } } });
  const data = await reviewData(values.pack);
  mkdirSync(dirname(values.out), { recursive: true });
  writeFileSync(values.out, `${JSON.stringify(data)}\n`);
  process.stderr.write(`wrote ${relative(process.cwd(), values.out)}\n`);
}

if (process.argv[1] !== undefined && import.meta.url === `file://${process.argv[1]}`) await main();
