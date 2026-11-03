/**
 * Capture (docs/merge-assist-v1-build.md §5.2): the corrections a person gave their coding agent in one session, turned into
 * candidate rules on the branch. It reads the session's stored events after its cursor, keeps the prompts the
 * correction phrases match (never the session's first prompt, which states the task), and asks the agent CLI whether
 * each records a lasting decision. A decision is written to `.rerail/candidates/` with a code rule when the drafted check
 * compiles, passes its own cases and matches nothing in the corrected checkout; a check that fails gets one repair round
 * with the failure, and is otherwise dropped, leaving a note: the decision alone.
 */
import { createHash } from "node:crypto";
import { lstatSync, readFileSync, realpathSync } from "node:fs";
import { basename, dirname, isAbsolute, join, relative, resolve, sep } from "node:path";
import { z } from "zod";
import type { RerailDatabase } from "../../engine/storage/database.js";
import { mergeAssistItems } from "../../engine/storage/schema.js";
import type { CheckoutContext } from "../../engine/catalog/repository-model.js";
import { followsGitignore } from "../../engine/catalog/repository-registry.js";
import { codeRuleSchema, type CodeRule } from "../../engine/code-rule/code-rule-schema.js";
import { checkCodeRule, type RuleCheckCase } from "../../engine/code-rule/rule-check.js";
import {
  CANDIDATE_DIRECTORY,
  CANDIDATE_FORMAT,
  candidateFileName,
  candidateHash,
  candidateSchema,
  scopePatternSchema,
  unusedCandidateId,
  type Candidate,
} from "../../engine/merge-assist/candidate.js";
import { readWorktreeCandidates, writeWorktreeCandidate } from "../../engine/merge-assist/candidate-files.js";
import { callSummary, correctionFamilies } from "../../engine/merge-assist/corrections.js";
import { deniedByPathBoundary } from "../../engine/path-boundary.js";
import { stableJsonText } from "../../engine/stable-json.js";
import type { SessionEventRecord } from "../session-store.js";
import type { AgentJudge } from "./agent-judge.js";
import {
  captureJsonSchema,
  capturePrompt,
  captureRepairPrompt,
  captureVerdictSchema,
  newJudgeNonce,
  type CaptureInput,
  type CaptureVerdict,
  type CheckNode,
  type CheckRule,
} from "./judge-prompts.js";
import { STATE_KEYS, isoSeconds, itemId, readState, writeState } from "./state.js";

/** The most corrections one capture judges; the rest of the read events are passed over. */
export const MAX_CORRECTIONS_PER_CAPTURE = 3;
/** A longer prompt is a brief, not a correction. */
const MAX_CORRECTION_CHARS = 2_000;
/** The agent's calls named before a correction. */
const PRIOR_CALLS = 6;
/** How far back a correction's prior calls are looked for. */
const PRIOR_EVENTS = 200;
const MAX_EDITS = 4;
const MAX_EDIT_SIDE_CHARS = 2_000;
/** The repository matches a failed check's repair round names. */
const REPAIR_MATCHES = 3;
const MAX_MATCH_TEXT_CHARS = 200;
const EVENT_PAGE = 500;

/** Where capture reads a session's stored events: the session store's `events`. */
export interface SessionEventSource {
  events(sessionId: string, query: { after?: number; before?: number; limit?: number }): readonly SessionEventRecord[];
}

export interface CaptureContext {
  readonly database: RerailDatabase;
  readonly checkout: CheckoutContext;
  /** The judge for this session's host. */
  readonly judge: AgentJudge;
  /** The author a new candidate names: a login or name. */
  readonly author: () => Promise<string>;
  readonly now: () => number;
  /** The random suffix of a new candidate id; by default three random bytes in hex (a test seam). */
  readonly idSuffix?: () => string;
}

export interface CaptureRequest {
  readonly sessionId: string;
  readonly events: SessionEventSource;
}

export interface CaptureResult {
  /** Candidates written to the checkout, in the order of their corrections. */
  readonly saved: readonly Candidate[];
  /** Corrections the judge could not answer, with why. */
  readonly unjudged: readonly string[];
}

const cursorSchema = z.object({ after: z.number().int().nonnegative(), tasked: z.boolean() });

interface Edit {
  readonly path: string;
  readonly before: string | null;
  readonly after: string | null;
}

interface Correction {
  readonly sequence: number;
  readonly text: string;
  readonly families: string[];
  /** The agent's edits after the correction, up to the next prompt: what it changed because of it. */
  readonly edits: Edit[];
}

function record(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value) ? value as Record<string, unknown> : {};
}

function cut(text: string): string {
  return text.length <= MAX_EDIT_SIDE_CHARS ? text : `${text.slice(0, MAX_EDIT_SIDE_CHARS)}\n[cut]`;
}

/**
 * `path` relative to the checkout with `/`, or undefined for one outside it or behind the data boundary (a hidden or
 * secret-like path), which never reaches the judge.
 */
function pathInCheckout(root: string, path: string): string | undefined {
  let absolute = isAbsolute(path) ? path : resolve(root, path);
  try {
    // The checkout root is physical; a host may name the file through a symlinked folder (macOS's /var).
    absolute = join(realpathSync(dirname(absolute)), basename(absolute));
  } catch {
    // A folder that is gone keeps the path as named.
  }
  const inside = relative(root, absolute);
  if (inside === "" || inside.startsWith("..") || isAbsolute(inside)) return undefined;
  const portable = inside.split(sep).join("/");
  return deniedByPathBoundary(portable) ? undefined : portable;
}

/** The files a Codex `apply_patch` text changes: for each, the lines it removes (and keeps) and the lines it adds (and keeps). */
function patchEdits(patch: string): Array<{ path: string; before: string | null; after: string | null }> {
  const edits: Array<{ path: string; before: string[] | null; after: string[] }> = [];
  let current: { path: string; before: string[] | null; after: string[] } | undefined;
  for (const line of patch.split("\n")) {
    const header = /^\*\*\* (Add|Update|Delete) File: (.+)$/u.exec(line);
    if (header !== null) {
      current = header[1] === "Delete" ? undefined : { path: header[2]!.trim(), before: header[1] === "Add" ? null : [], after: [] };
      if (current !== undefined) edits.push(current);
      continue;
    }
    if (current === undefined || line.startsWith("***")) continue;
    if (line.startsWith("+")) current.after.push(line.slice(1));
    else if (line.startsWith("-")) current.before?.push(line.slice(1));
    else if (line.startsWith(" ")) {
      current.before?.push(line.slice(1));
      current.after.push(line.slice(1));
    } else if (line.startsWith("@@")) {
      current.before?.push("…");
      current.after.push("…");
    }
  }
  return edits.map(edit => ({ path: edit.path, before: edit.before?.join("\n") ?? null, after: edit.after.join("\n") }));
}

/**
 * What an edit-like call changed: Claude Code's `Edit` (`file_path`, `old_string`, `new_string`), `MultiEdit`
 * (`edits`), `Write` (`file_path`, `content`), a write that names `path` and `content`, and a Codex patch text.
 */
export function callEdits(payload: Record<string, unknown>, root: string): Edit[] {
  const input = record(payload.input);
  const raw: Array<{ path: string; before: string | null; after: string | null }> = [];
  const file = [input.file_path, input.filePath, input.path].find(value => typeof value === "string") as string | undefined;
  if (file !== undefined && typeof input.old_string === "string" && typeof input.new_string === "string") {
    raw.push({ path: file, before: input.old_string, after: input.new_string });
  } else if (file !== undefined && Array.isArray(input.edits)) {
    for (const entry of input.edits.map(record)) {
      if (typeof entry.old_string === "string" && typeof entry.new_string === "string") {
        raw.push({ path: file, before: entry.old_string, after: entry.new_string });
      }
    }
  } else if (file !== undefined && typeof input.content === "string") {
    raw.push({ path: file, before: null, after: input.content });
  } else {
    const patch = [payload.input, input.patch, input.input, input.command]
      .find((value): value is string => typeof value === "string" && value.includes("*** Begin Patch"));
    if (patch !== undefined) raw.push(...patchEdits(patch));
  }
  return raw.flatMap(edit => {
    const path = pathInCheckout(root, edit.path);
    if (path === undefined) return [];
    return [{ path, before: edit.before === null ? null : cut(edit.before), after: edit.after === null ? null : cut(edit.after) }];
  });
}

/** Every stored event of the session after `after`, oldest first. */
function eventsAfter(source: SessionEventSource, sessionId: string, after: number): SessionEventRecord[] {
  const all: SessionEventRecord[] = [];
  for (let cursor = after; ;) {
    const page = source.events(sessionId, { after: cursor, limit: EVENT_PAGE });
    all.push(...page);
    if (page.length < EVENT_PAGE) return all;
    cursor = page.at(-1)!.sequence;
  }
}

/** The agent's last calls before the event at `sequence`, oldest first, each as `<tool> <command or path>`. */
function priorCalls(source: SessionEventSource, sessionId: string, sequence: number): string[] {
  return source.events(sessionId, { before: sequence, limit: PRIOR_EVENTS })
    .filter(event => event.kind === "call")
    .slice(-PRIOR_CALLS)
    .map(event => callSummary(record(event.payload)));
}

/** A scope for a decision whose verdict named none usable: the folders the corrected files are in. */
function fallbackScope(edits: readonly Edit[]): string[] {
  const folders = [...new Set(edits.map(edit => dirname(edit.path)))];
  return folders.length === 0 || folders.includes(".") ? ["**"] : folders.map(folder => `${folder}/**`);
}

/** The engine's ast-grep node for one node of an AST check. */
function astNode(node: CheckNode): { pattern: string } | { kind: string } {
  return node.pattern !== null ? { pattern: node.pattern } : { kind: node.nodeKind ?? "" };
}

/** The engine's ast-grep rule for an AST check: the node it forbids, with `has` and `inside` looking at any depth. */
function astRule(rule: CheckRule): Record<string, unknown> {
  return {
    ...astNode(rule),
    ...(rule.has === null ? {} : { has: { ...astNode(rule.has), stopBy: "end" } }),
    ...(rule.inside === null ? {} : { inside: { ...astNode(rule.inside), stopBy: "end" } }),
  };
}

/** Why a drafted check cannot be kept: `reason` for the item's evidence, `detail` for the judge's repair round. */
interface CheckFailure {
  readonly reason: string;
  readonly detail: string;
}

/** The line at `line` of the checkout's file at `path`, trimmed and cut, as a repair round names a match. */
function matchedText(root: string, path: string, line: number): string {
  try {
    const text = readFileSync(join(root, path), "utf8").split("\n")[line - 1]?.trim() ?? "";
    return text.length <= MAX_MATCH_TEXT_CHARS ? text : `${text.slice(0, MAX_MATCH_TEXT_CHARS)} [cut]`;
  } catch {
    return "";
  }
}

/**
 * The `rerail.policy/v1` rule a verdict's check drafts, kept only when it compiles, catches every caught case, passes
 * every passed case and matches nothing in the checkout as it is after the correction; otherwise why not.
 */
async function checkedRule(
  id: string,
  verdict: CaptureVerdict & { check: NonNullable<CaptureVerdict["check"]> },
  scope: readonly string[],
  root: string,
  followGitignore: boolean,
): Promise<{ rule: CodeRule } | CheckFailure> {
  const { check } = verdict;
  const parsed = codeRuleSchema.safeParse({
    format: "rerail.policy/v1",
    id,
    revision: 1,
    mode: "steer",
    description: verdict.decision,
    scope: { include: scope, exclude: [] },
    clauses: [{
      id: "check",
      forbid: check.kind === "regex"
        ? { regex: { source: "file.content", pattern: check.pattern } }
        : { ast: { language: check.language, rule: astRule(check.rule) } },
      violation: { code: "MERGE_ASSIST_DECISION", anchor: "witness.span", message: verdict.message },
    }],
  });
  if (!parsed.success) {
    const reason = `the drafted check is not a valid rule: ${parsed.error.issues[0]?.message ?? "invalid"}`;
    return { reason, detail: reason };
  }
  const rule = parsed.data as CodeRule;
  if (verdict.cases.caught.length === 0) return { reason: "the drafted check has no case it must catch", detail: "the answer has no caught case: give at least one" };
  const cases: RuleCheckCase[] = [
    ...verdict.cases.caught.map((entry, index) => ({ expected: "caught" as const, label: `the code the agent wrote (${index + 1})`, ...entry })),
    ...verdict.cases.passed.map((entry, index) => ({ expected: "passed" as const, label: `the corrected code (${index + 1})`, ...entry })),
  ];
  const result = await checkCodeRule(rule, { root, cases, followGitignore, maxMatches: REPAIR_MATCHES });
  if (!result.valid) {
    const reason = `the drafted check does not compile: ${result.error ?? "unknown error"}`;
    return { reason, detail: reason };
  }
  const failing = result.cases.findIndex(entry => !entry.ok);
  if (failing >= 0) {
    const entry = result.cases[failing]!;
    const outcome = entry.notChecked !== undefined ? `could not be checked: ${entry.notChecked}` : entry.caught ? "is caught" : "is not caught";
    return {
      reason: `the drafted check fails its case "${cases[failing]!.label}"`,
      detail: `the ${entry.expected} case ${cases[failing]!.path} ("${entry.label}") must be ${entry.expected === "caught" ? "caught" : "passed"} but ${outcome}`,
    };
  }
  if (result.matchCount > 0) {
    const [first] = result.matches;
    const places = result.matches.map(match => `${match.path}:${match.line}: ${matchedText(root, match.path, match.line)}`);
    return {
      reason: `the drafted check matches ${result.matchCount} ${result.matchCount === 1 ? "place" : "places"} in the corrected repository, first ${first!.path}:${first!.line}`,
      detail: [`the check matches ${result.matchCount} ${result.matchCount === 1 ? "place" : "places"} in the repository after the correction, which must match none:`, ...places].join("\n"),
    };
  }
  return { rule };
}

/** A cache key for one judge task over `inputs`. */
function judgeCacheKey(kind: string, inputs: unknown): string {
  return createHash("sha256").update(`${kind}\0${stableJsonText(inputs) ?? ""}`).digest("hex");
}

/**
 * The rule a verdict's check drafts when it holds, or after one repair round in which the judge sees the failure and
 * corrects the check (the decision and scope stay); otherwise why there is none.
 */
async function draftedRule(
  id: string,
  verdict: CaptureVerdict,
  input: CaptureInput,
  scope: readonly string[],
  context: CaptureContext,
): Promise<{ rule: CodeRule } | { problem: string }> {
  if (verdict.check === null) return { problem: "the judgment drafted no check" };
  const root = context.checkout.checkoutPath;
  const followGitignore = followsGitignore(context.database, context.checkout);
  const first = await checkedRule(id, { ...verdict, check: verdict.check }, scope, root, followGitignore);
  if ("rule" in first) return first;
  const outcome = await context.judge.judge({
    kind: "capture",
    repositoryKey: context.checkout.repositoryKey,
    prompt: captureRepairPrompt(input, verdict, first.detail, newJudgeNonce()),
    jsonSchema: captureJsonSchema,
    parse: value => captureVerdictSchema.parse(value),
    cacheKey: judgeCacheKey("capture-repair", { input, verdict, failure: first.detail }),
  });
  if (!outcome.ok) return { problem: `${first.reason}; the repair round failed: ${outcome.detail}` };
  const { check, message, cases } = outcome.value;
  if (check === null) return { problem: `${first.reason}; the repair round drafted no check` };
  const second = await checkedRule(id, { ...verdict, check, message, cases }, scope, root, followGitignore);
  return "rule" in second ? second : { problem: `${second.reason} (after a repair round)` };
}

/** The candidate a verdict states, with its rule when the drafted check holds; why the rule was dropped, if it was. */
async function candidateOf(
  verdict: CaptureVerdict,
  input: CaptureInput,
  context: CaptureContext,
): Promise<{ candidate: Candidate; dropped: string | null } | undefined> {
  const { checkout } = context;
  const usable = verdict.scope.filter(pattern => scopePatternSchema.safeParse(pattern).success);
  const scope = usable.length > 0 ? usable : fallbackScope(input.edits);
  // A random suffix can repeat: an id whose file is already there (even one that no longer reads) is never reused.
  const id = unusedCandidateId(verdict.decision, taken => {
    try {
      lstatSync(join(checkout.checkoutPath, CANDIDATE_DIRECTORY, candidateFileName(taken)));
      return true;
    } catch {
      return false;
    }
  }, context.idSuffix);
  const base = {
    format: CANDIDATE_FORMAT,
    id,
    decision: verdict.decision,
    generality: verdict.generality === "repository" ? "repository" : "branch",
    scope,
    author: await context.author(),
    createdAt: isoSeconds(context.now()),
    state: "active",
    supersedes: null,
  };
  const drafted = await draftedRule(id, verdict, input, scope, context);
  let dropped = "problem" in drafted ? drafted.problem : null;
  if ("rule" in drafted) {
    const withRule = candidateSchema.safeParse({ ...base, rule: drafted.rule });
    if (withRule.success) return { candidate: withRule.data, dropped: null };
    dropped = "the drafted check does not fit a candidate file";
  }
  const note = candidateSchema.safeParse(base);
  return note.success ? { candidate: note.data, dropped } : undefined;
}

/** The corrections among `events`, each with the agent's edits that followed it; the first prompt is the task. */
function corrections(events: readonly SessionEventRecord[], root: string, tasked: boolean): { found: Correction[]; tasked: boolean } {
  const found: Correction[] = [];
  let current: Correction | undefined;
  for (const event of events) {
    const payload = record(event.payload);
    if (event.kind === "user-prompt") {
      current = undefined;
      if (!tasked) {
        tasked = true;
        continue;
      }
      const text = typeof payload.text === "string" ? payload.text : "";
      const families = text.length <= MAX_CORRECTION_CHARS ? correctionFamilies(text) : [];
      if (families.length === 0) continue;
      current = { sequence: event.sequence, text, families, edits: [] };
      found.push(current);
    } else if (event.kind === "call" && current !== undefined && current.edits.length < MAX_EDITS) {
      current.edits.push(...callEdits(payload, root).slice(0, MAX_EDITS - current.edits.length));
    }
  }
  return { found, tasked };
}

/**
 * Judge the session's corrections since its cursor and write each decision as a candidate in the checkout, with an
 * own item linking it to the session and prompt it came from. At most {@link MAX_CORRECTIONS_PER_CAPTURE} corrections,
 * the newest, are judged per run; the cursor moves past every event read, so none is judged twice.
 */
export async function captureCorrections(context: CaptureContext, request: CaptureRequest): Promise<CaptureResult> {
  const { database, checkout } = context;
  const root = checkout.checkoutPath;
  const cursorKey = STATE_KEYS.cursor(request.sessionId);
  const cursor = readState(database, cursorKey, cursorSchema) ?? { after: 0, tasked: false };
  const events = eventsAfter(request.events, request.sessionId, cursor.after);
  if (events.length === 0) return { saved: [], unjudged: [] };
  const { found, tasked } = corrections(events, root, cursor.tasked);
  const saved: Candidate[] = [];
  const unjudged: string[] = [];
  if (found.length > 0) {
    const own = await readWorktreeCandidates(root);
    const ownDecisions = own.candidates.filter(candidate => candidate.state === "active").map(({ id, decision }) => ({ id, decision }));
    for (const correction of found.slice(-MAX_CORRECTIONS_PER_CAPTURE)) {
      const input: CaptureInput = {
        correction: correction.text,
        priorCalls: priorCalls(request.events, request.sessionId, correction.sequence),
        edits: correction.edits,
        ownDecisions,
      };
      const outcome = await context.judge.judge({
        kind: "capture",
        repositoryKey: checkout.repositoryKey,
        prompt: capturePrompt(input, newJudgeNonce()),
        jsonSchema: captureJsonSchema,
        parse: value => captureVerdictSchema.parse(value),
        cacheKey: judgeCacheKey("capture", input),
      });
      if (!outcome.ok) {
        unjudged.push(outcome.detail);
        continue;
      }
      const verdict = outcome.value;
      if (verdict.generality === "one-off" || verdict.duplicateOf !== null) continue;
      const made = await candidateOf(verdict, input, context);
      if (made === undefined) continue;
      await writeWorktreeCandidate(root, made.candidate);
      const now = context.now();
      database.insert(mergeAssistItems).values({
        id: itemId(root, "own", "", made.candidate.id),
        checkoutPath: root,
        repositoryKey: checkout.repositoryKey,
        candidateId: made.candidate.id,
        candidateHash: candidateHash(made.candidate),
        origin: "own",
        peerRef: "",
        trusted: true,
        status: "applied",
        candidate: made.candidate,
        evidence: { families: correction.families, ruleDropped: made.dropped },
        sessionId: request.sessionId,
        eventSequence: correction.sequence,
        createdAt: now,
        updatedAt: now,
      }).onConflictDoNothing().run();
      ownDecisions.push({ id: made.candidate.id, decision: made.candidate.decision });
      saved.push(made.candidate);
    }
  }
  writeState(database, cursorKey, { after: events.at(-1)!.sequence, tasked }, context.now());
  return { saved, unjudged };
}
