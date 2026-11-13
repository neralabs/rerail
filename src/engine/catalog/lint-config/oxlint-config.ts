/**
 * oxlint's `.oxlintrc.json` at the root, read as data: its `rules`, `plugins` and `categories`. A port's oxlint rule is
 * the built-in rule oxlint runs for the ESLint rule the port ports (`oxlintRuleOfPort`), named bare in its
 * `suppress.oxlint`. A config key resolves to a built-in rule as oxlint resolves it (`oxlintRuleOfKey`), not by its bare
 * name as its suppression comments are: `custom/no-console` is another plugin's rule and leaves ESLint's `no-console`
 * alone.
 *
 * A rule the config's `rules` sets is set explicitly; one it does not set follows its category: the level `categories`
 * gives the category, else oxlint's default, which runs the `correctness` category (oxlint 1.88.0
 * `--print-config` with an empty `.oxlintrc.json`), and only for the plugins oxlint runs: `eslint` always, the
 * `plugins` the config lists, or `unicorn`, `typescript` and `oxc` when it lists none. `extends` and `overrides` are not
 * read: what they may set is unknown.
 */
import { z } from "zod";
import { entryText, jsoncFile, jsonObject, severityOf, type Severity } from "./jsonc-lint-config.js";
import { combineDecisions } from "./linter-decisions.js";
import type { LinterConfig, LinterNote, LinterReader, Port, PortDecision, PortVerdict } from "./linter-config.js";
import { OXLINT_RULES, OXLINT_TYPE_AWARE } from "./generated/oxlint-rules.generated.js";

const FILE = ".oxlintrc.json";

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

/** oxlint's name for a plugin the config names: a built-in one's internal name, any other as written. */
function builtInPlugin(written: string): string {
  const normalized = normalizeOxlintPluginName(written);
  // `LintPlugins::try_from` normalises the name once more.
  return OXLINT_PLUGINS[normalized] ?? OXLINT_PLUGINS[normalizeOxlintPluginName(normalized)] ?? normalized;
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
  const plugin = builtInPlugin(key.slice(0, split));
  return { plugin: plugin === "typescript" && OXLINT_TYPESCRIPT_AS_ESLINT.has(name) ? "eslint" : plugin, name };
}

/**
 * The oxlint rules a port models: for each ESLint rule it ports, the rule oxlint configures for that ESLint key, except
 * that eslint-plugin-n's `n/<rule>` is oxlint's `node/<rule>` (its `node` plugin ports eslint-plugin-node, of which
 * eslint-plugin-n is the fork: oxc `config/plugins.rs` l.119), while oxlint itself takes an `n/` key for an external
 * plugin. `suppress.oxlint` names the rule bare, as oxlint's comments do; a port without one has no oxlint rule.
 */
function oxlintRulesOfPort(port: Port): OxlintRule[] {
  const rules = port.suppress.eslint.map(name => oxlintRuleOfKey(name.startsWith("n/") ? `node/${name.slice(2)}` : name));
  return rules.filter((rule, index) => port.suppress.oxlint.includes(rule.name)
    && rules.findIndex(known => known.plugin === rule.plugin && known.name === rule.name) === index);
}

/** The category of a built-in oxlint rule at oxlint 1.88.0, or undefined when it is not one. */
function categoryOf(rule: OxlintRule): string | undefined {
  return Object.entries(OXLINT_RULES[rule.plugin] ?? {}).find(([, names]) => names.includes(rule.name))?.[0];
}

/** An `.oxlintrc.json` as read. */
interface OxlintConfig {
  readonly rules: Readonly<Record<string, unknown>>;
  /** The plugins oxlint runs (internal names); undefined when the config's `plugins` cannot be read. */
  readonly plugins: ReadonlySet<string> | undefined;
  /** `categories`, by category; undefined when it cannot be read. */
  readonly categories: Readonly<Record<string, unknown>> | undefined;
  /** What the config takes that is not read and may set any rule (`extends`, `overrides`), as named to the person. */
  readonly unread: readonly string[];
}

const pluginList = z.array(z.string());

/** The plugins oxlint runs for the config's `plugins`; undefined (with a note) when it is not a list of names. */
function pluginsOf(value: unknown, notes: LinterNote[]): ReadonlySet<string> | undefined {
  if (value === undefined) return new Set([ALWAYS_PLUGIN, ...DEFAULT_PLUGINS]);
  const listed = pluginList.safeParse(value);
  if (listed.success) return new Set([ALWAYS_PLUGIN, ...listed.data.map(builtInPlugin)]);
  notes.push({ linter: "oxlint", text: `${FILE}: its plugins are not read: they are not a list of plugin names.` });
  return undefined;
}

/** The config's `categories`; undefined (with a note) when it is not an object. */
function categoriesOf(value: unknown, notes: LinterNote[]): Readonly<Record<string, unknown>> | undefined {
  const categories = jsonObject.safeParse(value ?? {});
  if (categories.success) return categories.data;
  notes.push({ linter: "oxlint", text: `${FILE}: its categories are not read: they are not an object.` });
  return undefined;
}

/** The `.oxlintrc.json` at `root`; undefined when there is none, or (with a note) when its rules cannot be read. */
function readOxlintConfig(root: string, notes: LinterNote[]): OxlintConfig | undefined {
  const config = jsoncFile(root, FILE, "oxlint", notes);
  if (config === undefined) return undefined;
  const unread = ["extends", "overrides"].filter(key => Object.hasOwn(config, key));
  if (unread.length > 0) notes.push({ linter: "oxlint", text: `${FILE}: only its own rules are read, not ${unread.join(" or ")}.` });
  const rules = jsonObject.safeParse(config.rules ?? {});
  if (!rules.success) {
    notes.push({ linter: "oxlint", text: `${FILE} is not read: its rules are not an object.` });
    return undefined;
  }
  return {
    rules: rules.data,
    plugins: pluginsOf(config.plugins, notes),
    categories: categoriesOf(config.categories, notes),
    unread: unread.map(key => `${FILE} ${key}`),
  };
}

/** What one oxlint rule of a port comes to, with notes on what is not used. */
interface RuleVerdict {
  readonly decision?: PortDecision;
  readonly notes: string[];
}

/** The rule's last `rules` key, as oxlint applies a rule's last key: `{"no-console": "off", "eslint/no-console": "error"}` keeps it on. */
function lastKeyOf(config: OxlintConfig, rule: OxlintRule): string | undefined {
  return Object.keys(config.rules).findLast(candidate => {
    const resolved = oxlintRuleOfKey(candidate, name => (name === rule.name ? rule.plugin : undefined));
    return resolved.plugin === rule.plugin && resolved.name === rule.name;
  });
}

/** Why an entry is not used, if it is not: not a severity oxlint takes, or one the unread `overrides` may replace. */
function unusedEntry(ruleId: string, config: OxlintConfig, because: string, severity: Severity | undefined): string | undefined {
  if (severity === undefined) return `${ruleId}: ${because} is not a severity oxlint takes; not read.`;
  const overrides = config.unread.find(unread => unread.endsWith("overrides"));
  return overrides === undefined ? undefined : `${ruleId}: ${because} is not used: ${overrides}, which Rerail does not read, may set it for some files.`;
}

/** An entry turning `rule` on: on, unless its plugin is one oxlint does not run, which leaves it off. */
function onDecision(config: OxlintConfig, rule: OxlintRule, because: string): PortDecision {
  return config.plugins?.has(rule.plugin) === false
    ? { on: false, explicit: false, because: `${because}, but ${FILE} does not run the ${rule.plugin} plugin` }
    : { on: true, explicit: true, because };
}

/** What the config's `rules` entry `key` says about `rule`. */
function keyVerdict(ruleId: string, config: OxlintConfig, rule: OxlintRule, key: string): RuleVerdict {
  const value = config.rules[key];
  const because = entryText(FILE, key, value);
  const severity = severityOf(value, "oxlint");
  const unused = unusedEntry(ruleId, config, because, severity);
  if (unused !== undefined) return { decision: { on: "unknown", because }, notes: [unused] };
  if (severity!.off) return { decision: { on: false, explicit: true, because }, notes: [] };
  // The ports' options follow the upstream rule's; oxlint's own options are not mapped onto them.
  const notes = severity!.options.length > 0 ? [`${ruleId}: option not mapped: Rerail maps ESLint options, not oxlint's (${because}).`] : [];
  return { decision: onDecision(config, rule, because), notes };
}

/** What a category's level, given or oxlint's default, makes of `rule`. */
function categoryDecision(config: OxlintConfig, rule: OxlintRule, category: string): PortDecision {
  const level = config.categories![category];
  if (level === undefined) {
    return category === DEFAULT_CATEGORY
      ? { on: true, explicit: false, because: `${FILE}: oxlint 1.88.0's default (the ${DEFAULT_CATEGORY} category)` }
      : { on: false, explicit: false, because: `${FILE} does not turn it on` };
  }
  const because = `${FILE}: categories.${category}: ${JSON.stringify(level)}`;
  const severity = severityOf(level, "oxlint");
  return severity === undefined ? { on: "unknown", because: `${because} is not a level oxlint takes` } : { on: !severity.off, explicit: true, because };
}

/** Why the config cannot decide a rule its `rules` do not set: something it takes is not read. */
function undecidable(config: OxlintConfig): PortDecision | undefined {
  if (config.unread.length > 0) return { on: "unknown", because: `${config.unread.join(" and ")}, which Rerail does not read, may set it` };
  return config.categories === undefined || config.plugins === undefined ? { on: "unknown", because: `${FILE} is not read in full` } : undefined;
}

/** What the config says about `rule`, which its `rules` do not set: its category decides, in the plugins oxlint runs. */
function unsetVerdict(config: OxlintConfig, rule: OxlintRule): PortDecision | undefined {
  const category = categoryOf(rule);
  if (category === undefined) return undefined;
  const unknown = undecidable(config);
  if (unknown !== undefined) return unknown;
  if (!config.plugins!.has(rule.plugin)) return { on: false, explicit: false, because: `${FILE} does not turn it on` };
  const decision = categoryDecision(config, rule, category);
  // A type-aware rule runs only with --type-aware, which a config file does not say.
  return decision.on === true && OXLINT_TYPE_AWARE.has(`${rule.plugin}/${rule.name}`)
    ? { on: "unknown", because: `${decision.because}, for a rule oxlint runs only with --type-aware` }
    : decision;
}

/** What the config says about one oxlint rule of a port. */
function ruleVerdict(ruleId: string, config: OxlintConfig, rule: OxlintRule): RuleVerdict {
  const key = lastKeyOf(config, rule);
  if (key !== undefined) return keyVerdict(ruleId, config, rule, key);
  const decision = unsetVerdict(config, rule);
  return decision === undefined ? { notes: [] } : { decision, notes: [] };
}

/** The verdict of the oxlint config on `port`, over the oxlint rules it ports. */
function oxlintVerdict(config: OxlintConfig, port: Port): PortVerdict | undefined {
  const rules = oxlintRulesOfPort(port);
  if (rules.length === 0) return undefined;
  const verdicts = rules.map(rule => ruleVerdict(port.ruleId, config, rule));
  const decision = combineDecisions(verdicts.flatMap(verdict => (verdict.decision === undefined ? [] : [verdict.decision])));
  return { ...(decision === undefined ? {} : { decision }), notes: verdicts.flatMap(verdict => verdict.notes) };
}

export const oxlintReader: LinterReader = {
  linter: "oxlint",
  kinds: ["oxlint"],
  read(root: string, notes: LinterNote[]): LinterConfig | undefined {
    const config = readOxlintConfig(root, notes);
    return config === undefined ? undefined : { verdict: port => oxlintVerdict(config, port) };
  },
};
