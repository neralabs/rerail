/**
 * A repository's own lint configuration, read at set-up so Rerail does not steer against what the repository
 * deliberately configured. Only config files in the repository are read, and only as data: ESLint's flat config
 * (`eslint.config.*` at the root, parsed and never run: `eslint-flat-config.ts`), `.eslintrc.json`, `.eslintrc` (JSON)
 * or `package.json` `eslintConfig` (`rules` and `overrides`), oxlint's `.oxlintrc.json` (top-level `rules`) and Ruff's
 * rule selection (`select`, `extend-select`, `ignore`, `extend-ignore`, `per-file-ignores`; `.ruff.toml`, `ruff.toml` or
 * `pyproject.toml`, and the configs its `extend` names). Config code is never run or imported; `.eslintrc.js` and YAML
 * configs are reported as not read.
 *
 * What is read becomes settings for the rules of the packs being set up, matched through each port's `violation.
 * suppress` (the upstream rule names the port honours suppression comments for): a rule turned off for every file
 * (`"off"`, `0`, a Ruff ignore that wins over the selects) starts off; one turned off for exactly one folder's files
 * (`files: ["tests/**"]`, a Ruff per-file ignore of `tests/**`) starts off in that folder; ESLint options become the
 * port's `config` where they map onto the options its document declares. A rule some config turns off starts off even
 * where another config keeps it on; a flat config entry whose value cannot be read without running the config turns
 * the port off. Nothing narrower than a folder turns a rule off: it stays on, with a note. An oxlint config key is
 * resolved to a built-in oxlint rule as oxlint resolves it (`oxlintRuleOfKey`), not by its bare name as its
 * suppression comments are: `custom/no-console` is another plugin's rule and leaves ESLint's `no-console` alone.
 */
import { lstatSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";
import type { CodeRuleDslOptionDeclaration, CodeRuleDslValue } from "../code-rule/code-rule.js";
import { stableJsonText } from "../stable-json.js";
import { folderOfGlob, readEslintFlatConfig, type FlatConfigRead } from "./eslint-flat-config.js";
import { LintConfigFormatError, parseJsonc, readTomlStringLists } from "./lint-config-formats.js";
import type { BundledCatalog, RuleConfig } from "./repository-model.js";
import { configureBundledCodeRule } from "./rule-config.js";

/** A config file larger than this is not one a person wrote; it is reported as not read. As `package.json` in `rule-pack-matcher.ts`. */
const MAX_CONFIG_BYTES = 1024 * 1024;

/** ESLint's flat config files: code, never run. */
const ESLINT_FLAT_CONFIGS = ["eslint.config.js", "eslint.config.mjs", "eslint.config.cjs", "eslint.config.ts", "eslint.config.mts", "eslint.config.cts"];
/** ESLint's legacy config files in the order ESLint looks for them; the first present is the one it uses. */
const ESLINTRC_FILES = [".eslintrc.js", ".eslintrc.cjs", ".eslintrc.yaml", ".eslintrc.yml", ".eslintrc.json", ".eslintrc"];
const ESLINTRC_CODE = new Set([".eslintrc.js", ".eslintrc.cjs"]);
/** Ruff's config files in the order of precedence when one folder has several. */
const RUFF_FILES = [".ruff.toml", "ruff.toml", "pyproject.toml"];
/** Ruff's rule selection lists; each is read from the `lint` section, else from the top level as Ruff does. */
const RUFF_LISTS = ["select", "extend-select", "ignore", "extend-ignore"] as const;
const RUFF_TABLES = ["per-file-ignores", "extend-per-file-ignores"] as const;
/** A Ruff selector: `ALL`, or a rule code or prefix of one (`E`, `E7`, `E711`, `PLR09`). */
const RUFF_SELECTOR = /^(?:ALL|[A-Z]+[0-9]*)$/u;
const NEVER_RUN = "Rerail never runs your config.";

/**
 * A rule built into oxlint: oxlint's internal plugin name (`eslint`, `typescript`, `react`, `jsx_a11y`, `nextjs`,
 * `node`, ...) and the rule's name.
 */
interface OxlintRule {
  readonly plugin: string;
  readonly name: string;
}

/**
 * A config key's plugin, as oxlint 1.88.0 names its built-in plugins (oxc `crates/oxc_linter/src/config/plugins.rs`
 * `TryFrom<&str> for LintPlugins` l.164-194, then `unalias_plugin_name` in `config/rules.rs` l.672-688: `eslint` stays,
 * the others take their `From<LintPlugins>` name with `-` as `_`). Any other plugin is an external (JS) plugin.
 */
const OXLINT_PLUGINS: Readonly<Record<string, string>> = {
  eslint: "eslint",
  react: "react", "react-hooks": "react", react_hooks: "react",
  unicorn: "unicorn",
  typescript: "typescript", "typescript-eslint": "typescript", typescript_eslint: "typescript", "@typescript-eslint": "typescript",
  oxc: "oxc", deepscan: "oxc",
  import: "import", "import-x": "import",
  jsdoc: "jsdoc",
  jest: "jest",
  vitest: "vitest",
  "jsx-a11y": "jsx_a11y", jsx_a11y: "jsx_a11y", "jsx-a11y-x": "jsx_a11y", "jsx_a11y-x": "jsx_a11y",
  nextjs: "nextjs", "@next": "nextjs", "@next/next": "nextjs",
  "react-perf": "react_perf", react_perf: "react_perf",
  promise: "promise",
  node: "node",
  vue: "vue",
};

/**
 * The plugin a bare config key names when several built-in plugins have a rule of that name: the plugin of the first
 * such rule in oxlint 1.88.0's `RULES` (`crates/oxc_linter/src/generated/rules_enum.rs` l.22192), which
 * `parse_rule_key` takes (`config/rules.rs` l.647-657). A name only one plugin has names that plugin; a name no
 * built-in rule has names `eslint`.
 */
const OXLINT_SHARED_RULE_NAMES: Readonly<Record<string, string>> = Object.fromEntries(Object.entries({
  eslint: [
    "no-dupe-keys", "no-implied-eval", "no-lonely-if", "no-negated-condition", "no-nested-ternary", "prefer-promise-reject-errors",
    "prefer-spread", "require-await",
  ],
  import: ["no-anonymous-default-export", "no-namespace"],
  typescript: ["prefer-includes", "prefer-string-starts-ends-with"],
  react: ["require-render-return"],
  jest: [
    "consistent-test-it", "expect-expect", "max-expects", "max-nested-describe", "no-alias-methods", "no-commented-out-tests",
    "no-conditional-expect", "no-conditional-in-test", "no-disabled-tests", "no-duplicate-hooks", "no-focused-tests", "no-hooks",
    "no-identical-title", "no-interpolation-in-snapshots", "no-large-snapshots", "no-mocks-import", "no-restricted-matchers",
    "no-standalone-expect", "no-test-prefixes", "no-test-return-statement", "no-unneeded-async-expect-function",
    "padding-around-after-all-blocks", "padding-around-test-blocks", "prefer-called-with", "prefer-comparison-matcher", "prefer-each",
    "prefer-equality-matcher", "prefer-expect-assertions", "prefer-expect-resolves", "prefer-hooks-in-order", "prefer-hooks-on-top",
    "prefer-lowercase-title", "prefer-mock-promise-shorthand", "prefer-mock-return-shorthand", "prefer-snapshot-hint", "prefer-spy-on",
    "prefer-strict-equal", "prefer-to-be", "prefer-to-contain", "prefer-to-have-been-called-times", "prefer-to-have-length",
    "prefer-todo", "require-hook", "require-to-throw-message", "require-top-level-describe", "valid-describe-callback", "valid-expect",
    "valid-expect-in-promise", "valid-title",
  ],
}).flatMap(([plugin, rules]) => rules.map(rule => [rule, plugin])));

/**
 * The ESLint rules oxlint runs for typescript-eslint's extension rules of the same name (oxc
 * `crates/oxc_linter/src/utils/mod.rs` l.51-90, applied by `transform_rule_and_plugin_name` in `config/rules.rs`
 * l.244-253): `typescript/no-unused-vars` configures oxlint's `no-unused-vars`.
 */
const OXLINT_TYPESCRIPT_AS_ESLINT = new Set([
  "class-methods-use-this", "default-param-last", "init-declarations", "max-params", "no-array-constructor", "no-dupe-class-members",
  "no-empty-function", "no-invalid-this", "no-loop-func", "no-loss-of-precision", "no-magic-numbers", "no-redeclare",
  "no-restricted-imports", "no-shadow", "no-unused-expressions", "no-unused-vars", "no-use-before-define", "no-useless-constructor",
]);

/** A plugin package name without its `eslint-plugin`/`oxlint-plugin` part (oxc `config/plugins.rs` l.30-63). */
function normalizeOxlintPluginName(name: string): string {
  const prefixes = ["eslint-plugin", "oxlint-plugin"];
  const slash = name.indexOf("/");
  if (slash >= 0) {
    const scope = name.slice(0, slash);
    const rest = name.slice(slash + 1);
    for (const prefix of prefixes) {
      if (rest === prefix) return scope;
      if (rest.startsWith(`${prefix}-`)) return `${scope}/${rest.slice(prefix.length + 1)}`;
    }
  }
  const prefix = prefixes.find(candidate => name.startsWith(`${candidate}-`));
  return prefix === undefined ? name : name.slice(prefix.length + 1);
}

/**
 * The rule an `.oxlintrc.json` `rules` key configures, resolved as oxlint 1.88.0 resolves it (oxc
 * `crates/oxc_linter/src/config/rules.rs`: `parse_rule_key` l.641-660, `unalias_plugin_name` l.672-688,
 * `transform_rule_and_plugin_name` l.244-253). A key with a `@scope` splits at its last `/`, any other at its first;
 * the plugin part loses `eslint-plugin-`/`oxlint-plugin-` and its aliases (`@typescript-eslint` → `typescript`,
 * `react-hooks` → `react`, `@next/next` → `nextjs`); an unknown plugin stays as written, an external plugin that never
 * names a built-in rule. A bare key names the plugin of the first built-in rule of that name, `eslint` when there is
 * none; `soleRulePlugin` gives the plugin of a name only one plugin has, where the caller knows it.
 */
function oxlintRuleOfKey(key: string, soleRulePlugin: (name: string) => string | undefined = () => undefined): OxlintRule {
  const split = key.startsWith("@") ? key.lastIndexOf("/") : key.indexOf("/");
  if (split < 0) return { plugin: OXLINT_SHARED_RULE_NAMES[key] ?? soleRulePlugin(key) ?? "eslint", name: key };
  const name = key.slice(split + 1);
  const normalized = normalizeOxlintPluginName(key.slice(0, split));
  // `LintPlugins::try_from` normalises the name once more.
  const plugin = OXLINT_PLUGINS[normalized] ?? OXLINT_PLUGINS[normalizeOxlintPluginName(normalized)] ?? normalized;
  return { plugin: plugin === "typescript" && OXLINT_TYPESCRIPT_AS_ESLINT.has(name) ? "eslint" : plugin, name };
}

/**
 * The oxlint rule a port of ESLint rule `eslintName` models: the rule oxlint configures for that ESLint key, except
 * that eslint-plugin-n's `n/<rule>` is oxlint's `node/<rule>` (its `node` plugin ports eslint-plugin-node, of which
 * eslint-plugin-n is the fork: oxc `config/plugins.rs` l.119), while oxlint itself takes an `n/` key for an external
 * plugin.
 */
function oxlintRuleOfPort(eslintName: string): OxlintRule {
  return oxlintRuleOfKey(eslintName.startsWith("n/") ? `node/${eslintName.slice(2)}` : eslintName);
}

export type LintConfigLinter = "eslint" | "oxlint" | "ruff";

/** A rule entry limited to some files (a flat config object's, an `.eslintrc` override's), after the last entry for its rule that every file gets. */
export interface ScopedRule {
  readonly key: string;
  readonly value: unknown;
  /** The folders it covers exactly, if it covers whole folders; `files` names them for the person. */
  readonly folders: readonly string[] | undefined;
  readonly files: string;
  /** What comes after it that Rerail does not read and that may replace it. */
  readonly shadowedBy?: string;
}

/** The `rules` of an ESLint or oxlint config file. */
export interface LintConfigRules {
  /** How the file is named to the person: `.eslintrc.json`, `package.json eslintConfig`. */
  readonly file: string;
  /** What every file gets, by rule key. */
  readonly rules: Readonly<Record<string, unknown>>;
  /** A flat config's entries limited to some files; none for other configs. */
  readonly scoped: readonly ScopedRule[];
  /** A flat config's rule keys whose value cannot be read without running the config, with why. */
  readonly unresolved: ReadonlyMap<string, string>;
  /** Rule keys of `rules` that something after them Rerail does not read may replace, with what. */
  readonly shadowed: ReadonlyMap<string, string>;
}

/** A Ruff selector in one of the rule selection options, with the file and key path that hold it. */
export interface RuffSelector {
  readonly file: string;
  readonly key: string;
  readonly selector: string;
}

/** One config file's rule selection, as Ruff reads it into one rule selection. */
export interface RuffLayer {
  /** `select`, which replaces every earlier selection; undefined where the file does not set it. */
  readonly select: readonly RuffSelector[] | undefined;
  readonly extendSelect: readonly RuffSelector[];
  /** `ignore` and `extend-ignore`. */
  readonly ignore: readonly RuffSelector[];
}

/** Ruff's rule selection in the config Ruff uses at the root. */
export interface RuffSelection {
  /** Each config's selection in the order Ruff applies them: the configs `extend` names first, the root's last. */
  readonly layers: readonly RuffLayer[];
  /** `per-file-ignores` and `extend-per-file-ignores`: a selector ignored in the files a pattern matches. */
  readonly perFileIgnores: readonly { readonly pattern: string; readonly folder: string | undefined; readonly ignore: RuffSelector }[];
}

/** What `readLintConfigFiles` read at a repository root. */
export interface LintConfigFiles {
  readonly eslint: LintConfigRules | undefined;
  readonly oxlint: LintConfigRules | undefined;
  readonly ruff: RuffSelection | undefined;
  /** What was present and not read, or read only in part, and why; by the linter it configures. */
  readonly notes: readonly { readonly linter: LintConfigLinter; readonly text: string }[];
}

/** A setting set-up proposes for a rule of a pack being set up. */
export interface LintConfigSetting {
  readonly ruleId: string;
  /** The folder it applies in, relative to the repository root; `""` for the whole repository. */
  readonly folder: string;
  /** `false`: the repository's config turns the rule off. `null`: leaves it as the pack has it. */
  readonly on: false | null;
  /** Option values from the repository's config, or `null`. */
  readonly config: RuleConfig | null;
  /** Where it comes from, for the person: `.eslintrc.json: "no-console": "off"`. */
  readonly because: string;
}

export interface RepositoryLintConfig {
  readonly settings: readonly LintConfigSetting[];
  /** Files not read, options not mapped and conflicts, for the person. */
  readonly notes: readonly string[];
}

type FileRead = { readonly present: false } | { readonly present: true; readonly text: string | undefined };

/**
 * A file at `name` (a path relative to the root) as text: absent, present but not a regular file of at most
 * `MAX_CONFIG_BYTES` in the repository (no text), or its text.
 */
function readRootFile(root: string, name: string): FileRead {
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

/** A JSON object; arrays and `null` are not. Zod leaves out `__proto__` keys. */
const jsonObject = z.record(z.string(), z.unknown());
const configFile = z.looseObject({ rules: z.unknown().optional(), overrides: z.unknown().optional() });

const NOT_A_FILE = "it is not a regular file of at most 1 MiB in the repository.";

type Notes = { linter: LintConfigLinter; text: string }[];

/**
 * The `rules` of a JSON config file's object `config`, noting what the file sets that is not read. Undefined (with a
 * note) when `rules` is not an object.
 */
function topLevelRules(file: string, config: z.infer<typeof configFile>, unread: readonly string[], linter: LintConfigLinter, notes: Notes): LintConfigRules | undefined {
  const skipped = unread.filter(key => Object.hasOwn(config, key));
  if (skipped.length > 0) notes.push({ linter, text: `${file}: only its own rules are read, not ${skipped.join(" or ")}.` });
  const rules = jsonObject.safeParse(config.rules ?? {});
  if (!rules.success) {
    notes.push({ linter, text: `${file} is not read: its rules are not an object.` });
    return undefined;
  }
  return { file, rules: rules.data, scoped: linter === "eslint" ? eslintrcOverrides(file, config, notes) : [], unresolved: new Map(), shadowed: new Map() };
}

const eslintrcOverride = z.looseObject({
  files: z.union([z.string(), z.array(z.string())]),
  rules: jsonObject.optional(),
});

/**
 * An `.eslintrc`'s `overrides` as rule entries for some files, in order: ESLint applies each after the config's own
 * rules to the files its `files` match, less its `excludedFiles`. Their `extends` and nested `overrides` are not read.
 */
function eslintrcOverrides(file: string, config: z.infer<typeof configFile>, notes: Notes): ScopedRule[] {
  const overrides = config.overrides;
  if (overrides === undefined) return [];
  if (!Array.isArray(overrides)) {
    notes.push({ linter: "eslint", text: `${file}: its overrides are not read: they are not a list.` });
    return [];
  }
  return overrides.flatMap((value, index) => {
    const override = eslintrcOverride.safeParse(value);
    if (!override.success) {
      notes.push({ linter: "eslint", text: `${file}: overrides[${index}] is not read: it has no files or its rules are not an object.` });
      return [];
    }
    const unread = ["extends", "overrides"].filter(key => Object.hasOwn(override.data, key));
    if (unread.length > 0) notes.push({ linter: "eslint", text: `${file}: overrides[${index}]: only its own rules are read, not ${unread.join(" or ")}.` });
    const patterns = typeof override.data.files === "string" ? [override.data.files] : override.data.files;
    const folders = Object.hasOwn(override.data, "excludedFiles") ? [] : patterns.map(folderOfGlob);
    const exact = folders.length > 0 && folders.every(folder => folder !== undefined);
    return Object.entries(override.data.rules ?? {}).map(([key, value]) => ({
      key,
      value,
      folders: exact ? (folders as string[]) : undefined,
      files: JSON.stringify(override.data.files),
    }));
  });
}

/** A JSONC config file at the root as an object; undefined (with a note when present) otherwise. */
function jsoncFile(root: string, name: string, linter: LintConfigLinter, notes: Notes): z.infer<typeof configFile> | undefined {
  const read = readRootFile(root, name);
  if (!read.present) return undefined;
  if (read.text === undefined) {
    notes.push({ linter, text: `${name} is not read: ${NOT_A_FILE}` });
    return undefined;
  }
  try {
    const config = configFile.safeParse(parseJsonc(read.text));
    if (config.success) return config.data;
  } catch (error) {
    if (!(error instanceof LintConfigFormatError)) throw error;
  }
  notes.push({ linter, text: `${name} is not read: it is not a JSON object (Rerail reads JSON lint configs only).` });
  return undefined;
}

const ESLINT_UNREAD = ["extends", "plugins"];

/** A severity with no options: a severity, or a list of just one, which ESLint merges as a severity alone. */
const severityAlone = (value: unknown): boolean => !Array.isArray(value) || value.length === 1;

/** A severity with no options after it, as a later config object sets one: ESLint keeps the earlier options. */
function withEarlierOptions(earlier: unknown, later: unknown): unknown {
  if (!severityAlone(later) || !Array.isArray(earlier) || earlier.length <= 1) return later;
  return [Array.isArray(later) ? later[0] : later, ...earlier.slice(1)];
}

/**
 * A flat config's entries as ESLint applies them, in order: a later entry every file gets replaces the earlier ones
 * (keeping their options when it sets only a severity), and the entries limited to some files that come after it are
 * kept apart. An entry Rerail cannot read stands for its rule until a later entry for every file replaces it.
 */
function flatRules(file: string, read: FlatConfigRead): LintConfigRules {
  const rules: Record<string, unknown> = {};
  const unresolved = new Map<string, string>();
  const shadowed = new Map<string, string>();
  const scopedUnresolved = new Map<string, string>();
  let scoped: ScopedRule[] = [];
  for (const { key, scope, value, shadowedBy } of read.entries) {
    if (scope.kind !== "all") {
      if (value.ok) {
        scoped.push({ key, value: value.value, folders: scope.kind === "folders" ? scope.folders : undefined, files: scope.files, ...(shadowedBy === undefined ? {} : { shadowedBy }) });
      } else if (!scopedUnresolved.has(key)) {
        scopedUnresolved.set(key, `${value.why} (for ${scope.files})`);
      }
      continue;
    }
    scoped = scoped.filter(entry => entry.key !== key);
    scopedUnresolved.delete(key);
    // Only a value that replaces the earlier one entirely stands in for an unread one: an "off", or a severity with options.
    const replaces = value.ok && (!severityAlone(value.value) || severityOf(value.value, "eslint")?.off === true);
    if (!value.ok) {
      unresolved.set(key, value.why);
      delete rules[key];
    } else if (unresolved.has(key) && !replaces) {
      // Rerail cannot tell the options a severity keeps.
    } else {
      unresolved.delete(key);
      Object.defineProperty(rules, key, { value: withEarlierOptions(rules[key], value.value), enumerable: true, writable: true, configurable: true });
    }
    if (shadowedBy === undefined) shadowed.delete(key);
    else shadowed.set(key, shadowedBy);
  }
  for (const [key, why] of scopedUnresolved) if (!unresolved.has(key)) unresolved.set(key, why);
  return { file, rules, scoped, unresolved, shadowed };
}

function readEslint(root: string, notes: Notes): LintConfigRules | undefined {
  const flat = ESLINT_FLAT_CONFIGS.filter(name => readRootFile(root, name).present);
  const legacy = ESLINTRC_FILES.filter(name => readRootFile(root, name).present);
  if (flat.length > 0) {
    const used = flat[0]!;
    // ESLint uses the first flat config it finds wherever there is one.
    for (const name of [...flat.slice(1), ...legacy]) notes.push({ linter: "eslint", text: `${name} is not read: ESLint uses ${used} instead.` });
    const read = readRootFile(root, used);
    if (read.present && read.text === undefined) {
      notes.push({ linter: "eslint", text: `${used} is not read: ${NOT_A_FILE}` });
      return undefined;
    }
    let config: FlatConfigRead;
    try {
      config = readEslintFlatConfig(read.present ? read.text! : "", used);
    } catch (error) {
      if (!(error instanceof LintConfigFormatError)) throw error;
      notes.push({ linter: "eslint", text: `${used} is not read: ${error.message}. ${NEVER_RUN}` });
      return undefined;
    }
    for (const note of config.notes) notes.push({ linter: "eslint", text: `${used}: ${note}` });
    return flatRules(used, config);
  }
  const used = legacy[0];
  if (used !== undefined) {
    for (const name of legacy.slice(1)) notes.push({ linter: "eslint", text: `${name} is not read: ESLint uses ${used} instead.` });
    if (ESLINTRC_CODE.has(used)) {
      notes.push({ linter: "eslint", text: `${used} is not read: ${NEVER_RUN}` });
      return undefined;
    }
    if (used.endsWith(".yaml") || used.endsWith(".yml")) {
      notes.push({ linter: "eslint", text: `${used} is not read: Rerail reads JSON lint configs only.` });
      return undefined;
    }
    const config = jsoncFile(root, used, "eslint", notes);
    return config === undefined ? undefined : topLevelRules(used, config, ESLINT_UNREAD, "eslint", notes);
  }
  // Without a config file, ESLint reads `package.json`'s `eslintConfig`.
  const manifest = readRootFile(root, "package.json");
  if (!manifest.present || manifest.text === undefined) return undefined;
  let parsed: unknown;
  try {
    parsed = JSON.parse(manifest.text);
  } catch {
    // An unreadable manifest is not an ESLint config; the matcher skips it the same way.
    return undefined;
  }
  const eslintConfig = z.looseObject({ eslintConfig: z.unknown().optional() }).safeParse(parsed).data?.eslintConfig;
  if (eslintConfig === undefined) return undefined;
  const config = configFile.safeParse(eslintConfig);
  if (!config.success) {
    notes.push({ linter: "eslint", text: "package.json eslintConfig is not read: it is not an object." });
    return undefined;
  }
  return topLevelRules("package.json eslintConfig", config.data, ESLINT_UNREAD, "eslint", notes);
}

function readOxlint(root: string, notes: Notes): LintConfigRules | undefined {
  const config = jsoncFile(root, ".oxlintrc.json", "oxlint", notes);
  return config === undefined ? undefined : topLevelRules(".oxlintrc.json", config, ["extends", "overrides", "categories"], "oxlint", notes);
}

type RuffPerFileIgnore = RuffSelection["perFileIgnores"][number];

/** One Ruff config file as read: its rule selection, its per-file ignores, and the config it extends. */
interface RuffFile {
  readonly name: string;
  readonly layer: RuffLayer;
  /** `per-file-ignores`, which replaces the extended config's; undefined where the file does not set it. */
  readonly perFileIgnores: readonly RuffPerFileIgnore[] | undefined;
  readonly extendPerFileIgnores: readonly RuffPerFileIgnore[];
  readonly extend: string | undefined;
}

/** How many configs deep `extend` is followed; Ruff has no limit, but a chain this long is not one a person wrote. */
const MAX_RUFF_EXTENDS = 8;

/**
 * Ruff config file `name` (a path relative to the root), as Ruff reads it: from `[tool.ruff]` in a `pyproject.toml`,
 * from the top in any other file. `absent` when there is no such file, `not-ruff` for a `pyproject.toml` without
 * `[tool.ruff]`, undefined (with a note) when it cannot be read.
 */
function readRuffFile(root: string, name: string, notes: Notes): RuffFile | "absent" | "not-ruff" | undefined {
  const read = readRootFile(root, name);
  if (!read.present) return "absent";
  const prefix = name === "pyproject.toml" || name.endsWith("/pyproject.toml") ? "tool.ruff." : "";
  if (read.text === undefined) {
    notes.push({ linter: "ruff", text: `${name} is not read: ${NOT_A_FILE}` });
    return undefined;
  }
  const sections = [`${prefix}lint.`, prefix];
  let found;
  try {
    found = readTomlStringLists(
      read.text,
      RUFF_LISTS.flatMap(option => sections.map(section => `${section}${option}`)),
      RUFF_TABLES.flatMap(option => sections.map(section => `${section}${option}`)),
      [`${prefix}extend`],
    );
  } catch (error) {
    if (!(error instanceof LintConfigFormatError)) throw error;
    // A pyproject.toml that may hold no Ruff config at all is not worth a note.
    if (prefix === "" || read.text.includes("ruff")) notes.push({ linter: "ruff", text: `${name} is not read: ${error.message}.` });
    return prefix === "" ? undefined : "not-ruff";
  }
  // Ruff skips a pyproject.toml without a [tool.ruff] section.
  if (prefix !== "" && !found.paths.some(path => path === "tool.ruff" || path.startsWith("tool.ruff."))) return "not-ruff";
  const selectors = (option: string): RuffSelector[] | undefined => {
    // Ruff reads an option from the lint section where it is set there, else from the top level.
    const key = sections.map(section => `${section}${option}`).find(path => found.lists.has(path));
    if (key === undefined) return undefined;
    return found.lists.get(key)!.flatMap(selector => {
      if (RUFF_SELECTOR.test(selector)) return [{ file: name, key, selector }];
      notes.push({ linter: "ruff", text: `${name}: ${JSON.stringify(selector)} in ${key} is not a Ruff rule code or prefix.` });
      return [];
    });
  };
  const perFileIgnores = (option: string): RuffPerFileIgnore[] | undefined => {
    const key = sections.map(section => `${section}${option}`).find(path => found.tables.has(path));
    if (key === undefined) return undefined;
    return [...found.tables.get(key)!].flatMap(([pattern, ignored]) => ignored.flatMap(selector => {
      if (!RUFF_SELECTOR.test(selector)) {
        notes.push({ linter: "ruff", text: `${name}: ${JSON.stringify(selector)} in ${key} is not a Ruff rule code or prefix.` });
        return [];
      }
      return [{ pattern, folder: pattern.startsWith("!") ? undefined : folderOfGlob(pattern), ignore: { file: name, key: `${key}.${JSON.stringify(pattern)}`, selector } }];
    }));
  };
  return {
    name,
    layer: {
      select: selectors("select"),
      extendSelect: selectors("extend-select") ?? [],
      ignore: [...selectors("ignore") ?? [], ...selectors("extend-ignore") ?? []],
    },
    perFileIgnores: perFileIgnores("per-file-ignores"),
    extendPerFileIgnores: perFileIgnores("extend-per-file-ignores") ?? [],
    extend: found.strings.get(`${prefix}extend`),
  };
}

/** The config `extend` names from config `from`, as a path relative to the root; undefined outside the repository. */
function extendedPath(from: string, extend: string): string | undefined {
  if (extend.startsWith("/") || extend.startsWith("~") || /^[A-Za-z]:/u.test(extend) || extend.includes("\\")) return undefined;
  const parts: string[] = from.split("/").slice(0, -1);
  for (const part of extend.split("/")) {
    if (part === "" || part === ".") continue;
    if (part === "..") {
      if (parts.length === 0) return undefined;
      parts.pop();
    } else {
      parts.push(part);
    }
  }
  return parts.length === 0 ? undefined : parts.join("/");
}

function readRuff(root: string, notes: Notes): RuffSelection | undefined {
  for (const name of RUFF_FILES) {
    const first = readRuffFile(root, name, notes);
    if (first === "absent" || first === "not-ruff") continue;
    if (first === undefined) return undefined;
    // The chain of configs `extend` names; a config Rerail cannot read ends it, and the rules only it ignores stay on.
    const chain: RuffFile[] = [first];
    for (let current = first; current.extend !== undefined;) {
      const unread = (why: string): void => {
        notes.push({ linter: "ruff", text: `${current.name}: the config it extends, ${JSON.stringify(current.extend)}, is not read (${why}), so the rules only that config ignores stay on.` });
      };
      const target = extendedPath(current.name, current.extend);
      if (target === undefined) {
        unread("it is outside the repository");
        break;
      }
      if (chain.some(file => file.name === target) || chain.length > MAX_RUFF_EXTENDS) {
        unread("its extend chain loops or runs too deep");
        break;
      }
      const base = readRuffFile(root, target, notes);
      if (base === "absent" || base === undefined) {
        if (base === "absent") unread("there is no such file");
        break;
      }
      if (base === "not-ruff") break;
      chain.push(base);
      current = base;
    }
    // A per-file ignore's pattern is relative to its config's folder; Rerail reads those of configs at the root.
    const onlyAtRoot = (files: readonly RuffFile[]): RuffPerFileIgnore[] => files.flatMap(file => {
      if (!file.name.includes("/")) return [file.perFileIgnores ?? [], file.extendPerFileIgnores].flat();
      notes.push({ linter: "ruff", text: `${file.name}: its per-file ignores are not read: their patterns are relative to its folder.` });
      return [];
    });
    // `per-file-ignores` of the nearest config that sets it replaces the others'; every `extend-per-file-ignores` adds.
    const nearest = chain.find(file => file.perFileIgnores !== undefined);
    return {
      layers: chain.map(file => file.layer).reverse(),
      perFileIgnores: [
        ...(nearest === undefined ? [] : onlyAtRoot([{ ...nearest, extendPerFileIgnores: [] }])),
        ...onlyAtRoot(chain.filter(file => file.extendPerFileIgnores.length > 0).map(file => ({ ...file, perFileIgnores: undefined }))),
      ],
    };
  }
  return undefined;
}

/** What the repository at `root` configures for its linters, read from static files at its root only. Runs nothing. */
export function readLintConfigFiles(root: string): LintConfigFiles {
  const notes: Notes = [];
  const eslint = readEslint(root, notes);
  const oxlint = readOxlint(root, notes);
  const ruff = readRuff(root, notes);
  return { eslint, oxlint, ruff, notes };
}

/**
 * Whether Ruff selector `selector` selects rule `code`: `ALL` every rule; otherwise a prefix of the code within the
 * same linter, so `E` selects `E711` but not `ERA001`, and `PL` selects Pylint's `PLR0913`.
 */
export function ruffSelects(selector: string, code: string): boolean {
  if (selector === "ALL") return true;
  if (!code.startsWith(selector)) return false;
  const selectorLetters = /^[A-Z]*/u.exec(selector)![0];
  const codeLetters = /^[A-Z]*/u.exec(code)![0];
  return selectorLetters === codeLetters || (selectorLetters === "PL" && /^PL[CERW]$/u.test(codeLetters));
}

/**
 * How specific a Ruff selector is, as Ruff orders them (`Specificity` in ruff_linter's `rule_selector.rs`): `ALL`, the
 * legacy `C` and `T`, a linter (`E`, `PL`), a prefix by its length (`E7`, `PLR09`), a rule (`E711`). Two selectors that
 * select the same code are both prefixes of it, so the longer one is the more specific.
 */
const ruffSpecificity = (selector: string): number => (selector === "ALL" ? 0 : selector.length);

/** Whether Ruff runs a rule, and the selector that decided it; `by` is undefined for a rule a `select` leaves out. */
export interface RuffDecision {
  readonly enabled: boolean;
  readonly by: RuffSelector | undefined;
}

/**
 * What Ruff's rule selection decides for rule `code`, replaying `as_rule_table` (ruff_workspace's `configuration.rs`,
 * 0.15.0) over the configs in order. Within one config, selectors apply from the least specific to the most specific,
 * a select before an ignore of the same specificity, so the most specific selector that names the code decides. A
 * config that sets `select` replaces the earlier configs' result (a code it does not select is off, decided by no
 * ignore); one that does not changes only the codes its selectors name; `select = []` with ignores and no
 * `extend-select` carries its ignores into the next config. Undefined when no selector names the code: Ruff's defaults
 * decide.
 */
export function ruffDecision(selection: RuffSelection, code: string): RuffDecision | undefined {
  let decided: RuffDecision | undefined;
  let carried: readonly RuffSelector[] = [];
  for (const layer of selection.layers) {
    const updates = [
      ...[...(layer.select ?? []), ...layer.extendSelect].map(by => ({ enabled: true, by })),
      ...[...layer.ignore, ...carried].map(by => ({ enabled: false, by })),
    ].filter(({ by }) => ruffSelects(by.selector, code));
    // A stable sort keeps selects before ignores of the same specificity.
    updates.sort((left, right) => ruffSpecificity(left.by.selector) - ruffSpecificity(right.by.selector));
    const last = updates.at(-1);
    carried = [];
    if (layer.select !== undefined) {
      decided = last ?? { enabled: false, by: undefined };
      if (layer.select.length === 0 && layer.extendSelect.length === 0) carried = layer.ignore;
    } else if (last !== undefined) {
      decided = last;
    }
  }
  return decided;
}

const ruffText = ({ file, key, selector }: RuffSelector): string => `${file}: ${JSON.stringify(selector)} in ${key}`;

/** A rule entry's severity: `off`, `on`, or undefined when it is not one the linter takes; with the options after it. */
function severityOf(value: unknown, linter: "eslint" | "oxlint"): { off: boolean; options: unknown[] } | undefined {
  const [level, ...options] = Array.isArray(value) ? value : [value];
  const off = linter === "eslint" ? ["off", 0] : ["off", "allow", 0];
  const on = linter === "eslint" ? ["warn", "error", 1, 2] : ["warn", "error", "deny", 1, 2];
  if (off.includes(level as string)) return { off: true, options };
  if (on.includes(level as string)) return { off: false, options };
  return undefined;
}

type Declared = Readonly<Record<string, CodeRuleDslOptionDeclaration>>;
/** ESLint options mapped onto a port's `config`, or why they are not. */
type MappedOptions = { readonly ok: true; readonly config: RuleConfig } | { readonly ok: false; readonly reason: string };

const isOptionValue = (value: unknown): value is CodeRuleDslValue =>
  value === null || ["string", "number", "boolean"].includes(typeof value)
  || (Array.isArray(value) && value.every(item => item === null || ["string", "number", "boolean"].includes(typeof item)));

/**
 * ESLint's options for a rule as the port's `config`: the ports name the first positional option `option` and the
 * second `option2`, an option object's keys by themselves and a nested object's keys `outer.inner`. Every value must
 * land on an option the document declares; otherwise nothing is mapped.
 */
function eslintOptionsConfig(declared: Declared, options: readonly unknown[]): MappedOptions {
  const config: Record<string, CodeRuleDslValue> = {};
  const set = (name: string, value: unknown): string | undefined => {
    if (!Object.hasOwn(declared, name)) return `${name} is not an option the port declares`;
    if (!isOptionValue(value)) return `${name}: ${JSON.stringify(value)} is not a value Rerail maps`;
    if (Object.hasOwn(config, name)) return `${name} is set twice`;
    config[name] = value;
    return undefined;
  };
  for (const [index, option] of options.entries()) {
    const problems: (string | undefined)[] = [];
    const object = jsonObject.safeParse(option);
    if (object.success) {
      for (const [key, value] of Object.entries(object.data)) {
        const nested = jsonObject.safeParse(value);
        if (nested.success) for (const [inner, innerValue] of Object.entries(nested.data)) problems.push(set(`${key}.${inner}`, innerValue));
        else problems.push(set(key, value));
      }
    } else {
      problems.push(set(index === 0 ? "option" : `option${index + 1}`, option));
    }
    const problem = problems.find(found => found !== undefined);
    if (problem !== undefined) return { ok: false, reason: problem };
  }
  return { ok: true, config };
}

/** The upstream rules a code rule's violations honour suppression comments for, and its declared options. */
function portOf(bundle: BundledCatalog, ruleId: string): { eslint: string[]; oxlint: OxlintRule[]; noqa: string[]; declared: Declared } | undefined {
  const document = bundle.documents.get(ruleId);
  if (document?.kind !== "code") return undefined;
  const port = { eslint: [] as string[], oxlint: [] as OxlintRule[], noqa: [] as string[], declared: {} as Record<string, CodeRuleDslOptionDeclaration> };
  for (const clause of document.rule.clauses) {
    if (clause.define !== undefined) Object.assign(port.declared, clause.define.options ?? {});
    const suppress = clause.violation?.suppress;
    port.eslint.push(...(suppress?.eslint?.rules ?? []));
    // `suppress.oxlint` names the rule bare, as oxlint's comments do; which plugin's rule it is follows from the
    // ESLint rule the port ports. A port without one is matched to no oxlint config key.
    const bare = suppress?.oxlint?.rules ?? [];
    for (const rule of (suppress?.eslint?.rules ?? []).map(oxlintRuleOfPort)) {
      if (bare.includes(rule.name) && !port.oxlint.some(known => known.plugin === rule.plugin && known.name === rule.name)) port.oxlint.push(rule);
    }
    port.noqa.push(...(suppress?.noqa?.codes ?? []));
  }
  return port;
}

/** One thing a config file says about one rule. */
interface Finding {
  readonly off: boolean;
  readonly because: string;
  /** ESLint options mapped onto the port, or why they are not; absent without options. */
  readonly config?: MappedOptions;
}

const MAX_VALUE_TEXT = 60;

function entryText(file: string, key: string, value: unknown): string {
  const text = JSON.stringify(value) ?? String(value);
  return `${file}: ${JSON.stringify(key)}: ${text.length > MAX_VALUE_TEXT ? `${text.slice(0, MAX_VALUE_TEXT - 1)}…` : text}`;
}

/**
 * Settings for the rules of packs `packIds` (in `bundle`) from what the repository's lint config files say, with
 * notes on what could not be used. Notes about a linter none of these rules ports are left out.
 */
export function proposeLintConfigSettings(files: LintConfigFiles, bundle: BundledCatalog, packIds: readonly string[]): RepositoryLintConfig {
  const ruleIds = [...new Set(packIds.flatMap(id => (bundle.packs.get(id)?.rules ?? []).map(rule => rule.id)))];
  const ports = new Map(ruleIds.flatMap(id => {
    const port = portOf(bundle, id);
    return port === undefined ? [] : [[id, port] as const];
  }));
  const findings = new Map<string, Finding[]>();
  /** Rule id → folder → why: a rule a config turns off for exactly that folder's files. */
  const folderOffs = new Map<string, Map<string, string>>();
  const notes: string[] = [];
  const add = (ruleId: string, finding: Finding): void => {
    findings.set(ruleId, [...(findings.get(ruleId) ?? []), finding]);
  };
  const addFolderOff = (ruleId: string, folder: string, because: string): void => {
    const folders = folderOffs.get(ruleId) ?? new Map<string, string>();
    folders.set(folder, folders.has(folder) ? `${folders.get(folder)!}; ${because}` : because);
    folderOffs.set(ruleId, folders);
  };
  /** ESLint options as the port's `config`, only where the port accepts them as a stored rule setting would be. */
  const mappedOptions = (ruleId: string, declared: Declared, options: readonly unknown[]): MappedOptions => {
    const config = eslintOptionsConfig(declared, options);
    if (!config.ok) return config;
    const configured = configureBundledCodeRule(bundle, ruleId, config.config);
    return configured.ok ? config : { ok: false, reason: configured.reason };
  };
  const eslint = files.eslint;
  for (const [ruleId, port] of ports) {
    for (const key of new Set(port.eslint)) {
      if (eslint === undefined) break;
      const why = eslint.unresolved.get(key);
      if (why !== undefined) {
        // The repository sets the rule, to what Rerail cannot tell without running the config.
        add(ruleId, { off: true, because: `${eslint.file}: ${JSON.stringify(key)} is not read (${why})` });
        continue;
      }
      const later = eslint.scoped.filter(entry => entry.key === key);
      const global = Object.hasOwn(eslint.rules, key);
      const replacedBy = (by: string): string => `${by}, after it, is not read and may replace it`;
      if (global) {
        const value = eslint.rules[key];
        const because = entryText(eslint.file, key, value);
        const severity = severityOf(value, "eslint");
        const shadowedBy = eslint.shadowed.get(key);
        if (severity === undefined) {
          notes.push(`${ruleId}: ${because} is not a severity ESLint takes; not read.`);
        } else if (shadowedBy !== undefined) {
          notes.push(`${ruleId}: ${because} is not used: ${replacedBy(shadowedBy)}.`);
        } else {
          const back = severity.off ? later.find(entry => severityOf(entry.value, "eslint")?.off === false) : undefined;
          if (back !== undefined) {
            notes.push(`${ruleId}: ${because} turns it off, but ${entryText(eslint.file, key, back.value)} turns it back on for ${back.files}; it stays on.`);
          } else {
            const config = severity.off || severity.options.length === 0 ? undefined : mappedOptions(ruleId, port.declared, severity.options);
            add(ruleId, { off: severity.off, because, ...(config === undefined ? {} : { config }) });
          }
        }
      }
      for (const [index, entry] of later.entries()) {
        const because = `${entryText(eslint.file, key, entry.value)} for ${entry.files}`;
        const severity = severityOf(entry.value, "eslint");
        if (severity === undefined) {
          notes.push(`${ruleId}: ${because} is not a severity ESLint takes; not read.`);
          continue;
        }
        if (entry.shadowedBy !== undefined) {
          notes.push(`${ruleId}: ${because} is not used: ${replacedBy(entry.shadowedBy)}.`);
          continue;
        }
        if (severity.off) {
          // A later entry that turns it back on may cover any of these files.
          const back = later.slice(index + 1).find(next => severityOf(next.value, "eslint")?.off === false);
          if (back !== undefined) {
            notes.push(`${ruleId}: ${because} turns it off, but ${entryText(eslint.file, key, back.value)} turns it back on for ${back.files}; it stays on.`);
          } else if (entry.folders !== undefined) {
            for (const folder of entry.folders) addFolderOff(ruleId, folder, because);
          } else {
            notes.push(`${ruleId}: ${because} turns it off only for some files; it stays on.`);
          }
        } else if (severity.options.length > 0) {
          // A rule setting's config is for the whole repository, so options set for some files are used only when no
          // entry for every file sets the rule: otherwise they would change it for files they do not cover.
          const forEvery = global ? severityOf(eslint.rules[key], "eslint") : undefined;
          if (forEvery === undefined) add(ruleId, { off: false, because, config: mappedOptions(ruleId, port.declared, severity.options) });
          else if (stableJsonText(forEvery.options) !== stableJsonText(severity.options)) {
            notes.push(`${ruleId}: the options of ${because} are not used: ${entryText(eslint.file, key, eslint.rules[key])} sets the rule for every file, and Rerail sets options for the whole repository only.`);
          }
        }
      }
    }
    for (const rule of port.oxlint) {
      // oxlint applies a rule's last key: `{"no-unused-vars": "off", "typescript/no-unused-vars": "error"}` keeps it on.
      const key = Object.keys(files.oxlint?.rules ?? {}).findLast(candidate => {
        const resolved = oxlintRuleOfKey(candidate, name => (name === rule.name ? rule.plugin : undefined));
        return resolved.plugin === rule.plugin && resolved.name === rule.name;
      });
      if (key === undefined) continue;
      const value = files.oxlint!.rules[key];
      const because = entryText(files.oxlint!.file, key, value);
      const severity = severityOf(value, "oxlint");
      if (severity === undefined) {
        notes.push(`${ruleId}: ${because} is not a severity oxlint takes; not read.`);
        continue;
      }
      // The ports' options follow the upstream rule's; oxlint's own options are not mapped onto them.
      if (!severity.off && severity.options.length > 0) notes.push(`${ruleId}: option not mapped: Rerail maps ESLint options, not oxlint's (${because}).`);
      add(ruleId, { off: severity.off, because });
    }
    const ruff = files.ruff;
    if (ruff !== undefined) {
      for (const code of new Set(port.noqa)) {
        const decision = ruffDecision(ruff, code);
        if (decision?.enabled === false && decision.by !== undefined) {
          add(ruleId, { off: true, because: ruffText(decision.by) });
          continue;
        }
        if (decision?.enabled === true && decision.by !== undefined) {
          const overridden = ruff.layers.flatMap(layer => layer.ignore).find(by => ruffSelects(by.selector, code));
          if (overridden !== undefined) notes.push(`${ruleId}: ${ruffText(overridden)} is overridden by ${ruffText(decision.by)}, which Ruff applies after it; it stays on.`);
        }
        for (const { pattern, folder, ignore } of ruff.perFileIgnores) {
          if (!ruffSelects(ignore.selector, code)) continue;
          if (folder !== undefined) addFolderOff(ruleId, folder, ruffText(ignore));
          else notes.push(`${ruleId}: ${ruffText(ignore)} ignores it only in the files ${JSON.stringify(pattern)} matches; it stays on.`);
        }
      }
    }
  }
  const settings: LintConfigSetting[] = [];
  for (const [ruleId, found] of findings) {
    const offs = found.filter(finding => finding.off);
    if (offs.length > 0) {
      const kept = found.filter(finding => !finding.off);
      if (kept.length > 0) notes.push(`${ruleId}: ${kept[0]!.because} keeps it on, but ${offs[0]!.because} turns it off; it starts off.`);
      settings.push({ ruleId, folder: "", on: false, config: null, because: offs.map(finding => finding.because).join("; ") });
      // Off everywhere: no folder needs its own setting.
      folderOffs.delete(ruleId);
      continue;
    }
    const configured = found.flatMap(finding => (finding.config === undefined ? [] : [{ because: finding.because, mapped: finding.config }]));
    const unmapped = configured.find(({ mapped }) => !mapped.ok);
    if (unmapped !== undefined && !unmapped.mapped.ok) {
      notes.push(`${ruleId}: option not mapped (${unmapped.mapped.reason}); it keeps its default options. From ${unmapped.because}.`);
      continue;
    }
    const configs = configured.flatMap(({ mapped }) => (mapped.ok ? [mapped.config] : []));
    if (new Set(configs.map(config => stableJsonText(config))).size > 1) {
      notes.push(`${ruleId}: ${configured.map(finding => finding.because).join(" and ")} set different options; none is used.`);
      continue;
    }
    const config = configs[0];
    if (config !== undefined && Object.keys(config).length > 0) {
      settings.push({ ruleId, folder: "", on: null, config, because: configured.map(finding => finding.because).join("; ") });
    }
  }
  for (const [ruleId, folders] of folderOffs) {
    for (const [folder, because] of folders) settings.push({ ruleId, folder, on: false, config: null, because });
  }
  const linters = new Set<LintConfigLinter>([...ports.values()].flatMap(port => [
    ...(port.eslint.length > 0 ? ["eslint" as const] : []),
    ...(port.oxlint.length > 0 ? ["oxlint" as const] : []),
    ...(port.noqa.length > 0 ? ["ruff" as const] : []),
  ]));
  return { settings, notes: [...files.notes.filter(note => linters.has(note.linter)).map(note => note.text), ...notes] };
}

/** {@link proposeLintConfigSettings} for the lint config files at `root`. */
export function readRepositoryLintConfig(root: string, bundle: BundledCatalog, packIds: readonly string[]): RepositoryLintConfig {
  return proposeLintConfigSettings(readLintConfigFiles(root), bundle, packIds);
}
