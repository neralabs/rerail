/**
 * oxlint's config files, read as data as oxlint 1.88.0 reads them: its `rules`, `plugins`, `categories` and
 * `options.typeAware`. A port's oxlint rule is the built-in rule oxlint runs for the ESLint rule the port ports
 * (`oxlintRulesOfPort`), named bare in its `suppress.oxlint`. A config key resolves to a built-in rule as oxlint
 * resolves it (`oxlintRuleOfKey`), not by its bare name as its suppression comments are: `react/no-console` is no rule
 * of oxlint's at all, and leaves ESLint's `no-console` alone.
 *
 * oxlint takes one config file per folder, `.oxlintrc.json`, `.oxlintrc.jsonc` or `oxlint.config.ts`, and lints each
 * file by the nearest one alone: a config below the root replaces the root's for its folder, not merged with it
 * (oxlint 1.88.0, its nested configs on by default). It refuses to run at all where a folder holds two config files,
 * or a config names a plugin, a rule, a category or a severity it does not have; Rerail then reads none. An
 * `oxlint.config.ts` is not read: the rules in its folder keep the pack's values.
 *
 * A rule the config's `rules` sets is set explicitly; one it does not set follows its category: the level `categories`
 * gives the category, else oxlint's default, which runs the `correctness` category (oxlint 1.88.0 `--print-config`
 * with an empty `.oxlintrc.json`), and only for the plugins oxlint runs: `eslint` always, the `plugins` the config
 * lists, or `unicorn`, `typescript` and `oxc` when it lists none. A type-aware rule runs only with `--type-aware`, or
 * `options.typeAware`. `extends` and `overrides` are not read: what they may set is unknown. A rule's options are taken
 * to be ones oxlint takes for it, as Rerail does not check them.
 */
import { posix } from "node:path";
import { z } from "zod";
import { entryText, jsoncFile, jsonObject, severityOf } from "./jsonc-lint-config.js";
import { combineDecisions } from "./linter-decisions.js";
import { inFolder, readRootFile, settlesAlike, type FolderDecision, type LinterConfig, type LinterNote, type LinterReader, type Port, type PortDecision, type PortVerdict } from "./linter-config.js";
import { OXLINT_RULES, OXLINT_TYPE_AWARE } from "./generated/oxlint-rules.generated.js";
import { repositoryFiles } from "./repository-files.js";

/** The config files oxlint 1.88.0 takes in a folder, of which a folder may hold one. */
const CONFIG_FILES = [".oxlintrc.json", ".oxlintrc.jsonc", "oxlint.config.ts"];
/** The config file oxlint runs as JavaScript, which Rerail never does. */
const SCRIPT_CONFIG = "oxlint.config.ts";
/** The keys an oxlint 1.88.0 config takes; any other makes oxlint refuse it. */
const CONFIG_KEYS = new Set(["$schema", "plugins", "jsPlugins", "categories", "rules", "settings", "env", "globals", "overrides", "options", "ignorePatterns", "extends"]);
/** oxlint 1.88.0's categories. */
const CATEGORIES = new Set(["correctness", "suspicious", "pedantic", "perf", "style", "restriction", "nursery"]);

/**
 * A rule built into oxlint: oxlint's internal plugin name (`eslint`, `typescript`, `react`, `jsx_a11y`, `nextjs`,
 * `node`, ...) and the rule's name.
 */
interface OxlintRule {
  readonly plugin: string;
  readonly name: string;
}

/**
 * The plugin names `plugins` takes, as oxlint 1.88.0 names its built-in plugins (oxc
 * `crates/oxc_linter/src/config/plugins.rs` `TryFrom<&str> for LintPlugins` l.164-194): `eslint` stays, the others
 * take their `From<LintPlugins>` name with `-` as `_`. Any other name makes oxlint refuse the config.
 */
const LISTED_PLUGINS: Readonly<Record<string, string>> = {
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
  nextjs: "nextjs",
  "react-perf": "react_perf", react_perf: "react_perf",
  promise: "promise",
  node: "node",
  vue: "vue",
};

/** A rule key's plugin names: those `plugins` takes, and Next.js's scopes (`unalias_plugin_name`, `config/rules.rs` l.672-688). */
const KEY_PLUGINS: Readonly<Record<string, string>> = { ...LISTED_PLUGINS, "@next": "nextjs", "@next/next": "nextjs" };
const BUILT_IN_PLUGINS = new Set(Object.values(LISTED_PLUGINS));

/**
 * The plugins oxlint 1.88.0 runs when the config lists none (`--print-config` of an empty `.oxlintrc.json` gives
 * `"plugins": ["unicorn", "typescript", "oxc"]`), and the one it always runs whatever the list (`"plugins": []` keeps
 * ESLint's correctness rules).
 */
const DEFAULT_PLUGINS = ["unicorn", "typescript", "oxc"];
const ALWAYS_PLUGIN = "eslint";
/** The category oxlint runs when `categories` does not name it (`--print-config` of an empty `.oxlintrc.json`). */
const DEFAULT_CATEGORY = "correctness";

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

/** Each built-in rule name with the plugins that have a rule of that name. */
const PLUGINS_OF_NAME = new Map<string, string[]>();
for (const [plugin, categories] of Object.entries(OXLINT_RULES)) {
  for (const name of Object.values(categories).flat()) PLUGINS_OF_NAME.set(name, [...PLUGINS_OF_NAME.get(name) ?? [], plugin]);
}

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
  const scoped = /^(@[^/]+)\/(?:eslint|oxlint)-plugin(?:-(.+))?$/u.exec(name);
  if (scoped !== null) return scoped[2] === undefined ? scoped[1]! : `${scoped[1]!}/${scoped[2]}`;
  return /^(?:eslint|oxlint)-plugin-(.+)$/u.exec(name)?.[1] ?? name;
}

/** oxlint's name for a plugin `names` holds as written or normalised (`LintPlugins::try_from` normalises it once more); undefined for any other. */
function builtInPlugin(written: string, names: Readonly<Record<string, string>>): string | undefined {
  const normalized = normalizeOxlintPluginName(written);
  return names[normalized] ?? names[normalizeOxlintPluginName(normalized)];
}

/**
 * The rule an oxlint config's `rules` key configures, resolved as oxlint 1.88.0 resolves it (oxc
 * `crates/oxc_linter/src/config/rules.rs`: `parse_rule_key` l.641-660, `unalias_plugin_name` l.672-688,
 * `transform_rule_and_plugin_name` l.244-253). A key with a `@scope` splits at its last `/`, any other at its first;
 * the plugin part loses `eslint-plugin-`/`oxlint-plugin-` and its aliases (`@typescript-eslint` → `typescript`,
 * `react-hooks` → `react`, `@next/next` → `nextjs`); an unknown plugin stays as written, an external plugin's. A bare
 * key names the plugin of the first built-in rule of that name, `eslint` when there is none.
 */
function oxlintRuleOfKey(key: string): OxlintRule {
  const split = key.startsWith("@") ? key.lastIndexOf("/") : key.indexOf("/");
  if (split < 0) return { plugin: OXLINT_SHARED_RULE_NAMES[key] ?? PLUGINS_OF_NAME.get(key)?.[0] ?? ALWAYS_PLUGIN, name: key };
  const name = key.slice(split + 1);
  const written = key.slice(0, split);
  const plugin = builtInPlugin(written, KEY_PLUGINS) ?? written;
  return { plugin: plugin === "typescript" && OXLINT_TYPESCRIPT_AS_ESLINT.has(name) ? ALWAYS_PLUGIN : plugin, name };
}

/** The category of a built-in oxlint rule at oxlint 1.88.0, or undefined when it is not one. */
function categoryOf(rule: OxlintRule): string | undefined {
  return Object.entries(OXLINT_RULES[rule.plugin] ?? {}).find(([, names]) => names.includes(rule.name))?.[0];
}

/** A built-in oxlint rule a port models, with its category. */
interface PortRule extends OxlintRule {
  readonly category: string;
}

/**
 * The oxlint rules a port models: for each ESLint rule it ports, the built-in rule oxlint configures for that ESLint
 * key, except that eslint-plugin-n's `n/<rule>` is oxlint's `node/<rule>` (its `node` plugin ports eslint-plugin-node,
 * of which eslint-plugin-n is the fork: oxc `config/plugins.rs` l.119), while oxlint itself refuses an `n/` key as a
 * plugin it does not load. `suppress.oxlint` names the rule bare, as oxlint's comments do; a port without one has no
 * oxlint rule.
 */
function oxlintRulesOfPort(port: Port): PortRule[] {
  const rules = port.suppress.eslint.flatMap(name => {
    const rule = oxlintRuleOfKey(name.startsWith("n/") ? `node/${name.slice(2)}` : name);
    const category = categoryOf(rule);
    return category === undefined || !port.suppress.oxlint.includes(rule.name) ? [] : [{ ...rule, category }];
  });
  return rules.filter((rule, index) => rules.findIndex(known => known.plugin === rule.plugin && known.name === rule.name) === index);
}

/** One oxlint config file as read. */
interface OxlintConfig {
  readonly file: string;
  readonly rules: Readonly<Record<string, unknown>>;
  /** The plugins oxlint runs (internal names). */
  readonly plugins: ReadonlySet<string>;
  readonly categories: Readonly<Record<string, unknown>>;
  /** Whether `options.typeAware` runs the type-aware rules. */
  readonly typeAware: boolean;
  /** What the config takes that is not read and may set any rule (`extends`, `overrides`), as named to the person. */
  readonly unread: readonly string[];
}

const pluginList = z.array(z.string());
const configOptions = z.object({ typeAware: z.boolean().optional() }).loose().optional();

/** Why oxlint refuses a rule entry: a key naming no rule it has, or a value that is no severity; undefined when it takes it. */
function entryRefusal(key: string, value: unknown, jsPlugins: boolean): string | undefined {
  if (severityOf(value, "oxlint") === undefined) return `${entryText("its rules", key, value)} is not a severity oxlint takes`;
  const rule = oxlintRuleOfKey(key);
  if (BUILT_IN_PLUGINS.has(rule.plugin)) return categoryOf(rule) === undefined ? `${JSON.stringify(key)} names no rule oxlint's ${rule.plugin} plugin has` : undefined;
  // A plugin `jsPlugins` loads is named by its own metadata, which Rerail does not read.
  return jsPlugins ? undefined : `${JSON.stringify(key)} names a plugin it does not load`;
}

/** Why oxlint refuses the `categories` value; undefined when it takes it. */
function categoriesRefusal(value: Record<string, unknown>): string | undefined {
  const wrong = Object.entries(value).find(([category, level]) => !CATEGORIES.has(category) || Array.isArray(level) || severityOf(level, "oxlint") === undefined);
  return wrong === undefined ? undefined : `${entryText("its categories", wrong[0], wrong[1])} is not a category and level oxlint takes`;
}

/** Why oxlint refuses `plugins`; undefined when it takes it. */
function pluginsRefusal(value: unknown): string | undefined {
  const listed = pluginList.safeParse(value ?? []);
  if (!listed.success) return "its plugins are not a list of plugin names";
  const unknown = listed.data.find(name => builtInPlugin(name, LISTED_PLUGINS) === undefined);
  return unknown === undefined ? undefined : `its plugins name ${JSON.stringify(unknown)}, which is no plugin built into oxlint`;
}

/** Why oxlint 1.88.0 refuses the config, in the order Rerail checks; undefined when it takes it as far as Rerail reads it. */
function configRefusal(config: Record<string, unknown>): string | undefined {
  const key = Object.keys(config).find(name => !CONFIG_KEYS.has(name));
  if (key !== undefined) return `it has a key ${JSON.stringify(key)} oxlint does not take`;
  const rules = jsonObject.safeParse(config.rules ?? {});
  const categories = jsonObject.safeParse(config.categories ?? {});
  if (!rules.success || !categories.success || !configOptions.safeParse(config.options).success) return "its rules, categories or options are not objects of what oxlint takes";
  const jsPlugins = config.jsPlugins !== undefined;
  const entry = Object.entries(rules.data).map(([name, value]) => entryRefusal(name, value, jsPlugins)).find(why => why !== undefined);
  return entry ?? categoriesRefusal(categories.data) ?? pluginsRefusal(config.plugins);
}

/** The plugins oxlint runs for the config's `plugins`, which oxlint takes. */
function pluginsOf(value: unknown): ReadonlySet<string> {
  if (value === undefined) return new Set([ALWAYS_PLUGIN, ...DEFAULT_PLUGINS]);
  return new Set([ALWAYS_PLUGIN, ...pluginList.parse(value).map(name => builtInPlugin(name, LISTED_PLUGINS)!)]);
}

/** The JSON config `file` with text `text` (undefined: not a regular file), as oxlint takes it; undefined when it is not read, `"refused"` (with a note) when oxlint refuses it. */
function readOxlintConfig(file: string, text: string | undefined, notes: LinterNote[]): OxlintConfig | "refused" | undefined {
  const config = jsoncFile(file, text, "oxlint", notes);
  if (config === undefined) return undefined;
  const refused = configRefusal(config);
  if (refused !== undefined) {
    notes.push({ linter: "oxlint", text: `${file} is not read: oxlint 1.88.0 refuses it (${refused}), so it runs no rule.` });
    return "refused";
  }
  const unread = ["extends", "overrides"].filter(key => Object.hasOwn(config, key));
  if (unread.length > 0) notes.push({ linter: "oxlint", text: `${file}: only its own rules are read, not ${unread.join(" or ")}.` });
  if (config.ignorePatterns !== undefined) notes.push({ linter: "oxlint", text: `${file}: its ignorePatterns are not read; the rules Rerail sets apply to the files they ignore too.` });
  return {
    file,
    rules: config.rules as Record<string, unknown> | undefined ?? {},
    plugins: pluginsOf(config.plugins),
    categories: config.categories as Record<string, unknown> | undefined ?? {},
    typeAware: (config.options as { typeAware?: boolean } | undefined)?.typeAware === true,
    unread: unread.map(key => `${file} ${key}`),
  };
}

/** What one oxlint rule of a port comes to, with notes on what is not used. */
interface RuleVerdict {
  readonly decision: PortDecision;
  readonly notes: string[];
}

/** The rule's last `rules` key, as oxlint applies a rule's last key: `{"no-console": "off", "eslint/no-console": "error"}` keeps it on. */
function lastKeyOf(config: OxlintConfig, rule: OxlintRule): string | undefined {
  return Object.keys(config.rules).findLast(candidate => {
    const resolved = oxlintRuleOfKey(candidate);
    return resolved.plugin === rule.plugin && resolved.name === rule.name;
  });
}

/** Why an entry is not used, if it is not: one the unread `overrides` may replace. */
function unusedEntry(ruleId: string, config: OxlintConfig, because: string): string | undefined {
  const overrides = config.unread.find(unread => unread.endsWith("overrides"));
  return overrides === undefined ? undefined : `${ruleId}: ${because} is not used: ${overrides}, which Rerail does not read, may set it for some files.`;
}

/** An entry turning `rule` on: on, unless its plugin is one oxlint does not run, which leaves it off. */
function onDecision(config: OxlintConfig, rule: OxlintRule, because: string): PortDecision {
  return config.plugins.has(rule.plugin)
    ? { on: true, explicit: true, because }
    : { on: false, explicit: false, because: `${because}, but ${config.file} does not run the ${rule.plugin} plugin` };
}

/** What the config's `rules` entry `key`, one oxlint takes, says about `rule`. */
function keyVerdict(ruleId: string, config: OxlintConfig, rule: OxlintRule, key: string): RuleVerdict {
  const value = config.rules[key];
  const because = entryText(config.file, key, value);
  const unused = unusedEntry(ruleId, config, because);
  if (unused !== undefined) return { decision: { on: "unknown", because }, notes: [unused] };
  const severity = severityOf(value, "oxlint")!;
  if (severity.off) return { decision: { on: false, explicit: true, because }, notes: [] };
  // The ports' options follow the upstream rule's; oxlint's own options are not mapped onto them.
  const notes = severity.options.length > 0 ? [`${ruleId}: option not mapped: Rerail maps ESLint options, not oxlint's (${because}).`] : [];
  return { decision: onDecision(config, rule, because), notes };
}

/** What a category's level, given or oxlint's default, makes of `rule`. */
function categoryDecision(config: OxlintConfig, category: string): PortDecision {
  const level = config.categories[category];
  if (level === undefined) {
    return category === DEFAULT_CATEGORY
      ? { on: true, explicit: false, because: `${config.file}: oxlint 1.88.0's default (the ${DEFAULT_CATEGORY} category)` }
      : { on: false, explicit: false, because: `${config.file} does not turn it on` };
  }
  return { on: !severityOf(level, "oxlint")!.off, explicit: true, because: `${config.file}: categories.${category}: ${JSON.stringify(level)}` };
}

/** What the config says about `rule`, which its `rules` do not set: its category decides, in the plugins oxlint runs. */
function unsetDecision(config: OxlintConfig, rule: PortRule): PortDecision {
  if (config.unread.length > 0) return { on: "unknown", because: `${config.unread.join(" and ")}, which Rerail does not read, may set it` };
  if (!config.plugins.has(rule.plugin)) return { on: false, explicit: false, because: `${config.file} does not turn it on` };
  return categoryDecision(config, rule.category);
}

/** `decision` for `rule`, unknown where it turns on a type-aware rule the config does not run type-aware: only --type-aware runs it. */
function typeAwareDecision(config: OxlintConfig, rule: OxlintRule, decision: PortDecision): PortDecision {
  return decision.on === true && !config.typeAware && OXLINT_TYPE_AWARE.has(`${rule.plugin}/${rule.name}`)
    ? { on: "unknown", because: `${decision.because}, for a rule oxlint runs only with --type-aware` }
    : decision;
}

/** What the config says about one oxlint rule of a port. */
function ruleVerdict(ruleId: string, config: OxlintConfig, rule: PortRule): RuleVerdict {
  const key = lastKeyOf(config, rule);
  const verdict = key === undefined ? { decision: unsetDecision(config, rule), notes: [] } : keyVerdict(ruleId, config, rule, key);
  return { decision: typeAwareDecision(config, rule, verdict.decision), notes: verdict.notes };
}

/** A folder's config file: read, or not (an `oxlint.config.ts`, or a file Rerail cannot read as JSON). */
interface FolderConfig {
  readonly folder: string;
  readonly file: string;
  readonly config?: OxlintConfig;
}

/** What the folder's config says about the port's oxlint `rules`, of which there is at least one. */
function folderVerdict(port: Port, rules: readonly PortRule[], folder: FolderConfig): RuleVerdict {
  const config = folder.config;
  if (config === undefined) return { decision: { on: "unknown", because: `${folder.file} is not read` }, notes: [] };
  const verdicts = rules.map(rule => ruleVerdict(port.ruleId, config, rule));
  return { decision: combineDecisions(verdicts.map(verdict => verdict.decision))!, notes: verdicts.flatMap(verdict => verdict.notes) };
}

/**
 * The verdict of the configs on `port`: the root's for every file, and each nested one's for its folder where it
 * differs from the nearest folder holding it (or, in none, from every file's).
 */
function oxlintVerdict(configs: readonly FolderConfig[], port: Port): PortVerdict | undefined {
  const rules = oxlintRulesOfPort(port);
  if (rules.length === 0) return undefined;
  const verdicts = configs.map(folder => ({ folder: folder.folder, ...folderVerdict(port, rules, folder) }));
  const root = verdicts.find(verdict => verdict.folder === "")?.decision;
  const folders: FolderDecision[] = [];
  // Sorted, a folder comes after every folder holding it.
  for (const { folder, decision } of verdicts.filter(verdict => verdict.folder !== "")) {
    const holder = folders.findLast(known => folder.startsWith(`${known.folder}/`)) ?? root;
    if (!settlesAlike(decision, holder)) folders.push(inFolder(folder, decision));
  }
  return { ...(root === undefined ? {} : { decision: root }), ...(folders.length > 0 ? { folders } : {}), notes: verdicts.flatMap(verdict => verdict.notes) };
}

/** A config file present in the repository: its path, and its text where it is a regular file Rerail reads. */
interface PresentConfig {
  readonly path: string;
  readonly text: string | undefined;
}

/** Per folder holding any (`""`: the root), the config files oxlint would take there. */
function configFiles(root: string): Map<string, PresentConfig[]> {
  const nested = repositoryFiles(root).filter(path => path.includes("/") && CONFIG_FILES.includes(posix.basename(path)));
  const present = [...CONFIG_FILES, ...nested].flatMap(path => {
    const read = readRootFile(root, path);
    return read.present ? [{ path, text: read.text }] : [];
  });
  return Map.groupBy(present, file => (file.path.includes("/") ? posix.dirname(file.path) : ""));
}

/** The folder's config, as read; `"refused"` (with a note) where oxlint refuses to run. */
function folderConfig(folder: string, files: readonly PresentConfig[], notes: LinterNote[]): FolderConfig | "refused" {
  const { path: file, text } = files[0]!;
  if (files.length > 1) {
    notes.push({ linter: "oxlint", text: `${files.map(found => found.path).join(" and ")} are not read: oxlint 1.88.0 takes one config file per folder and refuses to run with more, so it runs no rule.` });
    return "refused";
  }
  if (posix.basename(file) === SCRIPT_CONFIG) {
    notes.push({ linter: "oxlint", text: `${file} is not read: Rerail never runs a config; the oxlint rules in ${folder === "" ? "the repository" : `${folder}/`} keep the pack's values.` });
    return { folder, file };
  }
  const config = readOxlintConfig(file, text, notes);
  return config === "refused" ? config : { folder, file, ...(config === undefined ? {} : { config }) };
}

export const oxlintReader: LinterReader = {
  linter: "oxlint",
  kinds: ["oxlint"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const found = [...configFiles(root)].sort(([left], [right]) => left.localeCompare(right));
    if (found.length === 0) return undefined;
    const configs = found.map(([folder, files]) => folderConfig(folder, files, notes));
    if (configs.includes("refused")) return undefined;
    return { verdict: port => oxlintVerdict(configs as FolderConfig[], port) };
  },
};
