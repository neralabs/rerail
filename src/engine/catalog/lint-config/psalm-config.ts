/**
 * Psalm's `psalm.xml` at the repository root, read as data (`xml-format.ts`) for the php pack's Psalm ports, as
 * Psalm 6.19.1 reads it (vimeo/psalm@e2ca442, the release `packs/php/conformance.json` names).
 *
 * Psalm reports every issue, at a level: an error, or info, which fails nothing and is shown only with `--show-info`
 * (off unless passed; `Psalm::initShowInfo`, src/Psalm/Internal/Cli/Psalm.php l.491-496). With no handler for an
 * issue, `errorLevel` (1, the strictest, to 8; 2 when absent) decides: an issue whose own level is above 0 and below
 * the config's is info (`Config::getReportingLevelForFile`, src/Psalm/Config.php l.2065-2086). An `<issueHandlers>`
 * entry names the issue: its `errorLevel` (`error` when absent, src/Psalm/Config/IssueHandler.php l.25) decides for
 * every file, and its `<errorLevel type="…">` children for the files they cover, the first that covers a file
 * winning. So an issue is on where it is an error, and off where it is info or suppressed. A plugin is code Rerail
 * never runs, and may suppress any issue: an issue only the level turns on keeps the pack's value under one.
 */
import { LintConfigFormatError, readFormat } from "./formats/lint-config-formats.js";
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
} from "./linter-config.js";
import { isRepositoryFolder, repositoryPath } from "./repository-paths.js";
import { parseXml, xmlChildren, type XmlElement } from "./formats/xml-format.js";

/** The config files Psalm looks for, in its order (`Config::DEFAULT_FILE_NAMES`, src/Psalm/Config.php l.142-145). */
const PSALM_FILES = ["psalm.xml", "psalm.xml.dist"];
/** The level Psalm runs at when `errorLevel` is absent (src/Psalm/Config.php l.1168-1180). */
const DEFAULT_LEVEL = 2;
/** An `errorLevel` Psalm takes (src/Psalm/Config.php l.1171). */
const LEVEL_TEXT = /^[1-8]$/u;
/** The reporting levels an issue handler takes (`Config::$ERROR_LEVELS`). */
const REPORTING_LEVELS = ["error", "info", "suppress"] as const;
type ReportingLevel = typeof REPORTING_LEVELS[number];
const XINCLUDE = "http://www.w3.org/2001/XInclude";

/**
 * Per issue the php pack ports, its `ERROR_LEVEL` at vimeo/psalm@e2ca442: ForbiddenCode (src/Psalm/Issue/
 * ForbiddenCode.php l.9). An issue of level L (above 0) is an error at config levels 1 to L and info above.
 */
const ISSUE_LEVELS: Readonly<Record<string, number>> = {
  ForbiddenCode: 4,
};

/** One `<errorLevel type="…">` of an issue handler: the folders it covers, and what else it names. */
interface PsalmFilter {
  readonly type: ReportingLevel;
  readonly folders: readonly string[];
  /** What else it covers (files, wildcards, functions, classes), for the person. */
  readonly unread: readonly string[];
}

/** An `<issueHandlers>` entry. */
interface PsalmHandler {
  readonly level: ReportingLevel;
  /** The entry, for the person: `psalm.xml: <ForbiddenCode errorLevel="suppress"/>`. */
  readonly because: string;
  readonly filters: readonly PsalmFilter[];
}

/** What Psalm's config sets for the ports. */
interface PsalmRead {
  readonly file: string;
  /** The config's `errorLevel`, or undefined when it sets none. */
  readonly level: number | undefined;
  readonly handlers: ReadonlyMap<string, PsalmHandler>;
  /** The plugins the config loads. */
  readonly plugins: readonly string[];
}

const reportingLevel = (written: string, what: string): ReportingLevel => {
  const level = REPORTING_LEVELS.find(known => known === written);
  if (level === undefined) throw new LintConfigFormatError(`${what} ${JSON.stringify(written)} is not error, info or suppress`);
  return level;
};

/** A `<directory name>`, as Psalm resolves it against the config's folder (`FileFilter::loadFromArray`, src/Psalm/Config/FileFilter.php l.121-146), as a folder of the repository; undefined for a wildcard or anything else. */
function filterFolder(root: string, written: string): string | undefined {
  const name = written.replace(/(?:\/\*\*)+\/?$/u, "");
  const path = name.includes("*") ? undefined : repositoryPath("", name);
  return path !== undefined && path !== "" && isRepositoryFolder(root, path) ? path : undefined;
}

/** One `<errorLevel type="…">` child of an issue handler. */
function filterOf(root: string, element: XmlElement): PsalmFilter {
  const type = reportingLevel(element.attributes.get("type") ?? "", "an <errorLevel> type");
  const folders: string[] = [];
  const unread: string[] = [];
  for (const child of element.children) {
    const name = child.attributes.get("name") ?? "";
    const folder = child.localName === "directory" ? filterFolder(root, name) : undefined;
    if (folder === undefined) unread.push(`<${child.localName} name=${JSON.stringify(name)}/>`);
    else folders.push(folder);
  }
  return { type, folders, unread };
}

/** The `<issueHandlers>` entries, by issue; a later entry for an issue replaces an earlier one, as Psalm's map does. */
function handlersOf(root: string, file: string, config: XmlElement): Map<string, PsalmHandler> {
  const handlers = new Map<string, PsalmHandler>();
  for (const element of xmlChildren(config, "issueHandlers").flatMap(handlers => handlers.children)) {
    const issue = element.localName === "PluginIssue" ? element.attributes.get("name") ?? "" : element.localName;
    const written = element.attributes.get("errorLevel");
    const level = written === undefined ? "error" : reportingLevel(written, `<${issue}> errorLevel`);
    const tag = written === undefined ? `<${issue}/>` : `<${issue} errorLevel="${written}"/>`;
    handlers.set(issue, { level, because: `${file}: ${tag}`, filters: xmlChildren(element, "errorLevel").map(filter => filterOf(root, filter)) });
  }
  return handlers;
}

/** The config as read; throws `LintConfigFormatError` for a config Psalm would refuse or Rerail does not read. */
function readConfig(root: string, file: string, text: string): PsalmRead {
  const config = parseXml(text);
  if (config.localName !== "psalm") throw new LintConfigFormatError("its root element is not <psalm>");
  if (containsXInclude(config)) throw new LintConfigFormatError("it includes other files (XInclude), which Rerail does not follow");
  const written = config.attributes.get("errorLevel");
  if (written !== undefined && !LEVEL_TEXT.test(written.trim())) throw new LintConfigFormatError(`errorLevel ${JSON.stringify(written)} is not a level Psalm takes`);
  const plugins = xmlChildren(config, "plugins").flatMap(element => element.children)
    .map(plugin => plugin.attributes.get("class") ?? plugin.attributes.get("filename") ?? plugin.localName);
  return { file, level: written === undefined ? undefined : Number(written.trim()), handlers: handlersOf(root, file, config), plugins };
}

/** Whether `element` or any element in it is an XInclude element, which Psalm expands (src/Psalm/Config.php l.745). */
function containsXInclude(element: XmlElement): boolean {
  return element.namespace === XINCLUDE || element.children.some(containsXInclude);
}

/** The level the config runs at, and the words that say where it comes from. */
function levelText(read: PsalmRead): { level: number; because: string } {
  return read.level === undefined
    ? { level: DEFAULT_LEVEL, because: `${read.file} sets no errorLevel, so Psalm runs at level ${DEFAULT_LEVEL}` }
    : { level: read.level, because: `${read.file}: errorLevel ${read.level}` };
}

/** What the level decides for `issue`, with no handler for it. */
function levelDecision(read: PsalmRead, issue: string): PortDecision {
  const issueLevel = ISSUE_LEVELS[issue];
  if (issueLevel === undefined) return { on: "unknown", because: `the level Psalm reports ${issue} at is not known to Rerail` };
  const { level, because } = levelText(read);
  if (issueLevel > 0 && issueLevel < level) {
    return { on: false, explicit: false, because: `${because}, where ${issue} is info, not an error (Psalm reports it as an error at levels 1 to ${issueLevel})` };
  }
  const plugin = read.plugins[0];
  if (plugin !== undefined) return { on: "unknown", because: `${read.file} loads plugin ${plugin}, which Rerail never runs and which may suppress ${issue}` };
  return { on: true, explicit: false, because };
}

/** The folders a handler's filters set, the first filter covering a folder winning; what else they cover goes to `notes`. */
function filterFolders(port: Port, handler: PsalmHandler, notes: string[]): FolderDecision[] {
  const decided: FolderDecision[] = [];
  for (const filter of handler.filters) {
    const because = `${handler.because.replace(/\/>$/u, ">")} <errorLevel type="${filter.type}">`;
    for (const unread of filter.unread) notes.push(`${port.ruleId}: ${because} covers ${unread}, which is not a folder of the repository; Rerail sets rules for whole folders only.`);
    const covered = (folder: string): boolean => decided.some(earlier => folder === earlier.folder || folder.startsWith(`${earlier.folder}/`));
    decided.push(...filter.folders.filter(folder => !covered(folder)).map(folder => ({ folder, on: filter.type === "error", because: `${because} <directory name="${folder}"/>` })));
  }
  return decided;
}

/** One issue's verdict: its handler's, or the level's. */
function issueVerdict(read: PsalmRead, port: Port, issue: string): { decision: PortDecision; folders: FolderDecision[]; notes: string[] } {
  const handler = read.handlers.get(issue);
  if (handler === undefined) return { decision: levelDecision(read, issue), folders: [], notes: [] };
  const notes: string[] = [];
  const folders = filterFolders(port, handler, notes);
  return { decision: { on: handler.level === "error", explicit: true, because: handler.because }, folders, notes };
}

/** The decisions for each issue of a port as one: the same state for each, else the pack's value. */
function combined(decisions: readonly PortDecision[]): PortDecision {
  const unknown = decisions.find(decision => decision.on === "unknown");
  if (unknown !== undefined) return unknown;
  const first = decisions[0]!;
  if (decisions.some(decision => decision.on !== first.on)) return { on: "unknown", because: `the config turns some of its issues on and others off (${decisions.map(decision => decision.because).join("; ")})` };
  return decisions.length === 1 ? first : { ...first, because: decisions.map(decision => decision.because).join("; ") };
}

class PsalmConfig implements LinterConfig {
  constructor(private readonly read: PsalmRead) {}

  verdict(port: Port): PortVerdict | undefined {
    if (port.suppress.psalm.length === 0) return undefined;
    const verdicts = port.suppress.psalm.map(issue => issueVerdict(this.read, port, issue));
    const folders = verdicts.flatMap(verdict => verdict.folders);
    const notes = verdicts.flatMap(verdict => verdict.notes);
    return { decision: combined(verdicts.map(verdict => verdict.decision)), ...folders.length > 0 ? { folders } : {}, ...notes.length > 0 ? { notes } : {} };
  }
}

/** The config file Psalm uses at the root, with its text; undefined (with a note when present but not read) otherwise. */
function usedConfig(root: string, notes: LinterNote[]): { file: string; text: string } | undefined {
  const present = PSALM_FILES.filter(name => readRootFile(root, name).present);
  const file = present[0];
  if (file === undefined) return undefined;
  for (const name of present.slice(1)) notes.push({ linter: "psalm", text: `${name} is not read: Psalm uses ${file} instead.` });
  const found = readRootFile(root, file);
  if (found.present && found.text !== undefined) return { file, text: found.text };
  notes.push({ linter: "psalm", text: `${file} is not read: ${NOT_A_FILE}` });
  return undefined;
}

export const psalmReader: LinterReader = {
  linter: "psalm",
  kinds: ["psalm"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const used = usedConfig(root, notes);
    if (used === undefined) return undefined;
    const read = readFormat(() => readConfig(root, used.file, used.text));
    if (!read.ok) {
      notes.push({ linter: "psalm", text: `${used.file} is not read: ${read.why}.` });
      return undefined;
    }
    for (const plugin of read.value.plugins) notes.push({ linter: "psalm", text: `${used.file}: plugin ${plugin} is not run, so the Psalm rules Rerail ports that only its level turns on keep the pack's values.` });
    return new PsalmConfig(read.value);
  },
};
