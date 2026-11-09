/**
 * Merge Assist's resolver: given a conflict record and the repository, it runs the policy and returns the verdict
 * with its evidence, or an escalation with the reason. Steps run in the policy's order and the first that decides
 * wins: precedent from the ledger view, hard checks (each side's checks run by a `CheckRunner`, undeclared public
 * contract breaks), measurable criteria, then the judge panel; a reserved class goes to a person before the panel, and
 * so does whatever nothing decides. The resolver reads Git and what the record carries; it never sees how the work
 * ended (merge state, comments, closing text, cross-references), so a record's intents must be sanitised.
 */
import { spawn } from "node:child_process";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { gitExecutable } from "../../engine/git.js";
import { baseDocs, contractBreaks, declaresBreak, reservedClassesOf, sideChanges, sideDiff, type SideChanges } from "../../engine/merge-assist/conflict-evidence.js";
import { refLabel } from "../../engine/merge-assist/conflicts.js";
import { DECISION_ENTRY_FORMAT, type DecisionEntry, MAX_RESULT_TEXT, type Position, sealDecisionEntry } from "../../engine/merge-assist/decision-entry.js";
import { decidePrecedent, type LedgerView, readLedger } from "../../engine/merge-assist/ledger.js";
import {
  type ConflictRecord,
  type DecidingStep,
  sanitizeIntent,
  stripCrossReferences,
  SIDES,
  type SideId,
  type StepResult,
  type Verdict,
} from "../../engine/merge-assist/conflict-record.js";
import {
  type CheckResult,
  type ContractBreak,
  decideCriteria,
  decideHardChecks,
  type Policy,
  type ReservedClass,
  reservedHits,
  type SideMeasures,
} from "../../engine/merge-assist/policy.js";
import type { AgentJudge } from "./agent-judge.js";
import { type PanelResult, runPanel } from "./panel.js";

/** How much of the docs at the base, and of each side's diff, the panel reads. */
const PANEL_DOC_CHARS = 24_000;
const PANEL_DIFF_CHARS = 40_000;

/** What one side's checks produced: each check's result, and each configured benchmark's measurement. */
export interface SideCheckOutcome {
  readonly checks: readonly CheckResult[];
  readonly benchmarks: Readonly<Record<string, number | null>>;
}

/** Runs the policy's checks and benchmarks on one side's head. The resolver's seam to wherever checks can run. */
export interface CheckRunner {
  run(side: SideId, sha: string, policy: Policy): Promise<SideCheckOutcome>;
}

export interface ResolveInput {
  readonly root: string;
  /** The repository's key, for the judge's budget. */
  readonly repositoryKey: string;
  readonly record: ConflictRecord;
  /** The policy at the record's base (`loadPolicy`). */
  readonly policy: Policy;
  /** Null: the panel cannot run, and a conflict it would judge escalates. */
  readonly judge: AgentJudge | null;
  /** Null: no check runs, and every check counts as unknown. */
  readonly checks: CheckRunner | null;
  /** The ledger view; read from the repository (`readLedger`) when absent. */
  readonly ledger?: Pick<LedgerView, "items">;
  /** The panel's seed; 0 by default. */
  readonly seed?: number;
}

export interface SideEvidence {
  readonly changes: SideChanges | null;
  readonly breaks: readonly ContractBreak[];
  readonly reserved: readonly ReservedClass[];
}

export interface ResolutionEvidence {
  readonly precedent: { readonly entry: { readonly id: string } | null } | null;
  readonly sides: Readonly<Record<SideId, SideEvidence>>;
  readonly checks: Readonly<Record<SideId, SideCheckOutcome>> | null;
  readonly measures: Readonly<Record<SideId, SideMeasures>> | null;
  readonly reserved: readonly ReservedClass[];
  readonly panel: PanelResult | null;
}

export interface Resolution {
  readonly verdict: Verdict;
  /** The step that decided, or `escalate`. */
  readonly step: DecidingStep;
  readonly reason: string;
  /** Every step that ran, in order, with its outcome. */
  readonly steps: readonly StepResult[];
  readonly evidence: ResolutionEvidence;
}

/** The existing callers a side's changed declarations leave to adapt: (declaration, dependent file) pairs. */
function callersAffected(breaks: readonly ContractBreak[]): number {
  return breaks.reduce((sum, change) => sum + change.dependents.length, 0);
}

/** One side's evidence from Git: what it changes from where it left the base, its contract breaks and reserved classes. */
async function sideEvidence(root: string, record: ConflictRecord, side: SideId, policy: Policy): Promise<SideEvidence> {
  const { sha, intent } = record.sides[side];
  const changes = sideChanges(root, record.base, sha);
  if (changes === null) return { changes: null, breaks: [], reserved: [] };
  const cleaned = sanitizeIntent(intent);
  const [breaks, declared] = await Promise.all([contractBreaks(root, sha, changes, { intent: cleaned, declared: policy.contracts.declared }), declaresBreak(root, sha, changes, cleaned)]);
  return { changes, breaks, reserved: reservedClassesOf(changes, breaks, declared) };
}

function escalation(reason: string, steps: StepResult[], evidence: ResolutionEvidence): Resolution {
  return { verdict: "escalated", step: "escalate", reason, steps, evidence };
}

/**
 * Resolves `record` under `policy`. Precedent, hard checks and criteria decide without a model; the panel runs only
 * when they don't and no reserved class applies. Escalates, with the reason, when a side's head is not in the
 * repository, the panel is needed but no judge is available, or nothing decides.
 */
export async function resolveConflict(input: ResolveInput): Promise<Resolution> {
  const { root, record, policy } = input;
  const steps: StepResult[] = [];
  const [a, b] = await Promise.all([sideEvidence(root, record, "a", policy), sideEvidence(root, record, "b", policy)]);
  const sides = { a, b };
  let evidence: ResolutionEvidence = { precedent: null, sides, checks: null, measures: null, reserved: [], panel: null };
  const missing = SIDES.filter(side => sides[side].changes === null);
  if (missing.length > 0) return escalation(`The repository does not hold side ${missing.join(" and ")}'s head or the base.`, steps, evidence);

  if (policy.steps.includes("precedent")) {
    const ledger = input.ledger ?? await readLedger(root);
    // A record carries no positions: precedent matches it by its sides' refs.
    const precedent = decidePrecedent({ question: record.question, sides: { a: { ref: record.sides.a.ref, fingerprint: null }, b: { ref: record.sides.b.ref, fingerprint: null } } }, ledger);
    steps.push({ step: precedent.step, winner: precedent.winner, reason: precedent.reason });
    evidence = { ...evidence, precedent: { entry: precedent.entry } };
    if (precedent.winner !== null) return decided(precedent, steps, evidence);
  }

  const needsRuns = policy.checks.run.length > 0 || policy.criteria.benchmarks.length > 0;
  if (needsRuns && input.checks !== null && (policy.steps.includes("hard-checks") || policy.steps.includes("criteria"))) {
    const runner = input.checks;
    const [left, right] = await Promise.all(SIDES.map(side => runner.run(side, record.sides[side].sha, policy)));
    evidence = { ...evidence, checks: { a: left!, b: right! } };
  }
  if (policy.steps.includes("hard-checks")) {
    const results = evidence.checks === null ? null : { a: evidence.checks.a.checks, b: evidence.checks.b.checks };
    const hard = decideHardChecks(results, policy.contracts.enabled ? { a: a.breaks, b: b.breaks } : null);
    steps.push(hard);
    if (hard.winner !== null) return decided(hard, steps, evidence);
  }

  if (policy.steps.includes("criteria")) {
    const measures = Object.fromEntries(SIDES.map(side => {
      const { changes, breaks } = sides[side];
      return [side, { callersAffected: callersAffected(breaks), sourceLines: changes!.sourceLines, sourceFiles: changes!.sourceFiles, benchmarks: evidence.checks?.[side].benchmarks ?? {} }];
    })) as Record<SideId, SideMeasures>;
    evidence = { ...evidence, measures };
    const criteria = decideCriteria(policy.criteria, measures);
    steps.push(criteria);
    if (criteria.winner !== null) return decided(criteria, steps, evidence);
  }

  const reserved = reservedHits(policy.reserve, { a: a.reserved, b: b.reserved });
  evidence = { ...evidence, reserved };
  if (reserved.length > 0) return escalation(`The conflict is in a class the policy reserves for a person: ${reserved.join(", ")}.`, steps, evidence);

  if (!policy.steps.includes("panel")) return escalation(`No step decided: ${steps.map(step => step.reason).join(" ")}`.trim(), steps, evidence);
  if (input.judge === null) return escalation("The panel is needed, and no judge is available.", steps, evidence);
  const docs = await baseDocs(root, record.base, {
    files: [...new Set([...record.sides.a.anchors.files, ...record.sides.b.anchors.files])],
    symbols: [...new Set([...record.sides.a.anchors.symbols, ...record.sides.b.anchors.symbols])],
  }, PANEL_DOC_CHARS);
  const designs = Object.fromEntries(SIDES.map(side => [side, {
    intent: sanitizeIntent(record.sides[side].intent),
    diff: stripCrossReferences(sideDiff(root, record.sides[side].sha, sides[side].changes!, PANEL_DIFF_CHARS)),
  }])) as Record<SideId, { intent: string; diff: string }>;
  const panel = await runPanel(input.judge, {
    repositoryKey: input.repositoryKey,
    record,
    docs,
    designs,
    judges: policy.panel.judges,
    agreement: policy.panel.agreement,
    seed: input.seed ?? 0,
  });
  evidence = { ...evidence, panel };
  steps.push({ step: panel.step, winner: panel.winner, reason: panel.reason });
  if (panel.winner !== null) return decided(panel, steps, evidence);
  return escalation(`No step decided. ${panel.reason}`, steps, evidence);
}

function decided(result: StepResult, steps: StepResult[], evidence: ResolutionEvidence): Resolution {
  return { verdict: result.winner!, step: result.step, reason: result.reason, steps, evidence };
}

/**
 * The ledger entry for a resolution: the record's question and sides with their positions, the verdict, the deciding
 * step (precedent names the entry it followed), and the losing side's decision to follow the winner's design. An
 * escalation is no entry: the conflict stays open for a person.
 */
export function decisionEntryFor(
  record: ConflictRecord,
  resolution: Resolution,
  positions: Readonly<Record<SideId, { readonly fingerprint: string; readonly at: Position }>>,
  recordedAt: Date = new Date(),
): DecisionEntry | null {
  if (resolution.verdict === "escalated") return null;
  const winner = resolution.verdict;
  const precedent = resolution.evidence.precedent?.entry ?? null;
  const decidedBy = resolution.step === "precedent" && precedent !== null
    ? { kind: "precedent" as const, entry: precedent.id }
    : resolution.step === "hard-checks" || resolution.step === "criteria" || resolution.step === "panel" ? { kind: "policy" as const, step: resolution.step } : null;
  if (decidedBy === null) return null;
  const side = (id: SideId) => ({ ref: record.sides[id].ref, label: refLabel(record.sides[id].ref), sha: record.sides[id].sha, author: null, ...positions[id] });
  const intent = firstLine(record.sides[winner].intent);
  return sealDecisionEntry({
    format: DECISION_ENTRY_FORMAT,
    question: record.question,
    title: (intent || record.question).slice(0, 300),
    base: record.base,
    sides: { a: side("a"), b: side("b") },
    verdict: winner,
    decidedBy,
    result: {
      candidateId: null,
      kind: "decision",
      text: `Follow the design of ${record.sides[winner].ref} (${record.sides[winner].sha.slice(0, 12)}): ${intent || "its change"}.`.slice(0, MAX_RESULT_TEXT),
      rule: null,
      follows: null,
    },
    supersedes: null,
    recordedAt: recordedAt.toISOString(),
  });
}

function firstLine(text: string): string {
  return sanitizeIntent(text).split("\n").find(line => line.trim() !== "")?.replace(/^#+\s*/u, "").trim().slice(0, 300) ?? "";
}

// ------------------------------------------------------------------------------------------------- local checks

/** The end of a check's output kept with its result. */
export const CHECK_LOG_CHARS = 4_000;
const KILL_GRACE_MS = 5_000;

interface CommandResult {
  readonly status: "exited" | "timeout" | "error";
  readonly code: number | null;
  readonly output: string;
}

/** `sh -c command` in `cwd`, its process group stopped at `timeoutMs`; the end of its combined output. */
function runShell(command: string, cwd: string, env: NodeJS.ProcessEnv, timeoutMs: number): Promise<CommandResult> {
  const { promise, resolve } = Promise.withResolvers<CommandResult>();
  let output = "";
  let timedOut = false;
  const child = spawn("/bin/sh", ["-c", command], { cwd, env, detached: true, stdio: ["ignore", "pipe", "pipe"] });
  const keep = (chunk: Buffer) => {
    output = (output + chunk.toString("utf8")).slice(-CHECK_LOG_CHARS);
  };
  const stop = (signal: NodeJS.Signals) => {
    try {
      process.kill(-child.pid!, signal);
    } catch {
      // already gone
    }
  };
  const timer = setTimeout(() => {
    timedOut = true;
    stop("SIGTERM");
    setTimeout(() => stop("SIGKILL"), KILL_GRACE_MS).unref();
  }, timeoutMs);
  child.stdout.on("data", keep);
  child.stderr.on("data", keep);
  child.on("error", error => {
    clearTimeout(timer);
    resolve({ status: "error", code: null, output: error.message });
  });
  child.on("close", code => {
    clearTimeout(timer);
    resolve({ status: timedOut ? "timeout" : "exited", code, output });
  });
  return promise;
}

/** The last number a benchmark printed, or null. */
export function lastNumber(output: string): number | null {
  const numbers = output.match(/-?\d+(?:\.\d+)?(?:e[+-]?\d+)?/giu);
  const last = numbers?.at(-1);
  return last === undefined ? null : Number(last);
}

/**
 * Runs checks on this machine: each side's tree exported with `git archive` into a fresh temporary folder (nothing in
 * the repository changes), the policy's setup, then each check and benchmark under the policy's timeout. A check that
 * exits 0 passes and one that exits otherwise fails; a timeout, a command that cannot start, or a failed setup is
 * unknown. The folder is removed afterwards.
 */
export function createLocalCheckRunner(options: { root: string; env?: NodeJS.ProcessEnv }): CheckRunner {
  const env = options.env ?? process.env;
  return {
    async run(_side, sha, policy) {
      const timeoutMs = policy.checks.timeoutMinutes * 60_000;
      const unknownAll = (detail: string, log: string): SideCheckOutcome => ({
        checks: policy.checks.run.map(check => ({ name: check.name, kind: check.kind, status: "unknown", detail, log })),
        benchmarks: Object.fromEntries(policy.criteria.benchmarks.map(benchmark => [benchmark.name, null])),
      });
      if (sha.startsWith("-") || !/^[0-9a-f]{40,64}$/u.test(sha)) return unknownAll(`not a commit: ${sha}`, "");
      const folder = await mkdtemp(join(tmpdir(), "rerail-checks-"));
      try {
        const exported = await runShell(`'${gitExecutable().replaceAll("'", "'\\''")}' -C '${options.root.replaceAll("'", "'\\''")}' archive --format=tar ${sha} | tar -x -f - -C .`, folder, env, timeoutMs);
        if (exported.status !== "exited" || exported.code !== 0) return unknownAll("the side's tree could not be exported", exported.output);
        if (policy.checks.setup !== null) {
          const setup = await runShell(policy.checks.setup, folder, env, timeoutMs);
          if (setup.status !== "exited" || setup.code !== 0) return unknownAll(setup.status === "timeout" ? "setup timed out" : `setup failed (exit ${setup.code ?? "none"})`, setup.output);
        }
        const checks: CheckResult[] = [];
        for (const check of policy.checks.run) {
          const result = await runShell(check.run, folder, env, timeoutMs);
          const status = result.status !== "exited" ? "unknown" : result.code === 0 ? "pass" : "fail";
          const detail = result.status === "timeout" ? `timed out after ${policy.checks.timeoutMinutes} minutes` : result.status === "error" ? "could not start" : `exit ${result.code ?? "none"}`;
          checks.push({ name: check.name, kind: check.kind, status, detail, log: result.output });
        }
        const benchmarks: Record<string, number | null> = {};
        for (const benchmark of policy.criteria.benchmarks) {
          const result = await runShell(benchmark.run, folder, env, timeoutMs);
          benchmarks[benchmark.name] = result.status === "exited" && result.code === 0 ? lastNumber(result.output) : null;
        }
        return { checks, benchmarks };
      } finally {
        await rm(folder, { recursive: true, force: true });
      }
    },
  };
}
