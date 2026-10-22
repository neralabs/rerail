/**
 * The dashboard's browser-test scenarios: each builds a throwaway Rerail home, Git checkouts and database under one
 * folder, written through the product's own stores (checkout registry, set-up, attachments, settings, library,
 * session and audit stores), at instants before the seed clock ({@link SEED_NOW} unless the caller passes another).
 * Run by `seed-scenario.ts` in a process of its own: the stores that stamp rows with `Date.now()` (audits, steer
 * receipts) are given the seed clock there.
 */
import { spawnSync } from "node:child_process";
import { mkdirSync, realpathSync, symlinkSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import {
  addAttachment,
  type AuditInput,
  type CheckoutContext,
  checkoutStatus,
  type CodeRule,
  type CodeRuleViolation,
  type EffectiveRule,
  installedPacksPrefix,
  listRepositories,
  listRepositoryRules,
  loadRuleCatalog,
  openRerailDatabase,
  type RerailDatabase,
  type RerailHost,
  type RuleCatalog,
  type RuleMode,
  recordCheckout,
  rulePackPackageName,
  setRuleSetting,
  setUpRepository,
  SqliteAuditStore,
  upsertLibraryRule,
} from "../../src/engine/index.js";
import { setRepositoryStarted, SqliteSessionStore } from "../../src/runtime/index.js";
import { DAY, HOUR, MINUTE, SEED_NOW, type ScenarioManifest, type ScenarioName, type ScenarioRepository } from "./scenario-manifest.js";

const REPOSITORY_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
const HOSTS: readonly RerailHost[] = ["claude-code", "codex", "cursor"];

type RuleSetting = Parameters<typeof setRuleSetting>[1];

/** Runs `action` with `Date.now()` answering `time`: the stores that take no clock stamp their rows with it. */
function atTime<T>(time: number, action: () => T): T {
  const realNow = Date.now;
  Date.now = () => time;
  try {
    return action();
  } finally {
    Date.now = realNow;
  }
}

// ---------------------------------------------------------------------------------------------------------------
// Checkouts and installed packs

interface CheckoutSpec {
  readonly folder: string;
  readonly remote: string;
  readonly files: Readonly<Record<string, string>>;
}

const STOREFRONT_FILES: Readonly<Record<string, string>> = {
  "package.json": `${JSON.stringify({
    name: "storefront",
    private: true,
    dependencies: { next: "15.0.0", react: "19.1.0" },
    devDependencies: { typescript: "5.9.3" },
  }, null, 2)}\n`,
  "src/cart/cart.ts": "export function total(prices: number[]): number {\n  return prices.reduce((sum, price) => sum + price, 0);\n}\n",
  "src/checkout/payment.ts": "export async function pay(amount: number): Promise<void> {\n  console.log('paying', amount);\n}\n",
  "src/api/client.ts": "export async function products(): Promise<unknown> {\n  return (await fetch('/api/products')).json();\n}\n",
  "src/components/product-card.tsx": "export function ProductCard({ name }: { name: string }) {\n  return <article>{name}</article>;\n}\n",
  "scripts/seed.js": "console.log('seeding');\n",
};

const STOREFRONT: CheckoutSpec = { folder: "storefront", remote: "https://github.com/acme/storefront.git", files: STOREFRONT_FILES };

const BILLING: CheckoutSpec = {
  folder: "billing",
  remote: "https://github.com/acme/billing.git",
  files: {
    "package.json": `${JSON.stringify({ name: "billing", private: true, devDependencies: { typescript: "5.9.3" } }, null, 2)}\n`,
    "src/invoice.ts": "export const currency = 'EUR';\n",
    "jobs/reconcile.py": "def reconcile(rows):\n    return [row for row in rows if row]\n",
  },
};

const DOCS_SITE: CheckoutSpec = {
  folder: "docs-site",
  remote: "git@github.com:acme/docs-site.git",
  files: { "README.md": "# Docs site\n", "docs/index.md": "# Welcome\n" },
};

/** A Git checkout with an `origin` remote and the given files; no commit is needed for Rerail to record it. */
function createCheckout(reposRoot: string, spec: CheckoutSpec, gitEnvironment: NodeJS.ProcessEnv): string {
  const checkout = join(reposRoot, spec.folder);
  for (const [path, text] of Object.entries(spec.files)) {
    mkdirSync(dirname(join(checkout, path)), { recursive: true });
    writeFileSync(join(checkout, path), text);
  }
  for (const args of [["init", "-q", "-b", "main"], ["remote", "add", "origin", spec.remote]]) {
    const result = spawnSync("git", args, { cwd: checkout, env: gitEnvironment, encoding: "utf8" });
    if (result.status !== 0) throw new Error(`git ${args.join(" ")} failed in ${checkout}: ${result.stderr}`);
  }
  return checkout;
}

/**
 * Installs this repository's `packs/<id>` into the scenario home the way `rerail packs add <folder>` does: npm links the
 * folder into the prefix's `node_modules` and records it as `file:<folder>` in the prefix's `package.json`.
 */
function installPacks(home: string, packIds: readonly string[]): void {
  const prefix = installedPacksPrefix(home);
  const scope = join(prefix, "node_modules", "@neralabs");
  mkdirSync(scope, { recursive: true });
  const dependencies: Record<string, string> = {};
  for (const id of packIds) {
    const folder = join(REPOSITORY_ROOT, "packs", id);
    symlinkSync(folder, join(scope, `rerail-pack-${id}`), process.platform === "win32" ? "junction" : "dir");
    dependencies[rulePackPackageName(id)] = `file:${folder}`;
  }
  writeFileSync(join(prefix, "package.json"), `${JSON.stringify({ dependencies }, null, 2)}\n`);
}

/** Every pack a scenario adds is installed, plus one more, so the catalog offers a pack nobody added. */
const INSTALLED_PACKS = ["complexity", "javascript", "python", "typescript"] as const;

// ---------------------------------------------------------------------------------------------------------------
// Rules created by you

function regexRule(id: string, mode: RuleMode, description: string, pattern: string, code: string, message: string): CodeRule {
  return {
    format: "rerail.policy/v1",
    id,
    revision: 1,
    mode,
    description,
    scope: { include: ["src/**/*.ts", "src/**/*.tsx"], exclude: [] },
    clauses: [{
      id: "match",
      forbid: { regex: { source: "file.content", pattern } },
      violation: { anchor: "witness.span", code, message },
    }],
  };
}

const MONEY_AS_CENTS = regexRule(
  "storefront-money-as-cents",
  "block",
  "Prices are integer cents end to end; parsing them as floats loses cents on every round trip.",
  "parseFloat\\(",
  "STOREFRONT-MONEY",
  "Keep money in integer cents; parseFloat rounds prices.",
);

const NO_RAW_FETCH = regexRule(
  "storefront-no-raw-fetch",
  "steer",
  "Storefront calls its API through the generated client so the OpenAPI schema stays the contract.",
  "fetch\\(\\s*[\"'`]/api/",
  "STOREFRONT-FETCH",
  "Call the API through the generated client in src/api/generated.",
);

// ---------------------------------------------------------------------------------------------------------------
// History: sessions, audits and steer receipts

/** How many violations of `ruleId` session `index` (0-based, oldest first), `daysAgo` days before today, records. */
interface RuleSchedule {
  readonly ruleId: string;
  readonly count: (index: number, daysAgo: number) => number;
}

interface SessionPlan {
  readonly sessionId: string;
  readonly host: RerailHost;
  readonly startedAt: number;
  /** `null`: still open. */
  readonly endedAt: number | null;
  readonly index: number;
  readonly daysAgo: number;
}

/** A repository change at an instant: set-up, a setting, a new library rule, a stop. */
interface Change {
  readonly at: number;
  readonly apply: (db: RerailDatabase, repositoryKey: string, at: number) => void;
}

const PROMPTS = [
  "Add a discount code field to the cart.",
  "Fix the rounding in the order total.",
  "Show the delivery estimate on the product card.",
  "Refactor the payment step into its own module.",
  "Make the checkout page load faster.",
  "Add tests for the cart reducer.",
];

const TOUCHED_FILES = ["src/cart/cart.ts", "src/checkout/payment.ts", "src/api/client.ts", "src/components/product-card.tsx"];

/** `days` days before the seed day, at `hour` of it (fractions are minutes). */
type DayAt = (days: number, hour: number) => number;

/** At {@link SEED_NOW} the seed day starts at UTC midnight; another clock keeps the same distance to its day's start. */
function dayClock(now: number): DayAt {
  const dayStart = now - (SEED_NOW % DAY);
  return (days, hour) => dayStart - days * DAY + Math.round(hour * HOUR);
}

/**
 * One session per entry of `days` (days before today, oldest first), each at a distinct hour of its day; `today` gives
 * today's sessions their start hours instead (a fraction is minutes), and the last session of the plan is left open
 * when `openLast`.
 */
function sessionPlans(dayAt: DayAt, prefix: string, days: readonly number[], today: readonly number[], openLast: boolean): SessionPlan[] {
  let todayIndex = 0;
  return days.map((daysAgo, index) => {
    const hour = daysAgo === 0 ? today[todayIndex++] ?? 9 : 9 + ((index * 5) % 8);
    const startedAt = dayAt(daysAgo, hour);
    const host = HOSTS[index % HOSTS.length]!;
    const open = openLast && index === days.length - 1;
    return {
      sessionId: `${prefix}-${String(index + 1).padStart(2, "0")}-${host}`,
      host,
      startedAt,
      endedAt: open ? null : startedAt + (25 + ((index * 7) % 45)) * MINUTE,
      index,
      daysAgo,
    };
  });
}

/** The reporting clause of a code rule: the first clause with a `violation`, for the id, code and message it reports. */
function reportingClause(rule: EffectiveRule): { clauseId: string; code: string; message: string } {
  const document = rule.document.rule as unknown as {
    description?: unknown;
    clauses?: readonly { id?: unknown; violation?: { code?: unknown; message?: unknown } }[];
  };
  const clause = document.clauses?.find(entry => entry.violation !== undefined);
  const description = typeof document.description === "string" ? document.description : rule.document.rule.id;
  return {
    clauseId: typeof clause?.id === "string" ? clause.id : "clause",
    code: typeof clause?.violation?.code === "string" ? clause.violation.code : rule.document.rule.id,
    message: typeof clause?.violation?.message === "string" ? clause.violation.message : description,
  };
}

interface HistoryContext {
  readonly db: RerailDatabase;
  readonly catalog: RuleCatalog;
  readonly sessions: SqliteSessionStore;
  readonly audits: SqliteAuditStore;
  readonly repositoryKey: string;
  readonly checkoutPath: string;
}

/**
 * Records one session the way a coding agent's hooks do: it opens, a prompt and a write are recorded, the touched file
 * is audited against the rules on in the checkout at that instant (each violation carries the mode its rule had then),
 * the violations of rules on Steer or Block are steered, and the session ends unless it is the open one.
 */
function recordSession(context: HistoryContext, plan: SessionPlan, schedules: readonly RuleSchedule[], violating: Set<string>): void {
  const { sessions, audits, repositoryKey, checkoutPath } = context;
  const session = { host: plan.host, sessionId: plan.sessionId, cwd: checkoutPath };
  const rules = listRepositoryRules(context.db, context.catalog, repositoryKey, { checkoutPath }).rules
    .filter(rule => rule.document.kind === "code" && rule.on);
  const byId = new Map(rules.map(rule => [rule.document.rule.id, rule]));
  const path = TOUCHED_FILES[plan.index % TOUCHED_FILES.length]!;
  const at = (offset: number): number => plan.startedAt + offset;

  sessions.openSession(session, plan.startedAt, repositoryKey);
  sessions.append({ sessionId: plan.sessionId, eventId: `${plan.sessionId}-start`, kind: "session-lifecycle", observedAt: at(0), payload: { type: "session-initialize" } });
  sessions.append({ sessionId: plan.sessionId, eventId: `${plan.sessionId}-prompt`, kind: "user-prompt", observedAt: at(MINUTE), payload: { text: PROMPTS[plan.index % PROMPTS.length] } });
  sessions.append({ sessionId: plan.sessionId, eventId: `${plan.sessionId}-write`, kind: "call", observedAt: at(4 * MINUTE), payload: { toolName: "write", args: { path } } });

  const violations: CodeRuleViolation[] = [];
  for (const schedule of schedules) {
    const count = schedule.count(plan.index, plan.daysAgo);
    if (count === 0) continue;
    const rule = byId.get(schedule.ruleId);
    if (rule === undefined) throw new Error(`${plan.sessionId}: ${schedule.ruleId} is not on in ${repositoryKey} at ${new Date(plan.startedAt).toISOString()}`);
    const { clauseId, code, message } = reportingClause(rule);
    for (let ordinal = 0; ordinal < count; ordinal += 1) {
      const line = 3 + plan.index * 4 + ordinal * 2;
      violations.push({
        id: `sha256:${createHash("sha256").update(`${plan.sessionId}\0${schedule.ruleId}\0${ordinal}`).digest("hex")}`,
        ruleId: schedule.ruleId,
        clauseId,
        code,
        mode: rule.mode,
        message,
        path,
        span: { startByte: line * 40, endByte: line * 40 + 12, startLine: line, startColumn: 3, endLine: line, endColumn: 15 },
        captures: {},
        variables: {},
      });
    }
    violating.add(schedule.ruleId);
  }

  const provenance: NonNullable<AuditInput["ruleProvenance"]> = rules.map(rule => ({
    ruleId: rule.document.rule.id,
    revision: rule.revision,
    payloadHash: rule.payloadHash,
    source: { ...rule.source },
    repositoryKey,
    checkoutPath,
    placement: [{
      folder: rule.attachmentFolder, on: rule.on, mode: rule.mode, onFrom: rule.decidedBy.on, modeFrom: rule.decidedBy.mode,
      suppressions: rule.suppressions, suppressionsFrom: rule.decidedBy.suppressions,
    }],
  }));
  const auditAt = at(5 * MINUTE);
  const auditId = atTime(auditAt, () => audits.start({
    sessionId: plan.sessionId,
    workspaceRoot: checkoutPath,
    scope: "touched-files",
    mode: "steer",
    revision: 1,
    catalogRevision: context.catalog.digest,
    ruleRevisions: provenance.map(({ ruleId, revision }) => ({ ruleId, revision })),
    paths: [path],
    ruleProvenance: provenance,
  }));
  atTime(auditAt + 2_000, () => audits.finish(auditId, { status: "completed", violations }));

  const steered = [...new Set(violations.filter(violation => violation.mode !== "log").map(violation => violation.ruleId))];
  if (steered.length > 0) atTime(auditAt + 3_000, () => sessions.recordSteer(session, `${plan.sessionId}-steer`, steered));
  sessions.append({ sessionId: plan.sessionId, eventId: `${plan.sessionId}-turn`, kind: "turn-end", observedAt: at(6 * MINUTE), payload: { turn: 1 } });
  if (plan.endedAt !== null) sessions.closeSession(plan.sessionId, plan.endedAt);
}

// ---------------------------------------------------------------------------------------------------------------
// Repositories

interface RepositoryPlan {
  readonly checkout: CheckoutSpec;
  /** When Rerail first recorded the checkout. */
  readonly recordedAt: number;
  readonly changes: readonly Change[];
  readonly sessions: readonly SessionPlan[];
  readonly schedules: readonly RuleSchedule[];
}

function setting(repositoryKey: string, targetId: string, on: boolean | null, mode: RuleMode | null): RuleSetting {
  return { owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId, on, mode };
}

/** Set-up with `packIds` added to the whole repository and `settings` (rule id → on, mode), then started. */
function setUpAndStart(packIds: readonly string[], settings: readonly [string, boolean | null, RuleMode | null][] = []): Change["apply"] {
  return (db, repositoryKey, at) => {
    setUpRepository(db, repositoryKey, {
      packs: packIds.map(packId => ({ packId, folder: "" })),
      settings: settings.map(([id, on, mode]) => setting(repositoryKey, id, on, mode)),
    }, at);
    setRepositoryStarted(db, repositoryKey, true, at);
  };
}

/** A rule created by you, stored in the library and added to the whole repository. */
function addLibraryRule(rule: CodeRule, catalog: () => RuleCatalog): Change["apply"] {
  return (db, repositoryKey, at) => {
    upsertLibraryRule(db, catalog(), rule, "code", { now: at, createdAt: at });
    addAttachment(db, { repositoryKey, folder: "", sourceKind: "library_rule", sourceId: rule.id }, at);
  };
}

function plansFor(scenario: ScenarioName, catalog: () => RuleCatalog, now: number): RepositoryPlan[] {
  const dayAt = dayClock(now);
  const noSessions = { sessions: [], schedules: [] };
  switch (scenario) {
    case "no-packs":
      return [{ checkout: { ...STOREFRONT, files: { "README.md": "# Storefront\n" } }, recordedAt: now - 30 * MINUTE, changes: [{ at: now - 30 * MINUTE, apply: setUpAndStart([]) }], ...noSessions }];
    case "packs-no-session":
      return [{ checkout: STOREFRONT, recordedAt: now - HOUR, changes: [{ at: now - HOUR, apply: setUpAndStart(["typescript", "javascript"]) }], ...noSessions }];
    case "not-set-up":
      // `rerail start` before set-up: started, but nothing runs until the repository is set up.
      return [{
        checkout: STOREFRONT,
        recordedAt: now - 20 * MINUTE,
        changes: [{ at: now - 20 * MINUTE, apply: (db, repositoryKey, at) => setRepositoryStarted(db, repositoryKey, true, at) }],
        ...noSessions,
      }];
    case "stopped":
      return [{
        checkout: STOREFRONT,
        recordedAt: dayAt(10, 9),
        changes: [
          { at: dayAt(10, 9), apply: setUpAndStart(["typescript", "javascript"], [["eslint-no-console", true, "log"]]) },
          { at: dayAt(2, 17), apply: (db, repositoryKey, at) => setRepositoryStarted(db, repositoryKey, false, at) },
        ],
        sessions: sessionPlans(dayAt, "session", [5, 3], [], false),
        schedules: [
          { ruleId: "eslint-no-console", count: () => 2 },
          { ruleId: "typescript-eslint-no-unused-expressions", count: () => 1 },
        ],
      }];
    case "mature":
      return [{
        checkout: STOREFRONT,
        recordedAt: dayAt(35, 9),
        changes: [
          {
            at: dayAt(35, 9),
            apply: setUpAndStart(["typescript", "javascript", "complexity"], [
              ["eslint-no-console", true, "log"],
              ["typescript-eslint-no-non-null-assertion", true, "log"],
              ["complexity-cyclomatic-typescript", null, "log"],
              ["eslint-no-debugger", null, "block"],
            ]),
          },
          { at: dayAt(30, 10.5), apply: addLibraryRule(MONEY_AS_CENTS, catalog) },
          { at: dayAt(25, 11), apply: (db, repositoryKey, at) => setRuleSetting(db, setting(repositoryKey, "eslint-eqeqeq", true, "block"), at) },
          // Logged for three weeks, then steered: its violations carry the mode they were recorded in.
          {
            at: dayAt(14, 12),
            apply: (db, repositoryKey, at) => setRuleSetting(db, setting(repositoryKey, "typescript-eslint-no-non-null-assertion", true, "steer"), at),
          },
          { at: dayAt(8, 16), apply: addLibraryRule(NO_RAW_FETCH, catalog) },
        ],
        sessions: sessionPlans(
          dayAt,
          "session",
          [34, 33, 31, 29, 28, 26, 24, 23, 21, 20, 18, 16, 15, 13, 12, 10, 9, 8, 6, 5, 4, 3, 2, 2, 1, 1, 0, 0, 0, 0],
          [8, 10, 13, 14 + 20 / 60],
          true,
        ),
        schedules: [
          { ruleId: "eslint-no-console", count: index => 1 + (index % 2) },
          { ruleId: "complexity-cyclomatic-typescript", count: index => (index % 3 === 0 ? 1 : 0) },
          { ruleId: "typescript-eslint-no-non-null-assertion", count: index => (index % 2 === 1 ? 1 : 0) },
          { ruleId: "typescript-eslint-no-empty-object-type", count: (_, daysAgo) => (daysAgo >= 31 ? 1 : 0) },
          { ruleId: MONEY_AS_CENTS.id, count: (index, daysAgo) => (daysAgo <= 29 && index % 4 === 0 ? 1 : 0) },
          { ruleId: "eslint-eqeqeq", count: (index, daysAgo) => (daysAgo <= 20 && index % 5 === 1 ? 2 : 0) },
          { ruleId: "typescript-eslint-no-unused-expressions", count: (index, daysAgo) => (daysAgo <= 6 && index % 2 === 0 ? 1 : 0) },
          { ruleId: NO_RAW_FETCH.id, count: (index, daysAgo) => (daysAgo <= 6 && index % 3 !== 0 ? 1 : 0) },
          { ruleId: "eslint-no-debugger", count: (_, daysAgo) => (daysAgo === 0 ? 1 : 0) },
        ],
      }];
    case "multi-repo":
      return [
        {
          checkout: STOREFRONT,
          recordedAt: dayAt(21, 9),
          changes: [{ at: dayAt(21, 9), apply: setUpAndStart(["typescript", "javascript"], [["eslint-no-console", true, "log"]]) }],
          sessions: sessionPlans(dayAt, "session", [20, 17, 14, 11, 9, 6, 4, 2, 1, 0], [14 + 30 / 60], true),
          schedules: [
            { ruleId: "eslint-no-console", count: () => 1 },
            { ruleId: "typescript-eslint-no-unused-expressions", count: index => (index % 2 === 0 ? 1 : 0) },
            { ruleId: "eslint-no-debugger", count: index => (index % 3 === 0 ? 1 : 0) },
          ],
        },
        { checkout: BILLING, recordedAt: dayAt(3, 11), changes: [{ at: dayAt(3, 11), apply: setUpAndStart(["typescript", "complexity"]) }], ...noSessions },
        // Recorded by a session hook, never set up or started.
        { checkout: DOCS_SITE, recordedAt: dayAt(2, 16), changes: [], ...noSessions },
      ];
  }
}

function stateOf(db: RerailDatabase, checkout: CheckoutContext): ScenarioRepository["state"] {
  const status = checkoutStatus(db, checkout);
  if (!status.started) return "stopped";
  return status.setUp ? "started" : "not-set-up";
}

/** Seeds `scenario` under `folder` (created; must be empty or absent) as of `now`, and describes what it holds. */
export function seedScenario(scenario: ScenarioName, folder: string, now: number = SEED_NOW): ScenarioManifest {
  mkdirSync(folder, { recursive: true });
  const root = realpathSync(folder);
  const rerailHome = join(root, "home");
  const userHome = join(root, "user");
  const reposRoot = join(root, "repos");
  for (const directory of [rerailHome, userHome, reposRoot]) mkdirSync(directory, { recursive: true });
  if (resolve(process.env.RERAIL_HOME ?? "") !== rerailHome) throw new Error(`seedScenario needs RERAIL_HOME=${rerailHome} in its environment`);
  installPacks(rerailHome, INSTALLED_PACKS);
  const catalog = (): RuleCatalog => loadRuleCatalog(rerailHome);
  const databasePath = join(rerailHome, "rerail.sqlite");
  const gitEnvironment = { ...process.env, HOME: userHome, GIT_CONFIG_NOSYSTEM: "1", GIT_CONFIG_GLOBAL: join(userHome, ".gitconfig") };

  const db = openRerailDatabase(databasePath);
  const sessions = new SqliteSessionStore(databasePath);
  const audits = new SqliteAuditStore(databasePath);
  try {
    const repositories = plansFor(scenario, catalog, now).map((plan): ScenarioRepository => {
      const checkout = createCheckout(reposRoot, plan.checkout, gitEnvironment);
      const recorded = recordCheckout(db, checkout, plan.recordedAt);
      const context: HistoryContext = { db, catalog: catalog(), sessions, audits, repositoryKey: recorded.repositoryKey, checkoutPath: recorded.checkoutPath };
      const violating = new Set<string>();
      const steps = [
        ...plan.changes.map(change => ({ at: change.at, run: () => atTime(change.at, () => change.apply(db, recorded.repositoryKey, change.at)) })),
        ...plan.sessions.map(session => ({ at: session.startedAt, run: () => recordSession(context, session, plan.schedules, violating) })),
      ].sort((left, right) => left.at - right.at);
      for (const step of steps) step.run();
      const listing = listRepositories(db).find(repository => repository.repositoryKey === recorded.repositoryKey)!;
      const rules = listRepositoryRules(db, catalog(), recorded.repositoryKey).rules;
      return {
        repositoryKey: recorded.repositoryKey,
        displayName: listing.displayName,
        checkoutPath: recorded.checkoutPath,
        state: stateOf(db, recorded),
        packIds: [...new Set(rules.filter(rule => rule.source.kind === "pack").map(rule => rule.source.id))].sort(),
        libraryRuleIds: rules.filter(rule => rule.source.kind === "library_rule").map(rule => rule.source.id).sort(),
        violatingRuleIds: [...violating].sort(),
        sessionIds: plan.sessions.map(session => session.sessionId),
        openSessionIds: plan.sessions.filter(session => session.endedAt === null).map(session => session.sessionId),
      };
    });
    return { scenario, now, root, rerailHome, userHome, databasePath, primary: repositories[0]!, repositories };
  } finally {
    audits.close();
    sessions.close();
    db.$client.close();
  }
}
