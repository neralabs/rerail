/**
 * `rerail packs calibrate`: reads the repository's own lint config again, as set-up does (docs/rule-scoping.md, "The
 * repository's own lint config"), for a repository already set up, and brings the settings it gave the rules of the
 * repository's packs up to date. A setting the lint config gave is marked so (`rule_settings.origin`); a person's
 * setting (no origin, as is every setting set-up wrote before origins were recorded) is never changed. A running daemon
 * follows the change as it follows the dashboard's: each audit compares the repository's rule configuration signature.
 */
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { and, eq } from "drizzle-orm";
import { parseArguments } from "../daemon/daemon.js";
import type { LintConfigSetting } from "../engine/catalog/lint-config/linter-config.js";
import { readRepositoryLintConfig } from "../engine/catalog/lint-config/repository-lint-config.js";
import { repositoryIdentityLocation, repositoryName } from "../engine/catalog/repository-identity.js";
import { normalizeFolder, type CheckoutContext, type RepositoryKey, type RuleConfig, type RuleSetting } from "../engine/catalog/repository-model.js";
import { checkoutStatus, lookupCheckout } from "../engine/catalog/repository-registry.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { setRuleSetting } from "../engine/catalog/rule-configuration.js";
import { openRerailDatabase, RERAIL_DATABASE_FILE, type RerailDatabase } from "../engine/storage/database.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { ruleAttachments, ruleSettings } from "../engine/storage/schema.js";
import { stableJsonText } from "../engine/stable-json.js";
import { clean } from "../harness/setup-screens.js";

/** What calibrating did, or with `--dry-run` would do, to one rule's setting in one folder. */
export type CalibrationChangeKind =
  | "turned-on"
  | "turned-off"
  /** The lint config gives the rule options and leaves it as the pack has it. */
  | "options-set"
  /** The lint config no longer gives the setting it gave: the setting is removed, and the pack's value returns. */
  | "returned"
  /** The lint config sets the rule otherwise than a person did themselves: theirs stays. */
  | "kept";

export interface CalibrationChange {
  readonly kind: CalibrationChangeKind;
  readonly ruleId: string;
  /** Relative to the repository root; `""` for the whole repository. */
  readonly folder: string;
  /** The option values the lint config gives, or `null`. */
  readonly config: RuleConfig | null;
  /** Whether the lint config names the rule; not: a linter's defaults or a preset decide. */
  readonly explicit: boolean;
  /** The config entry that says so; `null` for a setting returned to the pack's value. */
  readonly because: string | null;
}

export interface Calibration {
  readonly repositoryName: string;
  /** Nothing was written: `--dry-run`. */
  readonly dryRun: boolean;
  readonly changes: readonly CalibrationChange[];
  /** Files not read, options not mapped and conflicts, as set-up names them. */
  readonly notes: readonly string[];
}

export interface CalibrateOptions {
  /** The Rerail home whose installed packs are read; the default home when omitted. */
  readonly home?: string;
  /** The database; the home's when omitted. */
  readonly databasePath?: string;
  readonly dryRun?: boolean;
}

type SettingRow = typeof ruleSettings.$inferSelect;

const settingKey = (folder: string, ruleId: string): string => `${folder}\0${ruleId}`;

/** `config` as compared: no options and empty options alike. */
const configText = (config: RuleConfig | null): string | undefined => stableJsonText(config === null || Object.keys(config).length === 0 ? null : config);

/** Whether `row` already holds what `proposal` sets, and nothing else. */
function holds(row: SettingRow, proposal: LintConfigSetting): boolean {
  return row.on === proposal.on && row.mode === null && row.suppressions === null && configText(row.config) === configText(proposal.config);
}

/**
 * Whether a person's `row` gives the rule what `proposal` gives it: on or off (a row that leaves it unset has the value
 * around it: on where `proposal` only gives options, otherwise the opposite of `proposal`, which is proposed only where
 * it differs from the pack's value or the whole repository's) and, on, the same options.
 * Modes and suppressions are a person's alone; the lint config sets neither.
 */
function agrees(row: SettingRow, proposal: LintConfigSetting): boolean {
  const on = proposal.on ?? true;
  return (row.on ?? proposal.on !== true) === on && (!on || configText(row.config) === configText(proposal.config));
}

/** Whether `row` leaves `proposal` nothing to do: the lint config's setting holds it, or a person's agrees with it. */
function unchanged(row: SettingRow | undefined, proposal: LintConfigSetting): boolean {
  if (row === undefined) return false;
  return row.origin === null ? agrees(row, proposal) : holds(row, proposal);
}

/** What `proposal` does where the repository's own setting for its rule and folder is `row`; undefined: nothing. */
function proposalChange(proposal: LintConfigSetting, row: SettingRow | undefined): CalibrationChange | undefined {
  if (unchanged(row, proposal)) return undefined;
  const change = { ruleId: proposal.ruleId, folder: proposal.folder, config: proposal.config, explicit: proposal.explicit, because: proposal.because };
  if (row?.origin === null) return { kind: "kept", ...change };
  return { kind: proposal.on === null ? "options-set" : proposal.on ? "turned-on" : "turned-off", ...change };
}

/** A change and the write that makes it; none for a person's setting kept. */
interface Step {
  readonly change: CalibrationChange;
  readonly write?: RuleSetting;
}

/** The repository's setting of `ruleId` in `folder`: removed (`on` null and nothing else), or as `fields` give it. */
function repositorySetting(repositoryKey: RepositoryKey, folder: string, ruleId: string, fields: Partial<RuleSetting> = {}): RuleSetting {
  return { owner: { kind: "repository", repositoryKey }, folder, targetKind: "rule", targetId: ruleId, on: null, mode: null, ...fields };
}

/** What the lint config's `proposals` do where the repository's own rule settings are `rows`. */
function proposalSteps(repositoryKey: RepositoryKey, rows: ReadonlyMap<string, SettingRow>, proposals: readonly LintConfigSetting[]): Step[] {
  return proposals.flatMap(proposal => {
    const change = proposalChange(proposal, rows.get(settingKey(proposal.folder, proposal.ruleId)));
    if (change === undefined) return [];
    if (change.kind === "kept") return [{ change }];
    const { on, config } = proposal;
    return [{ change, write: repositorySetting(repositoryKey, proposal.folder, proposal.ruleId, { on, config, origin: "lint-config" }) }];
  });
}

/** The settings the lint config gave and no longer gives: each removed, so the pack's value returns. */
function returnedSteps(repositoryKey: RepositoryKey, rows: readonly SettingRow[], proposed: ReadonlySet<string>): Step[] {
  return rows.filter(row => row.origin === "lint-config" && !proposed.has(settingKey(row.folder, row.targetId))).map(row => ({
    change: { kind: "returned", ruleId: row.targetId, folder: row.folder, config: null, explicit: true, because: null },
    write: repositorySetting(repositoryKey, row.folder, row.targetId),
  }));
}

/** What the repository's own rule settings `rows` and the lint config's `proposals` reconcile to. */
function reconcile(repositoryKey: RepositoryKey, rows: readonly SettingRow[], proposals: readonly LintConfigSetting[]): Step[] {
  const proposed = proposals.map(proposal => ({ ...proposal, folder: normalizeFolder(proposal.folder) }));
  const byKey = new Map(rows.map(row => [settingKey(row.folder, row.targetId), row]));
  return [
    ...proposalSteps(repositoryKey, byKey, proposed),
    ...returnedSteps(repositoryKey, rows, new Set(proposed.map(proposal => settingKey(proposal.folder, proposal.ruleId)))),
  ];
}

/** The packs added to the repository, each once, whatever the folders. */
function packIdsOf(database: RerailDatabase, repositoryKey: RepositoryKey): string[] {
  const rows = database.select({ packId: ruleAttachments.sourceId }).from(ruleAttachments)
    .where(and(eq(ruleAttachments.repositoryKey, repositoryKey), eq(ruleAttachments.sourceKind, "pack"))).all();
  return [...new Set(rows.map(row => row.packId))].sort();
}

/** The repository's own rule settings, every folder. */
function repositoryRuleRows(database: RerailDatabase, repositoryKey: RepositoryKey): SettingRow[] {
  return database.select().from(ruleSettings).where(and(
    eq(ruleSettings.ownerKind, "repository"),
    eq(ruleSettings.ownerId, repositoryKey),
    eq(ruleSettings.targetKind, "rule"),
  )).all();
}

/** Reconciles the lint config's `proposals` with the repository's own rule settings as stored now; writes only when `write`. */
function reconcileIn(database: RerailDatabase, repositoryKey: RepositoryKey, proposals: readonly LintConfigSetting[], write: boolean): Step[] {
  const steps = reconcile(repositoryKey, repositoryRuleRows(database, repositoryKey), proposals);
  if (write) {
    const now = Date.now();
    for (const { write: setting } of steps) if (setting !== undefined) setRuleSetting(database, setting, now);
  }
  return steps;
}

const notSetUp = (name: string): Error => new Error(`${name} is not set up, so it has no rule packs to calibrate. Set it up with rerail start.`);

/** The checkout `workspace` is in, recorded and its repository set up; throws otherwise. */
function setUpCheckout(database: RerailDatabase, workspace: string, name: string): CheckoutContext {
  const checkout = lookupCheckout(database, workspace);
  if (checkout === undefined || !checkoutStatus(database, checkout).setUp) throw notSetUp(name);
  return checkout;
}

/**
 * Calibrates the set-up repository of the checkout `repo` is in from its lint config. The rule catalog is loaded and the
 * config read first; only reading the repository's settings, reconciling and writing hold the immediate transaction.
 * With `dryRun`, reads only. Throws when the repository is not set up.
 */
export function calibratePacks(repo: string, options: CalibrateOptions = {}): Calibration {
  const workspace = resolve(repo);
  const name = repositoryName(workspace);
  const home = options.home ?? defaultRerailHome();
  const databasePath = options.databasePath ?? join(home, RERAIL_DATABASE_FILE);
  if (!existsSync(databasePath)) throw notSetUp(name);
  const dryRun = options.dryRun === true;
  const database = openRerailDatabase(databasePath, { readonly: dryRun });
  try {
    const { repositoryKey } = setUpCheckout(database, workspace, name);
    const root = repositoryIdentityLocation(workspace).workspaceRoot;
    const { settings, notes } = readRepositoryLintConfig(root, loadRuleCatalog(home), packIdsOf(database, repositoryKey));
    const steps = dryRun
      ? reconcileIn(database, repositoryKey, settings, false)
      : database.transaction(() => reconcileIn(database, repositoryKey, settings, true), { behavior: "immediate" });
    return { repositoryName: name, dryRun, changes: steps.map(step => step.change), notes };
  } finally {
    database.$client.close();
  }
}

/** Each section of the text, in order: its heading and the changes it lists. */
const SECTIONS: readonly { readonly kind: CalibrationChangeKind; readonly heading: string }[] = [
  { kind: "turned-on", heading: "Turned on:" },
  { kind: "turned-off", heading: "Turned off:" },
  { kind: "options-set", heading: "Options set:" },
  { kind: "returned", heading: "Back to the pack's value (your lint config no longer sets them):" },
  { kind: "kept", heading: "Kept yours (set by you, or by an earlier Rerail release's set-up; your lint config says otherwise):" },
];

/** One change the lint config names, or that has options, as a row: the rule, where, with which options, and why. */
function changeRow(change: CalibrationChange): string {
  const options = Object.entries(change.config ?? {}).map(([name, value]) => `${name} = ${JSON.stringify(value)}`).join(", ");
  const parts = [
    clean(change.ruleId),
    ...(change.folder === "" ? [] : [`in ${clean(change.folder)}/`]),
    ...(options === "" ? [] : [`with ${options}`]),
  ];
  return `    ${parts.join(" ")}${change.because === null ? "" : `  ${clean(change.because)}`}`;
}

/** A change counted with others for its reason: a linter's default or a preset's, for the whole repository, without options. */
const isCounted = (change: CalibrationChange): change is CalibrationChange & { readonly because: string } =>
  !change.explicit && change.config === null && change.folder === "" && change.because !== null;

/**
 * A section's rows: a linter's defaults or a preset decide many rules for one reason, so those settings for the whole
 * repository without options are counted per reason; the rest are listed.
 */
function sectionRows(changes: readonly CalibrationChange[]): string[] {
  const groups = Map.groupBy(changes.filter(isCounted), change => change.because);
  return [
    ...[...groups].map(([because, group]) => (group.length === 1 ? changeRow(group[0] as CalibrationChange) : `    ${group.length} rules  ${clean(because)}`)),
    ...changes.filter(change => !isCounted(change)).map(changeRow),
  ];
}

/** The first line: what calibrating changed or would change; or that nothing, but for a person's settings kept. */
function headline(calibration: Calibration, changed: boolean): string {
  const name = clean(calibration.repositoryName);
  if (changed) return `● ${calibration.dryRun ? "Calibrating would change" : "Calibrated"} ${name}'s rule settings from its lint config (read as data, never run):`;
  return calibration.changes.length === 0
    ? `● ${name}'s rule settings already follow its lint config (read as data, never run).`
    : `● ${name}'s rule settings already follow its lint config (read as data, never run), except yours:`;
}

/** `calibration` as `rerail packs calibrate` prints it. */
export function calibrationText(calibration: Calibration): string {
  const changed = calibration.changes.some(change => change.kind !== "kept");
  const sections = SECTIONS.flatMap(({ kind, heading }) => {
    const changes = calibration.changes.filter(change => change.kind === kind);
    return changes.length === 0 ? [] : [`  ${heading}`, ...sectionRows(changes)];
  });
  return [
    headline(calibration, changed),
    ...sections,
    ...(calibration.notes.length === 0 ? [] : ["  About your lint config:", ...calibration.notes.map(note => `    ${clean(note)}`)]),
    ...(calibration.dryRun && changed ? ["  Nothing was written (--dry-run)."] : []),
  ].join("\n");
}

/** Where `rerail packs calibrate` reads and writes, and prints. */
export interface CalibrateCommandIo extends Omit<CalibrateOptions, "dryRun"> {
  readonly stdout?: { write(text: string): unknown };
}

/** `rerail packs calibrate [--repo <dir>] [--dry-run] [--json]`. */
export function runPacksCalibrateCommand(args: readonly string[], usage: () => string, io: CalibrateCommandIo = {}): number {
  const { flags, options } = parseArguments(
    args,
    { flags: ["--dry-run", "--json"], options: ["--repo"] },
    argument => `Unknown packs calibrate option: ${argument}\n\n${usage()}`,
  );
  const { stdout = process.stdout, ...where } = io;
  const calibration = calibratePacks(options.get("--repo") ?? process.cwd(), { ...where, dryRun: flags.has("--dry-run") });
  stdout.write(`${flags.has("--json") ? JSON.stringify(calibration) : calibrationText(calibration)}\n`);
  return 0;
}
