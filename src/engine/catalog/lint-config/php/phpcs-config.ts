/**
 * PHP_CodeSniffer's ruleset at the repository root, read as data (`phpcs-ruleset.ts`) for the php pack's
 * PHP_CodeSniffer ports, as PHP_CodeSniffer 4.0.4 reads it (PHPCSStandards/PHP_CodeSniffer@bbdc3d0, the release
 * `packs/php/conformance.json` names), each port's messages decided by replaying the rulesets (`phpcs-leaf-replay.ts`)
 * and the arguments they set (`phpcs-arguments.ts`).
 *
 * `File::addMessage` (src/Files/File.php l.840-1030) leaves out a file any `<exclude-pattern>` of a message's code,
 * sniff, category or standard matches, as every ruleset's own `<exclude-pattern>`s and the `ignore` argument do for
 * every sniff (src/Filters/Filter.php l.225-295). A pattern is a regular expression matched anywhere in a file's path:
 * one that names a folder (`src/Legacy/*`, or `tests` behind `*` and `/`) turns the rule off in that folder, and a note
 * names the files elsewhere it also matches, where the rule stays on; any other pattern is a note. An
 * `<include-pattern>` limits the rule to the files it matches, so the rule keeps the pack's value.
 */
import {
  NOT_A_FILE,
  readRootFile,
  type FolderDecision,
  type LinterConfig,
  type LinterNote,
  type LinterReader,
  type Port,
  type PortDecision,
  type PortVerdict,
} from "../linter-config.js";
import { readFormat } from "../formats/lint-config-formats.js";
import { repositoryFiles } from "../repository-files.js";
import { isRepositoryFolder, repositoryPath } from "../repository-paths.js";
import { argsOf, argumentIgnores, thresholdArgument, type PhpcsRead } from "./phpcs-arguments.js";
import { combined, leafDecision, leavesOf, LeafReplay } from "./phpcs-leaf-replay.js";
import { RulesetReader, type Pattern, type PhpcsRuleset } from "./phpcs-ruleset.js";

/** The config files PHP_CodeSniffer looks for, in its order (`Config::CONFIG_FILENAMES`, src/Config.php l.143-148); it uses the first present. */
const PHPCS_FILES = [".phpcs.xml", "phpcs.xml", ".phpcs.xml.dist", "phpcs.xml.dist"];
/** A folder pattern: path segments, written as PHP_CodeSniffer's own rulesets write them, with `*` only around them. */
const FOLDER_PATTERN = /^(\*?\/)?([A-Za-z0-9_.-]+(?:\/[A-Za-z0-9_.-]+)*)(?:\/\**)?$/u;
/** How many of the files a folder pattern also matches elsewhere a note names. */
const SHOWN_FILES = 3;

/**
 * The folder of the repository a pattern names, which it leaves out whole, and the PHP files elsewhere it also
 * matches: PHP_CodeSniffer matches a pattern, `*` as `.*`, anywhere in a file's path, case-insensitively; undefined
 * for a pattern that names no folder.
 */
function patternFolder(root: string, pattern: string): { folder: string; elsewhere: string[] } | undefined {
  const written = FOLDER_PATTERN.exec(pattern)?.[2];
  const folder = written === undefined ? undefined : repositoryPath("", written);
  if (folder === undefined || folder === "" || !isRepositoryFolder(root, folder)) return undefined;
  const matches = new RegExp(pattern.replaceAll("*", ".*"), "iu");
  return { folder, elsewhere: repositoryFiles(root).filter(file => file.endsWith(".php") && !file.startsWith(`${folder}/`) && matches.test(`/${file}`)) };
}

/** The `<exclude-pattern>`s of `ruleset` and of every ruleset it references. */
function rulesetIgnores(ruleset: PhpcsRuleset): Pattern[] {
  return [...ruleset.ignores, ...ruleset.rules.flatMap(rule => (rule.ref.kind === "ruleset" ? rulesetIgnores(rule.ref.ruleset) : []))];
}

/** The folders every leaf's patterns leave out, noting the patterns that are not folders or leave out only some leaves. */
function folderDecisions(root: string, port: Port, patterns: readonly (readonly Pattern[])[], notes: string[]): FolderDecision[] {
  const unique = new Map(patterns.flat().map(entry => [entry.pattern, entry.because]));
  return [...unique].flatMap(([pattern, because]): FolderDecision[] => {
    const folder = patternFolder(root, pattern);
    const everyLeaf = patterns.every(leaf => leaf.some(entry => entry.pattern === pattern));
    if (folder === undefined || !everyLeaf) {
      notes.push(`${port.ruleId}: ${because} leaves out ${folder === undefined ? "files that are not one folder of the repository" : "only some of its messages"}; Rerail sets rules for whole folders only.`);
      return [];
    }
    noteElsewhere(port, because, folder, notes);
    return [{ folder: folder.folder, on: false, explicit: true, because }];
  });
}

/** Notes the files outside its folder a folder pattern also leaves out, where the rule stays on. */
function noteElsewhere(port: Port, because: string, folder: { folder: string; elsewhere: readonly string[] }, notes: string[]): void {
  const files = folder.elsewhere;
  if (files.length === 0) return;
  const more = files.length > SHOWN_FILES ? ` and ${files.length - SHOWN_FILES} more` : "";
  notes.push(`${port.ruleId}: ${because} also matches ${files.slice(0, SHOWN_FILES).join(", ")}${more} outside ${folder.folder}; Rerail sets rules for whole folders only, so the rule stays on there.`);
}

/** An `<include-pattern>` limits the rule to the files it matches, which Rerail does not set: the pack's value, unless the rule is off. */
function limitedDecision(decision: PortDecision, limited: string | undefined): PortDecision {
  if (limited === undefined || decision.on === false) return decision;
  return { on: "unknown", because: `${limited} limits it to the files it matches; Rerail sets rules for whole folders only` };
}

class PhpcsConfig implements LinterConfig {
  constructor(private readonly root: string, private readonly read: PhpcsRead) {}

  /**
   * The port's verdict. A code with include patterns has its exclude patterns passed over (`File::addMessage`,
   * l.968-1019), so under an include pattern only the patterns for every sniff set folders.
   */
  verdict(port: Port): PortVerdict | undefined {
    if (port.suppress.phpcs.length === 0) return undefined;
    const notes: string[] = [];
    const replays = leavesOf(port.suppress.phpcs).map(leaf => {
      const replay = new LeafReplay(leaf);
      return { leaf, result: replay.ruleset(this.read.ruleset), state: replay.state };
    });
    const states = replays.map(replay => replay.state);
    const decided = this.#thresholdDecision(combined(port, replays.map(({ leaf, result, state }) => leafDecision(this.read, leaf, result, state)), notes));
    const limited = states.flatMap(state => state.includePatterns)[0];
    const folders = folderDecisions(this.root, port, states.map(state => [...limited === undefined ? state.excludePatterns : [], ...this.read.ignores]), notes);
    for (const because of new Set(states.flatMap(state => state.properties))) notes.push(`${port.ruleId}: the properties of ${because} are not mapped; it keeps its default options.`);
    const decision = limitedDecision(decided, limited);
    return { decision, ...folders.length > 0 ? { folders } : {}, ...notes.length > 0 ? { notes } : {} };
  }

  /** A reporting threshold Rerail does not read leaves the pack's value, unless the rule is turned off by name. */
  #thresholdDecision(decision: PortDecision): PortDecision {
    const threshold = thresholdArgument(this.read);
    if (threshold === undefined || (decision.on === false && decision.explicit)) return decision;
    return { on: "unknown", because: `${threshold} changes which severities are reported` };
  }
}

/** The ruleset PHP_CodeSniffer uses at the root, its text (undefined when it is not a regular file), and the rulesets it passes over. */
interface UsedRuleset {
  readonly file: string;
  readonly text: string | undefined;
  readonly shadowed: readonly string[];
}

/** The ruleset PHP_CodeSniffer uses at the root, the first present in its order; undefined when there is none. */
function usedConfig(root: string): UsedRuleset | undefined {
  const present = PHPCS_FILES.flatMap(file => {
    const found = readRootFile(root, file);
    return found.present ? [{ file, text: found.text }] : [];
  });
  const used = present[0];
  return used === undefined ? undefined : { ...used, shadowed: present.slice(1).map(shadowed => shadowed.file) };
}

/** The ruleset files read: `ruleset` and every ruleset it references. */
function rulesetFiles(ruleset: PhpcsRuleset): string[] {
  return [ruleset.file, ...ruleset.rules.flatMap(rule => (rule.ref.kind === "ruleset" ? rulesetFiles(rule.ref.ruleset) : []))];
}

/**
 * The used ruleset as read, noting the rulesets PHP_CodeSniffer passes over (but those it references, which it reads
 * in turn), and why the used one is not read, if it is not.
 */
function readUsed(root: string, used: UsedRuleset, notes: LinterNote[]): PhpcsRuleset | undefined {
  const text = used.text;
  const parsed = text === undefined ? { ok: false as const, why: NOT_A_FILE.replace(/\.$/u, "") } : readFormat(() => new RulesetReader(root).ruleset(used.file, text, 0));
  const read = new Set(parsed.ok ? rulesetFiles(parsed.value) : []);
  for (const name of used.shadowed.filter(shadowed => !read.has(shadowed))) notes.push({ linter: "phpcs", text: `${name} is not read: PHP_CodeSniffer uses ${used.file} instead.` });
  if (parsed.ok) return parsed.value;
  notes.push({ linter: "phpcs", text: `${used.file} is not read: ${parsed.why}.` });
  return undefined;
}

/** Notes each reference not read, once, wherever it is. */
function noteUnread(ruleset: PhpcsRuleset, notes: LinterNote[]): void {
  for (const rule of ruleset.rules) {
    for (const ref of [rule.ref, ...rule.excludes]) {
      if (ref.kind === "unread") notes.push({ linter: "phpcs", text: `${ruleset.file}: ${JSON.stringify(ref.name)} is not read (${ref.why}), so the PHP_CodeSniffer rules Rerail ports that only a standard turns on keep the pack's values.` });
      if (ref.kind === "ruleset") noteUnread(ref.ruleset, notes);
    }
  }
}

export const phpcsReader: LinterReader = {
  linter: "phpcs",
  kinds: ["phpcs"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const used = usedConfig(root);
    if (used === undefined) return undefined;
    const ruleset = readUsed(root, used, notes);
    if (ruleset === undefined) return undefined;
    noteUnread(ruleset, notes);
    const args = argsOf(ruleset);
    const read: PhpcsRead = { ruleset, args, ignores: [...rulesetIgnores(ruleset), ...argumentIgnores(args)] };
    const threshold = thresholdArgument(read);
    if (threshold !== undefined) notes.push({ linter: "phpcs", text: `${threshold} changes which severities are reported, so the PHP_CodeSniffer rules Rerail ports keep the pack's values unless the config turns them off by name.` });
    return new PhpcsConfig(root, read);
  },
};
