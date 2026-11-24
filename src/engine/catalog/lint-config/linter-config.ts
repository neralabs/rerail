/**
 * The contract between set-up's reading of a repository's own lint config (`repository-lint-config.ts`) and each
 * linter's reader, and how their verdicts settle into rule settings, so a pack's ports run the rules the repository's
 * linters run: the same rules on, the same rules off.
 *
 * A port is matched to a linter's rules through the names its document honours suppression comments for
 * (`violation.suppress`, over every clause): ESLint and oxlint rule names, Biome's `lint/<group>/<rule>`, Ruff's noqa
 * codes, Bandit's test ids, PHPStan identifiers, Psalm issues, PHP_CodeSniffer sniffs, Clippy lints and their groups,
 * SQLFluff's codes, aliases and groups. A rule without a `suppress` entry for a linter is not one of its ports.
 *
 * Each linter's reader reads that linter's config at the repository root as data, never running it, and gives a
 * verdict per port. A repository with no config for a linter gets no verdict from it: its ports keep the pack's
 * values. A verdict is explicit when the config names the rule (by itself, a prefix or a group of it) and implicit
 * when the linter's own defaults, or an upstream preset the config takes, decide it; `unknown` when something the
 * config takes that Rerail cannot read (an unrecognised shared config, a file outside the repository) may set it.
 *
 * Verdicts from several linters settle in this order: an explicit off, then an explicit on, then an implicit on, then
 * unknown (the pack's value stands), then an implicit off. A setting is proposed only where the outcome differs from
 * the pack's starting value (`on`), or where the config gives the rule options. A folder settles in the same order,
 * over each linter's decision there (its own for the folder, else for the nearest folder holding it, else for every
 * file), and is set only where it differs from the nearest folder holding it, or from every file. A rule a reviewer
 * holds off (`pack.json` `heldOff`) is never turned on, by any route: what would turn it on is noted and set aside.
 */
import { lstatSync, readFileSync } from "node:fs";
import { join } from "node:path";
import type { CodeRuleDslDocument, CodeRuleDslOptionDeclaration } from "../../code-rule/code-rule.js";
import type { CodeRuleDslSuppress } from "../../code-rule/code-rule-semantic.js";
import type { RuleCatalog, RuleConfig } from "../repository-model.js";
import type { RulePackRule } from "../rule-packs.js";
import { stableJsonText } from "../../stable-json.js";
import { combineDecisions } from "./linter-decisions.js";

/** A config file larger than this is not one a person wrote; it is reported as not read. As `package.json` in `rule-pack-matcher.ts`. */
export const MAX_CONFIG_BYTES = 1024 * 1024;

export const NOT_A_FILE = "it is not a regular file of at most 1 MiB in the repository.";

/** The linters whose configs Rerail reads. */
export type LintConfigLinter = "eslint" | "oxlint" | "biome" | "ruff" | "bandit" | "clippy" | "phpstan" | "psalm" | "phpcs" | "sqlfluff";

/** Something about a config file, for the person, by the linter it configures. */
export interface LinterNote {
  readonly linter: LintConfigLinter;
  readonly text: string;
}

/** The kinds of a violation's `suppress` (`codeRuleSuppressSchema`), each naming one linter's rules. */
export type SuppressKind = "eslint" | "noqa" | "nosec" | "oxlint" | "biome" | "phpstan" | "psalm" | "phpcs" | "clippy" | "sqlfluff";

export type Declared = Readonly<Record<string, CodeRuleDslOptionDeclaration>>;

/** A port of a pack being set up: the upstream names it honours suppression comments for, per kind, over every clause. */
export interface Port {
  readonly ruleId: string;
  /** Per suppress kind, its names in document order, each once; empty for a kind the document does not name. */
  readonly suppress: { readonly [K in SuppressKind]: readonly string[] };
  /** The options its `define` clauses declare. */
  readonly declared: Declared;
  /** Whether the rule is on when its pack is added (`pack.json` `on`). */
  readonly startsOn: boolean;
  /** Why a reviewer holds the rule off (`pack.json` `heldOff`): no config turns it on. */
  readonly heldOff?: string;
  /** The document's own scope, the files it applies to; absent when the document has none. */
  readonly scope?: { readonly include: readonly string[]; readonly exclude: readonly string[] };
}

/** Upstream options mapped onto a port's `config`, or why they are not. */
export type MappedOptions = { readonly ok: true; readonly config: RuleConfig } | { readonly ok: false; readonly reason: string };

/**
 * What one linter's config decides for a port, for every file. `because` names the config entry, for the person.
 * Reasons can appear after a count of rules, so wording must work for any number.
 */
export type PortDecision =
  | {
    readonly on: boolean;
    /** The config names the rule (itself, a prefix, a group of it); not: the linter's defaults or a preset decide. */
    readonly explicit: boolean;
    readonly because: string;
    /** Upstream options for the rule, mapped or not; only with `on: true`. */
    readonly config?: MappedOptions;
  }
  /** Something the config takes and Rerail cannot read may set the rule: the pack's value stands. */
  | { readonly on: "unknown"; readonly because: string };

/**
 * A whole folder below the repository root, relative to it, where the config decides the rule otherwise than for every
 * file, explicit or not as a `PortDecision` is; `unknown` where something there Rerail cannot read may set it.
 */
export type FolderDecision = { readonly folder: string } & (
  | { readonly on: boolean; readonly explicit: boolean; readonly because: string }
  | { readonly on: "unknown"; readonly because: string }
);

/** `decision`, for the folder `folder`; its options, for every file only, are left out. */
export function inFolder(folder: string, decision: PortDecision | FolderDecision): FolderDecision {
  return decision.on === "unknown" ? { folder, on: "unknown", because: decision.because } : { folder, on: decision.on, explicit: decision.explicit, because: decision.because };
}

/** Whether two decisions settle alike: the same state, as explicit. A folder deciding as the folder holding it needs no decision. */
export function settlesAlike(left: PortDecision | FolderDecision, right: PortDecision | FolderDecision | undefined): boolean {
  if (right === undefined || left.on !== right.on) return false;
  return left.on === "unknown" || right.on === "unknown" || left.explicit === right.explicit;
}

/** What one linter's config says about one port. */
export interface PortVerdict {
  /** For every file; absent when the config says nothing about the port for every file. */
  readonly decision?: PortDecision;
  readonly folders?: readonly FolderDecision[];
  /** What the config says about the port that Rerail does not use, and why. */
  readonly notes?: readonly string[];
}

/** One linter's config, read at a repository root. */
export interface LinterConfig {
  /** The verdict on `port`; undefined when the port has no `suppress` name of this linter's. */
  verdict(port: Port, ruleCatalog: RuleCatalog): PortVerdict | undefined;
}

/** How one linter's config is read. */
export interface LinterReader {
  readonly linter: LintConfigLinter;
  /** The suppress kinds that name this linter's rules: a port with a name of one of them is a port of this linter. */
  readonly kinds: readonly SuppressKind[];
  /**
   * The linter's config at `root`, read as data and never run; undefined when the repository has no config for the
   * linter, so its ports keep the pack's values. What is present and not read, or read in part, goes to `notes`.
   */
  read(root: string, notes: LinterNote[]): LinterConfig | undefined;
}

/** A setting set-up proposes for a rule of a pack being set up. */
export interface LintConfigSetting {
  readonly ruleId: string;
  /** The folder it applies in, relative to the repository root; `""` for the whole repository. */
  readonly folder: string;
  /** Whether the rule is on; `null`: as the pack has it (the setting carries options only). */
  readonly on: boolean | null;
  /** Option values from the repository's config, or `null`. */
  readonly config: RuleConfig | null;
  /** Whether a config names the rule; not: a linter's defaults, or a config naming none of the rules it runs, decide. */
  readonly explicit: boolean;
  /** Where it comes from, for the person: `.eslintrc.json: "no-console": "off"`. */
  readonly because: string;
}

/**
 * What the lint config gives a rule in one folder once every linter's say there settles, whether or not it differs from
 * the pack's value: for every file, where any linter decides for every file, and in each folder any linter decides in.
 */
export interface LintConfigOutcome {
  readonly ruleId: string;
  /** Relative to the repository root; `""` for the whole repository. */
  readonly folder: string;
  /** `unknown`: something Rerail cannot read may set it there, so the pack's value stands. */
  readonly on: boolean | "unknown";
  /** Option values for every file, or `null`. */
  readonly config: RuleConfig | null;
  readonly explicit: boolean;
  readonly because: string;
}

export interface RepositoryLintConfig {
  readonly settings: readonly LintConfigSetting[];
  /** What the settings come from: every outcome, those equal to the pack's value too. */
  readonly outcomes: readonly LintConfigOutcome[];
  /** Files not read, options not mapped and conflicts, for the person. */
  readonly notes: readonly string[];
}

export type FileRead = { readonly present: false } | { readonly present: true; readonly text: string | undefined };

/**
 * A file at `name` (a path relative to the root) as text: absent, present but not a regular file of at most
 * `MAX_CONFIG_BYTES` in the repository (no text), or its text.
 */
export function readRootFile(root: string, name: string): FileRead {
  let info;
  try {
    info = lstatSync(join(root, name));
  } catch {
    return { present: false };
  }
  // A symlink, the file's or a folder's on the way to it, could point outside the repository or at a device.
  const folders = name.split("/").slice(0, -1).map((_, index, parts) => parts.slice(0, index + 1).join("/"));
  const throughLink = folders.some(folder => lstatSync(join(root, folder)).isSymbolicLink());
  if (throughLink || !info.isFile() || info.size > MAX_CONFIG_BYTES) return { present: true, text: undefined };
  try {
    return { present: true, text: readFileSync(join(root, name), "utf8") };
  } catch {
    return { present: true, text: undefined };
  }
}

/** Per suppress kind, the names one violation's `suppress` gives. */
const SUPPRESS_NAMES: { readonly [K in SuppressKind]: (written: CodeRuleDslSuppress) => readonly string[] | undefined } = {
  eslint: written => written.eslint?.rules,
  noqa: written => written.noqa?.codes,
  nosec: written => written.nosec?.codes,
  oxlint: written => written.oxlint?.rules,
  biome: written => written.biome?.rules,
  phpstan: written => written.phpstan?.identifiers,
  psalm: written => written.psalm?.issues,
  phpcs: written => written.phpcs?.sniffs,
  clippy: written => written.clippy?.lints,
  sqlfluff: written => written.sqlfluff?.rules,
};

/** The options `document`'s `define` clauses declare, a later clause's over an earlier's. */
function declaredOptions(document: CodeRuleDslDocument): Declared {
  const declared: Record<string, CodeRuleDslOptionDeclaration> = {};
  for (const clause of document.clauses) if (clause.define !== undefined) Object.assign(declared, clause.define.options ?? {});
  return declared;
}

/** Pack rule `rule`, a code rule, as a port. */
function portOf(rule: RulePackRule, document: CodeRuleDslDocument): Port {
  const written = document.clauses.flatMap(clause => (clause.violation?.suppress === undefined ? [] : [clause.violation.suppress]));
  const names = (kind: SuppressKind): string[] => [...new Set(written.flatMap(suppress => SUPPRESS_NAMES[kind](suppress) ?? []))];
  return {
    ruleId: rule.id,
    suppress: {
      eslint: names("eslint"), noqa: names("noqa"), nosec: names("nosec"), oxlint: names("oxlint"), biome: names("biome"),
      phpstan: names("phpstan"), psalm: names("psalm"), phpcs: names("phpcs"), clippy: names("clippy"), sqlfluff: names("sqlfluff"),
    },
    declared: declaredOptions(document),
    startsOn: rule.on,
    ...(rule.heldOff === undefined ? {} : { heldOff: rule.heldOff }),
    ...(document.scope === undefined ? {} : { scope: { include: document.scope.include, exclude: document.scope.exclude } }),
  };
}

/** The code rules of packs `packIds` as ports, each once; a conversation rule is no port. */
export function portsOf(ruleCatalog: RuleCatalog, packIds: readonly string[]): Port[] {
  const rules = new Map(packIds.flatMap(packId => ruleCatalog.packs.get(packId)?.rules ?? []).map(rule => [rule.id, rule]));
  return [...rules.values()].flatMap(rule => {
    const document = ruleCatalog.documents.get(rule.id);
    return document?.kind === "code" ? [portOf(rule, document.rule)] : [];
  });
}

/** Whether `port` has a suppress name of one of `kinds`. */
export const portsLinter = (port: Port, kinds: readonly SuppressKind[]): boolean => kinds.some(kind => port.suppress[kind].length > 0);

type Decided = Extract<PortDecision, { on: boolean }>;

/** The options the decisions agree on, saying why when they do not; undefined when none sets options. */
function agreedConfig(ruleId: string, decided: readonly Decided[], notes: string[]): { config: RuleConfig; because: string } | undefined {
  const configured = decided.flatMap(decision => (decision.config === undefined ? [] : [{ because: decision.because, mapped: decision.config }]));
  const configs: RuleConfig[] = [];
  for (const { because, mapped } of configured) {
    if (!mapped.ok) {
      notes.push(`${ruleId}: option not mapped (${mapped.reason}); it keeps its default options. From ${because}.`);
      return undefined;
    }
    configs.push(mapped.config);
  }
  if (new Set(configs.map(config => stableJsonText(config))).size > 1) {
    notes.push(`${ruleId}: ${configured.map(found => found.because).join(" and ")} set different options; none is used.`);
    return undefined;
  }
  const config = configs[0];
  return config !== undefined && Object.keys(config).length > 0 ? { config, because: configured.map(found => found.because).join("; ") } : undefined;
}

/** Notes that one config keeps the port on and another turns it off. */
function noteConflict(port: Port, decided: readonly Decided[], notes: string[]): void {
  const keptOn = decided.find(decision => decision.on && decision.explicit);
  const turnedOff = decided.find(decision => !decision.on && decision.explicit);
  if (keptOn !== undefined && turnedOff !== undefined) notes.push(`${port.ruleId}: ${keptOn.because} keeps it on, but ${turnedOff.because} turns it off; it starts off.`);
}

/** Where `decisions` settle (`combineDecisions`); undefined where an unknown, or none, leaves the pack's value. */
function settle(decisions: readonly (PortDecision | FolderDecision)[]): { on: boolean; explicit: boolean; because: string } | undefined {
  const combined = combineDecisions(decisions);
  return combined === undefined || combined.on === "unknown" ? undefined : { on: combined.on, explicit: combined.explicit, because: combined.because };
}

/** The setting for every file: the state that differs from the pack's, if it does, and the options, if any. */
function settingOf(port: Port, changed: Omit<Decided, "config"> | undefined, options: { config: RuleConfig; because: string } | undefined): LintConfigSetting {
  const because = [changed?.because, options?.because].filter((text): text is string => text !== undefined);
  return {
    ruleId: port.ruleId,
    folder: "",
    on: changed === undefined ? null : changed.on,
    config: options?.config ?? null,
    explicit: changed?.explicit ?? true,
    because: [...new Set(because)].join("; "),
  };
}

/** Where `decisions` settle for `port` in `folder`, with `options` for every file; `unknown` where they do not. */
function outcomeOf(port: Port, folder: string, decisions: readonly (PortDecision | FolderDecision)[], options?: { config: RuleConfig; because: string }): LintConfigOutcome {
  const settled = settle(decisions);
  if (settled === undefined) {
    const because = decisions.filter(decision => decision.on === "unknown").map(decision => decision.because).join("; ");
    return { ruleId: port.ruleId, folder, on: "unknown", config: null, explicit: false, because };
  }
  const because = [...new Set([settled.because, ...options === undefined ? [] : [options.because]])].join("; ");
  return { ruleId: port.ruleId, folder, on: settled.on, config: options?.config ?? null, explicit: settled.explicit, because };
}

/** Whether `port` is on where `outcome` applies: the pack's value where it is unknown. */
const stateOf = (port: Port, outcome: LintConfigOutcome): boolean => (outcome.on === "unknown" ? port.startsOn : outcome.on);

/** The setting for every file, if any, and the outcome for every file. */
function wholeSetting(port: Port, decisions: readonly PortDecision[], notes: string[]): { setting: LintConfigSetting | undefined; outcome: LintConfigOutcome } {
  noteConflict(port, decisions.filter((decision): decision is Decided => decision.on !== "unknown"), notes);
  const settled = settle(decisions);
  const options = (settled?.on ?? port.startsOn) ? agreedConfig(port.ruleId, decisions.filter((decision): decision is Decided => decision.on === true), notes) : undefined;
  const changed = settled?.on === !port.startsOn ? settled : undefined;
  return { setting: changed === undefined && options === undefined ? undefined : settingOf(port, changed, options), outcome: outcomeOf(port, "", decisions, options) };
}

/** One linter's say about a port, with what held-off ports may not take set aside. */
interface Settling {
  readonly decision: PortDecision | undefined;
  readonly folders: readonly FolderDecision[];
}

/** `settling`'s decisions in `folder`: its own there, else for the nearest folder holding it, else for every file. */
function decisionsIn(settling: Settling, folder: string): readonly (PortDecision | FolderDecision)[] {
  const holding = settling.folders.filter(decision => folder === decision.folder || folder.startsWith(`${decision.folder}/`));
  const deepest = Math.max(...holding.map(decision => decision.folder.length));
  const nearest = holding.filter(decision => decision.folder.length === deepest);
  return nearest.length > 0 ? nearest : settling.decision === undefined ? [] : [settling.decision];
}

/**
 * Per folder any linter decides in, the outcome of every linter's decisions there (`decisionsIn`), settled as for every
 * file, an unknown leaving the pack's value; and the settings they come to. A folder the same as the nearest folder
 * holding it (or, in none, as every file, on as `whole`) needs no setting: a rule's setting in the deepest folder
 * holding a file applies to it.
 */
function folderSettings(port: Port, settlings: readonly Settling[], whole: boolean): { settings: LintConfigSetting[]; outcomes: LintConfigOutcome[] } {
  const folders = [...new Set(settlings.flatMap(settling => settling.folders.map(decision => decision.folder)))];
  const outcomes = folders.map(folder => outcomeOf(port, folder, settlings.flatMap(settling => decisionsIn(settling, folder))));
  const settings = outcomes.flatMap(outcome => {
    const on = stateOf(port, outcome);
    const holder = outcomes.filter(other => outcome.folder.startsWith(`${other.folder}/`)).sort((left, right) => right.folder.length - left.folder.length)[0];
    if (on === (holder === undefined ? whole : stateOf(port, holder))) return [];
    return [{ ruleId: port.ruleId, folder: outcome.folder, on, config: null, explicit: outcome.explicit, because: outcome.because }];
  });
  return { settings, outcomes };
}

/**
 * Each linter's say about a held-off port without what turns it on; what turns it on by name (its own, a prefix or a
 * group the config selects) goes to `notes`, defaults and presets unremarked. Any other port's say is as it is.
 */
function heldOffSettlings(port: Port, verdicts: readonly PortVerdict[], notes: string[]): Settling[] {
  const settlings = verdicts.map(verdict => ({ decision: verdict.decision, folders: verdict.folders ?? [] }));
  if (port.heldOff === undefined) return settlings;
  const named = settlings.flatMap(settling => [...settling.decision === undefined ? [] : [settling.decision], ...settling.folders]).filter(decision => decision.on === true && decision.explicit);
  if (named.length > 0) notes.push(`${port.ruleId}: ${named.map(decision => decision.because).join("; ")} would turn it on, but its pack holds it off (${port.heldOff}); it stays off.`);
  return settlings.map(settling => ({
    decision: settling.decision?.on === true ? undefined : settling.decision,
    folders: settling.folders.filter(decision => decision.on !== true),
  }));
}

/**
 * The settings `verdicts` (each linter's, for one port) come to, and the outcomes they come from, adding to `notes` what
 * is not used.
 */
export function settleVerdicts(port: Port, verdicts: readonly PortVerdict[], notes: string[]): Pick<RepositoryLintConfig, "settings" | "outcomes"> {
  for (const verdict of verdicts) notes.push(...verdict.notes ?? []);
  const settlings = heldOffSettlings(port, verdicts, notes);
  const decisions = settlings.flatMap(settling => (settling.decision === undefined ? [] : [settling.decision]));
  const whole = wholeSetting(port, decisions, notes);
  const folders = folderSettings(port, settlings, stateOf(port, whole.outcome));
  return {
    settings: [...whole.setting === undefined ? [] : [whole.setting], ...folders.settings],
    outcomes: [...decisions.length === 0 ? [] : [whole.outcome], ...folders.outcomes],
  };
}
