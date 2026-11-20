/**
 * The `.rerail` runs of one session's checks of one checkout: the runs after its edits (`rr-edit-runs.ts`), and the
 * `.rerail` part of each audit it was asked for (`rr-audit-run.ts`). The checkout's code-rule enforcer holds one, and
 * tells it of edits, audits, rule changes and its end.
 */
import type { AuditMode, AuditRecorder, AuditRuleProvenance, AuditScope } from "../audit/audit.js";
import type { CodeRuleEdit } from "../code-rule/code-rule.js";
import type { CodeRuleDslCatalog, RerailRuleEntry } from "../code-rule/code-rule-dsl-catalog.js";
import type { SteerTarget } from "../code-rule/steer-publication.js";
import { RrAuditRun, type RrAuditResults } from "./rr-audit-run.js";
import { EDIT_RUN_DEADLINE_MS, RerailEditRuns } from "./rr-edit-runs.js";
import { rrRunQueue } from "./rr-run-queue.js";

export interface CheckoutRerailHost {
  readonly sessionId: string;
  readonly workspaceRoot: string;
  readonly catalog: CodeRuleDslCatalog;
  readonly audits: AuditRecorder;
  readonly steerTarget: SteerTarget;
  /** Folders of the checkout that are checkouts of their own, judged in their own lanes. */
  readonly nestedCheckouts: () => readonly string[];
  /** An audit's budget, which its `.rerail` run has too. */
  readonly auditDeadlineMs: number;
}

/** An audit's results as its native evaluation gave them: what the `.rerail` run's are added to. */
export interface StreamedAuditResults extends RrAuditResults {
  /** Whether the results went to the audit's pages (`AuditRecorder.appendResultPage`) rather than `violations`. */
  readonly paged: boolean;
  readonly pageCount: number;
  readonly violationCount: number;
}

export class CheckoutRerailRuns {
  readonly #host: CheckoutRerailHost;
  readonly #edits: RerailEditRuns;
  readonly #audits = new Map<string, RrAuditRun>();

  constructor(host: CheckoutRerailHost) {
    this.#host = host;
    const rules = (): readonly RerailRuleEntry[] => host.catalog.snapshot().rerailRules;
    this.#edits = new RerailEditRuns({
      ...host,
      rules,
      catalogRevision: () => host.catalog.status.version,
      queue: rrRunQueue(host.workspaceRoot, EDIT_RUN_DEADLINE_MS),
    });
  }

  /** Whether one of the checkout's `.rerail` rules takes a write of `path` (relative to the checkout). */
  takes(path: string): boolean {
    return this.#edits.takes(path);
  }

  /** Gathers the edits of a revision for the next run after edits; returns at once. */
  gather(edits: readonly CodeRuleEdit[], revision: number, scope: AuditScope, mode: AuditMode): void {
    this.#edits.gather(edits, revision, scope, mode);
  }

  /** The checkout's rules were read again: a run after edits that carries rules since changed starts again. */
  rulesChanged(): void {
    this.#edits.rulesChanged();
  }

  /**
   * Starts an audit with `start`, given the provenance of the `.rerail` rules it also runs, and starts their run beside
   * it: over `paths` (relative to the checkout) for a touched-files audit, over the whole checkout for a repository one.
   */
  beginAudit(
    scope: "repository" | "touched-files",
    paths: readonly string[],
    start: (provenance: readonly AuditRuleProvenance[]) => string | undefined,
  ): string | undefined {
    const run = new RrAuditRun({
      root: this.#host.workspaceRoot,
      rules: this.#host.catalog.snapshot().rerailRules,
      paths: scope === "repository" ? undefined : paths,
      nestedCheckouts: this.#host.nestedCheckouts(),
      deadlineMs: this.#host.auditDeadlineMs,
    });
    const auditId = start(run.provenance);
    if (auditId === undefined || run.empty) return auditId;
    this.#audits.set(auditId, run);
    run.start();
    return auditId;
  }

  /**
   * The audit's results with its `.rerail` run's, once that run has ended: a paged audit gets them as one more page
   * (`appendPage`), any other in its `violations`; both count their failures.
   */
  async withAuditResults<T extends StreamedAuditResults>(
    auditId: string | undefined,
    streamed: T,
    appendPage: (pageIndex: number, page: RrAuditResults) => void,
  ): Promise<T> {
    const run = auditId === undefined ? undefined : this.#audits.get(auditId);
    if (run === undefined) return streamed;
    this.#audits.delete(auditId!);
    const added = await run.results();
    const failures = [...streamed.failures, ...added.failures];
    const violationCount = streamed.violationCount + added.violations.length;
    if (!streamed.paged) return { ...streamed, violations: [...streamed.violations, ...added.violations], failures, violationCount };
    appendPage(streamed.pageCount, added);
    return { ...streamed, failures, violationCount, pageCount: streamed.pageCount + 1 };
  }

  /** The audit ended without its results (it failed): its `.rerail` run is killed. */
  abandonAudit(auditId: string | undefined): void {
    if (auditId === undefined) return;
    this.#audits.get(auditId)?.abort();
    this.#audits.delete(auditId);
  }

  /** The session's checks end: every run of theirs is left or killed, and no audit of theirs is left running. */
  close(): void {
    this.#edits.close();
    for (const run of this.#audits.values()) run.abort();
    this.#audits.clear();
  }
}
