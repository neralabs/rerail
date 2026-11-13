/** `rerail packs calibrate`: a set-up repository's settings from its lint config, read again; a person's stay. */
import { mkdirSync, realpathSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  listRepositoryRules,
  loadRuleCatalog,
  lookupCheckout,
  openRerailDatabase,
  recordCheckout,
  RERAIL_DATABASE_FILE,
  ruleSettings,
  setUpRepository,
  type RuleSetting,
} from "../../src/engine/index.js";
import { runSetup } from "../../src/cli/repository-setup.js";
import { calibrationText, runPacksCalibrateCommand, type Calibration, type CalibrationChange } from "../../src/cli/packs-calibrate.js";
import { fakeSetupEnvironment } from "../support/fake-setup-environment.js";
import { installPack, removeInstalledPacks } from "../support/installed-packs.js";
import { agentIo, fixture, removeFixtures, request, SETUP_TIMEOUT_MS, TYPESCRIPT } from "../support/repository-setup.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  removeInstalledPacks();
  await removeFixtures();
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

/** Start on in the TypeScript pack. */
const NO_EXPLICIT_ANY = "typescript-eslint-no-explicit-any";
const NO_NAMESPACE = "typescript-eslint-no-namespace";
const TS_COMMENT = "typescript-eslint-ban-ts-comment";
const NO_REQUIRE_IMPORTS = "typescript-eslint-no-require-imports";
const NO_THIS_ALIAS = "typescript-eslint-no-this-alias";
/** Starts off in the TypeScript pack. */
const ARRAY_TYPE = "typescript-eslint-array-type";

const usage = (): string => "usage";

/** A repository folder and the Rerail home beside it, the TypeScript pack installed there. */
interface Repository {
  readonly repo: string;
  readonly home: string;
  readonly databasePath: string;
}

/**
 * Writes the repository's `.eslintrc.json` with `rules`. It extends a shared config Rerail does not read, so the rules
 * it does not name keep the pack's values; with `complete`, it extends nothing, and those rules start off.
 */
function eslintrc(repo: string, rules: Record<string, unknown>, complete = false): void {
  writeFileSync(join(repo, ".eslintrc.json"), JSON.stringify({ ...(complete ? {} : { extends: ["airbnb"] }), rules }));
}

/** A setting of the repository's for the whole of it: from its lint config with `origin`, a person's without. */
const setting = (repositoryKey: string, ruleId: string, fields: Partial<RuleSetting>): RuleSetting =>
  ({ owner: { kind: "repository", repositoryKey }, folder: "", targetKind: "rule", targetId: ruleId, on: null, mode: null, ...fields });

/**
 * Repository `acme-api`, a folder outside Git, with the TypeScript pack added and `settings` (by rule) stored, as an
 * earlier set-up or calibration and a person left them; recorded but not set up with `setUp: false`.
 */
async function repository(settings: Readonly<Record<string, Partial<RuleSetting>>> = {}, setUp = true): Promise<Repository> {
  const home = realpathSync(await mkdtemp(join(tmpdir(), "rerail-calibrate-")));
  temporaryDirectories.push(home);
  const repo = join(home, "acme-api");
  mkdirSync(repo);
  installPack("typescript", home);
  const databasePath = join(home, RERAIL_DATABASE_FILE);
  const database = openRerailDatabase(databasePath);
  try {
    const { repositoryKey } = recordCheckout(database, repo);
    const stored = Object.entries(settings).map(([ruleId, fields]) => setting(repositoryKey, ruleId, fields));
    if (setUp) setUpRepository(database, repositoryKey, { packs: [{ packId: "typescript", folder: "" }], settings: stored });
  } finally {
    database.$client.close();
  }
  return { repo, home, databasePath };
}

type Row = { on: boolean | null; mode: string | null; config: unknown; origin: string | null } | undefined;

/** The repository's own setting of each rule of `ruleIds` for the whole of it, as stored; undefined where it has none. */
function rows(databasePath: string, ruleIds: readonly string[]): Record<string, Row> {
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    const all = database.select().from(ruleSettings).all();
    return Object.fromEntries(ruleIds.map(id => {
      const row = all.find(candidate => candidate.ownerKind === "repository" && candidate.folder === "" && candidate.targetId === id);
      return [id, row === undefined ? undefined : { on: row.on, mode: row.mode, config: row.config, origin: row.origin }];
    }));
  } finally {
    database.$client.close();
  }
}

/** Runs the command as `rerail packs calibrate <args>` would, returning what it printed. */
function calibrate(where: Repository, args: readonly string[]): string {
  let printed = "";
  const { home, databasePath } = where;
  runPacksCalibrateCommand(["--repo", where.repo, ...args], usage, { home, databasePath, stdout: { write: text => (printed += text) } });
  return printed;
}

const calibrated = (where: Repository, args: readonly string[] = []): Calibration => JSON.parse(calibrate(where, ["--json", ...args])) as Calibration;

/** Each change for the whole repository, by rule. */
const kinds = (calibration: Calibration): Record<string, string> =>
  Object.fromEntries(calibration.changes.filter(change => change.folder === "").map(change => [change.ruleId, change.kind]));

const because = (rule: string, value: unknown): string => `.eslintrc.json: "@typescript-eslint/${rule}": ${JSON.stringify(value)}`;

/** What the lint config says now, against what set-up and a person left (`STORED`). */
const RULES = {
  "@typescript-eslint/no-explicit-any": ["error", { ignoreRestArgs: true }],
  "@typescript-eslint/no-namespace": "off",
  "@typescript-eslint/ban-ts-comment": "off",
  "@typescript-eslint/no-require-imports": "off",
  "@typescript-eslint/array-type": "error",
};
const STORED: Record<string, Partial<RuleSetting>> = {
  [NO_NAMESPACE]: { on: true, mode: "block" },
  [TS_COMMENT]: { on: true, origin: "lint-config" },
  [NO_REQUIRE_IMPORTS]: { on: false, origin: "lint-config" },
  [NO_THIS_ALIAS]: { on: false, origin: "lint-config" },
};
const ALL = [NO_EXPLICIT_ANY, NO_NAMESPACE, TS_COMMENT, NO_REQUIRE_IMPORTS, NO_THIS_ALIAS, ARRAY_TYPE];

/** The note on a config that extends a shared config, as `eslintrc` writes it by default. */
const EXTENDS_NOTE = ".eslintrc.json: only its own rules are read, not extends.";
/** Each change `RULES` makes where `STORED` holds. */
const CHANGES = { [NO_EXPLICIT_ANY]: "options-set", [NO_NAMESPACE]: "kept", [TS_COMMENT]: "turned-off", [ARRAY_TYPE]: "turned-on", [NO_THIS_ALIAS]: "returned" };

describe("rerail packs calibrate", () => {
  it("says what it would change with --dry-run, writing nothing; then brings the lint config's settings up to date, keeping a person's", async () => {
    const where = await repository(STORED);
    eslintrc(where.repo, RULES);
    const stored = {
      [NO_EXPLICIT_ANY]: undefined,
      [NO_NAMESPACE]: { on: true, mode: "block", config: null, origin: null },
      [TS_COMMENT]: { on: true, mode: null, config: null, origin: "lint-config" },
      [NO_REQUIRE_IMPORTS]: { on: false, mode: null, config: null, origin: "lint-config" },
      [NO_THIS_ALIAS]: { on: false, mode: null, config: null, origin: "lint-config" },
      [ARRAY_TYPE]: undefined,
    };
    expect(rows(where.databasePath, ALL)).toEqual(stored);

    const text = calibrate(where, ["--dry-run"]);
    expect(text.split("\n")[0]).toBe("● Calibrating would change acme-api's rule settings from its lint config (read as data, never run):");
    expect(text).toContain(`  Turned off:\n    ${TS_COMMENT}  ${because("ban-ts-comment", "off")}\n`);
    expect(text).toMatch(/\n {2}Nothing was written \(--dry-run\)\.\n$/u);
    const preview = calibrated(where, ["--dry-run"]);
    expect(preview.dryRun).toBe(true);
    expect(kinds(preview)).toEqual(CHANGES);
    expect(rows(where.databasePath, ALL)).toEqual(stored);

    const calibration = calibrated(where);
    expect(calibration).toMatchObject({ repositoryName: "acme-api", dryRun: false, notes: [EXTENDS_NOTE] });
    expect(kinds(calibration)).toEqual(CHANGES);
    expect(calibration.changes.find(change => change.ruleId === NO_EXPLICIT_ANY)).toEqual({
      kind: "options-set", ruleId: NO_EXPLICIT_ANY, folder: "", config: { ignoreRestArgs: true }, explicit: true, because: because("no-explicit-any", RULES["@typescript-eslint/no-explicit-any"]),
    });
    expect(calibration.changes.find(change => change.ruleId === NO_THIS_ALIAS)).toEqual({ kind: "returned", ruleId: NO_THIS_ALIAS, folder: "", config: null, explicit: true, because: null });
    expect(rows(where.databasePath, ALL)).toEqual({
      ...stored,
      [NO_EXPLICIT_ANY]: { on: null, mode: null, config: { ignoreRestArgs: true }, origin: "lint-config" },
      [TS_COMMENT]: { on: false, mode: null, config: null, origin: "lint-config" },
      [NO_THIS_ALIAS]: undefined,
      [ARRAY_TYPE]: { on: true, mode: null, config: null, origin: "lint-config" },
    });

    // Again with nothing changed: every setting it wrote, options too, already holds, and the person's is still theirs.
    expect(kinds(calibrated(where))).toEqual({ [NO_NAMESPACE]: "kept" });
  });

  it("names the notes on the lint config of the packs' linters, and leaves another linter's config alone", async () => {
    const where = await repository();
    eslintrc(where.repo, { "@typescript-eslint/no-explicit-any": ["error", { noSuchOption: true }] });
    // Ruff's config, read, and SQLFluff's, not read: no TypeScript rule ports either linter.
    writeFileSync(join(where.repo, "ruff.toml"), '[lint]\nselect = ["E"]\n');
    writeFileSync(join(where.repo, ".sqlfluff"), "not = [an ini file");
    const calibration = calibrated(where);
    expect(calibration.changes).toEqual([]);
    expect(calibration.notes).toEqual([
      EXTENDS_NOTE,
      `${NO_EXPLICIT_ANY}: option not mapped (noSuchOption is not an option the port declares); it keeps its default options. From ${because("no-explicit-any", ["error", { noSuchOption: true }])}.`,
    ]);
    expect(calibrate(where, [])).toBe([
      "● acme-api's rule settings already follow its lint config (read as data, never run).",
      "  About your lint config:",
      ...calibration.notes.map(note => `    ${note}`),
    ].join("\n") + "\n");
  });

  it("refuses a repository that is recorded but not set up, and an unknown option", async () => {
    const where = await repository({}, false);
    eslintrc(where.repo, { "@typescript-eslint/ban-ts-comment": "off" });
    expect(() => calibrate(where, [])).toThrow("acme-api is not set up, so it has no rule packs to calibrate. Set it up with rerail start.");
    expect(() => calibrate(where, ["--nope"])).toThrow("Unknown packs calibrate option: --nope\n\nusage");
  });

  it("calibrates the folder it runs in against the default home, which has no database there: not set up", async () => {
    const where = await repository();
    const cwd = process.cwd();
    process.chdir(where.repo);
    try {
      expect(() => runPacksCalibrateCommand([], usage, { stdout: { write: () => true } })).toThrow("acme-api is not set up, so it has no rule packs to calibrate.");
    } finally {
      process.chdir(cwd);
    }
  });

  it("follows a lint config changed since a real set-up, the rules on as it says", { timeout: SETUP_TIMEOUT_MS }, async () => {
    const setup = await fixture();
    eslintrc(setup.repo, { "@typescript-eslint/no-explicit-any": "off", "@typescript-eslint/no-namespace": "off" });
    const environment = fakeSetupEnvironment();
    await runSetup(agentIo(), request(setup, environment, { answer: "yes", repo: setup.repo }));
    await runSetup(agentIo(), request(setup, environment, { answer: "start", repo: setup.repo, packs: TYPESCRIPT }));
    eslintrc(setup.repo, { "@typescript-eslint/no-explicit-any": "error", "@typescript-eslint/ban-ts-comment": "off" });

    let printed = "";
    runPacksCalibrateCommand(["--repo", setup.repo, "--json"], usage, { databasePath: setup.databasePath, stdout: { write: text => (printed += text) } });
    // Set-up turned no-explicit-any and no-namespace off from the first config; the config no longer names either.
    expect(kinds(JSON.parse(printed) as Calibration)).toEqual({ [NO_EXPLICIT_ANY]: "returned", [NO_NAMESPACE]: "returned", [TS_COMMENT]: "turned-off" });
    const database = openRerailDatabase(setup.databasePath, { readonly: true });
    try {
      const rules = listRepositoryRules(database, loadRuleCatalog(), lookupCheckout(database, setup.repo)!.repositoryKey).rules;
      const on = (id: string): boolean | undefined => rules.find(rule => rule.document.rule.id === id)?.on;
      expect([NO_EXPLICIT_ANY, NO_NAMESPACE, TS_COMMENT].map(on)).toEqual([true, true, false]);
    } finally {
      database.$client.close();
    }
  });
});

describe("calibrationText", () => {
  const change = (kind: CalibrationChange["kind"], ruleId: string, fields: Partial<CalibrationChange> = {}): CalibrationChange =>
    ({ kind, ruleId, folder: "", config: null, explicit: true, because: `${ruleId} in config`, ...fields });
  const DEFAULTS = ".eslintrc.json does not turn it on";
  const text = (changes: readonly CalibrationChange[], options: { dryRun?: boolean; notes?: string[] } = {}): string =>
    calibrationText({ repositoryName: "acme-api", dryRun: options.dryRun ?? false, changes, notes: options.notes ?? [] });

  it("lists each kind of change in its section, in order, counting the rules one reason decides for the whole repository", () => {
    expect(text([
      change("kept", "mine"),
      change("returned", "back", { because: null }),
      change("options-set", "opts", { config: { max: 3, style: "x" } }),
      change("turned-off", "d1", { explicit: false, because: DEFAULTS }),
      change("turned-off", "folded", { folder: "src/legacy" }),
      change("turned-off", "d2", { explicit: false, because: DEFAULTS }),
      change("turned-off", "lone", { explicit: false, because: "preset" }),
      change("turned-off", "in-folder", { explicit: false, because: DEFAULTS, folder: "lib" }),
      change("turned-on", "with-options", { explicit: false, because: DEFAULTS, config: { max: 1 } }),
    ], { notes: ["a note"] })).toBe([
      "● Calibrated acme-api's rule settings from its lint config (read as data, never run):",
      "  Turned on:",
      `    with-options with max = 1  ${DEFAULTS}`,
      "  Turned off:",
      `    2 rules  ${DEFAULTS}`,
      "    lone  preset",
      "    folded in src/legacy/  folded in config",
      `    in-folder in lib/  ${DEFAULTS}`,
      "  Options set:",
      '    opts with max = 3, style = "x"  opts in config',
      "  Back to the pack's value (your lint config no longer sets them):",
      "    back",
      "  Kept yours (you set them yourself; your lint config says otherwise):",
      "    mine  mine in config",
      "  About your lint config:",
      "    a note",
    ].join("\n"));
  });

  it.each<[string, readonly CalibrationChange[], boolean, string[]]>([
    ["nothing to change", [], false, ["● acme-api's rule settings already follow its lint config (read as data, never run)."]],
    ["nothing to change, with --dry-run", [], true, ["● acme-api's rule settings already follow its lint config (read as data, never run)."]],
    [
      "a change, with --dry-run", [change("turned-on", "r")], true,
      ["● Calibrating would change acme-api's rule settings from its lint config (read as data, never run):", "  Turned on:", "    r  r in config", "  Nothing was written (--dry-run)."],
    ],
  ])("says %s", (_, changes, dryRun, lines) => {
    expect(text(changes, { dryRun })).toBe(lines.join("\n"));
  });
});
