/**
 * A session's `.rerail` runs after its edits in one checkout. A write in the scope of one of the checkout's active
 * `.rerail` rules is gathered; the checkout's queue (`rr-run-queue.ts`) runs the rules that take the gathered files
 * over the whole checkout, on the files on disk, one run at a time. The edit's own check never waits for it: its
 * findings are recorded as an audit of their own and steer at the session's next boundary, as a late steer does.
 */
import type { AuditFailure, AuditMode, AuditRecorder, AuditResult, AuditScope } from "../audit/audit.js";
import type { CodeRuleEdit, CodeRuleViolation } from "../code-rule/code-rule.js";
import type { RerailRuleEntry } from "../code-rule/code-rule-dsl-catalog.js";
import { editViolations, type ReportBase } from "../code-rule/edit-findings/generated-lines.js";
import { deliverSteer, steerSourceOf, type SteerTarget } from "../code-rule/edit-findings/steer-publication.js";
import { leftRun, type RrClaim, type RrRunOwner, type RrRunQueue, type RrRunTiming } from "./rr-run-queue.js";
import type { RrRunOutcome } from "./rr-runner.js";
import { insideAny, takes } from "./rr-rule-scope.js";
import { unchangedRules } from "./rr-rule-files.js";
import { rrRunFailures, rrStoppedFailures, rrViolations } from "./rr-violations.js";

/**
 * The deadline of a run after an edit: 5 minutes. A steer later than that reaches an agent that has moved on, so a
 * longer run is not worth its cost. An edit runs a pack's rules over the whole checkout again, with no store kept
 * between runs: the 27 Django rules took 13 s after a one-file edit of a small application and 123 s (4.7 GB) of
 * netbox's, on 16 threads. A laptop gives a run 2 to 4 threads, so a netbox-sized application passes 5 minutes there,
 * and its edits record the rules' budget gaps rather than findings; a scan has the runner's longer default
 * (`rr-audit-run.ts`). `RERAIL_RR_DEADLINE_MS` replaces both.
 */
const EDIT_RUN_DEADLINE_MS = 5 * 60_000;

/**
 * How long a checkout goes without a write before a queued run after edits starts: 10 seconds, unless the agent's turn
 * ends first, which starts it at once. A run reads the whole checkout again (above: over a minute and gigabytes for a
 * small application), so it is not started on the first write of a burst. An agent's writes in one burst come one
 * model step apart, a few seconds; 10 s is the longest a boundary already waits for an edit's own check
 * (`NATIVE_EDIT_CHECK_MS`), so a burst is gathered whole and a lone write waits no longer than its own check could.
 * `RERAIL_RR_QUIET_MS` replaces it.
 */
const EDIT_RUN_QUIET_MS = 10_000;

/** The longest delay a Node timer keeps (2^31 - 1 ms, about 24.8 days); a longer one fires at once. */
const LONGEST_TIMER_MS = 2_147_483_647;

/**
 * When the runs after edits start and how long each may take; `RERAIL_RR_QUIET_MS` (milliseconds) replaces the quiet
 * period. Throws for a value that is no whole number from 1 to `LONGEST_TIMER_MS`.
 */
export function editRunTiming(environment: NodeJS.ProcessEnv = process.env): RrRunTiming {
  const text = environment.RERAIL_RR_QUIET_MS;
  if (text === undefined || text === "") return { deadlineMs: EDIT_RUN_DEADLINE_MS, quietMs: EDIT_RUN_QUIET_MS };
  const quietMs = Number(text);
  if (!Number.isSafeInteger(quietMs) || quietMs <= 0 || quietMs > LONGEST_TIMER_MS)
    throw new Error(`RERAIL_RR_QUIET_MS is ${JSON.stringify(text)}: it takes a whole number of milliseconds from 1 to ${LONGEST_TIMER_MS}`);
  return { deadlineMs: EDIT_RUN_DEADLINE_MS, quietMs };
}

/** A written file waiting for a run: its newest edit, the text its generated lines are measured from, and how it is audited. */
interface Gathered {
  readonly edit: CodeRuleEdit;
  /** The preimage of the oldest edit of the file since its last run, whose generated lines no run has reported. */
  readonly base: ReportBase;
  readonly revision: number;
  readonly scope: AuditScope;
  readonly mode: AuditMode;
}

/** A run that carries this session's gathered files. */
interface Flight {
  readonly files: readonly Gathered[];
  /** The rules it runs: those the files take whose own files are as the catalog read them. */
  readonly rules: readonly RerailRuleEntry[];
  /** A failure for each rule the files take that was left out because a file of it changed (`rr-rule-files.ts`). */
  readonly changed: readonly AuditFailure[];
  readonly auditId: string | undefined;
  /** Set when the rules changed while it ran: its files went back to be run with the new rules. */
  requeued: boolean;
}

/** What the session's checks of one checkout give its `.rerail` runs. */
export interface RerailEditRunsHost {
  readonly sessionId: string;
  readonly workspaceRoot: string;
  /** The checkout's active `.rerail` rules now. */
  readonly rules: () => readonly RerailRuleEntry[];
  /** The digest of the checkout's rules now, recorded with each audit. */
  readonly catalogRevision: () => string;
  readonly audits: AuditRecorder;
  readonly steerTarget: SteerTarget;
  /** Folders of the checkout that are checkouts of their own: findings there are theirs, not this checkout's. */
  readonly nestedCheckouts: () => readonly string[];
  readonly queue: RrRunQueue;
}

/** The highest revision of `files`. */
function newest(files: readonly Gathered[]): number {
  return Math.max(...files.map(file => file.revision));
}

export class RerailEditRuns implements RrRunOwner {
  readonly #host: RerailEditRunsHost;
  readonly #gathered = new Map<string, Gathered>();
  #flight: Flight | undefined;
  #closed = false;
  /** What the rules were when the running flight claimed them: a change aborts it. */
  #rulesKey: string;

  constructor(host: RerailEditRunsHost) {
    this.#host = host;
    this.#rulesKey = rulesKey(host.rules());
  }

  /** Whether one of the checkout's `.rerail` rules takes a write of `path` (relative to the checkout). */
  takes(path: string): boolean {
    return this.#host.rules().some(rule => takes(rule, path));
  }

  /**
   * Gathers the files of `edits` some rule takes for the next run and asks the queue for one; returns at once. A file
   * already waiting keeps the preimage of its older edit, so the run reports every line written since its last run.
   */
  gather(edits: readonly CodeRuleEdit[], revision: number, scope: AuditScope, mode: AuditMode): void {
    if (this.#closed) return;
    const taken = edits.filter(edit => this.takes(edit.path));
    for (const edit of taken) {
      const waiting = this.#gathered.get(edit.path);
      this.#gathered.set(edit.path, { edit, base: waiting?.base ?? edit, revision, scope, mode });
    }
    if (taken.length > 0) this.#host.queue.want(this);
  }

  /**
   * After the checkout's rules were read again: where its `.rerail` rules changed while a run of this session goes on,
   * that run is left (killed when no other session is in it) and its files wait for the next run, with the new rules.
   */
  rulesChanged(): void {
    const key = rulesKey(this.#host.rules());
    if (key === this.#rulesKey) return;
    this.#rulesKey = key;
    const flight = this.#flight;
    if (flight === undefined || this.#closed) return;
    flight.requeued = true;
    for (const file of flight.files) this.#regather(file);
    this.#host.queue.leave(this);
    this.#host.queue.want(this);
    // Its files had their quiet period: the run with the new rules starts without another.
    this.#host.queue.flush();
  }

  /** The agent's turn ended: a run waiting for quiet starts now. */
  turnEnded(): void {
    this.#host.queue.flush();
  }

  /** The session ends: nothing more runs for it, and a run it is in is left, its audit interrupted. */
  close(): void {
    this.#closed = true;
    this.#gathered.clear();
    this.#host.queue.leave(this);
  }

  claim(): RrClaim | undefined {
    const files = [...this.#gathered.values()];
    this.#gathered.clear();
    const taken = this.#host.rules().filter(rule => files.some(file => takes(rule, file.edit.path)));
    if (this.#closed || taken.length === 0) return undefined;
    this.#rulesKey = rulesKey(this.#host.rules());
    const { rules, failures } = unchangedRules(taken);
    const flight: Flight = { files, rules, changed: failures, auditId: this.#startAudit(files, taken), requeued: false };
    if (rules.length === 0) {
      this.#finish(flight, { status: "failed", violations: [], failures });
      return undefined;
    }
    this.#flight = flight;
    return { policies: rules.map(rule => rule.path), settle: outcome => this.#settle(flight, outcome) };
  }

  /** A file of a flight that was left goes back to wait, behind any newer edit of it, keeping the older preimage. */
  #regather(file: Gathered): void {
    const newer = this.#gathered.get(file.edit.path);
    this.#gathered.set(file.edit.path, newer === undefined ? file : { ...newer, base: file.base });
  }

  #settle(flight: Flight, outcome: RrRunOutcome): void {
    if (this.#flight === flight) this.#flight = undefined;
    if (leftRun(outcome)) {
      this.#finish(flight, { status: flight.requeued ? "superseded" : "interrupted" });
      return;
    }
    const result = this.#results(flight, outcome);
    this.#finish(flight, result);
    if (!this.#closed) void this.#steer(flight, result.violations);
  }

  /**
   * The violations each file's edit reports (`editViolations`; in repository scope, as in touched-files scope, its
   * file's), and the rules' failures.
   */
  #results(flight: Flight, outcome: RrRunOutcome): AuditResult & Pick<Required<AuditResult>, "violations" | "failures"> {
    if (outcome.status !== "completed")
      return { status: "failed", violations: [], failures: [...flight.changed, ...rrStoppedFailures(outcome, flight.rules, this.#host.queue.deadlineMs)] };
    const nested = this.#host.nestedCheckouts();
    const found = rrViolations(outcome.findings, flight.rules, path => !insideAny(nested, path));
    const violations = flight.files.flatMap(file =>
      editViolations(file.scope === "repository" ? "touched-files" : file.scope, file.edit, found, file.base));
    const failures = [...flight.changed, ...rrRunFailures(outcome.failures, flight.rules)];
    return { status: failures.length > 0 ? "failed" : "completed", violations, failures };
  }

  /** The steering findings of the flight's files whose audit steers, as one background steer at their newest revision. */
  async #steer(flight: Flight, violations: readonly CodeRuleViolation[]): Promise<void> {
    const steering = flight.files.filter(file => file.mode === "steer");
    const paths = new Set(steering.map(file => file.edit.path));
    const own = violations.filter(violation => violation.path !== undefined && paths.has(violation.path));
    if (own.length === 0) return;
    const sourceOf = steerSourceOf(flight.rules.map(rule => rule.provenance));
    const request = { revision: newest(steering), sourceScope: [...paths], violations: own, trigger: { kind: "edit" } as const, sourceOf, generatedEdit: true };
    await deliverSteer(this.#host.steerTarget, request).catch((error: unknown) => {
      process.stderr.write(`rerail: a .rerail steer was not delivered: ${error instanceof Error ? error.message : String(error)}\n`);
    });
  }

  /** The audit of a flight, started as it claims its files: the rules it carries and the files it was run for. */
  #startAudit(files: readonly Gathered[], rules: readonly RerailRuleEntry[]): string | undefined {
    const last = files.at(-1)!;
    return this.#record(() => this.#host.audits.start({
      sessionId: this.#host.sessionId,
      workspaceRoot: this.#host.workspaceRoot,
      scope: last.scope,
      mode: files.some(file => file.mode === "steer") ? "steer" : "log",
      revision: newest(files),
      catalogRevision: this.#host.catalogRevision(),
      ruleRevisions: rules.map(rule => ({ ruleId: rule.id, revision: rule.provenance.revision })),
      paths: files.map(file => file.edit.path),
      ruleProvenance: rules.map(rule => rule.provenance),
    }));
  }

  #finish(flight: Flight, result: AuditResult): void {
    if (flight.auditId !== undefined) this.#record(() => this.#host.audits.finish(flight.auditId!, result));
  }

  /** Records through the audit store; a store that fails (closed with the daemon) costs the record, never the session. */
  #record<T>(operation: () => T): T | undefined {
    try {
      return operation();
    } catch (error) {
      process.stderr.write(`rerail: a .rerail run was not recorded: ${error instanceof Error ? error.message : String(error)}\n`);
      return undefined;
    }
  }
}

/** What identifies a set of rules for a run: each rule's file, content and placement. */
function rulesKey(rules: readonly RerailRuleEntry[]): string {
  return JSON.stringify(rules.map(rule => [rule.id, rule.path, rule.provenance.payloadHash, rule.placement]));
}
