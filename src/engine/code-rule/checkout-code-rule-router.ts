/**
 * A session's code-rule audit across checkouts: each file goes to the lane of the checkout that owns it,
 * one native enforcer per checkout, judged by that checkout's rules. Only checkouts that are recorded, started and
 * set up get a lane; files of any other checkout are not audited.
 *
 * A nested checkout owns its files only when it is a submodule the checkout around it declares (a gitlink), or a
 * recorded checkout that may run rules. Any other nested repository, such as an untracked `git init` or a stray
 * `.git` entry, is judged with the checkout around it, so a `.git` never exempts files from that checkout's rules. A
 * declared submodule that is not set up runs nothing.
 */
import { dirname, join, relative, resolve, sep } from "node:path";
import type { CheckoutOwner, CheckoutOwners } from "../catalog/checkout-owner.js";
import { languageFor, NativeCodeRuleEnforcer } from "./code-rule-enforcer.js";
import type {
  AuditorCloseOptions,
  CodeRuleEdit,
  CodeRuleEditBatch,
  CodeRuleAuditor,
  CodeRuleResponse,
  AuditSettings,
  RuleSteerDelivery,
  RepositoryAuditHooks,
} from "./code-rule.js";
import type { CheckoutCatalogs, CheckoutEligibility } from "./code-rule-dsl-catalog.js";
import type { AuditRecorder, AuditMode } from "../audit/audit.js";

export interface CheckoutCodeRuleRouterOptions {
  /** The session's folder, physical; edit and audit paths are relative to it. */
  readonly sessionRoot: string;
  /** The physical root of the checkout the session started in; its lane opens with the session. */
  readonly startingCheckout: string;
  readonly owners: CheckoutOwners;
  readonly catalogs: CheckoutCatalogs;
  readonly steerDelivery: RuleSteerDelivery;
  readonly audits: AuditRecorder;
  readonly scope?: Required<AuditSettings>["scope"];
  readonly mode?: AuditMode;
  /** Where each lane's enforcer writes its timing lines; none are written without it. */
  readonly timingLog?: (line: string) => void;
}

interface Lane {
  readonly root: string;
  readonly enforcer: NativeCodeRuleEnforcer;
  readonly opening: Promise<void>;
}

function merged(revision: number, responses: readonly CodeRuleResponse[]): CodeRuleResponse {
  return {
    revision,
    violations: responses.flatMap(response => response.violations),
    failures: responses.flatMap(response => response.failures),
    disposition: responses.some(response => response.disposition === "steer") ? "steer" : "continue",
  };
}

export class CheckoutCodeRuleRouter implements CodeRuleAuditor {
  readonly #options: CheckoutCodeRuleRouterOptions;
  readonly #sessionRoot: string;
  readonly #lanes = new Map<string, Lane>();
  readonly #eligibility = new Map<string, CheckoutEligibility>();
  #settings: Required<AuditSettings>;
  #sessionId: string | undefined;
  #closing = false;

  constructor(options: CheckoutCodeRuleRouterOptions) {
    this.#options = options;
    this.#sessionRoot = resolve(options.sessionRoot);
    this.#settings = { scope: options.scope ?? "touched-files", mode: options.mode ?? "steer" };
  }

  /** Interrupts the session's earlier runs once, then opens the starting checkout's lane. */
  async openWorkspace(sessionId: string): Promise<void> {
    if (this.#sessionId !== undefined || this.#closing) throw new Error("Checkout code-rule router cannot be reopened");
    this.#sessionId = sessionId;
    try {
      this.#options.audits.interrupt(sessionId);
    } catch (error) {
      process.stderr.write(`rerail: policy execution recording failed: ${error instanceof Error ? error.message : String(error)}\n`);
    }
    await this.#lane(this.#options.startingCheckout)?.opening;
  }

  /**
   * Re-read every known checkout's eligibility. A lane whose checkout may no longer run rules (stopped, not set up, or
   * the session's own checkout stopped) is closed, so its last result stops reaching later responses; it opens again
   * if the checkout becomes eligible.
   */
  refreshCodeRules(): void {
    if (this.#closing) return;
    for (const root of this.#eligibility.keys()) this.#eligibility.set(root, this.#options.catalogs.refresh(root));
    for (const lane of [...this.#lanes.values()]) {
      if (this.#mayRun(lane.root)) {
        lane.enforcer.refreshCodeRules();
        continue;
      }
      this.#lanes.delete(lane.root);
      lane.enforcer.interrupt();
      void lane.opening.catch(() => undefined).then(() => lane.enforcer.close()).catch(() => undefined);
    }
  }

  updateSettings(settings: Required<AuditSettings>): void {
    this.#settings = settings;
    for (const lane of this.#lanes.values()) lane.enforcer.updateSettings(settings);
  }

  acceptsPath(path: string): boolean {
    if (languageFor(path) === undefined) return false;
    const routed = this.#route(path);
    return routed !== undefined && routed.lane.enforcer.acceptsPath(routed.path);
  }

  sourceRoot(path: string): string | undefined {
    return this.#owner(resolve(this.#sessionRoot, path))?.checkoutRoot;
  }

  async auditGeneratedEdits(sessionId: string, batch: CodeRuleEditBatch): Promise<CodeRuleResponse> {
    this.#assertSession(sessionId);
    const partitioned = new Map<Lane, CodeRuleEdit[]>();
    for (const edit of batch.edits) {
      const routed = this.#route(edit.path);
      if (routed === undefined) continue;
      partitioned.set(routed.lane, [...(partitioned.get(routed.lane) ?? []), { ...edit, path: routed.path }]);
    }
    const responses: CodeRuleResponse[] = [];
    for (const lane of [...this.#lanes.values()]) {
      if (!this.#mayRun(lane.root)) continue;
      const edits = partitioned.get(lane);
      if (edits === undefined) {
        const { pending: _pending, ...status } = lane.enforcer.snapshotStatus();
        responses.push(status);
        continue;
      }
      await lane.opening;
      responses.push(await lane.enforcer.auditGeneratedEdits(sessionId, { ...batch, edits }));
    }
    return merged(batch.revision, responses);
  }

  async settleGeneratedEdits(): Promise<void> {
    await this.#settleLanes(lane => lane.enforcer.settleGeneratedEdits());
  }

  async settle(): Promise<void> {
    await this.#settleLanes(lane => lane.enforcer.settle());
  }

  async audit(
    sessionId: string,
    input: { scope: "repository" | "touched-files"; mode: AuditMode; paths?: string[]; audit?: RepositoryAuditHooks },
  ): Promise<readonly string[]> {
    this.#assertSession(sessionId);
    if (input.scope === "repository") {
      if ((input.paths ?? []).length > 0) throw new Error("Repository scope always covers the full repository");
      const auditIds: string[] = [];
      await this.#audit(sessionId, this.#options.startingCheckout, input.mode, input.audit, auditIds, new Set());
      return auditIds;
    }
    const partitioned = new Map<Lane, string[]>();
    for (const path of input.paths ?? []) {
      const routed = this.#route(path);
      if (routed !== undefined) partitioned.set(routed.lane, [...(partitioned.get(routed.lane) ?? []), routed.path]);
    }
    const auditIds: string[] = [];
    const visited = new Set<string>();
    for (const [lane, paths] of partitioned) {
      await lane.opening;
      auditIds.push(...await lane.enforcer.audit(sessionId, { scope: "touched-files", mode: input.mode, paths }));
      // The lane's walk skips the checkouts nested in a folder it was given; each is audited whole, in its own lane.
      for (const nested of this.#nestedCheckoutsOf(lane.root)) {
        if (!paths.some(path => path === "" || nested === path || nested.startsWith(`${path}/`))) continue;
        await this.#audit(sessionId, join(lane.root, nested), input.mode, undefined, auditIds, visited);
      }
    }
    return auditIds;
  }

  interrupt(options?: AuditorCloseOptions): void {
    if (this.#closing) return;
    this.#closing = true;
    for (const lane of this.#lanes.values()) lane.enforcer.interrupt(options);
  }

  async close(options?: AuditorCloseOptions): Promise<void> {
    this.interrupt(options);
    const lanes = [...this.#lanes.values()];
    this.#lanes.clear();
    await Promise.all(lanes.map(async lane => {
      await lane.opening.catch(() => undefined);
      await lane.enforcer.close(options);
    }));
  }

  /** Each checkout's run is claimed and reported on its own; nested checkouts the walk skipped are audited next. */
  async #audit(
    sessionId: string,
    root: string,
    mode: AuditMode,
    audit: RepositoryAuditHooks | undefined,
    auditIds: string[],
    visited: Set<string>,
  ): Promise<void> {
    if (visited.has(root) || this.#closing) return;
    visited.add(root);
    const lane = this.#lane(root);
    if (lane === undefined) return;
    await lane.opening;
    if (audit !== undefined && !audit.claim(root, lane.enforcer.catalogRevision())) return;
    const report = await lane.enforcer.auditRepository(sessionId, mode);
    audit?.finish(root, report.auditId);
    if (report.auditId !== undefined) auditIds.push(report.auditId);
    for (const nested of report.nestedCheckouts) {
      const owner = this.#owner(join(root, nested));
      if (owner !== undefined && owner.path === "" && owner.checkoutRoot !== root) {
        await this.#audit(sessionId, owner.checkoutRoot, mode, audit, auditIds, visited);
      }
    }
  }

  async #settleLanes(settle: (lane: Lane) => Promise<void>): Promise<void> {
    let settled = 0;
    while (settled !== this.#lanes.size) {
      const lanes = [...this.#lanes.values()];
      settled = lanes.length;
      await Promise.all(lanes.map(async lane => {
        await lane.opening;
        await settle(lane);
      }));
    }
  }

  #route(path: string): { lane: Lane; path: string } | undefined {
    const owner = this.#owner(resolve(this.#sessionRoot, path));
    if (owner === undefined) return undefined;
    const lane = this.#lane(owner.checkoutRoot);
    return lane === undefined ? undefined : { lane, path: owner.path };
  }

  /** The checkout whose rules judge `absolute`: its Git checkout, or the nearest one around it that owns its files. */
  #owner(absolute: string): CheckoutOwner | undefined {
    const owners = this.#options.owners;
    let owner = owners.ownerOf(absolute);
    while (owner !== undefined && !this.#ownsItsFiles(owner.checkoutRoot)) {
      const enclosing = owners.ownerOf(dirname(owner.checkoutRoot));
      if (enclosing === undefined) break;
      const real = join(owner.checkoutRoot, owner.path);
      owner = { checkoutRoot: enclosing.checkoutRoot, path: relative(enclosing.checkoutRoot, real).split(sep).join("/") };
    }
    return owner;
  }

  /**
   * A checkout inside another owns its files when Rerail recorded it or the outer one declares it as a submodule; a
   * stopped or not set up one then runs nothing. Only a Git folder Rerail never recorded (a stray `.git`, a `git init`
   * nobody started) is judged by the checkout around it.
   */
  #ownsItsFiles(root: string): boolean {
    return this.#eligibilityOf(root) !== "unrecorded" || this.#options.owners.isDeclaredSubmodule(root);
  }

  /** The checkouts inside `root` that own their files, relative to it: its walks skip them and audit them apart. */
  #nestedCheckoutsOf(root: string): string[] {
    const candidates = new Set([...this.#options.owners.declaredSubmodules(root), ...this.#options.catalogs.recordedCheckoutsWithin(root)]);
    return [...candidates]
      .filter(candidate => {
        const owner = this.#owner(candidate);
        return owner?.checkoutRoot === candidate && owner.path === "" && this.#owner(dirname(candidate))?.checkoutRoot === root;
      })
      .map(candidate => relative(root, candidate).split(sep).join("/"))
      .sort();
  }

  /**
   * The lane of an eligible checkout, opened on first use; undefined while the checkout may not run rules, and for
   * every checkout while the session's own checkout is stopped (the gate stays on the starting checkout).
   */
  #lane(root: string): Lane | undefined {
    const sessionId = this.#sessionId;
    if (sessionId === undefined || this.#closing || !this.#mayRun(root)) return undefined;
    let lane = this.#lanes.get(root);
    if (lane === undefined) {
      const enforcer = new NativeCodeRuleEnforcer(this.#options.catalogs.catalog(root), {
        workspaceRoot: root,
        steerDelivery: this.#options.steerDelivery,
        audits: this.#options.audits,
        ...this.#settings,
        ...(this.#options.timingLog === undefined ? {} : { timingLog: this.#options.timingLog }),
        presentPath: path => relative(this.#sessionRoot, join(root, path)).split(sep).join("/"),
        nestedCheckouts: () => this.#nestedCheckoutsOf(root),
      });
      // The enforcer registers its code rules synchronously, so the new lane answers `acceptsPath` at once.
      lane = { root, enforcer, opening: enforcer.openWorkspace(sessionId) };
      lane.opening.catch(() => undefined);
      this.#lanes.set(root, lane);
    }
    return lane;
  }

  /** Whether `root` may run rules in this session: it is eligible, and the session's own checkout is not stopped. */
  #mayRun(root: string): boolean {
    const starting = this.#eligibilityOf(this.#options.startingCheckout);
    return starting !== "stopped" && starting !== "unrecorded" && this.#eligibilityOf(root) === "ok";
  }

  #eligibilityOf(root: string): CheckoutEligibility {
    let eligibility = this.#eligibility.get(root);
    if (eligibility === undefined) {
      eligibility = this.#options.catalogs.refresh(root);
      this.#eligibility.set(root, eligibility);
    }
    return eligibility;
  }

  #assertSession(sessionId: string): void {
    if (this.#sessionId !== sessionId || this.#closing) throw new Error("Checkout code-rule router session mismatch");
  }
}
