import { existsSync, readFileSync } from "node:fs";
import { resolve } from "node:path";
import { bundledModelPath } from "../engine/package-root.js";
import { z } from "zod";
import { libraryUpsertBodySchema } from "../api/repository-contracts.js";
import { parseArguments } from "../daemon/daemon.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { checkCase, compileCheck, engineUnavailableMessage, repositoryPass, RuleCheckError } from "../engine/code-rule/rule-check.js";
import type { CodeRule } from "../engine/code-rule/code-rule-schema.js";
import { EngineUnavailableError } from "../engine/code-rule/code-rule-compile.js";
import { ConversationRuleRegistry } from "../engine/catalog/registry.js";
import { repositoryIdentityLocation, repositoryName } from "../engine/catalog/repository-identity.js";
import { normalizeFolder } from "../engine/catalog/repository-model.js";
import { checkoutStatus, followsGitignore, lookupCheckout, recordCheckout } from "../engine/catalog/repository-registry.js";
import { addAttachment, setUpRepository } from "../engine/catalog/rule-configuration.js";
import { upsertLibraryRule } from "../engine/catalog/rule-library.js";
import { parseConversationRule, sampleSourceSchema, SAMPLE_BOUNDARIES, type ConversationRule } from "../engine/catalog/rule-schema.js";
import { readRulesDashboard } from "../presentation/session-presentation.js";

/** How many repository matches the report shows; the count covers every match. */
const REPOSITORY_EXAMPLES = 5;
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
  | { kind: "code"; rule: CodeRule; folder: string; cases: ReadonlyArray<{ expected: Expectation; sample: CodeRuleCase }> }
  | { kind: "conversation"; rule: ConversationRule; folder: string; cases: ReadonlyArray<{ expected: Expectation; sample: ConversationCase }> };

/** A draft is added to the repository of the checkout it is added from; it never names another. */
const TARGET_FIELDS = ["repository", "checkout", "attach"] as const;

export interface RuleDraftCaseResult {
  expected: Expectation;
  caught: boolean;
  ok: boolean;
  label: string;
  /** Why a clause of the rule could not check the case's file, which then decides nothing: never `ok`. */
  notChecked?: string;
}

export interface RuleDraftRepositoryMatch {
  path: string;
  line: number;
  column: number;
  text: string;
}

/** A file some clause of the rule could not check: the first such failure's message. */
export interface RuleDraftUncheckedFile {
  path: string;
  message: string;
}

export interface RuleDraftReport {
  ok: boolean;
  valid: boolean;
  error?: string;
  cases: RuleDraftCaseResult[];
  repository: {
    matches: number;
    examples: RuleDraftRepositoryMatch[];
    /** Files some clause could not check, and the messages of such failures that name no file. */
    unchecked: { count: number; examples: RuleDraftUncheckedFile[]; unattributed: string[] };
    /**
     * Files and folders the data boundary kept the pass from reading: hidden, sensitive by name, or ignored by Git
     * while the repository follows `.gitignore`. A count only; their names stay out of the report.
     */
    leftOut: number;
  } | null;
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
  if (upsert.data.kind === "code") {
    const schema = codeRuleCaseSchema;
    return {
      kind: "code",
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
  return new RuleDraftError(engineUnavailableMessage(ruleId, reason));
}

/** Whether `root`'s repository follows `.gitignore`, read without recording anything: on where Rerail cannot tell. */
function repositoryFollowsGitignore(root: string, databasePath = defaultRerailDatabasePath()): boolean {
  if (!existsSync(databasePath)) return true;
  let database: RerailDatabase | undefined;
  try {
    database = openRerailDatabase(databasePath, { readonly: true });
    return followsGitignore(database, lookupCheckout(database, root));
  } catch {
    return true;
  } finally {
    database?.$client.close();
  }
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
  try {
    const check = compileCheck(draft.rule);
    const root = repositoryIdentityLocation(repo).workspaceRoot;
    const cases: RuleDraftCaseResult[] = [];
    for (const { expected, sample } of draft.cases) {
      cases.push(await checkCase(check, root, { expected, label: sample.path, path: sample.path, content: sample.content }));
    }
    const pass = await repositoryPass(check, root, { followGitignore: repositoryFollowsGitignore(root) });
    const repository = {
      matches: pass.matches.length,
      examples: pass.matches.slice(0, REPOSITORY_EXAMPLES).map(match => ({ path: match.path, line: match.line, column: match.column, text: pass.matchedLine(match) })),
      unchecked: { count: pass.unchecked.files.length, examples: pass.unchecked.files.slice(0, REPOSITORY_EXAMPLES), unattributed: [...pass.unchecked.unattributed] },
      leftOut: pass.leftOut,
    };
    return { ok: cases.every(result => result.ok), valid: true, cases, repository };
  } catch (error) {
    if (error instanceof RuleCheckError) throw new RuleDraftError(error.message);
    throw error;
  }
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
    const ruleCatalog = loadRuleCatalog();
    // A rule by Rerail is never overwritten by one the person drafts.
    if (ruleCatalog.documents.has(draft.rule.id)) {
      throw new RuleDraftError(`${draft.rule.id} is a rule by Rerail, so it was not replaced. Give the new rule another id.`);
    }
    try {
      upsertLibraryRule(database, ruleCatalog, draft.rule, draft.kind);
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
  if (result.notChecked !== undefined) return `  ✗ not checked (${result.notChecked}), expected ${result.expected}: ${result.label}`;
  const outcome = result.caught ? "caught" : "passed";
  return result.ok
    ? `  ✓ ${outcome}, as expected: ${result.label}`
    : `  ✗ ${outcome}, expected ${result.expected}: ${result.label}`;
}

function repositoryLines(repository: RuleDraftReport["repository"]): string[] {
  if (repository === null) return ["  Repo: not checked for conversation rules."];
  const { matches, examples, unchecked, leftOut } = repository;
  const lines = [`  Repo: ${matches} ${matches === 1 ? "place matches" : "places match"}.`];
  for (const example of examples) lines.push(`    ${example.path}:${example.line}:${example.column}  ${example.text}`);
  if (matches > examples.length) lines.push(`    … and ${matches - examples.length} more.`);
  if (unchecked.count > 0) {
    lines.push(`  ${unchecked.count} ${unchecked.count === 1 ? "file was" : "files were"} not checked, so the rule may miss places there:`);
    for (const file of unchecked.examples) lines.push(`    ${file.path}  ${file.message}`);
    if (unchecked.count > unchecked.examples.length) lines.push(`    … and ${unchecked.count - unchecked.examples.length} more.`);
  }
  for (const message of unchecked.unattributed) lines.push(`  Not checked in any one file (${message}), so the rule may miss places anywhere.`);
  if (leftOut > 0) {
    lines.push(`  ${leftOut} ${leftOut === 1 ? "file or folder was" : "files or folders were"} left out unread: hidden, sensitive by name, or ignored by Git.`);
  }
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
