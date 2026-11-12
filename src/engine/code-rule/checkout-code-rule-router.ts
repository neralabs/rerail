/**
 * A session's code-rule audit across checkouts: each file goes to the lane of the checkout that owns it,
 * one native enforcer per checkout, judged by that checkout's rules. Only checkouts that are recorded, started and
 * set up get a lane; files of any other checkout are not audited. A worktree Git registered for a started clone Rerail
 * knows is recorded when a path in it is first looked up (`recordStartedWorktree`), wherever it sits.
 *
 * A nested checkout owns its files only when it is a submodule the checkout around it declares (a gitlink), or a
 * recorded checkout that may run rules. Any other nested repository, such as an untracked `git init` or a stray
 * `.git` entry, is judged with the checkout around it, so a `.git` never exempts files from that checkout's rules. A
 * declared submodule that is not set up runs nothing.
 *
 * A lane opens for the starting checkout and when an audit needs one. Which files a checkout reads or ignores is a
 * question a walk asks of every worktree, so it opens no lane: a checkout with none answers from its catalog's code rule
 * scopes (`CodeRuleScopes`) and its Git exclude stack, as its lane would.
 */
import { existsSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { storedPath } from "../catalog/checkout-control.js";
import type { CheckoutOwner, CheckoutOwners } from "../catalog/checkout-owner.js";
import { worktreeRegistry } from "../catalog/repository-identity.js";
import { loadNativeBinding, type NativeGitIgnoreMatcher } from "../native/native.js";
import { NativeCodeRuleEnforcer } from "./code-rule-enforcer.js";
import { CodeRuleScopes } from "./code-rule-scopes.js";
import type {
  AuditorCloseOptions,
  CodeRuleEdit,
  CodeRuleEditBatch,
  CodeRuleAuditor,
  CodeRuleResponse,
  AuditSettings,
  RuleSteerDelivery,
  RepositoryAuditHooks,
  IgnoredEditMatch,
} from "./code-rule.js";
import type { CheckoutCatalogs, CheckoutEligibility, CodeRuleDslCatalog } from "./code-rule-dsl-catalog.js";
import type { AuditRecorder, AuditMode } from "../audit/audit.js";
import { DEFAULT_AUDIT_SETTINGS } from "../audit/audit-control-schema.js";

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
  /** Every lane leaves out what Git ignores in its checkout (`NativeCodeRuleEnforcer`'s option); off by default. */
  readonly followGitignore?: boolean;
}

interface Lane {
  readonly root: string;
  readonly enforcer: NativeCodeRuleEnforcer;
  readonly opening: Promise<void>;
}

/**
 * What a checkout with no lane answers from: its catalog's scopes, as read when `refreshCodeRules` had run `refreshed`
 * times, and why its catalog could not be read then, reported once while it lasts.
 */
interface UnlanedCheckout {
  readonly scopes: CodeRuleScopes;
  readonly refreshed: number;
  readonly failure?: string;
}

/**
 * Reads `catalog` again; why it could not, where it could not. Its rules as last read stand then, as a lane's enforcer
 * keeps its rules when they cannot be read (`refreshCodeRules`), so a walk's questions still get answers.
 */
function readAgain(catalog: CodeRuleDslCatalog): string | undefined {
  try {
    catalog.refresh();
    return undefined;
  } catch (error) {
    return error instanceof Error ? error.message : String(error);
  }
}

function merged(revision: number, responses: readonly CodeRuleResponse[]): CodeRuleResponse {
  return {
    revision,
    violations: responses.flatMap(response => response.violations),
    failures: responses.flatMap(response => response.failures),
    disposition: responses.some(response => response.disposition === "steer") ? "steer" : "continue",
  };
}

/** Whether `folder`, relative to a checkout ("" for all of it), holds `path` or is it. */
function folderHolds(folder: string, path: string): boolean {
  return folder === "" || path === folder || path.startsWith(`${folder}/`);
}

export class CheckoutCodeRuleRouter implements CodeRuleAuditor {
  readonly #options: CheckoutCodeRuleRouterOptions;
  readonly #sessionRoot: string;
  readonly #lanes = new Map<string, Lane>();
  readonly #eligibility = new Map<string, CheckoutEligibility>();
  /** The checkouts a path was asked of that have no lane, by root. */
  readonly #unlaned = new Map<string, UnlanedCheckout>();
  /** Git's exclude stack of each checkout with no lane that a path was asked of; null where it cannot be opened. */
  readonly #gitIgnore = new Map<string, NativeGitIgnoreMatcher | null>();
  /** How many times `refreshCodeRules` ran; a checkout with no lane reads its catalog again at its next question after one. */
  #refreshes = 0;
  #settings: Required<AuditSettings>;
  #sessionId: string | undefined;
  #closing = false;

  constructor(options: CheckoutCodeRuleRouterOptions) {
    this.#options = options;
    this.#sessionRoot = resolve(options.sessionRoot);
    this.#settings = { scope: options.scope ?? DEFAULT_AUDIT_SETTINGS.scope, mode: options.mode ?? DEFAULT_AUDIT_SETTINGS.mode };
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
    this.#refreshes += 1;
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

  /**
   * Whether the checkout that owns `path` reads it: a listed file in a code rule's scope, an unlisted one in a
   * text-reading rule's, never one Git ignores while the session follows `.gitignore` (`acceptsPath` of the enforcer).
   * Its lane answers, or with no lane its catalog's scopes and Git's exclude stack; nothing is read where no lane may run.
   */
  acceptsPath(path: string): boolean {
    const owner = this.#owner(resolve(this.#sessionRoot, path));
    if (owner === undefined) return false;
    const lane = this.#openLane(owner.checkoutRoot);
    if (lane !== undefined) return lane.enforcer.acceptsPath(owner.path);
    const scopes = this.#scopesOf(owner.checkoutRoot);
    return scopes !== undefined && scopes.readsPath(owner.path) && !this.#gitIgnores(owner, false);
  }

  /** Whether the checkout that owns `path` ignores it; a path of a checkout no lane may run in is never ignored here, only not accepted. */
  ignoresPath(path: string, directory: boolean): boolean {
    const owner = this.#owner(resolve(this.#sessionRoot, path));
    if (owner === undefined || !this.#mayAnswer(owner.checkoutRoot)) return false;
    const lane = this.#openLane(owner.checkoutRoot);
    return lane === undefined ? this.#gitIgnores(owner, directory) : lane.enforcer.ignoresPath(owner.path, directory);
  }

  /** Why the checkout that owns the file `path` leaves an edit of it unchecked for Git's rules (`ignoredEdit` of its enforcer). */
  ignoredEdit(path: string): IgnoredEditMatch | undefined {
    const owner = this.#owner(resolve(this.#sessionRoot, path));
    if (owner === undefined) return undefined;
    const lane = this.#openLane(owner.checkoutRoot);
    if (lane !== undefined) return lane.enforcer.ignoredEdit(owner.path);
    const scopes = this.#scopesOf(owner.checkoutRoot);
    if (scopes === undefined || !scopes.readsPath(owner.path)) return undefined;
    return this.#askGitIgnore(owner, matcher => matcher.ignoredBy(owner.path, false)) ?? undefined;
  }

  /** Whether Git ignores `owner`'s path in its checkout, while the session follows `.gitignore`. */
  #gitIgnores(owner: CheckoutOwner, directory: boolean): boolean {
    return this.#askGitIgnore(owner, matcher => matcher.ignored(owner.path, directory)) ?? false;
  }

  /**
   * `ask` of Git's exclude stack of `owner`'s checkout while the session follows `.gitignore`, opened once; undefined
   * otherwise, outside Git, and where the rules cannot be read. A lane's enforcer asks its own the same way.
   */
  #askGitIgnore<T>(owner: CheckoutOwner, ask: (matcher: NativeGitIgnoreMatcher) => T): T | undefined {
    if (this.#options.followGitignore !== true) return undefined;
    const root = owner.checkoutRoot;
    try {
      let matcher = this.#gitIgnore.get(root);
      if (matcher === undefined) {
        matcher = loadNativeBinding().openGitIgnoreMatcher(root);
        this.#gitIgnore.set(root, matcher);
      }
      return matcher === null ? undefined : ask(matcher);
    } catch (error) {
      // As a lane does: rules that cannot be read leave the path in, so a broken `.gitignore` never hides sources. A
      // stack that opened is kept and asked again at the next question; one that cannot open is not tried again.
      if (!this.#gitIgnore.has(root)) this.#gitIgnore.set(root, null);
      process.stderr.write(`rerail: cannot read Git ignore rules for ${owner.path || "."}: ${error instanceof Error ? error.message : String(error)}\n`);
      return undefined;
    }
  }

  sourceRoot(path: string): string | undefined {
    return this.#owner(resolve(this.#sessionRoot, path))?.checkoutRoot;
  }

  /**
   * The other worktrees Git registers for the session's checkout that may run rules: a shell write in one is an edit
   * like any other, wherever the worktree sits and whatever the session's checkout ignores. The walk cannot tell which
   * session wrote a file, so what it finds there is recorded under the repository and steers no session.
   */
  worktreeRoots(): string[] {
    const starting = this.#options.startingCheckout;
    return (worktreeRegistry(starting)?.worktrees ?? [])
      .map(worktree => storedPath(worktree))
      .filter(root => root !== starting && existsSync(root) && this.#owner(root)?.checkoutRoot === root && this.#mayRun(root));
  }

  /**
   * The checkouts whose ignore rules the session's checks follow: the session's checkout and, each level down, the
   * checkouts nested in it that own their files, while they may run rules. The other worktrees share the session
   * checkout's `info/exclude` and configuration, so they add none.
   */
  ignoreRuleRoots(): string[] {
    const roots: string[] = [];
    const pending = [this.#options.startingCheckout];
    for (let root = pending.pop(); root !== undefined; root = pending.pop()) {
      if (roots.includes(root) || !this.#mayRun(root)) continue;
      roots.push(root);
      pending.push(...this.#nestedCheckoutsOf(root).map(nested => join(root, nested)));
    }
    return roots;
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
    const auditIds: string[] = [];
    if (input.scope === "repository") {
      if ((input.paths ?? []).length > 0) throw new Error("Repository scope always covers the full repository");
      await this.#audit(sessionId, this.#options.startingCheckout, input.mode, input.audit, auditIds, new Set());
      return auditIds;
    }
    const visited = new Set<string>();
    for (const [lane, paths] of this.#byLane(input.paths ?? [])) {
      await lane.opening;
      auditIds.push(...await lane.enforcer.audit(sessionId, { scope: "touched-files", mode: input.mode, paths }));
      // The lane's walk skips the checkouts nested in a folder it was given; each is audited whole, in its own lane.
      for (const nested of this.#nestedCheckoutsOf(lane.root).filter(nested => paths.some(path => folderHolds(path, nested))))
        await this.#audit(sessionId, join(lane.root, nested), input.mode, undefined, auditIds, visited);
    }
    return auditIds;
  }

  /** Each path, relative to the checkout that owns it, under that checkout's lane; a path no lane owns is left out. */
  #byLane(paths: readonly string[]): Map<Lane, string[]> {
    const partitioned = new Map<Lane, string[]>();
    for (const path of paths) {
      const routed = this.#route(path);
      if (routed !== undefined) partitioned.set(routed.lane, [...(partitioned.get(routed.lane) ?? []), routed.path]);
    }
    return partitioned;
  }

  interrupt(options?: AuditorCloseOptions): void {
    if (this.#closing) return;
    this.#closing = true;
    for (const lane of this.#lanes.values()) lane.enforcer.interrupt(options);
  }

  async close(options?: AuditorCloseOptions): Promise<void> {
    this.interrupt(options);
    for (const scopes of new Set([...this.#unlaned.values()].map(unlaned => unlaned.scopes))) scopes.release(this);
    this.#unlaned.clear();
    this.#gitIgnore.clear();
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
    const report = await this.#auditCheckout(sessionId, root, mode, audit);
    if (report === undefined) return;
    if (report.auditId !== undefined) auditIds.push(report.auditId);
    for (const nested of this.#ownNestedCheckouts(root, report.nestedCheckouts))
      await this.#audit(sessionId, nested, mode, audit, auditIds, visited);
  }

  /** The checkout's own run once its lane has opened; undefined when it has no lane or `audit` did not let it claim the run. */
  async #auditCheckout(sessionId: string, root: string, mode: AuditMode, audit: RepositoryAuditHooks | undefined) {
    const lane = this.#lane(root);
    if (lane === undefined) return undefined;
    await lane.opening;
    if (audit !== undefined && !audit.claim(root, lane.enforcer.catalogRevision())) return undefined;
    const report = await lane.enforcer.auditRepository(sessionId, mode);
    audit?.finish(root, report.auditId);
    return report;
  }

  /** Of the checkouts a walk of `root` skipped, the roots of those that own their files and so are judged in their own lane. */
  #ownNestedCheckouts(root: string, nested: readonly string[]): string[] {
    return nested
      .map(path => this.#owner(join(root, path)))
      .filter((owner): owner is CheckoutOwner => owner !== undefined && owner.path === "" && owner.checkoutRoot !== root)
      .map(owner => owner.checkoutRoot);
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
   * A checkout inside another owns its files when Rerail recorded it, or would on this lookup (a worktree Git registered
   * for a started clone), or the outer one declares it as a submodule; a stopped or not set up one then runs nothing.
   * Only a Git folder Rerail never recorded (a stray `.git`, a `git init` nobody started) is judged by the checkout
   * around it.
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
    if (sessionId === undefined || !this.#mayAnswer(root)) return undefined;
    let lane = this.#lanes.get(root);
    if (lane === undefined) {
      // The lane answers for the checkout from now on, with its own exclude stack.
      this.#forgetUnlaned(root);
      const enforcer = new NativeCodeRuleEnforcer(this.#options.catalogs.catalog(root), {
        workspaceRoot: root,
        steerDelivery: this.#options.steerDelivery,
        audits: this.#options.audits,
        ...this.#settings,
        ...(this.#options.timingLog === undefined ? {} : { timingLog: this.#options.timingLog }),
        followGitignore: this.#options.followGitignore ?? false,
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

  /** The lane open for `root`, while it may run; a question asks it, and opens none. */
  #openLane(root: string): Lane | undefined {
    return this.#mayAnswer(root) ? this.#lanes.get(root) : undefined;
  }

  /** Whether `root`'s files get answers from rules: the session is open and its lane may run (`#lane`). */
  #mayAnswer(root: string): boolean {
    return this.#sessionId !== undefined && !this.#closing && this.#mayRun(root);
  }

  /**
   * The scopes a checkout with no lane answers from, its catalog read at its first question and again at its first
   * after each `refreshCodeRules`, its rules as last read standing where they cannot be read; undefined where no lane
   * may run.
   */
  #scopesOf(root: string): CodeRuleScopes | undefined {
    if (!this.#mayAnswer(root)) return undefined;
    const known = this.#unlaned.get(root);
    if (known?.refreshed === this.#refreshes) return known.scopes;
    const catalog = this.#options.catalogs.catalog(root);
    const failure = readAgain(catalog);
    if (failure !== undefined && failure !== known?.failure)
      process.stderr.write(`rerail: cannot read the rules of ${root} again, so its walk keeps the rules last read: ${failure}\n`);
    const scopes = known?.scopes.version === catalog.status.version ? known.scopes : CodeRuleScopes.hold(catalog.snapshot(), this);
    this.#unlaned.set(root, { scopes, refreshed: this.#refreshes, ...(failure === undefined ? {} : { failure }) });
    if (known !== undefined && known.scopes !== scopes) this.#letGo(known.scopes);
    return scopes;
  }

  /** Drops what `root` answered from while it had no lane. */
  #forgetUnlaned(root: string): void {
    const known = this.#unlaned.get(root);
    this.#unlaned.delete(root);
    this.#gitIgnore.delete(root);
    if (known !== undefined) this.#letGo(known.scopes);
  }

  /** Releases `scopes` once no checkout with no lane answers from them. */
  #letGo(scopes: CodeRuleScopes): void {
    for (const unlaned of this.#unlaned.values()) if (unlaned.scopes === scopes) return;
    scopes.release(this);
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
