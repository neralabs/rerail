import { existsSync, statSync } from "node:fs";
import { homedir } from "node:os";
import { resolve } from "node:path";
import { and, eq } from "drizzle-orm";
import type { RulesDashboardScope } from "./api/session-contracts.js";
import { CODING_AGENT_NAMES } from "./coding-agents.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "./database.js";
import { HARNESSES, type Harness } from "./harness-registration.js";
import type { CheckoutContext, RepositoryKey, RepositoryStartState } from "./repository-model.js";
import { repositoryIdentityLocation } from "./repository-identity.js";
import { listRepositories, lookupCheckout, recordCheckout } from "./repository-registry.js";
import {
  checkoutName,
  checkoutStatus,
  onRuleCount,
  setStarted,
  setUpAndNeverStarted,
  startSetUpRepository,
  stateOf,
  workspaceStartState,
  type RepositorySetupIo,
} from "./repository-start.js";
import { removeAttachment, setUpRepository as configureRepository } from "./rule-configuration.js";
import { loadRulePacks, recommendRulePacks, type RulePackRecommendation } from "./rule-packs.js";
import { ruleAttachments } from "./schema.js";
import {
  CONFIRM_PACKS_ANSWERS,
  NO_REPO_ANSWERS,
  noRepoQuestion,
  packReviewQuestion,
  READ_REPO_ANSWERS,
  readRepoQuestion,
  recommendedPacksQuestion,
  renderAgentsPrompt,
  renderAnswerPrompt,
  renderFolderNotFound,
  renderFolderPrompt,
  renderPickedMyself,
  renderQuestion,
  renderSkipped,
  renderStarted,
  renderStoppedRepo,
  renderSummary,
  setupQuestion,
  type CodingAgent,
  type DashboardLink,
  type QuestionForm,
  type RecommendedPacksInput,
} from "./setup-screens.js";
import type { SetupQuestion, SetupStep } from "./setup-step.js";

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

export type DashboardPage = "rules" | "review";

/** What set-up needs from the machine; the CLI builds it (`setupEnvironment`), tests fake it. */
export interface SetupEnvironment {
  /** Coding agents installed on this computer. */
  agentsFound(): readonly Harness[];
  /** Connect these coding agents. `repoRoot` is the repository the person said yes to, if any. */
  connect(agents: readonly Harness[], repoRoot: string | undefined): Promise<void>;
  /** Coding agents Rerail is connected to now, for a re-run that connects nothing. */
  connectedAgents(): Promise<readonly Harness[]>;
  /**
   * A dashboard page scoped to `scope`'s repository and checkout; the link is always returned. `open`: try a
   * browser (E6 says why none opened). False when an earlier run already opened it, so a re-run shows the link alone.
   */
  dashboard(scope: RulesDashboardScope, page: DashboardPage, open: boolean): Promise<DashboardLink>;
}

export interface RepositorySetupOptions {
  databasePath?: string;
  /** `--agents`: the coding agents to connect. Those found on this computer when absent. */
  agents?: readonly Harness[];
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

/** How a set-up run ended. `waiting`: the agent lane stopped at a question; `cancelled`: terminal input ended. */
export type SetupOutcome =
  | { status: "waiting" | "cancelled" | "skipped" }
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

const ANSWERS: readonly string[] = [...new Set<string>([...READ_REPO_ANSWERS, ...CONFIRM_PACKS_ANSWERS, ...NO_REPO_ANSWERS])];

interface Repository {
  root: string;
  name: string;
}

interface Run {
  readonly io: RepositorySetupIo;
  readonly request: SetupRequest;
  readonly databasePath: string;
}

function say(io: RepositorySetupIo, screen: string): void {
  io.say(screen.replace(/\n$/u, ""));
}

function codingAgents(ids: readonly Harness[]): CodingAgent[] {
  return ids.map(id => ({ id, name: CODING_AGENT_NAMES[id] }));
}

function halted(io: RepositorySetupIo): SetupOutcome {
  return { status: io.ask === undefined ? "waiting" : "cancelled" };
}

/** `""` takes the first answer (the cursor is on it); `2` the second; an answer key itself also works. */
function pickAnswer<K extends string>(line: string, keys: readonly K[]): K | undefined {
  const text = line.trim().toLowerCase();
  if (text === "") return keys[0];
  if (/^\d+$/u.test(text)) return keys[Number.parseInt(text, 10) - 1];
  return keys.find(key => key === text);
}

/** Say the question; in the terminal, wait for a valid answer. Undefined: the agent lane stops here, or input ended. */
async function choose<K extends string>(run: Run, form: QuestionForm, keys: readonly K[]): Promise<K | undefined> {
  const { io } = run;
  say(io, renderQuestion(form, io.lane));
  if (io.lane === "json") run.request.onQuestion?.(setupQuestion(form, run.request.nextArgs ?? DEFAULT_NEXT_ARGS));
  if (io.ask === undefined) return undefined;
  for (;;) {
    const line = await io.ask(renderAnswerPrompt());
    if (line === undefined) return undefined;
    const picked = pickAnswer(line, keys);
    if (picked !== undefined) return picked;
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

/** The Git repository set-up ran in, if any. A plain folder, or the home folder, is no repository to offer. */
function repositoryHere(cwd: string): Repository | undefined {
  const location = repositoryIdentityLocation(cwd);
  if (!location.git || resolve(location.workspaceRoot) === resolve(homedir())) return undefined;
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

async function askAgents(io: RepositorySetupIo, found: readonly Harness[]): Promise<Harness[] | undefined> {
  if (io.ask === undefined) return undefined;
  for (;;) {
    const line = await io.ask(renderAgentsPrompt({ found: codingAgents(found) }));
    if (line === undefined) return undefined;
    try {
      return parseCodingAgents(line);
    } catch (error) {
      say(io, error instanceof Error ? error.message : String(error));
    }
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

/** The database for set-up's writes, with the repository's checkout recorded: only after the person said yes. */
function withCheckout<T>(run: Run, repository: Repository, action: (database: RerailDatabase, checkout: CheckoutContext) => T): T {
  const database = openRerailDatabase(run.databasePath);
  try {
    return action(database, recordCheckout(database, repository.root));
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

/** Names of the other set-up repositories Rerail is on in: as a whole, or in one of their checkouts. */
function otherStartedRepositories(database: RerailDatabase, repositoryKey: RepositoryKey): string[] {
  return listRepositories(database)
    .filter(known => known.repositoryKey !== repositoryKey && known.configuredAt !== null
      && (known.started || known.checkouts.some(checkout => checkout.started === true)))
    .map(known => known.displayName);
}

/**
 * Where set-up finds the repository at `root`, read-only, like `workspaceStartState`. `awaiting-start`: the
 * dashboard's review set it up (the first change there adds the kept packs) and nobody has started or stopped it
 * yet, so set-up goes on to its confirm question.
 */
function setUpState(root: string, databasePath: string): "not-set-up" | "awaiting-start" | "set-up" {
  if (!existsSync(databasePath)) return "not-set-up";
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    const checkout = lookupCheckout(database, root);
    if (checkout === undefined || !checkoutStatus(database, checkout).setUp) return "not-set-up";
    return setUpAndNeverStarted(database, checkout.repositoryKey) ? "awaiting-start" : "set-up";
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

/** Step 6: what is on, the other started repos, the coding agents, and the link to manage rules. */
async function summary(run: Run, repository: Repository, connected: readonly Harness[], open: boolean): Promise<SetupOutcome> {
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
    connected: codingAgents(connected),
    dashboard,
    otherStartedRepos: others,
  }, run.io.lane));
  return { status: "started", repositoryName: repository.name, ruleCount };
}

/** Coding agents first: registration checks every config before writing, so a bad one changes nothing in Rerail. */
async function connect(run: Run, repository: Repository | undefined, chosen?: readonly Harness[]): Promise<readonly Harness[]> {
  const agents = chosen ?? run.request.agents ?? run.request.environment.agentsFound();
  await run.request.environment.connect(agents, repository?.root);
  return agents;
}

/**
 * Set the repository up and start Rerail in this checkout. A repository that is not set up adds `packs`, each at
 * its folder (none for "I'll pick rule packs myself"). One the dashboard's review set up keeps the packs kept on
 * there, or drops every pack for "I'll pick rule packs myself". Returns how many rule packs are added, or undefined
 * when the repository was set up and started (or stopped) already.
 */
function turnOn(run: Run, repository: Repository, packs: readonly RulePackRecommendation[] | "none"): number | undefined {
  return withCheckout(run, repository, (database, checkout) => {
    const { repositoryKey } = checkout;
    if (!checkoutStatus(database, checkout).setUp) {
      configureRepository(database, repositoryKey, {
        packs: packs === "none" ? [] : packs.map(({ pack, folder }) => ({ packId: pack.id, folder })),
      });
    } else if (!setUpAndNeverStarted(database, repositoryKey)) {
      return undefined;
    } else if (packs === "none") {
      for (const { packId, folder } of attachedPacks(database, repositoryKey)) {
        removeAttachment(database, { repositoryKey, folder, sourceKind: "pack", sourceId: packId });
      }
    }
    startSetUpRepository(database, checkout, run.request.repository === true);
    return new Set(attachedPacks(database, repositoryKey).map(({ packId }) => packId)).size;
  });
}

/** Steps 5 and 6. The packs kept on in the dashboard's review win over the recommendation. */
async function start(run: Run, repository: Repository, recommendations: readonly RulePackRecommendation[], agents?: readonly Harness[]): Promise<SetupOutcome> {
  const connected = await connect(run, repository, agents);
  const packCount = turnOn(run, repository, recommendations);
  if (packCount === undefined) return alreadySetUp(run, repository, connected);
  say(run.io, renderStarted({ repoName: repository.name, packCount, connected: codingAgents(connected) }, run.io.lane));
  // The pause is only for reading; Rerail is on either way, so the summary follows even if input ended.
  await run.io.ask?.("");
  return summary(run, repository, connected, true);
}

/** E4: on, without reading any code, and every rule pack off. */
async function pickMyself(run: Run, repository: Repository): Promise<SetupOutcome> {
  const connected = await connect(run, repository);
  if (turnOn(run, repository, "none") === undefined) return alreadySetUp(run, repository, connected);
  const dashboard = await run.request.environment.dashboard(dashboardScope(run, repository), "rules", true);
  say(run.io, renderPickedMyself({ repoName: repository.name, connected: codingAgents(connected), dashboard }, run.io.lane));
  return { status: "started", repositoryName: repository.name, ruleCount: 0 };
}

/** E5: installed and connected, on in no repo. */
async function skip(run: Run): Promise<SetupOutcome> {
  const connected = await connect(run, undefined);
  say(run.io, renderSkipped({ connected: codingAgents(connected) }, run.io.lane));
  return { status: "skipped" };
}

/**
 * Set-up ran again where Rerail was set up before: say how things are, changing nothing. Coding agents
 * are connected again only when named (`--agents`), and the browser is not opened a second time.
 */
async function alreadySetUp(run: Run, repository: Repository, connected?: readonly Harness[]): Promise<SetupOutcome> {
  const agents = connected
    ?? (run.request.agents === undefined ? await run.request.environment.connectedAgents() : await connect(run, repository));
  if (workspaceStartState(repository.root, run.databasePath) === "started") return summary(run, repository, agents, false);
  say(run.io, renderStoppedRepo({ repoName: repository.name }, run.io.lane));
  return { status: "stopped", repositoryName: repository.name, ruleCount: 0 };
}

function packsInput(run: Run, repository: Repository, recommendations: readonly RulePackRecommendation[]): RecommendedPacksInput {
  const found = run.request.environment.agentsFound();
  return {
    repoName: repository.name,
    repoPath: repository.root,
    packs: recommendations.map(({ pack, ruleCount, reason }) => ({
      name: pack.name,
      ruleCount,
      reason,
      logRuleCount: pack.rules.filter(rule => rule.mode === "log").length,
    })),
    agentsFound: codingAgents(found),
    agentsNotFound: codingAgents(HARNESSES.filter(id => !found.includes(id))),
    ...(run.request.setupCommand === undefined ? {} : { setupCommand: run.request.setupCommand }),
  };
}

/**
 * The review link, then the confirm question. The checkout is recorded first, so the dashboard can add packs there.
 * `open`: try a browser; false once the review has set the repository up, so a re-run shows the link alone.
 */
async function reviewQuestion(run: Run, repository: Repository, input: RecommendedPacksInput, open: boolean): Promise<QuestionForm> {
  const scope = withCheckout(run, repository, (_database, checkout) => ({ repositoryKey: checkout.repositoryKey, checkoutPath: checkout.checkoutPath }));
  return packReviewQuestion({ ...input, review: await run.request.environment.dashboard(scope, "review", open) }, run.io.lane);
}

/** Steps 4 and 5: the recommended packs, then the confirm question until the person starts Rerail or picks packs themselves. */
async function confirm(run: Run, repository: Repository, answer: string | undefined, reviewed = false): Promise<SetupOutcome> {
  const recommendations = recommendRulePacks(repository.root);
  if (answer === "start" || (answer === "other" && run.request.agents !== undefined)) return start(run, repository, recommendations);
  const input = packsInput(run, repository, recommendations);
  let question = answer === "review" ? await reviewQuestion(run, repository, input, !reviewed) : recommendedPacksQuestion(input, run.io.lane);
  for (;;) {
    const picked = await choose(run, question, CONFIRM_PACKS_ANSWERS);
    if (picked === undefined) return halted(run.io);
    if (picked === "start") return start(run, repository, recommendations);
    if (picked === "pick-myself") return pickMyself(run, repository);
    if (picked === "review") {
      question = await reviewQuestion(run, repository, input, true);
      continue;
    }
    const agents = await askAgents(run.io, run.request.environment.agentsFound());
    if (agents === undefined) return halted(run.io);
    return start(run, repository, recommendations, agents);
  }
}

/** Steps 3 to 6 for one repository. `chosen`: the person named it, which is the go-ahead to read it. */
async function setUpOneRepository(run: Run, repository: Repository, chosen: boolean, answer: string | undefined): Promise<SetupOutcome> {
  const state = setUpState(repository.root, run.databasePath);
  if (state === "set-up") return alreadySetUp(run, repository);
  if (answer === "pick-myself") return pickMyself(run, repository);
  // The review set the repository up: carry on at its confirm question, with the review link but no second browser.
  if (state === "awaiting-start") return confirm(run, repository, answer === "start" || answer === "other" ? answer : "review", true);
  if (chosen || answer === "yes" || answer === "start" || answer === "review" || answer === "other") return confirm(run, repository, answer);
  const picked = await choose(run, readRepoQuestion({
    repoName: repository.name,
    repoPath: repository.root,
    ...(run.request.setupCommand === undefined ? {} : { setupCommand: run.request.setupCommand }),
  }), READ_REPO_ANSWERS);
  if (picked === undefined) return halted(run.io);
  if (picked === "pick-myself") return pickMyself(run, repository);
  if (picked === "yes") return confirm(run, repository, undefined);
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

/** `rerail setup` after the install: steps 3 to 6 for the repository named or found, or E3 and E5 without one. */
export async function runSetup(io: RepositorySetupIo, request: SetupRequest): Promise<SetupOutcome> {
  const run: Run = { io, request, databasePath: request.databasePath ?? defaultRerailDatabasePath() };
  const answer = checkSetupAnswer(request.answer);
  if (answer === "skip") return skip(run);
  if (request.repo !== undefined) {
    const named = repositoryAt(request.repo);
    if (named !== undefined) return setUpOneRepository(run, named, true, answer);
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
  const run: Run = { io, request: { ...options, cwd: workspace }, databasePath: options.databasePath ?? defaultRerailDatabasePath() };
  const answer = checkSetupAnswer(options.answer);
  const repository = repositoryAt(workspace);
  if (repository === undefined) throw new Error(`There is no folder at ${workspace}. Nothing was read.`);
  if (repository.root === resolve(homedir())) throw new Error("Start Rerail inside a repository, not your home folder.");
  // Read before set-up writes anything: stopped, or started but not set up.
  const previous = workspaceStartState(repository.root, run.databasePath);
  const outcome = await setUpOneRepository(run, repository, false, answer);
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

/** `rerail start --json`: the set-up's step where the repository is not set up, `done` with one line otherwise. */
export function startRerailStep(workspace: string, options: RepositorySetupOptions): Promise<SetupStep> {
  return collectStep(async (io, onQuestion) => (await startRerail(workspace, io, { ...options, onQuestion })).status);
}
