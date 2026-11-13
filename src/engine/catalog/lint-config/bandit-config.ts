/**
 * Bandit's config, read as Bandit 1.9.4 (`packs/python/upstream/bandit/gates.yaml`) reads it, as data.
 *
 * Bandit reads two kinds of config (bandit/cli/main.py at the pin): a `.bandit` INI file it finds itself, by walking
 * the folders it scans (`_get_options_from_ini`, main.py:46), whose `[bandit]` section gives `tests`, `skips`,
 * `exclude`, `profile` and `configfile` as if on the command line; and a config file it reads only when given one, by
 * `-c` or by `.bandit`'s `configfile` (`BanditConfig`, bandit/core/config.py:26): the `[tool.bandit]` table of a TOML
 * file, or a YAML file. Rerail reads the `.bandit` at the repository root, the config file it names, and, when it
 * names none, `pyproject.toml`'s `[tool.bandit]`: Bandit reads that table only with `-c pyproject.toml`, and a
 * repository writes one only to run Bandit with it. A YAML config is not read: the rules Rerail ports from Bandit keep
 * the pack's values.
 *
 * Bandit runs the tests both configs list in `tests` (all of them when neither does) minus those both list in `skips`
 * (`_get_profile` and main.py:619-623, `BanditTestSet._get_filter`, bandit/core/test_set.py:24). It selects tests by
 * id only (`B101`): a test's name names none. `B001` stands for every blacklist test, unless the same list names one.
 * `exclude` and `exclude_dirs` name paths Bandit does not scan; one naming a whole folder turns every test off there.
 */
import { posix } from "node:path";
import { readIni } from "./formats/ini-format.js";
import { readFormat, readTomlStringLists } from "./formats/lint-config-formats.js";
import { NOT_A_FILE, readRootFile, type FolderDecision, type LinterConfig, type LinterNote, type LinterReader, type PortDecision, type PortVerdict } from "./linter-config.js";

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
/** YAML configs Bandit reads with `-c`, by the names its documentation gives them. */
const YAML_CONFIGS = ["bandit.yaml", "bandit.yml", ".bandit.yaml", ".bandit.yml"];
/** An exclude naming a whole folder: `tests`, `./tests`, `/tests`, `tests/`, `tests/*` or `tests/**`. */
const FOLDER_EXCLUDE = /^(?:\.?\/)?((?:[A-Za-z0-9_@.-]+\/)*[A-Za-z0-9_@.-]+)(?:\/|\/\*|\/\*\*)?$/u;

/** What one config file gives Bandit: tests and skips by id, and the paths it does not scan. */
interface BanditSource {
  /** How the person knows it: `.bandit`, `pyproject.toml [tool.bandit]`. */
  readonly label: string;
  readonly tests: readonly string[];
  readonly skips: readonly string[];
  readonly exclude: readonly string[];
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
  readonly folders: readonly FolderDecision[];
  /** Why something Bandit takes, and Rerail does not read, may select or skip tests. */
  readonly unread: string | undefined;
}

/** `text` split at commas as Bandit splits `.bandit`'s lists: spaces kept, empty entries dropped. */
const commaList = (text: string | undefined): string[] => (text ?? "").split(",").filter(entry => entry !== "");

/** The `[bandit]` section of `.bandit`, or why it is not read. */
function readBanditIni(root: string, notes: LinterNote[]): IniRead | undefined {
  const read = readRootFile(root, ".bandit");
  if (!read.present) return undefined;
  const unread = (why: string): IniRead => {
    notes.push({ linter: "bandit", text: `.bandit is not read: ${why}` });
    return { sources: [], unread: ".bandit is not read" };
  };
  const text = read.text;
  if (text === undefined) return unread(NOT_A_FILE);
  const ini = readFormat(() => readIni(text, { delimiters: ["=", ":"], lowerCaseOptions: true, interpolation: true }));
  if (!ini.ok) return unread(`${ini.why}.`);
  const section = ini.value.get("bandit");
  if (section === undefined) {
    notes.push({ linter: "bandit", text: ".bandit has no [bandit] section, so Bandit does not read it." });
    return undefined;
  }
  return iniOptions(section, notes);
}

/** What `.bandit`'s `[bandit]` options give. */
function iniOptions(section: ReadonlyMap<string, string>, notes: LinterNote[]): IniRead {
  const profile = section.get("profile");
  const targets = section.get("targets");
  if (targets !== undefined) notes.push({ linter: "bandit", text: `.bandit: targets ${JSON.stringify(targets)} is not read: Rerail does not limit rules to the paths Bandit scans.` });
  return {
    sources: [{ label: ".bandit", tests: commaList(section.get("tests")), skips: commaList(section.get("skips")), exclude: commaList(section.get("exclude")) }],
    configFile: section.get("configfile"),
    unread: profile === undefined ? undefined : `.bandit selects profile ${JSON.stringify(profile)}, which Rerail does not read`,
  };
}

const TOML_PATHS = ["tests", "skips", "exclude_dirs"].map(key => `tool.bandit.${key}`);

/** The `[tool.bandit]` table of TOML `text` from file `name`; undefined when it has none. Throws `LintConfigFormatError`. */
function tomlSource(text: string, name: string): BanditSource | undefined {
  const found = readTomlStringLists(text, TOML_PATHS);
  if (!found.paths.some(path => path === "tool.bandit" || path.startsWith("tool.bandit."))) return undefined;
  const [tests, skips, exclude] = TOML_PATHS.map(path => found.lists.get(path) ?? []);
  return { label: `${name} [tool.bandit]`, tests: tests!, skips: skips!, exclude: exclude! };
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

/** The folders the sources' excludes name, each turning every test off there; other excludes go to `notes`. */
function excludedFolders(sources: readonly BanditSource[], notes: LinterNote[]): FolderDecision[] {
  return sources.flatMap(source => source.exclude.flatMap(entry => {
    const key = source.label === ".bandit" ? "exclude" : "exclude_dirs";
    const folder = FOLDER_EXCLUDE.exec(entry)?.[1];
    if (folder === undefined || folder.split("/").some(part => part === "." || part === "..")) {
      notes.push({ linter: "bandit", text: `${source.label}: ${JSON.stringify(entry)} in ${key} is not read: Rerail sets rules for whole folders only, so the tests Bandit skips there stay on.` });
      return [];
    }
    return [{ folder, on: false, because: `${source.label}: ${JSON.stringify(entry)} in ${key}` }];
  }));
}

/** Bandit's test selection over `sources`; `unread` when something it takes is not read. */
function selection(sources: readonly BanditSource[], unread: string | undefined, notes: LinterNote[]): BanditSelection {
  noteUnknownIds(sources, notes);
  const tests = listed(sources, "tests");
  const skips = listed(sources, "skips");
  // main.py:624, `validate_profile`: Bandit stops when an id is both selected and skipped.
  const both = sources.flatMap(source => source.tests).filter(id => sources.some(source => source.skips.includes(id)));
  const conflict = both.length === 0 ? undefined : `Bandit stops with an error: ${[...new Set(both)].join(", ")} in both tests and skips`;
  if (conflict !== undefined) notes.push({ linter: "bandit", text: `${conflict}.` });
  return { tests, skips, labels: sources.map(source => source.label).join(" and "), folders: excludedFolders(sources, notes), unread: unread ?? conflict };
}

/** What Bandit's selection decides for test `id`. */
function decide(selected: BanditSelection, id: string): PortDecision {
  const skipped = selected.skips.get(id);
  if (skipped !== undefined) return { on: false, explicit: true, because: skipped };
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
 * the config file's tests and skips (`_get_profile`), so that file is then not read.
 */
function readSelection(root: string, notes: LinterNote[]): BanditSelection | undefined {
  const ini = readBanditIni(root, notes) ?? { sources: [] };
  if (ini.unread !== undefined) return selection(ini.sources, ini.unread, notes);
  const file = readConfigFile(root, ini.configFile, notes);
  if (typeof file === "string") return selection(ini.sources, file, notes);
  const sources = file === undefined ? ini.sources : [...ini.sources, file];
  return sources.length === 0 ? undefined : selection(sources, undefined, notes);
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
