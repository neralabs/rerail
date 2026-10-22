import { existsSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";
import { and, eq } from "drizzle-orm";
import type { RulesDashboardScope } from "../api/session-contracts.js";
import { CODING_AGENT_NAMES } from "../setup/coding-agents.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { HARNESSES, type Harness } from "../setup/harness-registration.js";
import type { CheckoutContext, RepositoryKey, RepositoryStartState, RuleSetting } from "../engine/catalog/repository-model.js";
import { repositoryIdentityLocation } from "../engine/catalog/repository-identity.js";
import { listRepositories, lookupCheckout, recordCheckout } from "../engine/catalog/repository-registry.js";
import {
  awaitingStartAfterReview,
  checkoutName,
  checkoutStatus,
  onRuleCount,
  setStarted,
  startSetUpRepository,
  stateOf,
  workspaceStartState,
  type RepositorySetupIo,
} from "../runtime/repository-start.js";
import { addAttachment, recordSetUp, removeAttachment, setRuleSetting } from "../engine/catalog/rule-configuration.js";
import { loadRulePacks, recommendRulePacks, rulePackPackageName, type RulePack, type RulePackRecommendation } from "../engine/catalog/rule-packs.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import type { RuleCatalog } from "../engine/catalog/repository-model.js";
import { readRepositoryLintConfig } from "../engine/catalog/repository-lint-config.js";
import {
  discoverPacks,
  installedPackVersion,
  installPackVersions,
  parsePackVersions,
  type PackDiscovery,
  type PackStore,
  type PackVersion,
} from "../setup/rule-pack-store.js";
import semver from "semver";
import { ruleAttachments, ruleSettings } from "../engine/storage/schema.js";
import { setupReviewOffered } from "../engine/catalog/setup-review.js";
import {
  CONFIRM_PACKS_ANSWERS,
  confirmPacksAnswers,
  NO_PACKS,
  NO_REPO_ANSWERS,
  PICK_PACKS_ANSWERS,
  pickPacksQuestion,
  noRepoQuestion,
  READ_REPO_ANSWERS,
  readRepoQuestion,
  SESSIONS_ANSWERS,
  sessionsQuestion,
  SETUP_FLAGS,
  recommendedPacksQuestion,
  renderPacksPrompt,
  renderAnswerHint,
  renderAnswerPrompt,
  renderFolderNotFound,
  renderFolderPrompt,
  renderNotNow,
  renderPickedMyself,
  renderMenu,
  renderQuestion,
  renderQuestionHead,
  renderSkipped,
  renderSessionsDeclined,
  renderStoppedRepo,
  renderSummary,
  setupQuestion,
  type AvailablePack,
  type CodingAgent,
  type DashboardLink,
  type QuestionForm,
  type QuestionStep,
  type RecommendedPacksInput,
} from "../harness/setup-screens.js";
import type { SetupQuestion, SetupQuestionId, SetupStep } from "../harness/setup-step.js";

/**
 * The six-step set-up behind `rerail setup` and `rerail start`: ask before reading the
 * repository, recommend rule packs, confirm, start in this repository only, then the summary. Nothing
 * is written to the repository, its Git metadata, or the database before the person says yes.
 *
 * Terminal lane: one run, asking through `io.ask`. Agent lane: no `io.ask`; each run prints the next
 * question and stops, and the coding agent runs set-up again with `--answer`, `--repo` and `--agents`.
 * Every answer re-run starts from what it is told, so repeating one repeats no earlier side effect.
 */

/** `--agents claude-code,cursor`; each entry is an id or the name people see ("Claude Code"). */
export function parseCodingAgents(list: string): Harness[] {
  const entries = list.split(",").map(entry => entry.trim()).filter(Boolean);
  if (entries.length === 0) throw new Error(`Name at least one coding agent: ${HARNESSES.join(", ")}`);
  return [...new Set(entries.map(entry => {
    const wanted = entry.toLowerCase();
    const agent = HARNESSES.find(id => id === wanted || CODING_AGENT_NAMES[id].toLowerCase() === wanted);
    if (agent === undefined) throw new Error(`Unknown coding agent: ${entry}. Rerail works with ${HARNESSES.join(", ")}.`);
    return agent;
  }))];
}

/**
 * `--agents` as set-up takes it. A fresh run fails on a bad list before anything changes. An answer's bad list
 * (`--answer`, relayed from a person's typing) becomes `agentsError`, and set-up asks its confirm question again.
 */
export function setupAgents(list: string | undefined, answer: string | undefined): Pick<RepositorySetupOptions, "agents" | "agentsError"> {
  if (list === undefined) return {};
  try {
    return { agents: parseCodingAgents(list) };
  } catch (error) {
    if (answer === undefined) throw error;
    return { agentsError: error instanceof Error ? error.message : String(error) };
  }
}

/**
 * `--packs typescript@0.1.1,complexity@0.1.1`, or `none` for no pack, as `setupAgents` takes `--agents`: an answer's
 * bad list asks again.
 */
export function setupPacks(list: string | undefined, answer: string | undefined): Pick<RepositorySetupOptions, "packs" | "packsError"> {
  if (list === undefined) return {};
  if (list.trim() === NO_PACKS) return { packs: [] };
  try {
    return { packs: parsePackVersions(list) };
  } catch (error) {
    if (answer === undefined) throw error;
    return { packsError: error instanceof Error ? error.message : String(error) };
  }
}

export type DashboardPage = "rules";

/** What set-up needs from the machine; the CLI builds it (`setupEnvironment`), tests fake it. */
export interface SetupEnvironment {
  /** Coding agents installed on this computer. */
  agentsFound(): readonly Harness[];
  /** Connect these coding agents. `repoRoot` is the repository the person said yes to, if any. */
  connect(agents: readonly Harness[], repoRoot: string | undefined): Promise<void>;
  /** Whether the person has let Rerail read their coding agent sessions on this computer (T2b, asked once). */
  sessionsApproved(): boolean;
  /** Record their answer: yes is kept, no takes back an earlier yes. */
  setSessionsApproved(approved: boolean): Promise<void>;
  /**
   * A dashboard page scoped to `scope`'s repository and checkout; the link is always returned. `open`: try a
   * browser (E6 says why none opened). False when an earlier run already opened it, so a re-run shows the link alone.
   */
  dashboard(scope: RulesDashboardScope, page: DashboardPage, open: boolean): Promise<DashboardLink>;
  /** Where rule packs are fetched from and installed to: npm, into the Rerail home. */
  readonly packs: PackStore;
}

export interface RepositorySetupOptions {
  databasePath?: string;
  /** `--agents`: the coding agents to connect. Those found on this computer when absent. */
  agents?: readonly Harness[];
  /** An answer's `--agents` that `parseCodingAgents` rejected: set-up asks its confirm question again, with this error. */
  agentsError?: string;
  /**
   * `--packs`: the rule packs, at exact versions, an answer installs: the ones its question showed, or those the person
   * chose. Nothing is installed before an answer that names them.
   */
  packs?: readonly PackVersion[];
  /** An answer's `--packs` that is not `<id>@<version>,…`: set-up asks its confirm question again, with this error. */
  packsError?: string;
  /** `rerail start --repository`: start every checkout of the repository, not just this one and those inside it. */
  repository?: boolean;
  /** `--answer`: the person's answer, relayed by the coding agent. */
  answer?: string;
  /** What the agent lane's answer commands run; `rerail setup` when absent. */
  setupCommand?: string;
  /** `--json`: what each answer's `next.args` start with; `setup --json` when absent. */
  nextArgs?: readonly string[];
  /** `--json`: the question a run stopped at. */
  onQuestion?: (question: SetupQuestion) => void;
  environment: SetupEnvironment;
}

export interface SetupRequest extends RepositorySetupOptions {
  /** Where set-up ran. Its repository is the one Rerail offers to read. */
  cwd: string;
  /** `--repo`: the folder the person named. Naming it is the go-ahead to read it. */
  repo?: string;
}

/**
 * How a set-up run ended. `waiting`: the agent lane stopped at a question; `cancelled`: terminal input ended;
 * `declined`: the person did not let Rerail read their sessions, so nothing was set up; `not-now`: the person
 * answered "Not now" to the confirm question, so nothing was installed, connected or turned on.
 */
export type SetupOutcome =
  | { status: "waiting" | "cancelled" | "skipped" | "declined" | "not-now" }
  | { status: "started" | "stopped"; repositoryName: string; ruleCount: number };

export interface RepositoryStartReport {
  /** The state before this start. */
  previous: RepositoryStartState;
  /** The repository was not set up, so this start ran set-up's questions. */
  setUpRan: boolean;
  repositoryName: string;
  /** Rules on now; 0 while set-up waits for an answer. */
  ruleCount: number;
  /** Terminal input ended before set-up finished; nothing was turned on. */
  cancelled: boolean;
  /** How it ended: `started` for a repository set up before, the set-up's outcome otherwise. */
  status: SetupOutcome["status"];
}

const DEFAULT_NEXT_ARGS: readonly string[] = ["setup", "--json"];

const ANSWERS: readonly string[] = [...new Set<string>([...SESSIONS_ANSWERS, ...READ_REPO_ANSWERS, ...CONFIRM_PACKS_ANSWERS, ...NO_REPO_ANSWERS])];

interface Repository {
  root: string;
  name: string;
}

interface Run {
  readonly io: RepositorySetupIo;
  readonly request: SetupRequest;
  readonly databasePath: string;
  /** The main flow's questions this run expects to ask, in order, which the terminal numbers "Step 1 of 3". */
  readonly steps: readonly SetupQuestionId[];
}

/** The main flow's questions: whether Rerail may see sessions (asked once), whether to scan the repo, and the confirm question. */
function mainSteps(environment: SetupEnvironment, readRepo: boolean): SetupQuestionId[] {
  return [...(environment.sessionsApproved() ? [] : ["sessions" as const]), ...(readRepo ? ["read-repo" as const] : []), "confirm-packs"];
}

/** The Rerail home set-up installs packs into, and so reads them back from: its pack store's. */
const packsHome = (run: Run): string => run.request.environment.packs.home;

function say(io: RepositorySetupIo, screen: string): void {
  io.say(screen.replace(/\n$/u, ""));
}

function codingAgents(ids: readonly Harness[]): CodingAgent[] {
  return ids.map(id => ({ id, name: CODING_AGENT_NAMES[id] }));
}

function halted(io: RepositorySetupIo): SetupOutcome {
  return { status: io.ask === undefined ? "waiting" : "cancelled" };
}

/**
 * T2b, before set-up reads or connects anything: may Rerail read the person's coding agent sessions? Asked until they
 * allow it, then never again on this computer; declining takes back an earlier yes. The answer is used up here; `repo`
 * is the folder the run is for, which the allow answer carries on (set-up then still asks before reading it). Returns
 * the answer the rest of set-up goes on with, or how the run ended.
 */
async function approveSessions(run: Run, answer: string | undefined, repo: string | undefined): Promise<{ answer: string | undefined } | SetupOutcome> {
  const { environment } = run.request;
  if (answer === "decline") {
    await environment.setSessionsApproved(false);
    say(run.io, renderSessionsDeclined(run.io.lane));
    return { status: "declined" };
  }
  if (answer === "allow") {
    await environment.setSessionsApproved(true);
    return { answer: undefined };
  }
  if (environment.sessionsApproved()) return { answer };
  const form = sessionsQuestion({
    carriedArgs: repo === undefined ? [] : [SETUP_FLAGS.repo, repo],
    ...(run.request.setupCommand === undefined ? {} : { setupCommand: run.request.setupCommand }),
  });
  const picked = await choose(run, form, SESSIONS_ANSWERS);
  if (picked === undefined) return halted(run.io);
  return approveSessions(run, picked, repo);
}

/** `""` takes the first answer (the cursor is on it); `2` the second; an answer key itself also works. */
function pickAnswer<K extends string>(line: string, keys: readonly K[]): K | undefined {
  const text = line.trim().toLowerCase();
  if (text === "") return keys[0];
  if (/^\d+$/u.test(text)) return keys[Number.parseInt(text, 10) - 1];
  return keys.find(key => key === text);
}

/** Say the question; in the terminal, wait for a valid answer (a menu at a real terminal). Undefined: the agent lane stops here, or input ended. */
async function choose<K extends string>(run: Run, form: QuestionForm, keys: readonly K[]): Promise<K | undefined> {
  const { io } = run;
  const index = run.steps.indexOf(form.id);
  const step: QuestionStep | undefined = index < 0 ? undefined : { index: index + 1, total: run.steps.length };
  if (io.lane === "terminal" && io.choose !== undefined) {
    say(io, renderQuestionHead(form, step));
    const index = await io.choose({ count: keys.length, frame: highlight => renderMenu(form, highlight) });
    return index === undefined ? undefined : keys[index];
  }
  say(io, renderQuestion(form, io.lane, step));
  if (io.lane === "json") run.request.onQuestion?.(setupQuestion(form, run.request.nextArgs ?? DEFAULT_NEXT_ARGS));
  if (io.ask === undefined) return undefined;
  for (;;) {
    const line = await io.ask(renderAnswerPrompt());
    if (line === undefined) return undefined;
    const picked = pickAnswer(line, keys);
    if (picked !== undefined) return picked;
    io.say(renderAnswerHint(keys.length));
  }
}

/** The repository at a folder the person named: its Git root, or the folder itself outside Git. */
function repositoryAt(folder: string): Repository | undefined {
  const trimmed = folder.trim();
  const path = resolve(trimmed === "~" || trimmed.startsWith("~/") ? `${homedir()}${trimmed.slice(1)}` : trimmed);
  try {
    if (!statSync(path).isDirectory()) return undefined;
  } catch {
    return undefined;
  }
  const { workspaceRoot } = repositoryIdentityLocation(path);
  return { root: workspaceRoot, name: checkoutName(workspaceRoot, true) };
}

const HOME_FOLDER_REFUSED = "Start Rerail inside a repository, not your home folder.";

const isHomeFolder = (root: string): boolean => resolve(root) === resolve(homedir());

/** The Git repository set-up ran in, if any. A plain folder, or the home folder, is no repository to offer. */
function repositoryHere(cwd: string): Repository | undefined {
  const location = repositoryIdentityLocation(cwd);
  if (!location.git || isHomeFolder(location.workspaceRoot)) return undefined;
  return { root: location.workspaceRoot, name: checkoutName(location.workspaceRoot, true) };
}

async function askFolder(io: RepositorySetupIo): Promise<Repository | undefined> {
  if (io.ask === undefined) return undefined;
  for (;;) {
    const line = await io.ask(renderFolderPrompt());
    if (line === undefined) return undefined;
    if (line.trim() === "") continue;
    const repository = repositoryAt(line);
    if (repository !== undefined) return repository;
    say(io, renderFolderNotFound({ folder: line.trim() }, io.lane));
  }
}

/**
 * The packs a line names: each entry a number from the list, an id, or an id at the listed version. Undefined, with
 * the reason, for an entry that names none of them.
 */
export function pickPacks(line: string, available: readonly AvailablePack[]): PackVersion[] | string {
  const entries = line.split(",").map(entry => entry.trim()).filter(entry => entry !== "");
  if (entries.length === 0) return "Name at least one rule pack.";
  const picked: PackVersion[] = [];
  for (const entry of entries) {
    const byNumber = /^\d+$/u.test(entry) ? available[Number.parseInt(entry, 10) - 1] : undefined;
    const found = byNumber ?? available.find(pack => pack.id === entry || `${pack.id}@${pack.version}` === entry);
    if (found === undefined) return `${entry} is not one of the rule packs listed.`;
    if (!picked.some(pack => pack.id === found.id)) picked.push({ id: found.id, version: found.version });
  }
  return picked;
}

async function askPacks(io: RepositorySetupIo, available: readonly AvailablePack[]): Promise<PackVersion[] | undefined> {
  if (io.ask === undefined) return undefined;
  for (;;) {
    const line = await io.ask(renderPacksPrompt({ available }));
    if (line === undefined) return undefined;
    const picked = pickPacks(line, available);
    if (typeof picked !== "string") return picked;
    say(io, picked);
  }
}


/** The database, read only, for set-up's reads before the person says yes. Undefined before it exists. */
function readDatabase<T>(run: Run, read: (database: RerailDatabase) => T): T | undefined {
  if (!existsSync(run.databasePath)) return undefined;
  const database = openRerailDatabase(run.databasePath, { readonly: true });
  try {
    return read(database);
  } finally {
    database.$client.close();
  }
}

/**
 * The database for set-up's writes, with the repository's checkout recorded: only after the person said yes. Every
 * write, the checkout's included, is one immediate transaction, so a failure partway leaves nothing behind and no other
 * process sees half of it. The writes inside nest as savepoints.
 */
function withCheckout<T>(run: Run, repository: Repository, action: (database: RerailDatabase, checkout: CheckoutContext) => T): T {
  const database = openRerailDatabase(run.databasePath);
  try {
    return database.transaction(() => action(database, recordCheckout(database, repository.root)), { behavior: "immediate" });
  } finally {
    database.$client.close();
  }
}

/** The rule packs added to a repository, each at the folders it is added in. */
function attachedPacks(database: RerailDatabase, repositoryKey: RepositoryKey): { packId: string; folder: string }[] {
  return database.select({ packId: ruleAttachments.sourceId, folder: ruleAttachments.folder }).from(ruleAttachments)
    .where(and(eq(ruleAttachments.repositoryKey, repositoryKey), eq(ruleAttachments.sourceKind, "pack")))
    .orderBy(ruleAttachments.sourceId, ruleAttachments.folder).all();
}

/** Each added pack once, by name, with the folders it is added in: "TSLint", "Python in api". */
function packLabels(packs: readonly { packId: string; folder: string }[]): string[] {
  const folders = new Map<string, string[]>();
  for (const { packId, folder } of packs) folders.set(packId, [...(folders.get(packId) ?? []), folder]);
  const names = new Map(loadRulePacks().map(pack => [pack.id, pack.name]));
  return [...folders].map(([packId, at]) => {
    const name = names.get(packId) ?? packId;
    return at.includes("") ? name : `${name} in ${at.join(", ")}`;
  });
}

/**
 * The rule settings the repository's own lint config gives the rules of packs `packIds`, owned by the repository,
 * read from its files now. None for a rule the repository has a setting of its own for in that folder (one the person
 * made in the dashboard's review stands).
 */
function lintConfigSettings(database: RerailDatabase, home: string, root: string, repositoryKey: RepositoryKey, packIds: readonly string[]): RuleSetting[] {
  const own = new Set(database.select({ folder: ruleSettings.folder, targetId: ruleSettings.targetId }).from(ruleSettings).where(and(
    eq(ruleSettings.ownerKind, "repository"),
    eq(ruleSettings.ownerId, repositoryKey),
    eq(ruleSettings.targetKind, "rule"),
  )).all().map(row => `${row.folder}\0${row.targetId}`));
  return readRepositoryLintConfig(root, loadRuleCatalog(home), packIds).settings
    .filter(setting => !own.has(`${setting.folder}\0${setting.ruleId}`))
    .map(({ ruleId, folder, on, config }) => ({ owner: { kind: "repository", repositoryKey }, folder, targetKind: "rule", targetId: ruleId, on, mode: null, config }));
}

/** Names of the other set-up repositories Rerail is on in: as a whole, or in one of their checkouts. */
function otherStartedRepositories(database: RerailDatabase, repositoryKey: RepositoryKey): string[] {
  return listRepositories(database)
    .filter(known => known.repositoryKey !== repositoryKey && known.configuredAt !== null
      && (known.started || known.checkouts.some(checkout => checkout.started === true)))
    .map(known => known.displayName);
}

/**
 * Where set-up finds the repository at `root`, read-only, like `workspaceStartState`. `review-offered`: set-up offered
 * the dashboard's review and the person has changed nothing there yet. `awaiting-start`: set-up offered the review,
 * the person set the repository up there (the first change adds the kept packs), and nobody has started or stopped it
 * yet. Either way set-up goes on to its confirm question. A repository set up any other way and never started
 * (`rerail rules add`, a coding agent, `rules import`) is `set-up`: `rerail start` just starts it.
 */
function setUpState(root: string, databasePath: string): "not-set-up" | "review-offered" | "awaiting-start" | "set-up" {
  if (!existsSync(databasePath)) return "not-set-up";
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    const checkout = lookupCheckout(database, root);
    if (checkout === undefined) return "not-set-up";
    if (!checkoutStatus(database, checkout).setUp) return setupReviewOffered(database, checkout.repositoryKey) ? "review-offered" : "not-set-up";
    return awaitingStartAfterReview(database, checkout.repositoryKey) ? "awaiting-start" : "set-up";
  } finally {
    database.$client.close();
  }
}

/** The dashboard scope of a repository that is set up: its recorded checkout, or the one it resolves to. */
function dashboardScope(run: Run, repository: Repository): RulesDashboardScope {
  const checkout = readDatabase(run, database => lookupCheckout(database, repository.root));
  if (checkout === undefined) throw new Error(`Rerail has not recorded ${repository.root}.`);
  return { repositoryKey: checkout.repositoryKey, checkoutPath: checkout.checkoutPath };
}

/** Step 6: what is on, how to try it, the link to manage rules, and the other started repos. */
async function summary(run: Run, repository: Repository, open: boolean): Promise<SetupOutcome> {
  const scope = dashboardScope(run, repository);
  const { packNames, ruleCount, others } = readDatabase(run, database => {
    const checkout = lookupCheckout(database, repository.root);
    return {
      packNames: packLabels(attachedPacks(database, scope.repositoryKey)),
      ruleCount: checkout === undefined ? 0 : onRuleCount(database, checkout),
      others: otherStartedRepositories(database, scope.repositoryKey),
    };
  }) ?? { packNames: [], ruleCount: 0, others: [] };
  const dashboard = await run.request.environment.dashboard(scope, "rules", open);
  say(run.io, renderSummary({
    repoName: repository.name,
    packNames,
    ruleCount,
    dashboard,
    otherStartedRepos: others,
  }, run.io.lane));
  return { status: "started", repositoryName: repository.name, ruleCount };
}

/** Coding agents first: registration checks every config before writing, so a bad one changes nothing in Rerail. */
async function connect(run: Run, repository: Repository | undefined): Promise<readonly Harness[]> {
  const agents = run.request.agents ?? run.request.environment.agentsFound();
  await run.request.environment.connect(agents, repository?.root);
  return agents;
}

/** What starting Rerail adds to the repository. */
type StartWith =
  /** The packs set-up recommended, each at its folder; after the dashboard's review, the packs kept on there instead. */
  | { readonly kind: "recommended"; readonly packs: readonly RulePackRecommendation[] }
  /**
   * Exactly the packs the person chose, replacing any the dashboard's review added. `readRepository`: whether the
   * repository's lint config may be read for them; not when the person said Rerail may not read the repository.
   */
  | { readonly kind: "chosen"; readonly packs: readonly RulePackRecommendation[]; readonly readRepository: boolean };

const recommended = (packs: readonly RulePackRecommendation[]): StartWith => ({ kind: "recommended", packs });
const chosenPacks = (packs: readonly RulePackRecommendation[], readRepository: boolean): StartWith => ({ kind: "chosen", packs, readRepository });

/**
 * Set the repository up and start Rerail in it: every checkout without its own stop. A repository that is not set up
 * adds `startWith`'s packs, each at its folder. One the dashboard's review set up keeps the packs kept on there for
 * recommended packs, and swaps them for chosen ones: until Rerail starts, only the person in the dashboard adds packs
 * to it. The packs' rules take the settings the repository's lint config gives them, unless the person said Rerail may
 * not read the repository. Returns how many rule packs are added, or undefined when the repository was set up
 * otherwise, or started (or stopped) already.
 */
function turnOn(run: Run, repository: Repository, startWith: StartWith): number | undefined {
  const readRepository = startWith.kind === "recommended" || startWith.readRepository;
  const adding = startWith.packs.map(({ pack, folder }) => ({ packId: pack.id, folder }));
  return withCheckout(run, repository, (database, checkout) => {
    const { repositoryKey } = checkout;
    if (!checkoutStatus(database, checkout).setUp) {
      recordSetUp(database, repositoryKey, {
        packs: adding,
        settings: readRepository ? lintConfigSettings(database, packsHome(run), repository.root, repositoryKey, adding.map(({ packId }) => packId)) : [],
      });
    } else if (!awaitingStartAfterReview(database, repositoryKey)) {
      return undefined;
    } else {
      if (startWith.kind === "chosen") {
        for (const { packId, folder } of attachedPacks(database, repositoryKey)) removeAttachment(database, { repositoryKey, folder, sourceKind: "pack", sourceId: packId });
        for (const { packId, folder } of adding) addAttachment(database, { repositoryKey, folder, sourceKind: "pack", sourceId: packId });
      }
      const kept = [...new Set(attachedPacks(database, repositoryKey).map(({ packId }) => packId))];
      if (readRepository) for (const setting of lintConfigSettings(database, packsHome(run), repository.root, repositoryKey, kept)) setRuleSetting(database, setting);
    }
    startSetUpRepository(database, checkout, run.request.repository === true);
    return new Set(attachedPacks(database, repositoryKey).map(({ packId }) => packId)).size;
  });
}

/** Steps 5 and 6. The packs kept on in the dashboard's review win over recommended ones; chosen ones replace them. */
async function start(run: Run, repository: Repository, startWith: StartWith): Promise<SetupOutcome> {
  await connect(run, repository);
  if (turnOn(run, repository, startWith) === undefined) return alreadySetUp(run, repository, true);
  return summary(run, repository, true);
}

/** E4 with no pack: on, without reading any code, and every rule pack off. */
async function pickMyself(run: Run, repository: Repository): Promise<SetupOutcome> {
  const connected = await connect(run, repository);
  if (turnOn(run, repository, chosenPacks([], false)) === undefined) return alreadySetUp(run, repository, true);
  const dashboard = await run.request.environment.dashboard(dashboardScope(run, repository), "rules", true);
  say(run.io, renderPickedMyself({ repoName: repository.name, connected: codingAgents(connected), dashboard }, run.io.lane));
  return { status: "started", repositoryName: repository.name, ruleCount: 0 };
}

/**
 * E4: "Skip, I'll pick packs". Every pack set-up fetched, to pick from, unmatched, since Rerail may not read the
 * repository. The packs picked are installed and added for the whole repository, and its lint config is not read;
 * picking none starts Rerail with none. An answer run (`--packs`) installs what it names, `--packs none` nothing.
 */
async function pickPacksMyself(run: Run, repository: Repository): Promise<SetupOutcome> {
  const { packs, packsError } = run.request;
  if (packsError === undefined && packs !== undefined) return startPicked(run, repository, packs);
  const offered = discoveryOffering(await discoverPacks(undefined, run.request.environment.packs));
  // Nothing to pick, and nothing went wrong: on with none. When packs could not be fetched or installed, the person
  // is told why and asked, rather than started without the packs they wanted.
  if (offered.available.length === 0 && offered.unavailable.length === 0 && packsError === undefined) return pickMyself(run, repository);
  const question = pickPacksQuestion({
    repoName: repository.name,
    repoPath: repository.root,
    available: offered.available,
    unavailable: offered.unavailable,
    ...(run.request.setupCommand === undefined ? {} : { setupCommand: run.request.setupCommand }),
  }, run.io.lane);
  if (packsError !== undefined) say(run.io, packsError);
  // Only "None for now" when nothing could be fetched; the keys follow the choices the question shows.
  const keys = offered.available.length === 0 ? (["none"] as const) : PICK_PACKS_ANSWERS;
  for (;;) {
    const picked = await choose(run, question, keys);
    if (picked === undefined) return halted(run.io);
    if (picked === "none") return pickMyself(run, repository);
    const chosen = await askPacks(run.io, offered.available);
    if (chosen === undefined) return halted(run.io);
    const problem = await installAnswered(run, chosen);
    if (problem === undefined) return start(run, repository, chosenPacks(chosenRecommendations(run, repository, chosen, false), false));
    say(run.io, problem);
  }
}

/**
 * An answer run's `--packs` for "I'll pick rule packs myself": installed and started with, each added for the whole
 * repository. When one cannot be installed, nothing is changed and the pick question is asked again, with why.
 */
async function startPicked(run: Run, repository: Repository, chosen: readonly PackVersion[]): Promise<SetupOutcome> {
  if (chosen.length === 0) return pickMyself(run, repository);
  const problem = await installAnswered(run, chosen);
  if (problem !== undefined) return pickPacksMyself(withPacksError(run, problem), repository);
  return start(run, repository, chosenPacks(chosenRecommendations(run, repository, chosen, false), false));
}

/** The run as an answer whose rule packs could not be installed: no packs, and why, so the question is asked again. */
function withPacksError(run: Run, problem: string): Run {
  const { packs: _packs, ...request } = run.request;
  return { ...run, request: { ...request, packsError: problem } };
}

/**
 * Installs the packs an answer names (`installPackVersions`), refusing first to replace an installed pack with an
 * older version: set-up only offers the newer of the two, so an older one did not come from its question, and
 * downgrading would change the rules of every repository on this computer. `rerail packs add` can.
 */
async function installAnswered(run: Run, packs: readonly PackVersion[]): Promise<string | undefined> {
  const older = packs.flatMap(pack => {
    const installed = installedPackVersion(pack.id, packsHome(run));
    return installed !== null && semver.valid(installed) !== null && semver.lt(pack.version, installed) ? [`${pack.id}@${pack.version} is older than the installed ${installed}`] : [];
  });
  if (older.length > 0) return `${older.join("; ")}: set-up does not downgrade a pack every repository uses; rerail packs add does.`;
  return await installPackVersions(packs, run.request.environment.packs);
}

/** "Not now" at the confirm question: nothing installed, connected or turned on. */
function notNow(run: Run): SetupOutcome {
  say(run.io, renderNotNow(run.io.lane));
  return { status: "not-now" };
}

/** E5: installed and connected, on in no repo. */
async function skip(run: Run): Promise<SetupOutcome> {
  const connected = await connect(run, undefined);
  say(run.io, renderSkipped({ connected: codingAgents(connected) }, run.io.lane));
  return { status: "skipped" };
}

/**
 * Set-up ran again where Rerail was set up before: say how things are, changing nothing. Coding agents
 * are connected again only when named (`--agents`) and not just `connected`, and the browser is not opened a second time.
 */
async function alreadySetUp(run: Run, repository: Repository, connected = false): Promise<SetupOutcome> {
  if (!connected && run.request.agents !== undefined) await connect(run, repository);
  if (workspaceStartState(repository.root, run.databasePath) === "started") return summary(run, repository, false);
  say(run.io, renderStoppedRepo({ repoName: repository.name }, run.io.lane));
  return { status: "stopped", repositoryName: repository.name, ruleCount: 0 };
}

/** A rule pack set-up offers to install and add, at the version it installs. */
interface Offer {
  readonly recommendation: RulePackRecommendation;
  readonly version: string;
  /** The version installed now, if any; installing another replaces it for every repository on this computer. */
  readonly installed: string | null;
}

/** What the confirm question shows: the packs it offers, every pack "Choose" lists, and where their rules come from. */
interface Offering {
  readonly offers: readonly Offer[];
  readonly available: readonly AvailablePack[];
  /** Packs set-up could not fetch or check, and why. */
  readonly unavailable: readonly string[];
  /** The catalog the offered packs' rules come from, for the lint config's settings. */
  readonly catalog: RuleCatalog;
}

const onRules = (pack: RulePack): number => pack.rules.filter(rule => rule.on).length;

/** What discovery found: the packs that fit, offered at the version fetched. */
function discoveryOffering(discovery: PackDiscovery): Offering {
  return {
    offers: discovery.recommended.flatMap(found => found.match === undefined ? [] : [{
      recommendation: { pack: found.pack, ruleCount: found.ruleCount, reason: found.match.reason, folder: found.match.folder },
      version: found.version,
      installed: found.installed,
    }]),
    available: discovery.available.map(found => ({ id: found.pack.id, version: found.version, name: found.pack.name, ...(found.match === undefined ? {} : { reason: found.match.reason }) })),
    unavailable: discovery.problems,
    catalog: discovery.catalog,
  };
}

/**
 * The installed packs `chosen` names, each added where its match puts it, or in the whole repository when it does not
 * fit or `readRepository` is false (the repository is then not read). `installPackVersions` has made sure each is
 * installed and loaded at that version; throws, naming them, when one is not.
 */
function chosenOffering(run: Run, repository: Repository, chosen: readonly PackVersion[], readRepository = true): Offering {
  const catalog = loadRuleCatalog(packsHome(run));
  const missing = chosen.filter(({ id, version }) => catalog.packs.get(id)?.version !== version);
  if (missing.length > 0) throw new Error(`These rule packs are not installed: ${missing.map(pack => `${pack.id}@${pack.version}`).join(", ")}.`);
  const packs = chosen.map(({ id }) => catalog.packs.get(id)!);
  const matched = new Map((readRepository ? recommendRulePacks(repository.root, packs) : []).map(found => [found.pack.id, found]));
  const offers = packs.map(pack => ({
    recommendation: matched.get(pack.id) ?? { pack, ruleCount: onRules(pack), reason: "you chose it", folder: "" },
    version: pack.version!,
    installed: pack.version,
  }));
  return {
    offers,
    available: offers.map(offer => ({ id: offer.recommendation.pack.id, version: offer.version, name: offer.recommendation.pack.name, reason: offer.recommendation.reason })),
    unavailable: [],
    catalog,
  };
}

const offeredVersions = (offering: Offering): PackVersion[] => offering.offers.map(offer => ({ id: offer.recommendation.pack.id, version: offer.version }));
const recommendationsOf = (offering: Offering): RulePackRecommendation[] => offering.offers.map(offer => offer.recommendation);
const chosenRecommendations = (run: Run, repository: Repository, chosen: readonly PackVersion[], readRepository: boolean): RulePackRecommendation[] =>
  recommendationsOf(chosenOffering(run, repository, chosen, readRepository));

function packsInput(run: Run, repository: Repository, offering: Offering): RecommendedPacksInput {
  // What the answer will connect: the agents `--agents` named, or every one found.
  const connecting = run.request.agents ?? run.request.environment.agentsFound();
  return {
    repoName: repository.name,
    repoPath: repository.root,
    packs: offering.offers.map(({ recommendation: { pack, ruleCount, reason }, version, installed }) => ({
      id: pack.id,
      name: pack.name,
      ruleCount,
      reason,
      packageName: rulePackPackageName(pack.id),
      version,
      installed,
    })),
    available: offering.available,
    unavailable: offering.unavailable,
    agentsFound: codingAgents(connecting),
    ...(run.request.setupCommand === undefined ? {} : { setupCommand: run.request.setupCommand }),
    lintConfig: readRepositoryLintConfig(repository.root, offering.catalog, offering.offers.map(({ recommendation }) => recommendation.pack.id)),
  };
}

/**
 * Steps 4 and 5: the rule packs that fit, then the confirm question until the person starts Rerail or says not now.
 * Discovery fetches every pack and matches it, installing none. Only an answer installs: an answer run installs
 * exactly the packs its `--packs` names (the versions its question showed, or those the person chose), and never
 * more; asking again after a bad answer installs nothing.
 */
async function confirm(run: Run, repository: Repository, answer: string | undefined): Promise<SetupOutcome> {
  const { agentsError, packsError, packs } = run.request;
  const errors = [agentsError, packsError].filter(error => error !== undefined);
  if (errors.length === 0 && packs !== undefined && (answer === "start" || answer === "choose")) {
    const problem = await installAnswered(run, packs);
    // The question again, with why, as for an answer's coding agents set-up does not know.
    if (problem !== undefined) return confirm(withPacksError(run, problem), repository, undefined);
    if (answer === "choose") return start(run, repository, chosenPacks(chosenRecommendations(run, repository, packs, true), true));
    return start(run, repository, recommended(recommendationsOf(chosenOffering(run, repository, packs))));
  }
  // An answer without `--packs` answers a question that offered no rule pack.
  if (errors.length === 0 && answer === "start") return start(run, repository, recommended([]));
  const offering = discoveryOffering(await discoverPacks(repository.root, run.request.environment.packs));
  const question = recommendedPacksQuestion(packsInput(run, repository, offering), run.io.lane);
  // A bad answer's coding agents or rule packs: the same question again, as the terminal asks again after one.
  for (const error of errors) say(run.io, error);
  return confirmLoop(run, repository, offering, question);
}

/**
 * The confirm question until the person answers it; each answer that installs installs first, then starts. Packs that
 * cannot be installed change nothing: the person is told why and asked again.
 */
async function confirmLoop(run: Run, repository: Repository, offering: Offering, question: QuestionForm): Promise<SetupOutcome> {
  for (;;) {
    const picked = await choose(run, question, confirmPacksAnswers(offering.available));
    if (picked === undefined) return halted(run.io);
    if (picked === "not-now") return notNow(run);
    if (picked === "choose") {
      const chosen = await askPacks(run.io, offering.available);
      if (chosen === undefined) return halted(run.io);
      const problem = await installAnswered(run, chosen);
      if (problem === undefined) return start(run, repository, chosenPacks(chosenRecommendations(run, repository, chosen, true), true));
      say(run.io, problem);
      continue;
    }
    // "Yes" installs the packs the question offered.
    const problem = await installAnswered(run, offeredVersions(offering));
    if (problem === undefined) return start(run, repository, recommended(recommendationsOf(offering)));
    say(run.io, problem);
  }
}

/**
 * Steps 3 to 6 for one repository. `chosen`: the person named it, which is the go-ahead to read it. The home folder,
 * named or typed, is no repository: set-up says so and asks which project instead.
 */
async function setUpOneRepository(run: Run, repository: Repository, chosen: boolean, answer: string | undefined): Promise<SetupOutcome> {
  if (isHomeFolder(repository.root)) {
    say(run.io, HOME_FOLDER_REFUSED);
    return noRepository(run);
  }
  if (answer === "not-now") return notNow(run);
  const state = setUpState(repository.root, run.databasePath);
  if (state === "set-up") return alreadySetUp(run, repository);
  // An earlier set-up offered the dashboard's review here, so it had the go-ahead to read: never back to the read-repo question.
  const reviewing = state === "awaiting-start" || state === "review-offered";
  const { agentsError, ...request } = run.request;
  if (answer === "pick-myself") {
    if (agentsError === undefined) return pickPacksMyself(run, repository);
    // "Rerail reads no code": a bad list asks again the question that offers it, and leaves the repository unread.
    say(run.io, agentsError);
    return askToRead({ ...run, request }, repository);
  }
  if (agentsError !== undefined) return confirm(run, repository, undefined);
  if (chosen || reviewing || answer === "yes" || answer === "start" || answer === "choose") return confirm(run, repository, answer);
  return askToRead(run, repository);
}

/** Step 3: may Rerail read the repository? Each answer goes on as `--answer` would. */
async function askToRead(run: Run, repository: Repository): Promise<SetupOutcome> {
  const picked = await choose(run, readRepoQuestion({
    repoName: repository.name,
    repoPath: repository.root,
    ...(run.request.setupCommand === undefined ? {} : { setupCommand: run.request.setupCommand }),
  }), READ_REPO_ANSWERS);
  if (picked === undefined) return halted(run.io);
  if (picked !== "other-folder") return setUpOneRepository(run, repository, false, picked);
  const other = await askFolder(run.io);
  return other === undefined ? halted(run.io) : setUpOneRepository(run, other, true, undefined);
}

/** E3: no repository where set-up ran. Choosing a folder is the go-ahead to read it. */
async function noRepository(run: Run): Promise<SetupOutcome> {
  const picked = await choose(run, noRepoQuestion(run.request.setupCommand === undefined ? {} : { setupCommand: run.request.setupCommand }, run.io.lane), NO_REPO_ANSWERS);
  if (picked === undefined) return halted(run.io);
  // With no repository to turn on in, "I'll pick rule packs myself" ends where skipping does.
  if (picked !== "folder") return skip(run);
  const repository = await askFolder(run.io);
  return repository === undefined ? halted(run.io) : setUpOneRepository(run, repository, true, undefined);
}

/** `--answer` must be one of the answer keys the questions print; callers check it before changing anything. */
export function checkSetupAnswer(answer: string | undefined): string | undefined {
  if (answer === undefined || ANSWERS.includes(answer)) return answer;
  throw new Error(`Unknown answer: ${answer} (expected one of ${ANSWERS.join(", ")})`);
}

/** `rerail setup` after the install: T2b once, then steps 3 to 6 for the repository named or found, or E3 and E5 without one. */
export async function runSetup(io: RepositorySetupIo, request: SetupRequest): Promise<SetupOutcome> {
  const run: Run = {
    io,
    request,
    databasePath: request.databasePath ?? defaultRerailDatabasePath(),
    steps: mainSteps(request.environment, request.repo === undefined),
  };
  const approval = await approveSessions(run, checkSetupAnswer(request.answer), request.repo);
  if ("status" in approval) return approval;
  const { answer } = approval;
  if (answer === "skip") return skip(run);
  if (answer === "not-now") return notNow(run);
  if (request.repo !== undefined) {
    const named = repositoryAt(request.repo);
    // The allow answer's `--repo` is the folder the run was for, which `rerail start` may have found itself: set-up
    // still asks before reading it. Only a folder the person named in this run is the go-ahead.
    if (named !== undefined) return setUpOneRepository(run, named, request.answer !== "allow", answer);
    say(io, renderFolderNotFound({ folder: request.repo }, io.lane));
    return noRepository(run);
  }
  const here = repositoryHere(request.cwd);
  if (here !== undefined) return setUpOneRepository(run, here, false, answer);
  return answer === "pick-myself" ? skip(run) : noRepository(run);
}

/**
 * First-time set-up of the repository at `workspace`, run by `rerail start` where its repository is not set up:
 * steps 3 to 6. Unlike `rerail setup`, the folder is the repository even outside Git.
 */
export async function runRepositorySetup(workspace: string, io: RepositorySetupIo, options: RepositorySetupOptions): Promise<RepositoryStartReport> {
  const run: Run = {
    io,
    request: { ...options, cwd: workspace },
    databasePath: options.databasePath ?? defaultRerailDatabasePath(),
    steps: mainSteps(options.environment, true),
  };
  const checked = checkSetupAnswer(options.answer);
  const repository = repositoryAt(workspace);
  if (repository === undefined) throw new Error(`There is no folder at ${workspace}. Nothing was read.`);
  if (isHomeFolder(repository.root)) throw new Error(HOME_FOLDER_REFUSED);
  // Read before set-up writes anything: stopped, or started but not set up.
  const previous = workspaceStartState(repository.root, run.databasePath);
  // The allow answer carries the folder on, as one set-up still asks before reading (`runSetup`).
  const approval = await approveSessions(run, checked, repository.root);
  const outcome = "status" in approval ? approval : await setUpOneRepository(run, repository, false, approval.answer);
  return {
    previous,
    setUpRan: true,
    repositoryName: repository.name,
    ruleCount: "ruleCount" in outcome ? outcome.ruleCount : 0,
    cancelled: outcome.status === "cancelled",
    status: outcome.status,
  };
}

const rulesNoun = (count: number): string => `${count} ${count === 1 ? "rule is" : "rules are"} on`;

/**
 * Start Rerail in the checkout `workspace` is in and the checkouts inside it, or with `repository` in every
 * checkout of its repository. A repository that is not set up runs set-up's questions first.
 */
export async function startRerail(workspace: string, io: RepositorySetupIo, options: RepositorySetupOptions): Promise<RepositoryStartReport> {
  const databasePath = options.databasePath ?? defaultRerailDatabasePath();
  // Read-only: a repository that is not set up gets no identity until the person says yes.
  if (setUpState(workspace, databasePath) !== "set-up") return runRepositorySetup(workspace, io, options);
  const wholeRepository = options.repository === true;
  const database = openRerailDatabase(databasePath);
  let previous: RepositoryStartState;
  let ruleCount: number;
  try {
    const checkout = recordCheckout(database, workspace);
    previous = stateOf(checkoutStatus(database, checkout));
    setStarted(database, checkout, true, wholeRepository);
    ruleCount = onRuleCount(database, checkout);
  } finally {
    database.$client.close();
  }
  const name = checkoutName(workspace, wholeRepository);
  io.say(previous === "started"
    ? `● Rerail is already on in ${name}. ${rulesNoun(ruleCount)}.`
    : `● Rerail is on in ${name}. ${rulesNoun(ruleCount)}, as you left them.`);
  return { previous, setUpRan: false, repositoryName: name, ruleCount, cancelled: false, status: "started" };
}

/**
 * `--json`: one run as one `SetupStep`, in the json lane: the agent lane's side effects and answers, the
 * person's text as `screen`, and the question it stopped at as data. A failure is a step too, so the
 * caller's stdout carries exactly one JSON object.
 */
async function collectStep(run: (io: RepositorySetupIo, onQuestion: (question: SetupQuestion) => void) => Promise<SetupOutcome["status"]>): Promise<SetupStep> {
  const said: string[] = [];
  let question: SetupQuestion | null = null;
  const io: RepositorySetupIo = { lane: "json", say: text => { said.push(text); } };
  try {
    const status = await run(io, asked => { question = asked; });
    const screen = said.join("\n");
    if (status === "waiting" && question !== null) return { step: (question as SetupQuestion).id, screen, question };
    return { step: status === "stopped" ? "stopped" : "done", screen, question: null };
  } catch (error) {
    return { step: "error", screen: [...said, error instanceof Error ? error.message : String(error)].join("\n"), question: null };
  }
}

/** `rerail setup --json`. */
export function runSetupStep(request: SetupRequest): Promise<SetupStep> {
  return collectStep(async (io, onQuestion) => (await runSetup(io, { ...request, onQuestion })).status);
}

/**
 * `rerail start --json`: the set-up's step where the repository is not set up. Otherwise `done` with one line and
 * the rules dashboard's link, as the text path ends, shown without opening a browser.
 */
export function startRerailStep(workspace: string, options: RepositorySetupOptions): Promise<SetupStep> {
  return collectStep(async (io, onQuestion) => {
    const report = await startRerail(workspace, io, { ...options, onQuestion });
    if (report.setUpRan) return report.status;
    const database = openRerailDatabase(options.databasePath ?? defaultRerailDatabasePath(), { readonly: true });
    let checkout: CheckoutContext | undefined;
    try {
      checkout = lookupCheckout(database, workspace);
    } finally {
      database.$client.close();
    }
    if (checkout === undefined) throw new Error(`Rerail has not recorded ${workspace}.`);
    const scope = { repositoryKey: checkout.repositoryKey, checkoutPath: checkout.checkoutPath };
    io.say(`  ${(await options.environment.dashboard(scope, "rules", false)).url}`);
    return report.status;
  });
}
