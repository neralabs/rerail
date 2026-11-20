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
 */
import { existsSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { storedPath } from "../catalog/checkout-control.js";
import type { CheckoutOwner, CheckoutOwners } from "../catalog/checkout-owner.js";
import { worktreeRegistry } from "../catalog/repository-identity.js";
import { GIT_OPEN_AGAIN_MS, gitReadTimedOut, nativeGitAccess } from "../native/git-access.js";
import type { NativeGitAccess, NativeGitIgnoreMatcher } from "../native/native.js";
import { NativeCodeRuleEnforcer } from "./code-rule-enforcer.js";
import { GitIgnoreMatchers } from "../native/git-ignore-matchers.js";
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
  WalkView,
} from "./code-rule.js";
import type { CheckoutCatalogs, CheckoutCodeRuleCatalog, CheckoutEligibility } from "./code-rule-dsl-catalog.js";
import type { AuditRecorder, AuditMode } from "../audit/audit.js";
import type { SteerShell } from "./rerail-steer.js";
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
  /** The shell the session's coding agent runs commands in, which every lane's steers are written for (`steerShell`). */
  readonly steerShell?: SteerShell | undefined;
  /** The daemon's Git ignore matchers, one per checkout for every session (`GitIgnoreMatchers`); the router's own without it. */
  readonly gitIgnoreMatchers?: GitIgnoreMatchers | undefined;
}

interface Lane {
  readonly root: string;
  readonly enforcer: NativeCodeRuleEnforcer;
  readonly opening: Promise<void>;
  /** The count of `refreshCodeRules` calls when its rules were last read; it reads them again at its next use after another. */
  refreshed: number;
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
  /** The daemon's Git ignore matchers (`gitIgnoreMatchers`), or the router's own. */
  readonly #gitIgnoreMatchers: GitIgnoreMatchers;
  /** Git's exclude stack of each checkout a path was asked of, and of each lane's checkout; null outside Git. */
  readonly #gitIgnore = new Map<string, NativeGitIgnoreMatcher | null>();
  /** The owner the matchers in `#gitIgnore` were opened with: a new one (Git answered at last) opens them again. */
  #gitIgnoreAccess: NativeGitAccess | undefined;
  /** When each checkout may open its rules again after an open that outran its time limit (`GIT_OPEN_AGAIN_MS`). */
  readonly #gitIgnoreOpenAgainAt = new Map<string, number>();
  /**
   * The checkouts with no lane whose Git ignore rules failed, reported once per checkout and kind (an open that timed
   * out, an open refused, a question that failed) under one owner.
   */
  readonly #gitIgnoreReported = new Set<string>();
  /**
   * The checkouts another lane answered for, each with the count of `refreshCodeRules` calls when its catalog was last
   * read: as a lane's rules, it is read again at its next question after another (`#laneWithRulesOf`).
   */
  readonly #answeredByOthers = new Map<string, number>();
  /** How many times `refreshCodeRules` ran; a lane whose `refreshed` is lower reads its rules again at its next use. */
  #refreshes = 0;
  /** The session checkout's rule signature and every repository's rule configuration, as the last refresh read them. */
  #rulesSeen: string | undefined;
  #settings: Required<AuditSettings>;
  #sessionId: string | undefined;
  #closing = false;

  constructor(options: CheckoutCodeRuleRouterOptions) {
    this.#options = options;
    this.#sessionRoot = resolve(options.sessionRoot);
    this.#settings = { scope: options.scope ?? DEFAULT_AUDIT_SETTINGS.scope, mode: options.mode ?? DEFAULT_AUDIT_SETTINGS.mode };
    this.#gitIgnoreMatchers = options.gitIgnoreMatchers ?? new GitIgnoreMatchers();
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
   * Re-read every known checkout's eligibility, in one statement however many the session knows: its walks come to
   * know every worktree of its repository. A lane whose checkout may no longer run rules (stopped, not set up, or the
   * session's own checkout stopped) is closed, so its last result stops reaching later responses; it opens again if
   * the checkout becomes eligible. The session's own checkout reads its rules again now, since the session's rule
   * coverage reports its catalog after every event. Every other lane reads them at its next use (`#lane`), before it
   * answers or audits anything, so a tool call reads the rules of the lanes it uses, not of every lane open; a checkout
   * another lane answers for reads its catalog at its next question (`#laneWithRulesOf`). When a rule configuration or
   * the session checkout's rules changed since the last refresh, every lane and every checkout answered for reads its
   * rules now, so a rule the change leaves unrunnable in any of them is recorded as skipped at this event, not at that
   * lane's next use: a cost paid once per change, not per event. A file written in another checkout's `.rerail/rules` changes
   * neither, so its lane reads it at its next use (the walk of the other worktrees a boundary starts, a tool call there)
   * or when the session ends (`interrupt`): reading every lane's rules folder at each refresh would cost a few
   * milliseconds per refresh at a few hundred lanes on Windows.
   */
  refreshCodeRules(): void {
    if (this.#closing) return;
    const ran = new Map([...this.#gitIgnore.keys()].map(root => [root, this.#mayRun(root)]));
    for (const [root, eligibility] of this.#options.catalogs.refreshAll([...this.#eligibility.keys()])) this.#eligibility.set(root, eligibility);
    this.#refreshes += 1;
    this.#forgetGitIgnoreOfChanged(ran);
    this.#closeLanesThatMayNotRun();
    const own = this.#lanes.get(this.#options.startingCheckout);
    if (own !== undefined) this.#readRulesAgain(own);
    if (!this.#rulesChanged(own)) return;
    this.#refreshAnsweredCatalogs();
    for (const lane of [...this.#lanes.values()]) this.#lane(lane.root);
  }

  /**
   * Drops the Git ignore matcher of each checkout in `ran` that starts or stops running rules: its lane, which a walk's
   * questions stand in for, opens or closes with that, and a lane reads Git's ignore rules as it opens (`#takeGitIgnore`),
   * so a checkout stopped and started again does not keep the rules it had before.
   */
  #forgetGitIgnoreOfChanged(ran: ReadonlyMap<string, boolean>): void {
    for (const [root, mayRun] of ran) if (this.#mayRun(root) !== mayRun) this.#forgetGitIgnore(root);
  }

  /**
   * Drops `root`'s Git ignore matcher and lets go of the daemon's (`GitIgnoreMatchers`), which is dropped too, and its
   * index with it, once no router holds it: a checkout this router no longer asks about keeps no index for it.
   */
  #forgetGitIgnore(root: string): void {
    this.#gitIgnore.delete(root);
    this.#gitIgnoreMatchers.releaseRoot(this, root);
  }

  /** Closes each lane whose checkout may no longer run rules, so its last result stops reaching later responses. */
  #closeLanesThatMayNotRun(): void {
    for (const lane of [...this.#lanes.values()]) {
      if (this.#mayRun(lane.root)) continue;
      this.#lanes.delete(lane.root);
      lane.enforcer.interrupt();
      void lane.opening.catch(() => undefined).then(() => lane.enforcer.close()).catch(() => undefined);
    }
  }

  /**
   * Whether the session checkout's rule signature (its installed packs, its own rules, its repository's configuration)
   * or any repository's rule configuration (`CheckoutCatalogs.configurationVersion`, one statement) differs from what
   * the last refresh read: then another lane's rules may read differently too.
   */
  #rulesChanged(own: Lane | undefined): boolean {
    const signature = own === undefined ? "" : this.#options.catalogs.catalog(own.root).status.sourceSignature;
    const seen = `${signature}\0${this.#options.catalogs.configurationVersion()}`;
    const changed = seen !== this.#rulesSeen;
    this.#rulesSeen = seen;
    return changed;
  }

  /**
   * Has the owners look again at each of `paths`, a tool call's files, and the folders above it
   * (`CheckoutOwners.lookAgainAt`). `git worktree add`, from any repository, can make a folder this session looked up
   * while it was plain a checkout, and the answers kept from then would leave the call's edits there unchecked.
   */
  refreshOwners(paths: readonly string[]): void {
    if (this.#closing) return;
    for (const path of paths) this.#options.owners.lookAgainAt(resolve(this.#sessionRoot, path));
  }

  updateSettings(settings: Required<AuditSettings>): void {
    this.#settings = settings;
    for (const lane of this.#lanes.values()) lane.enforcer.updateSettings(settings);
  }

  /** Whether the lane that owns `path` reads it: a listed file in a code rule's scope, an unlisted one in a text-reading rule's (`acceptsPath` of the enforcer). */
  acceptsPath(path: string): boolean {
    return this.#accepts(this.#owner(resolve(this.#sessionRoot, path)));
  }

  /**
   * Whether the checkout that owns `path` ignores it while the session follows `.gitignore`: its lane's answer, or for a
   * checkout that runs no rules (stopped, or started and not set up) Git's own. A walk of the session's folder asks this
   * (through `walkView`) of every folder and file, and enters no folder Git ignores, so a checkout with no lane still
   * keeps the walk out of its ignored folders. A path outside every Git checkout is never ignored.
   */
  ignoresPath(path: string, directory: boolean): boolean {
    return this.#ignores(this.#owner(resolve(this.#sessionRoot, path)), directory);
  }

  /**
   * `acceptsPath` and `ignoresPath` for one walk. A path's full lookup (`#owner`) resolves its real path and looks for
   * `.git` on disk, about 0.4 ms on Windows, and the walk asks two questions of every file. A file the walk found is
   * no link, so its checkout is its folder's: each folder gets the full lookup once, for itself and its files. A
   * checkout recorded while the walk runs owns its files from the next walk.
   */
  walkView(): Required<WalkView> {
    const folders = new Map<string, CheckoutOwner | undefined>();
    const folderOwner = (folder: string): CheckoutOwner | undefined => {
      if (!folders.has(folder)) folders.set(folder, this.#owner(resolve(this.#sessionRoot, folder)));
      return folders.get(folder);
    };
    const fileOwner = (path: string): CheckoutOwner | undefined => {
      const slash = path.lastIndexOf("/");
      const owner = folderOwner(slash < 0 ? "" : path.slice(0, slash));
      const name = path.slice(slash + 1);
      return owner === undefined ? undefined : { checkoutRoot: owner.checkoutRoot, path: owner.path === "" ? name : `${owner.path}/${name}` };
    };
    return {
      acceptsPath: path => this.#accepts(fileOwner(path)),
      ignoresPath: (path, directory) => this.#ignores(directory ? folderOwner(path) : fileOwner(path), directory),
    };
  }

  /**
   * Whether the lane of `owner`'s checkout reads its path; nothing is read where no lane runs. A walk asks this of every
   * file of every worktree it reads: a checkout with no lane open is answered by an open lane at its rules' version,
   * which reads the same paths, with Git's ignore rules of its own, and opens its lane only when no open lane holds its
   * rules. A lane holds a native workspace and its checkout's index, so opening one in every worktree to answer took
   * the daemon's memory to gigabytes with one session in a repository of many.
   */
  #accepts(owner: CheckoutOwner | undefined): boolean {
    if (owner === undefined) return false;
    const root = owner.checkoutRoot;
    // A lane of its own reads its rules again first at its first use after an event (`#lane`).
    const lane = this.#lanes.has(root) ? this.#lane(root) : this.#laneWithRulesOf(root) ?? this.#lane(root);
    return lane !== undefined && this.#readsIn(lane, owner);
  }

  /** Whether `lane` reads `owner`'s path: its own checkout's answer, or another's by the same rules and that checkout's Git ignore rules. */
  #readsIn(lane: Lane, owner: CheckoutOwner): boolean {
    if (lane.root === owner.checkoutRoot) return lane.enforcer.acceptsPath(owner.path);
    return lane.enforcer.readsPath(owner.path) && !this.#gitIgnores(owner, false);
  }

  /** A lane follows its own checkout's Git ignore rules, as `#gitIgnores` does, so a checkout needs no lane to answer. */
  #ignores(owner: CheckoutOwner | undefined, directory: boolean): boolean {
    if (owner === undefined) return false;
    const lane = this.#lanes.get(owner.checkoutRoot);
    return lane === undefined ? this.#gitIgnores(owner, directory) : lane.enforcer.ignoresPath(owner.path, directory);
  }

  /**
   * An open lane at the version of the rules `root`'s catalog holds; undefined for a checkout that may not run rules.
   * The catalog, which every session's lanes in `root` share, is read again at the first question after each
   * `refreshCodeRules`, as a lane's rules are (`#lane`, `#openLane`), so another session's earlier read never stands for
   * a change since. Rules that cannot be read throw, as they did from a lane opened for the question, and leave `root`
   * out of the checkouts answered for.
   */
  #laneWithRulesOf(root: string): Lane | undefined {
    if (this.#sessionId === undefined || this.#closing || !this.#mayRun(root)) return undefined;
    const catalog = this.#options.catalogs.catalog(root);
    if (this.#answeredByOthers.get(root) !== this.#refreshes) catalog.refresh();
    this.#answeredByOthers.set(root, this.#refreshes);
    return this.#laneAt(catalog.status.version);
  }

  /** An open lane whose rules are at `version`. */
  #laneAt(version: string): Lane | undefined {
    for (const lane of this.#lanes.values()) if (lane.enforcer.rulesVersion === version) return lane;
    return undefined;
  }

  /**
   * Reads again now the rules of each checkout another lane answers for, as every lane reads its own once the rules
   * changed (`refreshCodeRules`). Rules that fail to load open the checkout's own lane, whose read records the failure
   * in the session's responses as a lane that answered for itself did; the other checkouts and every lane still read.
   */
  #refreshAnsweredCatalogs(): void {
    const sessionId = this.#sessionId;
    if (sessionId === undefined) return;
    for (const root of [...this.#answeredByOthers.keys()]) {
      if (this.#lanes.has(root) || !this.#mayRun(root)) continue;
      const catalog = this.#options.catalogs.catalog(root);
      try {
        catalog.refresh();
        this.#answeredByOthers.set(root, this.#refreshes);
      } catch {
        this.#readRulesAgain(this.#addLane(root, sessionId, catalog));
      }
    }
  }

  /**
   * Whether Git ignores `owner`'s path in its checkout, while the session follows `.gitignore`; its rules open once.
   * Only a failed open leaves the checkout without a matcher: one path's failed read (an unreadable nested
   * `.gitignore`, the index caught mid-rewrite) used to drop it too, and every later walk then entered the checkout's
   * ignored folders. As a lane does, rules that cannot be read leave the path in, so a broken `.gitignore` never hides
   * sources.
   */
  #gitIgnores(owner: CheckoutOwner, directory: boolean): boolean {
    if (this.#options.followGitignore !== true) return false;
    try {
      return this.#gitIgnoreMatcher(owner)?.ignored(owner.path, directory) ?? false;
    } catch (error) {
      this.#reportGitIgnoreError(owner, error, "question");
      return false;
    }
  }

  /**
   * The exclude stack of `owner`'s checkout, taken on first use (`#openGitIgnore`); null where it is outside Git or
   * cannot be opened. An open that outran its time limit is not kept: the checkout opens again once GIT_OPEN_AGAIN_MS
   * has passed, and sees no matcher until then.
   */
  #gitIgnoreMatcher(owner: CheckoutOwner): NativeGitIgnoreMatcher | null {
    let matcher: NativeGitIgnoreMatcher | null | undefined;
    try {
      matcher = this.#keptGitIgnore(owner.checkoutRoot);
      if (matcher !== undefined) return matcher;
      if (Date.now() < (this.#gitIgnoreOpenAgainAt.get(owner.checkoutRoot) ?? 0)) return null;
      matcher = this.#openGitIgnore(owner.checkoutRoot);
    } catch (error) {
      return this.#gitIgnoreOpenFailed(owner, error);
    }
    this.#gitIgnore.set(owner.checkoutRoot, matcher);
    return matcher;
  }

  /** The exclude stack kept for `root` under the process's Git owner now (`#followGitOwner`). */
  #keptGitIgnore(root: string): NativeGitIgnoreMatcher | null | undefined {
    this.#followGitOwner();
    return this.#gitIgnore.get(root);
  }

  /**
   * Drops every exclude stack kept, and lets go of the daemon's, once the process's Git owner is not the one they were
   * kept under (Git answered at last). A lane's take and a walk's question both look first, so neither drops what the
   * other kept under the same owner.
   */
  #followGitOwner(): void {
    const access = nativeGitAccess();
    if (access === this.#gitIgnoreAccess) return;
    for (const kept of [...this.#gitIgnore.keys()]) this.#forgetGitIgnore(kept);
    this.#gitIgnoreReported.clear();
    this.#gitIgnoreAccess = access;
  }

  /** Says a failed open once per kind: one that timed out is tried again after GIT_OPEN_AGAIN_MS, one refused is kept as none. */
  #gitIgnoreOpenFailed(owner: CheckoutOwner, error: unknown): null {
    const timedOut = gitReadTimedOut(error);
    this.#reportGitIgnoreError(owner, error, timedOut ? "timed out" : "refused");
    if (timedOut) this.#gitIgnoreOpenAgainAt.set(owner.checkoutRoot, Date.now() + GIT_OPEN_AGAIN_MS);
    else this.#gitIgnore.set(owner.checkoutRoot, null);
    return null;
  }

  /**
   * `root`'s Git ignore rules as a matcher opened now reads them, kept for the questions that follow: the one the router
   * holds while its sources are unchanged (`GitIgnoreMatchers`), else one opened now. A failed open throws.
   */
  #takeGitIgnore(root: string): NativeGitIgnoreMatcher | null {
    this.#followGitOwner();
    const matcher = this.#openGitIgnore(root);
    this.#gitIgnore.set(root, matcher);
    return matcher;
  }

  /**
   * `root`'s Git ignore rules: the daemon's matcher for the checkout, which every session's router shares, or one of its
   * own. A router that closed takes its own, so a late question does not hold the shared one past its close.
   */
  #openGitIgnore(root: string): NativeGitIgnoreMatcher | null {
    return this.#closing ? nativeGitAccess().openGitIgnoreMatcher(root) : this.#gitIgnoreMatchers.matcher(root, this);
  }

  #reportGitIgnoreError(owner: CheckoutOwner, error: unknown, kind: "timed out" | "refused" | "question"): void {
    const key = `${owner.checkoutRoot}\0${kind}`;
    if (this.#gitIgnoreReported.has(key)) return;
    this.#gitIgnoreReported.add(key);
    process.stderr.write(`rerail: cannot read Git ignore rules for ${owner.path || "."}: ${error instanceof Error ? error.message : String(error)}\n`);
  }

  /** Why the checkout that owns the file `path` leaves an edit of it unchecked for Git's rules (`ignoredEdit` of its enforcer). */
  ignoredEdit(path: string): IgnoredEditMatch | undefined {
    const routed = this.#route(path);
    return routed?.lane.enforcer.ignoredEdit(routed.path);
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

  async settleGeneratedEdits(only?: "steering"): Promise<void> {
    await this.#settleLanes(lane => lane.enforcer.settleGeneratedEdits(only));
  }

  /** Every lane checks in the router's scope: in repository scope a steering wait waits for a walk's audits too. */
  steeringWaitsForLog(): boolean {
    return this.#settings.scope === "repository";
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

  /**
   * Stops every lane. A lane no walk or tool call used since the last refresh reads its rules once more first: a rule a
   * file written in its checkout's `.rerail/rules` leaves unrunnable there is recorded as skipped when the session ends,
   * if not before.
   */
  interrupt(options?: AuditorCloseOptions): void {
    if (this.#closing) return;
    for (const lane of [...this.#lanes.values()]) if (lane.refreshed !== this.#refreshes) this.#readRulesAgain(lane);
    this.#closing = true;
    for (const lane of this.#lanes.values()) lane.enforcer.interrupt(options);
  }

  async close(options?: AuditorCloseOptions): Promise<void> {
    this.interrupt(options);
    const lanes = [...this.#lanes.values()];
    this.#lanes.clear();
    this.#gitIgnore.clear();
    this.#gitIgnoreMatchers.release(this);
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
   * The lane of an eligible checkout, opened on first use with its rules read then, and reading them again on its first
   * use after each `refreshCodeRules`; undefined while the checkout may not run rules, and for every checkout while the
   * session's own checkout is stopped (the gate stays on the starting checkout).
   */
  #lane(root: string): Lane | undefined {
    const sessionId = this.#sessionId;
    if (sessionId === undefined || this.#closing || !this.#mayRun(root)) return undefined;
    const lane = this.#lanes.get(root);
    if (lane === undefined) return this.#openLane(root, sessionId);
    if (lane.refreshed !== this.#refreshes) this.#readRulesAgain(lane);
    return lane;
  }

  /** The lane of `root`, opened now with its checkout's rules read first. */
  #openLane(root: string, sessionId: string): Lane {
    // Every lane in this checkout, this session's earlier ones and other sessions', shares the catalog, and each reads
    // it again only at its own next use: it may hold rules from before the last event.
    const catalog = this.#options.catalogs.catalog(root);
    catalog.refresh();
    return this.#addLane(root, sessionId, catalog);
  }

  /** The lane of `root` over `catalog`, opened now with the rules the catalog holds. */
  #addLane(root: string, sessionId: string, catalog: CheckoutCodeRuleCatalog): Lane {
    const enforcer = new NativeCodeRuleEnforcer(catalog, {
      workspaceRoot: root,
      steerDelivery: this.#options.steerDelivery,
      audits: this.#options.audits,
      ...this.#settings,
      ...(this.#options.timingLog === undefined ? {} : { timingLog: this.#options.timingLog }),
      followGitignore: this.#options.followGitignore ?? false,
      // Asked as the lane first reads Git's ignore rules: the matcher a walk's questions opened for the checkout while
      // its sources are unchanged, so the checkout holds one index, else one opened now. A failed open throws to the
      // lane: one refused stays none for the lane, and one that timed out opens again after GIT_OPEN_AGAIN_MS.
      openGitIgnore: () => this.#takeGitIgnore(root),
      steerShell: this.#options.steerShell,
      presentPath: path => relative(this.#sessionRoot, join(root, path)).split(sep).join("/"),
      nestedCheckouts: () => this.#nestedCheckoutsOf(root),
    });
    // The enforcer registers its code rules synchronously, so the new lane answers `acceptsPath` at once.
    const lane: Lane = { root, enforcer, opening: enforcer.openWorkspace(sessionId), refreshed: this.#refreshes };
    lane.opening.catch(() => undefined);
    this.#lanes.set(root, lane);
    return lane;
  }

  /** `lane`'s first use since the last `refreshCodeRules`: it reads its checkout's rules again before it answers. */
  #readRulesAgain(lane: Lane): void {
    lane.refreshed = this.#refreshes;
    lane.enforcer.refreshCodeRules();
  }

  /** Whether `root` may run rules in this session: it is eligible, and the session's own checkout is not stopped. */
  #mayRun(root: string): boolean {
    return this.whyNoRules(root) === undefined;
  }

  /**
   * Why the checkout at `root` runs no rules in this session, or undefined when it may: the starting checkout's
   * eligibility while it is stopped or unrecorded, which keeps every lane closed, else `root`'s own unless it is `ok`.
   */
  whyNoRules(root: string): { checkoutRoot: string; eligibility: Exclude<CheckoutEligibility, "ok"> } | undefined {
    const starting = this.#options.startingCheckout;
    const startingEligibility = this.#eligibilityOf(starting);
    if (startingEligibility === "stopped" || startingEligibility === "unrecorded") return { checkoutRoot: starting, eligibility: startingEligibility };
    const eligibility = this.#eligibilityOf(root);
    return eligibility === "ok" ? undefined : { checkoutRoot: root, eligibility };
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
