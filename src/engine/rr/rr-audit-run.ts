/**
 * The `.rerail` part of a repository or touched-files audit: one run of the checkout's active `.rerail` rules over the
 * whole checkout, beside the native evaluation, whose findings and failures join the audit's own record. It has the
 * runner's default deadline (`rr-budgets.ts`), not the edit runs' shorter one: no agent waits for a scan, and a
 * large repository's run takes minutes (the Django rules: 251 s over edx-platform on 16 threads). Past a budget, only
 * its rules fail.
 */
import type { AuditFailure, RepositoryAuditRuleProvenance } from "../audit/audit.js";
import { folderContains } from "../catalog/repository-model.js";
import type { CodeRuleViolation } from "../code-rule/code-rule.js";
import type { RerailRuleEntry } from "../code-rule/code-rule-dsl-catalog.js";
import type { RrRun, RrRunOutcome } from "./rr-runner.js";
import { leftRun } from "./rr-run-queue.js";
import { runInTurn } from "./rr-run-slots.js";
import { unchangedRules } from "./rr-rule-files.js";
import { insideAny } from "./rr-rule-scope.js";
import { rrRunFailures, rrStoppedFailures, rrViolations } from "./rr-violations.js";

/** What an audit's `.rerail` run adds to the audit. */
export interface RrAuditResults {
  readonly violations: CodeRuleViolation[];
  readonly failures: AuditFailure[];
}

/** What an audit's `.rerail` run is of. */
export interface RrAuditRequest {
  readonly root: string;
  /** The checkout's active `.rerail` rules. */
  readonly rules: readonly RerailRuleEntry[];
  /** The files and folders a touched-files audit was given (relative to the checkout); undefined for the whole checkout. */
  readonly paths: readonly string[] | undefined;
  /** Folders of the checkout that are checkouts of their own, audited in their own lanes. */
  readonly nestedCheckouts: readonly string[];
}

/** Whether `rule` is on in some folder of the checkout. */
function isOn(rule: RerailRuleEntry): boolean {
  return rule.placement.some(entry => entry.mode !== null);
}

/** The outcome of a run with no rule left to run: nothing found, no child started. */
const NOTHING_RUN: RrRunOutcome = { status: "completed", findings: [], failures: [], files: 0, wallMs: 0, peakRssBytes: 0 };

export class RrAuditRun {
  readonly #request: RrAuditRequest;
  /** The rules on somewhere in the checkout: what the audit carries. */
  readonly #placed: readonly RerailRuleEntry[];
  /** Of those, the rules whose files are as the catalog read them: what the child runs. */
  readonly #rules: readonly RerailRuleEntry[];
  /** A failure for each rule left out because a file of it changed (`rr-rule-files.ts`). */
  readonly #changed: readonly AuditFailure[];
  readonly #run: (run: RrRun) => Promise<RrRunOutcome>;
  readonly #controller = new AbortController();
  #outcome: Promise<RrRunOutcome> | undefined;

  constructor(request: RrAuditRequest, run: (run: RrRun) => Promise<RrRunOutcome> = runInTurn) {
    this.#request = request;
    this.#placed = request.rules.filter(isOn);
    const unchanged = unchangedRules(this.#placed);
    this.#rules = unchanged.rules;
    this.#changed = unchanged.failures;
    this.#run = run;
  }

  /** Whether the audit carries no `.rerail` rule. */
  get empty(): boolean {
    return this.#placed.length === 0;
  }

  /** The provenance of the rules the audit carries, recorded with it. */
  get provenance(): RepositoryAuditRuleProvenance[] {
    return this.#placed.map(rule => rule.provenance);
  }

  /** Starts the run, once only; with no rule left to run, no child starts. */
  start(): void {
    if (this.#rules.length === 0) this.#outcome ??= Promise.resolve(NOTHING_RUN);
    this.#outcome ??= this.#run({ root: this.#request.root, policies: this.#rules.map(rule => rule.path), signal: this.#controller.signal })
      .catch((error: unknown): RrRunOutcome => ({ status: "failed", message: String(error) }));
  }

  /** Kills the run: the audit it belongs to ended without it. */
  abort(): void {
    this.#controller.abort();
  }

  /**
   * The run's violations in the audited files and its rules' failures (those left out for a changed file first), once
   * it has ended. Throws for a run stopped because its session closed: it has no results, and its audit ends
   * interrupted, not completed.
   */
  async results(): Promise<RrAuditResults> {
    this.start();
    const outcome = await this.#outcome!;
    if (leftRun(outcome)) throw new Error("The .rerail run was stopped: its session closed");
    if (outcome.status !== "completed") return { violations: [], failures: [...this.#changed, ...rrStoppedFailures(outcome, this.#rules, undefined)] };
    const violations = rrViolations(outcome.findings, this.#rules, path => this.#audits(path));
    return { violations, failures: [...this.#changed, ...rrRunFailures(outcome.failures, this.#rules)] };
  }

  /** Whether the audit covers `path`: in a folder it was given, and in no checkout nested in this one. */
  #audits(path: string): boolean {
    const { paths, nestedCheckouts } = this.#request;
    return !insideAny(nestedCheckouts, path) && (paths === undefined || paths.some(folder => folderContains(folder, path)));
  }
}
