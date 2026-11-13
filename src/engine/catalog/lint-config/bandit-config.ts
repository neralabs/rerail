/**
 * Bandit's config, read as Bandit 1.9.4 (`packs/python/upstream/bandit/gates.yaml`) reads it, as data.
 *
 * Bandit reads two kinds of config (bandit/cli/main.py at the pin): a `.bandit` INI file it finds itself, by walking
 * the folders it scans (`_get_options_from_ini`, main.py:46), whose `[bandit]` section gives `tests`, `skips`,
 * `exclude`, `profile` and `configfile` as if on the command line; and a config file it reads only when given one, by
 * `-c` or by `.bandit`'s `configfile` (`BanditConfig`, bandit/core/config.py:26): the `[tool.bandit]` table of a TOML
 * file, or a YAML file. Rerail reads it as `bandit -r .` run at the repository root reads it: the one `.bandit` in the
 * repository, wherever it is (with two Bandit stops), the config file it names, and, when it names none,
 * `pyproject.toml`'s `[tool.bandit]`: Bandit reads that table only with `-c pyproject.toml`, and a repository writes
 * one only to run Bandit with it. A YAML config is not read: the rules Rerail ports from Bandit keep the pack's values.
 *
 * Bandit runs the tests both configs list in `tests` (all of them when neither does) minus those both list in `skips`
 * (`_get_profile` and main.py:619-623, `BanditTestSet._get_filter`, bandit/core/test_set.py:24). It selects tests by
 * id only (`B101`): a test's name names none. `B001` stands for every blacklist test, unless the same list names one.
 *
 * Bandit does not scan a file one of its excludes matches (`discover_files` and `_is_file_included`, manager.py:200
 * and 392): `.bandit`'s `exclude` (else its default `-x`, constants.py:30), each entry a folder at the root becoming
 * `<entry>/*`, and the config file's `exclude_dirs`, as written. An exclude matches a path as `bandit -r .` forms it
 * (`./scripts/a.py`) when `fnmatch` matches the whole path, or when the exclude appears anywhere in it: `./scripts`
 * skips `./scripts/`, `scripts` (a folder at the root, so `scripts/*`) skips nothing, `exclude_dirs = ["tests"]` skips
 * every path holding `tests`. Rerail turns every test off in each folder of the repository whose Python files an
 * exclude skips, all of them whatever their names, and notes the files it skips outside such folders.
 */
import { statSync } from "node:fs";
import { join, posix } from "node:path";
import { fnmatchPattern } from "./formats/fnmatch.js";
import { readIni } from "./formats/ini-format.js";
import { readFormat, readTomlStringLists } from "./formats/lint-config-formats.js";
import { NOT_A_FILE, readRootFile, type FolderDecision, type LinterConfig, type LinterNote, type LinterReader, type PortDecision, type PortVerdict } from "./linter-config.js";
import { repositoryFiles } from "./repository-files.js";
import { repositoryPath } from "./repository-paths.js";

/**
 * Bandit's blacklist tests at the pin, which `B001` stands for: the calls of bandit/blacklists/calls.py (lines
 * 361-654) and the imports of bandit/blacklists/imports.py (lines 249-416).
 */
const BLACKLIST_TESTS: readonly string[] = [
  "B301", "B302", "B303", "B304", "B305", "B306", "B307", "B308", "B310", "B311", "B312", "B313", "B314", "B315", "B316",
  "B317", "B318", "B319", "B321", "B323", "B401", "B402", "B403", "B404", "B405", "B406", "B407", "B408", "B409", "B411",
  "B412", "B413", "B415",
];
const BLACKLIST = "B001";
const TEST_ID = /^B\d{3}$/u;
const BANDIT_INI = ".bandit";
/** YAML configs Bandit reads with `-c`, by the names its documentation gives them. */
const YAML_CONFIGS = ["bandit.yaml", "bandit.yml", ".bandit.yaml", ".bandit.yml"];
/** Bandit's `-x` default (bandit/core/constants.py:30), which `.bandit`'s `exclude` replaces. */
const DEFAULT_EXCLUDE = [".svn", "CVS", ".bzr", ".hg", ".git", "__pycache__", ".tox", ".eggs", "*.egg"];
/** A note names at most this many files. */
const MAX_NOTED_FILES = 3;

/** An exclude: where it is written, and the globs Bandit may match paths with; two when Rerail cannot tell which. */
interface Exclude {
  readonly because: string;
  readonly globs: readonly string[];
  /** Written in a config, not Bandit's default: one that skips nothing is worth a note. */
  readonly written: boolean;
}

/** What one config file gives Bandit: tests and skips by id, the paths it does not scan, and the files it does. */
interface BanditSource {
  /** How the person knows it: `.bandit`, `pyproject.toml [tool.bandit]`. */
  readonly label: string;
  readonly tests: readonly string[];
  readonly skips: readonly string[];
  readonly exclude: readonly Exclude[];
  /** `include`, when the config file sets it: the files Bandit scans. */
  readonly include?: readonly string[] | undefined;
}

/** What `.bandit` gives: its own options (none when it is not read), the config file it names, and why Rerail cannot tell all it selects. */
interface IniRead {
  readonly sources: readonly BanditSource[];
  readonly configFile?: string | undefined;
  readonly unread?: string | undefined;
}

/** Bandit's test selection over every config read: each listed id with where it is listed. */
interface BanditSelection {
  readonly tests: ReadonlyMap<string, string>;
  readonly skips: ReadonlyMap<string, string>;
  /** The configs read, for the person: `.bandit and pyproject.toml [tool.bandit]`. */
  readonly labels: string;
  /** Every test off, or not known, for every file: an exclude skips (or may skip) every Python file. */
  readonly everyFile: PortDecision | undefined;
  readonly folders: readonly FolderDecision[];
  /** Why something Bandit takes, and Rerail does not read, may select or skip tests. */
  readonly unread: string | undefined;
}

/** `text` split at commas as Bandit splits `.bandit`'s lists: spaces kept, empty entries dropped. */
const commaList = (text: string | undefined): string[] => (text ?? "").split(",").filter(entry => entry !== "");

/** `glob` once `os.path.join` puts `*` after it. */
const inFolder = (glob: string): string => (glob.endsWith("/") ? `${glob}*` : `${glob}/*`);

/** Whether `os.path.isdir` holds for `entry` run at `root`; undefined where it depends on what is outside the repository. */
function isFolder(root: string, entry: string): boolean | undefined {
  // `os.path.isdir("")` is false; an entry outside the repository depends on the machine Bandit runs on.
  if (entry === "") return false;
  const path = repositoryPath("", entry);
  if (path === undefined) return undefined;
  try {
    return statSync(join(root, path)).isDirectory();
  } catch {
    return false;
  }
}

/** The globs `-x` entry `entry` becomes (main.py `discover_files`): a folder at the root `<entry>/*`, else as written. */
function commandLineExclude(root: string, entry: string, because: string, written: boolean): Exclude {
  const folder = isFolder(root, entry);
  const globs = folder === undefined ? [entry, inFolder(entry)] : [folder ? inFolder(entry) : entry];
  return { because, globs, written };
}

/** `.bandit`'s `exclude`, split at commas as Bandit splits it (empty entries kept); Bandit's default `-x` when it is empty or unset. */
function iniExcludes(root: string, label: string, value: string | undefined): Exclude[] {
  if (value === undefined || value === "") return DEFAULT_EXCLUDE.map(entry => commandLineExclude(root, entry, `Bandit's default exclude ${JSON.stringify(entry)}`, false));
  return value.split(",").map(entry => commandLineExclude(root, entry, `${label}: ${JSON.stringify(entry)} in exclude`, true));
}

/** The `.bandit` files `bandit -r .` finds, each with its text (none when it is not read): the root's, and every other the repository lists. */
function banditInis(root: string): { name: string; text: string | undefined }[] {
  const names = new Set([BANDIT_INI, ...repositoryFiles(root).filter(path => posix.basename(path) === BANDIT_INI)]);
  return [...names].flatMap(name => {
    const read = readRootFile(root, name);
    return read.present ? [{ name, text: read.text }] : [];
  });
}

/** The `[bandit]` section of the `.bandit` Bandit finds, or why it is not read; undefined when there is none. */
function readBanditIni(root: string, notes: LinterNote[]): IniRead | undefined {
  const found = banditInis(root);
  const unread = (what: string, why: string): IniRead => {
    notes.push({ linter: "bandit", text: `${what}: ${why}` });
    return { sources: [], unread: what };
  };
  if (found.length > 1) return unread(`${found.map(file => file.name).join(", ")} are not read`, "bandit -r . stops with an error when it finds more than one .bandit.");
  if (found.length === 0) return undefined;
  const { name, text } = found[0]!;
  if (text === undefined) return unread(`${name} is not read`, NOT_A_FILE);
  const ini = readFormat(() => readIni(text, { delimiters: ["=", ":"], lowerCaseOptions: true, interpolation: true }));
  if (!ini.ok) return unread(`${name} is not read`, `${ini.why}.`);
  const section = ini.value.get("bandit");
  if (section === undefined) {
    notes.push({ linter: "bandit", text: `${name} has no [bandit] section, so Bandit does not read it.` });
    return undefined;
  }
  return iniOptions(root, name, section, notes);
}

/** What `.bandit`'s `[bandit]` options give. */
function iniOptions(root: string, label: string, section: ReadonlyMap<string, string>, notes: LinterNote[]): IniRead {
  const profile = section.get("profile");
  const targets = section.get("targets");
  if (targets !== undefined) notes.push({ linter: "bandit", text: `${label}: targets ${JSON.stringify(targets)} is not read: Rerail does not limit rules to the paths Bandit scans.` });
  return {
    sources: [{ label, tests: commaList(section.get("tests")), skips: commaList(section.get("skips")), exclude: iniExcludes(root, label, section.get("exclude")) }],
    configFile: section.get("configfile"),
    unread: profile === undefined ? undefined : `${label} selects profile ${JSON.stringify(profile)}, which Rerail does not read`,
  };
}

const TOML_PATHS = ["tests", "skips", "exclude_dirs", "include"].map(key => `tool.bandit.${key}`);

/** The `[tool.bandit]` table of TOML `text` from file `name`; undefined when it has none. Throws `LintConfigFormatError`. */
function tomlSource(text: string, name: string): BanditSource | undefined {
  const found = readTomlStringLists(text, TOML_PATHS);
  if (!found.paths.some(path => path === "tool.bandit" || path.startsWith("tool.bandit."))) return undefined;
  const label = `${name} [tool.bandit]`;
  const [tests, skips, exclude, include] = TOML_PATHS.map(path => found.lists.get(path));
  const excludes = (exclude ?? []).map(entry => ({ because: `${label}: ${JSON.stringify(entry)} in exclude_dirs`, globs: [entry], written: true }));
  return { label, tests: tests ?? [], skips: skips ?? [], exclude: excludes, include };
}

/** The `[tool.bandit]` table of TOML file `name`: undefined when it has none, a string when it is not read. */
function readBanditToml(root: string, name: string, notes: LinterNote[]): BanditSource | string | undefined {
  const read = readRootFile(root, name);
  if (!read.present) return undefined;
  const text = read.text;
  if (text === undefined) {
    notes.push({ linter: "bandit", text: `${name} is not read: ${NOT_A_FILE}` });
    return `${name} is not read`;
  }
  const source = readFormat(() => tomlSource(text, name));
  if (source.ok) return source.value;
  // A file that may hold no Bandit config at all is not worth a note.
  if (!text.includes("bandit")) return undefined;
  notes.push({ linter: "bandit", text: `${name} is not read: ${source.why}.` });
  return `${name} is not read`;
}

/** The config file `.bandit` names, as Bandit reads it from the repository root, or why it is not read. */
function readNamedConfig(root: string, named: string, notes: LinterNote[]): BanditSource | string | undefined {
  const name = posix.normalize(named);
  // `..` itself is no TOML file, so the `.toml` check refuses it.
  const inRepository = !posix.isAbsolute(name) && !name.startsWith("../");
  if (!inRepository || !name.endsWith(".toml")) {
    notes.push({ linter: "bandit", text: `.bandit: configfile ${JSON.stringify(named)} is not read: Rerail reads TOML configs in the repository only.` });
    return `.bandit: configfile ${JSON.stringify(named)} is not read`;
  }
  if (!readRootFile(root, name).present) {
    notes.push({ linter: "bandit", text: `.bandit: configfile ${JSON.stringify(named)} does not exist, so Bandit stops with an error.` });
    return `.bandit: configfile ${JSON.stringify(named)} does not exist`;
  }
  return readBanditToml(root, name, notes) ?? { label: `${name} [tool.bandit]`, tests: [], skips: [], exclude: [] };
}

/** The config file Bandit gets with `-c`: the one `.bandit` names, else `pyproject.toml`'s `[tool.bandit]`; YAML not read. */
function readConfigFile(root: string, named: string | undefined, notes: LinterNote[]): BanditSource | string | undefined {
  if (named !== undefined) return readNamedConfig(root, named, notes);
  const yaml = YAML_CONFIGS.find(name => readRootFile(root, name).present);
  if (yaml !== undefined) {
    notes.push({ linter: "bandit", text: `${yaml} is not read: Rerail reads Bandit's INI and TOML configs, not YAML. The rules Rerail ports from Bandit keep the pack's values.` });
    return `${yaml} is not read`;
  }
  return readBanditToml(root, "pyproject.toml", notes);
}

/** The ids the sources list under `key`, each with the first place it is listed; `B001` expanded as Bandit does. */
function listed(sources: readonly BanditSource[], key: "tests" | "skips"): Map<string, string> {
  const ids = new Map<string, string>();
  for (const source of sources) {
    for (const id of source[key]) if (!ids.has(id)) ids.set(id, `${source.label}: ${JSON.stringify(id)} in ${key}`);
  }
  expandBlacklist(ids);
  return ids;
}

/** Replaces `B001` in `ids` by every blacklist test, unless `ids` names one of them (test_set.py:37-47). */
function expandBlacklist(ids: Map<string, string>): void {
  const blacklist = ids.get(BLACKLIST);
  ids.delete(BLACKLIST);
  if (blacklist === undefined || BLACKLIST_TESTS.some(id => ids.has(id))) return;
  for (const id of BLACKLIST_TESTS) ids.set(id, `${blacklist} (every blacklist test)`);
}

/** Notes each entry of `tests` or `skips` that is no test id: Bandit warns and matches it to no test. */
function noteUnknownIds(sources: readonly BanditSource[], notes: LinterNote[]): void {
  for (const source of sources) {
    for (const key of ["tests", "skips"] as const) {
      const unknown = source[key].filter(id => !TEST_ID.test(id));
      for (const id of unknown) notes.push({ linter: "bandit", text: `${source.label}: ${JSON.stringify(id)} in ${key} is not a Bandit test id such as "B101", so it names no test.` });
    }
  }
}

/**
 * Whether `fnmatch` matches `glob` against every Python file under the folder `prefix` (`./src/`, `./` for the root):
 * it ends in `*` (or `*.py`) and matches the prefix with that `*` standing for the rest of the path.
 */
function matchesEveryFile(glob: string, prefix: string): boolean {
  const open = glob.endsWith("*.py") ? glob.slice(0, -3) : glob;
  return open.endsWith("*") && fnmatchPattern(open).test(prefix);
}

/** Whether `path` (a folder or file, relative to the root) is in folder `folder`. */
const within = (path: string, folder: string): boolean => folder === "" || path === folder || path.startsWith(`${folder}/`);

/** What `exclude` does in the folder `folder`: every test off when each of its globs skips all of it, unknown when some do. */
function folderVerdict(exclude: Exclude, folder: string): FolderDecision | undefined {
  const prefix = folder === "" ? "./" : `./${folder}/`;
  // A glob the folder's path holds skips every file in it, as does one `fnmatch` matches every file in it with.
  const skipping = exclude.globs.filter(glob => prefix.includes(glob) || matchesEveryFile(glob, prefix)).length;
  if (skipping === 0) return undefined;
  return skipping === exclude.globs.length ? { folder, on: false, explicit: true, because: exclude.because } : { folder, on: "unknown", because: exclude.because };
}

/** The decisions `excludes` make in `folders` (the root and every folder holding a Python file, outermost first), none inside a folder already off. */
function folderDecisions(excludes: readonly Exclude[], folders: readonly string[]): FolderDecision[] {
  const decided: FolderDecision[] = [];
  for (const folder of folders) {
    if (decided.some(decision => decision.on === false && within(folder, decision.folder))) continue;
    decided.push(...excludes.flatMap(exclude => folderVerdict(exclude, folder) ?? []));
  }
  return decided;
}

/** `files`, the first few by name, for a note. */
function fileList(files: readonly string[]): string {
  const named = files.slice(0, MAX_NOTED_FILES).join(", ");
  return files.length > MAX_NOTED_FILES ? `${named} and ${files.length - MAX_NOTED_FILES} more` : named;
}

/** Notes what `exclude` skips outside the folders turned off (`rest`), or that a written one skips nothing. */
function noteUnmapped(exclude: Exclude, rest: readonly string[], decided: readonly FolderDecision[], notes: LinterNote[]): void {
  // `_is_file_included`: a glob skips a file `fnmatch` matches whole, or one whose path (`./src/a.py`) holds it.
  const skipped = rest.filter(file => exclude.globs.some(glob => `./${file}`.includes(glob) || fnmatchPattern(glob).test(`./${file}`)));
  if (skipped.length > 0) {
    notes.push({ linter: "bandit", text: `${exclude.because}: Bandit skips ${fileList(skipped)}, which Rerail cannot set rules for alone (it sets them for whole folders), so the tests stay on there.` });
  } else if (exclude.written && !decided.some(decision => decision.because === exclude.because)) {
    notes.push({ linter: "bandit", text: `${exclude.because} skips no Python file: bandit -r . matches it against paths such as ./src/app.py, so the tests stay on.` });
  }
}

/** Every decision at the root: off when an exclude skips every file, else unknown when one may. */
function rootDecision(decided: readonly FolderDecision[]): PortDecision | undefined {
  const atRoot = decided.filter(decision => decision.folder === "");
  if (atRoot.length === 0) return undefined;
  const because = atRoot.map(decision => decision.because).join("; ");
  return atRoot.some(decision => decision.on === false) ? { on: false, explicit: true, because } : { on: "unknown", because };
}

/** Where `excludes` turn every test off (or may): for every file, per folder; what they skip otherwise goes to `notes`. */
function excludedFolders(root: string, excludes: readonly Exclude[], notes: LinterNote[]): Pick<BanditSelection, "everyFile" | "folders"> {
  const files = repositoryFiles(root).filter(path => path.endsWith(".py"));
  const folders = new Set([""]);
  for (const file of files) for (let at = file.indexOf("/"); at !== -1; at = file.indexOf("/", at + 1)) folders.add(file.slice(0, at));
  const decided = folderDecisions(excludes, [...folders].sort((left, right) => left.split("/").length - right.split("/").length));
  const rest = files.filter(file => !decided.some(decision => within(file, decision.folder)));
  for (const exclude of excludes) noteUnmapped(exclude, rest, decided, notes);
  return { everyFile: rootDecision(decided), folders: decided.filter(decision => decision.folder !== "") };
}

/** Why `include` leaves Rerail unable to tell which files Bandit scans: no glob of it takes every Python file. */
function limitedInclude(sources: readonly BanditSource[], notes: LinterNote[]): string | undefined {
  const source = sources.find(({ include }) => include !== undefined && !include.some(glob => matchesEveryFile(glob, "./")));
  if (source === undefined) return undefined;
  const why = `${source.label}: include ${JSON.stringify(source.include)} limits the files Bandit scans, which Rerail does not read`;
  notes.push({ linter: "bandit", text: `${why}.` });
  return why;
}

/** Bandit's test selection over `sources`, excluding theirs and `defaults`; `unread` when something it takes is not read. */
function selection(root: string, sources: readonly BanditSource[], unread: string | undefined, notes: LinterNote[], defaults: readonly Exclude[] = []): BanditSelection {
  noteUnknownIds(sources, notes);
  const tests = listed(sources, "tests");
  const skips = listed(sources, "skips");
  // main.py:624, `validate_profile`: Bandit stops when an id is both selected and skipped.
  const both = sources.flatMap(source => source.tests).filter(id => sources.some(source => source.skips.includes(id)));
  const conflict = both.length === 0 ? undefined : `Bandit stops with an error: ${[...new Set(both)].join(", ")} in both tests and skips`;
  if (conflict !== undefined) notes.push({ linter: "bandit", text: `${conflict}.` });
  const why = unread ?? conflict ?? limitedInclude(sources, notes);
  const excluded = why === undefined ? excludedFolders(root, [...defaults, ...sources.flatMap(source => source.exclude)], notes) : { everyFile: undefined, folders: [] };
  return { tests, skips, labels: sources.map(source => source.label).join(" and "), ...excluded, unread: why };
}

/** What Bandit's selection decides for test `id`. */
function decide(selected: BanditSelection, id: string): PortDecision {
  const skipped = selected.skips.get(id);
  if (skipped !== undefined) return { on: false, explicit: true, because: skipped };
  if (selected.everyFile !== undefined) return selected.everyFile;
  if (selected.unread !== undefined) return { on: "unknown", because: selected.unread };
  if (selected.tests.size === 0) return { on: true, explicit: false, because: `${selected.labels}: no tests set, so Bandit runs every test` };
  const tested = selected.tests.get(id);
  return tested === undefined ? { on: false, explicit: false, because: `${selected.labels} tests does not list it` } : { on: true, explicit: true, because: tested };
}

/** The order a port's decisions over several ids settle in, as across linters. */
const rank = (decision: PortDecision): number => {
  if (decision.on === "unknown") return 3;
  if (decision.explicit) return decision.on ? 1 : 0;
  return decision.on ? 2 : 4;
};

/**
 * Bandit's selection over the configs at `root`; undefined when there is none. A profile `.bandit` selects replaces
 * the config file's tests and skips (`_get_profile`), so that file is then not read. Without a `.bandit`, Bandit's
 * default `-x` excludes.
 */
function readSelection(root: string, notes: LinterNote[]): BanditSelection | undefined {
  const ini = readBanditIni(root, notes);
  if (ini?.unread !== undefined) return selection(root, ini.sources, ini.unread, notes);
  const file = readConfigFile(root, ini?.configFile, notes);
  return ini === undefined && file === undefined ? undefined : configSelection(root, ini, file, notes);
}

/** Bandit's selection over `.bandit` (`ini`, none: Bandit's default `-x`) and the config file it takes (`file`, a string when it is not read). */
function configSelection(root: string, ini: IniRead | undefined, file: BanditSource | string | undefined, notes: LinterNote[]): BanditSelection {
  const sources = [...(ini?.sources ?? []), ...(typeof file === "object" ? [file] : [])];
  return selection(root, sources, typeof file === "string" ? file : undefined, notes, ini === undefined ? iniExcludes(root, "", undefined) : []);
}

export const banditReader: LinterReader = {
  linter: "bandit",
  kinds: ["nosec"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const selected = readSelection(root, notes);
    if (selected === undefined) return undefined;
    return {
      verdict(port): PortVerdict | undefined {
        if (port.suppress.nosec.length === 0) return undefined;
        const decision = port.suppress.nosec.map(id => decide(selected, id)).sort((left, right) => rank(left) - rank(right))[0]!;
        return { decision, folders: selected.folders };
      },
    };
  },
};
