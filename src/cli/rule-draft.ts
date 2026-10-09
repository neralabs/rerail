import { readFileSync, type Dirent } from "node:fs";
import { opendir, readFile, stat } from "node:fs/promises";
import { join, relative, resolve, sep } from "node:path";
import { bundledModelPath } from "../engine/package-root.js";
import { z } from "zod";
import { libraryUpsertBodySchema } from "../api/repository-contracts.js";
import { parseArguments } from "../daemon/daemon.js";
import { defaultRerailDatabasePath, openRerailDatabase } from "../engine/storage/database.js";
import { loadBundledCatalog } from "../engine/catalog/bundled-rules.js";
import { IGNORED_DIRECTORIES, languageFor } from "../engine/code-rule/code-rule-enforcer.js";
import {
  encodeCodeRuleWorkspaceRequest,
  loadNativeBinding,
  type NativeBinding,
  type NativeCompiledCodeRuleDsl,
} from "../engine/native/native.js";
import { bundledCodeRuleDslDirectory, CodeRuleCompileError, EngineUnavailableError } from "../engine/code-rule/code-rule-compile.js";
import { CODE_RULE_DIALECTS } from "../engine/code-rule/code-rule-languages.js";
import type { CodeRule } from "../engine/code-rule/code-rule-schema.js";
import { ConversationRuleRegistry } from "../engine/catalog/registry.js";
import { repositoryIdentityLocation, repositoryName } from "../engine/catalog/repository-identity.js";
import { normalizeFolder } from "../engine/catalog/repository-model.js";
import { recordCheckout } from "../engine/catalog/repository-registry.js";
import { checkoutStatus } from "../engine/catalog/repository-registry.js";
import { addAttachment, setUpRepository } from "../engine/catalog/rule-configuration.js";
import { upsertLibraryRule } from "../engine/catalog/rule-library.js";
import { parseConversationRule, sampleSourceSchema, SAMPLE_BOUNDARIES, type ConversationRule } from "../engine/catalog/rule-schema.js";
import { readRulesDashboard } from "../presentation/session-presentation.js";
import { stableJsonText } from "../engine/stable-json.js";

/** How many repository matches the report shows; the count covers every match. */
const REPOSITORY_EXAMPLES = 5;
/** The same bounds a repository scan walks with. */
const REPOSITORY_MAX_FILES = 100_000;
const REPOSITORY_MAX_FILE_BYTES = 32_000_000;
const REPOSITORY_MAX_SOURCE_BYTES = 512 * 1024 * 1024;
const EXAMPLE_TEXT_LENGTH = 200;
const CASE_LABEL_LENGTH = 60;

const codeRuleCaseSchema = z.strictObject({ path: z.string().min(1), content: z.string() });
const conversationCaseSchema = z.strictObject({
  source: sampleSourceSchema,
  boundary: z.enum(SAMPLE_BOUNDARIES),
  text: z.string(),
  input: z.string().optional(),
});
const draftCasesSchema = z.strictObject({
  caught: z.array(z.unknown()).default([]),
  passed: z.array(z.unknown()).default([]),
});

type CodeRuleCase = z.infer<typeof codeRuleCaseSchema>;
type ConversationCase = z.infer<typeof conversationCaseSchema>;
type Expectation = "caught" | "passed";

/** A draft rule, the folder of its repository it is added at (`""`, the whole repository, unless the draft names one), and its cases. */
export type RuleDraft =
  | { kind: "file"; rule: CodeRule; folder: string; cases: ReadonlyArray<{ expected: Expectation; sample: CodeRuleCase }> }
  | { kind: "conversation"; rule: ConversationRule; folder: string; cases: ReadonlyArray<{ expected: Expectation; sample: ConversationCase }> };

/** A draft is added to the repository of the checkout it is added from; it never names another. */
const TARGET_FIELDS = ["repository", "checkout", "attach"] as const;

export interface RuleDraftCaseResult {
  expected: Expectation;
  caught: boolean;
  ok: boolean;
  label: string;
}

export interface RuleDraftRepositoryMatch {
  path: string;
  line: number;
  column: number;
  text: string;
}

export interface RuleDraftReport {
  ok: boolean;
  valid: boolean;
  error?: string;
  cases: RuleDraftCaseResult[];
  repository: { matches: number; examples: RuleDraftRepositoryMatch[] } | null;
}

/** A draft the schema, the Engine, or a case rejects; `rules test` reports it rather than throwing. */
export class RuleDraftError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RuleDraftError";
  }
}

function issuesText(error: z.ZodError, prefix: readonly PropertyKey[] = []): string {
  return error.issues
    .map(issue => {
      const path = [...prefix, ...issue.path].map(String).join(".");
      return path.length === 0 ? issue.message : `${path}: ${issue.message}`;
    })
    .join("; ");
}

function parseCases<T>(schema: z.ZodType<T>, values: readonly unknown[], list: Expectation): Array<{ expected: Expectation; sample: T }> {
  return values.map((value, index) => {
    const parsed = schema.safeParse(value);
    if (!parsed.success) throw new RuleDraftError(issuesText(parsed.error, ["cases", list, index]));
    return { expected: list, sample: parsed.data };
  });
}

/** Validate a draft exactly as `PUT /api/library-rules/{id}` validates its body, plus the optional `cases`. */
export function parseRuleDraft(value: unknown): RuleDraft {
  const envelope = z.looseObject({ cases: z.unknown().optional() }).safeParse(value);
  if (!envelope.success) throw new RuleDraftError("the draft must be a JSON object");
  const { cases: rawCases, ...body } = envelope.data;
  const target = TARGET_FIELDS.find(field => field in body);
  if (target !== undefined) throw new RuleDraftError(`${target}: a draft is added to this checkout's repository and cannot name ${target}`);
  const upsert = libraryUpsertBodySchema.safeParse(body);
  if (!upsert.success) throw new RuleDraftError(issuesText(upsert.error));
  let folder: string;
  try {
    folder = normalizeFolder(upsert.data.folder ?? "");
  } catch (error) {
    throw new RuleDraftError(`folder: ${errorMessage(error)}`);
  }
  const cases = draftCasesSchema.safeParse(rawCases ?? {});
  if (!cases.success) throw new RuleDraftError(issuesText(cases.error, ["cases"]));
  if (upsert.data.kind === "file") {
    const schema = codeRuleCaseSchema;
    return {
      kind: "file",
      rule: upsert.data.rule as CodeRule,
      folder,
      cases: [...parseCases(schema, cases.data.caught, "caught"), ...parseCases(schema, cases.data.passed, "passed")],
    };
  }
  let rule: ConversationRule;
  try {
    rule = parseConversationRule(upsert.data.rule);
  } catch (error) {
    throw new RuleDraftError(errorMessage(error));
  }
  return {
    kind: "conversation",
    rule,
    folder,
    cases: [...parseCases(conversationCaseSchema, cases.data.caught, "caught"), ...parseCases(conversationCaseSchema, cases.data.passed, "passed")],
  };
}

export function readRuleDraft(file: string): RuleDraft {
  const text = readFileSync(resolve(file), "utf8");
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new RuleDraftError(`${file} is not JSON: ${errorMessage(error)}`);
  }
  return parseRuleDraft(value);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function engineUnavailable(ruleId: string, reason: string): RuleDraftError {
  return new RuleDraftError(`the Engine is unavailable, so rule ${ruleId} cannot be checked: ${reason}`);
}

/** Compile with the same source text and bundled capabilities the catalog's compile check uses, keeping the result to evaluate. */
function compileCodeRule(rule: CodeRule): { binding: NativeBinding; compiled: NativeCompiledCodeRuleDsl } {
  const sourceText = stableJsonText(rule);
  if (sourceText === undefined) throw new RuleDraftError(new CodeRuleCompileError(rule.id, "the document is not JSON representable").message);
  let binding: NativeBinding;
  let capabilities: string;
  try {
    binding = loadNativeBinding();
    capabilities = readFileSync(resolve(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
  } catch (error) {
    throw engineUnavailable(rule.id, errorMessage(error));
  }
  try {
    return { binding, compiled: binding.CodeRuleDslRuntime.compile(sourceText, "json", capabilities) };
  } catch (error) {
    throw new RuleDraftError(new CodeRuleCompileError(rule.id, errorMessage(error)).message);
  }
}

type SourceFile = { path: string; language: string; source: string; module?: string | string[] };
/** The part of the Engine's evaluation the report reads: where each violation starts (zero-based). */
const evaluationSchema = z.object({
  evaluation: z.object({
    violations: z.array(z.object({
      path: z.string().optional(),
      span: z.object({ startLine: z.number().int(), startColumn: z.number().int() }).optional(),
    })),
  }),
});
type Violation = z.infer<typeof evaluationSchema>["evaluation"]["violations"][number];

/** Violations of `compiled` over `files`, in a private workspace with no content store attached. */
async function evaluate(binding: NativeBinding, compiled: NativeCompiledCodeRuleDsl, files: readonly SourceFile[]): Promise<Violation[]> {
  const workspace = new binding.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(compiled);
    const accepted = files.filter(file => workspace.acceptsPath(file.path));
    if (accepted.length === 0) return [];
    const result = await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: 1, files: accepted, removed: [] }));
    return evaluationSchema.parse(JSON.parse(result)).evaluation.violations;
  } finally {
    workspace.close();
  }
}

function moduleFor(binding: NativeBinding, root: string, path: string, language: string): { module?: string | string[] } {
  if (language !== CODE_RULE_DIALECTS.python.label) return {};
  const module = binding.pythonModulePath(resolve(root, path));
  return module === null ? {} : { module };
}

async function policyCaseResult(
  binding: NativeBinding,
  compiled: NativeCompiledCodeRuleDsl,
  root: string,
  { expected, sample }: { expected: Expectation; sample: CodeRuleCase },
): Promise<RuleDraftCaseResult> {
  const language = languageFor(sample.path);
  let caught = false;
  let note = "";
  if (language === undefined) note = " (not a file type rules read)";
  else if (!compiled.inScope(sample.path)) note = " (outside the files this rule covers)";
  else {
    const file = { path: sample.path, language, source: sample.content, ...moduleFor(binding, root, sample.path, language) };
    caught = (await evaluate(binding, compiled, [file])).some(violation => violation.path === undefined || violation.path === sample.path);
  }
  return { expected, caught, ok: caught === (expected === "caught"), label: `${sample.path}${note}` };
}

/** One file a repository scan would read, with its size on disk, or undefined when the scan skips it. */
async function repositoryFile(
  binding: NativeBinding,
  compiled: NativeCompiledCodeRuleDsl,
  root: string,
  absolute: string,
  bytesSoFar: number,
): Promise<{ file: SourceFile; size: number } | undefined> {
  const path = relative(root, absolute).split(sep).join("/");
  const language = languageFor(path);
  if (language === undefined || !compiled.inScope(path)) return undefined;
  const size = (await stat(absolute)).size;
  if (size > REPOSITORY_MAX_FILE_BYTES || bytesSoFar + size > REPOSITORY_MAX_SOURCE_BYTES) return undefined;
  const source = await readFile(absolute);
  if (source.includes(0)) return undefined;
  return { file: { path, language, source: source.toString("utf8"), ...moduleFor(binding, root, path, language) }, size };
}

async function sortedEntries(directory: string): Promise<Dirent[]> {
  const entries: Dirent[] = [];
  for await (const entry of await opendir(directory)) entries.push(entry);
  return entries.sort((left, right) => left.name.localeCompare(right.name));
}

/** Every regular file under `root` a scan visits, depth-first in name order, skipping ignored directories and symlinks. */
async function* repositoryPaths(root: string): AsyncGenerator<string> {
  const pending = [root];
  while (pending.length > 0) {
    const directory = pending.pop()!;
    for (const entry of await sortedEntries(directory)) {
      const absolute = join(directory, entry.name);
      if (entry.isDirectory() && !IGNORED_DIRECTORIES.has(entry.name)) pending.push(absolute);
      else if (entry.isFile()) yield absolute;
    }
  }
}

/** The files a repository scan would hand `compiled`: the same ignored directories, file types and bounds. */
async function repositoryFiles(binding: NativeBinding, compiled: NativeCompiledCodeRuleDsl, root: string): Promise<SourceFile[]> {
  const files: SourceFile[] = [];
  let bytes = 0;
  for await (const absolute of repositoryPaths(root)) {
    if (files.length >= REPOSITORY_MAX_FILES) break;
    const read = await repositoryFile(binding, compiled, root, absolute, bytes);
    if (read === undefined) continue;
    bytes += read.size;
    files.push(read.file);
  }
  return files;
}

async function repositoryPass(
  binding: NativeBinding,
  compiled: NativeCompiledCodeRuleDsl,
  root: string,
): Promise<NonNullable<RuleDraftReport["repository"]>> {
  const files = await repositoryFiles(binding, compiled, root);
  const sources = new Map(files.map(file => [file.path, file.source]));
  const located = (await evaluate(binding, compiled, files))
    .map(violation => ({
      path: violation.path ?? "",
      line: (violation.span?.startLine ?? 0) + 1,
      column: (violation.span?.startColumn ?? 0) + 1,
    }))
    .sort((left, right) => left.path.localeCompare(right.path) || left.line - right.line || left.column - right.column);
  const examples = located.slice(0, REPOSITORY_EXAMPLES).map(match => ({
    ...match,
    text: (sources.get(match.path)?.split("\n")[match.line - 1] ?? "").trim().slice(0, EXAMPLE_TEXT_LENGTH),
  }));
  return { matches: located.length, examples };
}

function conversationCaseResults(rule: ConversationRule, cases: ReadonlyArray<{ expected: Expectation; sample: ConversationCase }>): RuleDraftCaseResult[] {
  let registry: ConversationRuleRegistry;
  try {
    registry = new ConversationRuleRegistry({
      modelPath: process.env.RERAIL_MODEL_PATH ?? bundledModelPath(),
    });
  } catch (error) {
    throw engineUnavailable(rule.id, errorMessage(error));
  }
  try {
    registry.replace([rule]);
  } catch (error) {
    throw new RuleDraftError(errorMessage(error));
  }
  return cases.map(({ expected, sample }) => {
    const caught = registry.match(sample, { respectRepeat: false }).some(match => match.ruleId === rule.id);
    const text = sample.text.length > CASE_LABEL_LENGTH ? `${sample.text.slice(0, CASE_LABEL_LENGTH - 1)}…` : sample.text;
    return { expected, caught, ok: caught === (expected === "caught"), label: `${sample.source}: ${text}` };
  });
}

/**
 * Try a draft without storing it: validate it as storing would, run its cases, and for a policy rule count its
 * matches across `repo`'s files. Nothing is written to the catalog or the content store.
 */
export async function testRuleDraft(draft: RuleDraft, repo: string): Promise<RuleDraftReport> {
  if (draft.kind === "conversation") {
    const cases = conversationCaseResults(draft.rule, draft.cases);
    return { ok: cases.every(result => result.ok), valid: true, cases, repository: null };
  }
  const { binding, compiled } = compileCodeRule(draft.rule);
  const root = repositoryIdentityLocation(repo).workspaceRoot;
  const cases: RuleDraftCaseResult[] = [];
  for (const entry of draft.cases) cases.push(await policyCaseResult(binding, compiled, root, entry));
  const repository = await repositoryPass(binding, compiled, root);
  return { ok: cases.every(result => result.ok), valid: true, cases, repository };
}

/** `testRuleDraft` for a draft file; an unreadable or invalid draft is reported, not thrown. */
export async function testRuleDraftFile(file: string, repo: string): Promise<RuleDraftReport> {
  try {
    return await testRuleDraft(readRuleDraft(file), repo);
  } catch (error) {
    if (!(error instanceof RuleDraftError)) throw error;
    return { ok: false, valid: false, error: error.message, cases: [], repository: null };
  }
}

export interface AddedRule {
  ruleId: string;
  repositoryName: string;
  /** The repository folder the rule was added at; `""` is the whole repository. */
  folder: string;
  /** Whether adding the rule set the repository up, with this rule as its only one. */
  setUp: boolean;
}

/**
 * Store the draft's rule in your library, through the same validation as `library_rules_upsert`, and add it to
 * `repo`'s repository at the draft's folder. A repository that is not set up yet is set up with this rule (an
 * agent creating a rule may add it to the repository). A By Rerail rule is never replaced: its id is refused.
 */
export function addRuleDraft(draft: RuleDraft, repo: string, databasePath = defaultRerailDatabasePath()): AddedRule {
  const database = openRerailDatabase(databasePath);
  try {
    const checkout = recordCheckout(database, repo);
    const bundle = loadBundledCatalog();
    // A rule by Rerail is never overwritten by one the person drafts.
    if (bundle.documents.has(draft.rule.id)) {
      throw new RuleDraftError(`${draft.rule.id} is a rule by Rerail, so it was not replaced. Give the new rule another id.`);
    }
    try {
      upsertLibraryRule(database, bundle, draft.rule, draft.kind);
    } catch (error) {
      if (error instanceof EngineUnavailableError) throw new RuleDraftError(`the Engine is unavailable, so rule ${draft.rule.id} cannot be checked`);
      throw new RuleDraftError(errorMessage(error));
    }
    const attachment = { sourceKind: "library_rule" as const, sourceId: draft.rule.id, folder: draft.folder };
    const setUp = checkoutStatus(database, checkout).setUp === false;
    if (setUp) setUpRepository(database, checkout.repositoryKey, { packs: [], rules: [attachment] });
    else addAttachment(database, { repositoryKey: checkout.repositoryKey, ...attachment });
    return { ruleId: draft.rule.id, repositoryName: repositoryName(checkout.checkoutPath), folder: draft.folder, setUp };
  } finally {
    database.$client.close();
  }
}

function caseLine(result: RuleDraftCaseResult): string {
  const outcome = result.caught ? "caught" : "passed";
  return result.ok
    ? `  ✓ ${outcome}, as expected: ${result.label}`
    : `  ✗ ${outcome}, expected ${result.expected}: ${result.label}`;
}

function repositoryLines(repository: RuleDraftReport["repository"]): string[] {
  if (repository === null) return ["  Repo: not checked for conversation rules."];
  const { matches, examples } = repository;
  const lines = [`  Repo: ${matches} ${matches === 1 ? "place matches" : "places match"}.`];
  for (const example of examples) lines.push(`    ${example.path}:${example.line}:${example.column}  ${example.text}`);
  if (matches > examples.length) lines.push(`    … and ${matches - examples.length} more.`);
  return lines;
}

function verdictLine(cases: readonly RuleDraftCaseResult[]): string {
  const failed = cases.filter(result => !result.ok).length;
  if (failed === 0) return "● Every case matched.";
  return `● ${failed} ${failed === 1 ? "case did" : "cases did"} not match.`;
}

export function formatRuleDraftReport(report: RuleDraftReport): string {
  if (!report.valid) return `● The rule is not valid: ${report.error ?? "unknown error"}`;
  return [
    "● The rule is valid.",
    ...report.cases.map(caseLine),
    ...repositoryLines(report.repository),
    verdictLine(report.cases),
  ].join("\n");
}

export interface RuleDraftCommandIo {
  stdout?: { write(text: string): unknown };
  stderr?: { write(text: string): unknown };
}

function draftArguments(action: "test" | "add", args: readonly string[], usage: () => string): { file: string; repo: string; json: boolean } {
  const file = args[0];
  if (file === undefined || file.startsWith("--")) throw new Error(`rules ${action} needs a draft file\n\n${usage()}`);
  const parsed = parseArguments(
    args.slice(1),
    { flags: action === "test" ? ["--json"] : [], options: ["--repo"] },
    argument => `Unknown rules ${action} option: ${argument}\n\n${usage()}`,
  );
  return { file, repo: resolve(parsed.options.get("--repo") ?? process.cwd()), json: parsed.flags.has("--json") };
}

/** `rerail rules test <draft.json>` and `rerail rules add <draft.json>`. */
export async function runRuleDraftCommand(
  action: "test" | "add",
  args: readonly string[],
  usage: () => string,
  io: RuleDraftCommandIo = {},
): Promise<number> {
  const stdout = io.stdout ?? process.stdout;
  const { file, repo, json } = draftArguments(action, args, usage);
  if (action === "test") {
    const report = await testRuleDraftFile(file, repo);
    stdout.write(`${json ? JSON.stringify(report) : formatRuleDraftReport(report)}\n`);
    return report.ok ? 0 : 1;
  }
  let added: AddedRule;
  try {
    added = addRuleDraft(readRuleDraft(file), repo);
  } catch (error) {
    if (!(error instanceof RuleDraftError)) throw error;
    (io.stderr ?? process.stderr).write(`● The rule was not added: ${error.message}\n`);
    return 1;
  }
  const where = added.folder === "" ? added.repositoryName : `${added.repositoryName}/${added.folder}`;
  stdout.write(`● Added ${added.ruleId} to ${where}.\n`);
  if (added.setUp) stdout.write(`  ${added.repositoryName} was not set up, so it is now set up with this rule.\n`);
  // The same dashboard the footer and steers link to, scoped to this checkout's repository when the daemon knows it.
  const dashboard = await readRulesDashboard({ cwd: repo });
  if (dashboard.url !== null) {
    const link = new URL(dashboard.url);
    link.hash = `rule/${encodeURIComponent(added.ruleId)}`;
    stdout.write(`  ${link.href}\n`);
  }
  return 0;
}
