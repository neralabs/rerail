// Writes src/engine/catalog/lint-config/generated/eslint-presets.generated.ts: the shared ESLint configs ("presets") set-up
// recognises in a repository's ESLint config, each as the rule entries ESLint applies for it, in order, with the files
// each is limited to. Set-up reads a repository's config as data and never loads a plugin, so what a preset turns on or
// off comes from this table, made from the releases the packs port. Run offline, not in CI:
//   npm install --prefix <tools> --legacy-peer-deps typescript-eslint@8.70.1 @typescript-eslint/eslint-plugin@8.70.1 \
//     @typescript-eslint/parser@8.70.1 typescript@6.0.3 eslint@10.11.0 eslint-plugin-react@7.37.5 \
//     eslint-plugin-react-hooks@7.1.1 eslint-plugin-unicorn@76.0.0 eslint-plugin-n@18.4.1 \
//     @next/eslint-plugin-next@16.3.8 eslint-config-next@16.3.8 next@16.3.8 eslint-plugin-vue@10.11.1 \
//     vue-eslint-parser@10.4.1 @vitest/eslint-plugin@1.6.27
//   node scripts/generate/lint-config/eslint-presets.mjs <tools>
// The versions are the packs' pins (packs/{javascript,typescript,react,node,next,vue}/conformance.json `tools`); any
// other installed version is refused. `@eslint/js` is read from the ESLint commit packs/javascript/upstream/eslint/
// gates.yaml pins (tag v10.11.0), where `eslint:recommended` and `eslint:all` live (packages/js/src/configs). Each rule
// keeps its severity only: set-up maps a repository's own options, not a preset's, and some presets' options follow the
// directory they are loaded in (eslint-plugin-n's `recommended` reads the package.json there).
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const ESLINT_COMMIT = "3c0b7c6e1fe2dec778b97a996018126f22d437ae";
/** Package → pinned version → the name an eslintrc `plugin:<name>/<config>` gives it (none: no eslintrc configs). */
const PACKAGES = {
  "typescript-eslint": { version: "8.70.1" },
  "@typescript-eslint/eslint-plugin": { version: "8.70.1", plugin: "@typescript-eslint" },
  "eslint-plugin-react": { version: "7.37.5", plugin: "react" },
  "eslint-plugin-react-hooks": { version: "7.1.1", plugin: "react-hooks" },
  "eslint-plugin-unicorn": { version: "76.0.0", plugin: "unicorn" },
  "eslint-plugin-n": { version: "18.4.1", plugin: "n" },
  "@next/eslint-plugin-next": { version: "16.3.8", plugin: "@next/next" },
  "eslint-config-next": { version: "16.3.8", modules: ["eslint-config-next", "eslint-config-next/core-web-vitals", "eslint-config-next/typescript"] },
  "eslint-plugin-vue": { version: "10.11.1", plugin: "vue" },
  "@vitest/eslint-plugin": { version: "1.6.27", plugin: "@vitest" },
};

const tools = process.argv[2];
if (tools === undefined) throw new Error("usage: node scripts/generate/lint-config/eslint-presets.mjs <tools>");
const require = createRequire(join(tools, "node_modules", "noop.js"));

/** `@eslint/js`'s config file at the pinned ESLint commit: a frozen `{ name, rules }` of `"rule": "level"` lines. */
async function eslintJsConfig(file) {
  const url = `https://raw.githubusercontent.com/eslint/eslint/${ESLINT_COMMIT}/packages/js/src/configs/${file}`;
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: ${response.status}`);
  const rules = {};
  for (const match of (await response.text()).matchAll(/^\s+"?([A-Za-z0-9/@-]+)"?: "(error|warn|off)",?$/gmu)) rules[match[1]] = match[2];
  if (Object.keys(rules).length < 50) throw new Error(`${url}: no rules found`);
  return { rules };
}

const CONFIG_KEYS = ["rules", "plugins", "extends", "overrides", "languageOptions", "name", "env", "parserOptions", "parser", "settings", "files", "globals"];
const isConfigObject = value => value !== null && typeof value === "object" && !Array.isArray(value) && CONFIG_KEYS.some(key => Object.hasOwn(value, key));
/** An eslintrc config: what a flat config refuses (a plugins list, extends, overrides, env, parserOptions, parser, globals). */
const isEslintrc = value => !Array.isArray(value) && (Array.isArray(value.plugins) || ["extends", "overrides", "env", "parserOptions", "parser", "globals"].some(key => Object.hasOwn(value, key)));
/** A config of rules alone works either way. */
const isRulesOnly = value => !Array.isArray(value) && Object.keys(value).every(key => key === "rules" || key === "name");

/** A config's rules as their severities: `"error"` for `["error", { ... }]`. */
const severities = rules => Object.fromEntries(Object.entries(rules).map(([key, value]) => [key, Array.isArray(value) ? value[0] : value]));

/** A flat config (an object or a nested array of them) as entries: `files`, `ignores` and rule severities, in order. */
function flatEntries(config) {
  if (Array.isArray(config)) return config.flatMap(flatEntries);
  if (config.rules === undefined || Object.keys(config.rules).length === 0) return [];
  return [{
    ...(config.files === undefined ? {} : { files: config.files }),
    ...(config.ignores === undefined ? {} : { ignores: config.ignores }),
    rules: severities(config.rules),
  }];
}

/** An eslintrc `files`/`excludedFiles` value as a list. */
const patterns = value => (typeof value === "string" ? [value] : value);

/**
 * An eslintrc config as entries, as ESLint's eslintrc loader orders them: each `extends` element (and its overrides)
 * first, then the config's own rules, then its overrides, each limited to its files less its excluded files.
 */
function eslintrcEntries(config, from, eslintrc, scope = {}) {
  const extended = config.extends === undefined ? [] : patterns(config.extends).flatMap(name => eslintrcEntries(extendedConfig(name, from, eslintrc), from, eslintrc, scope));
  const own = config.rules === undefined || Object.keys(config.rules).length === 0 ? [] : [{ ...scope, rules: severities(config.rules) }];
  const overrides = (config.overrides ?? []).flatMap(override => eslintrcEntries(override, from, eslintrc, {
    files: patterns(override.files),
    ...(override.excludedFiles === undefined ? {} : { ignores: patterns(override.excludedFiles) }),
  }));
  return [...extended, ...own, ...overrides];
}

/** The config an eslintrc `extends` names from a plugin's file `from`: a preset of the table, or a file of the plugin. */
function extendedConfig(name, from, eslintrc) {
  if (eslintrc.has(name)) return eslintrc.get(name).config;
  if (!name.startsWith(".") && !name.startsWith("/")) throw new Error(`${from}: extends ${name}, which is not in the table`);
  return require(createRequire(from).resolve(name));
}

/** Each config a package's `configs` holds, at its member path; `configs.flat.recommended` too. */
function configsOf(configs, path = ["configs"]) {
  return Object.entries(configs ?? {}).flatMap(([key, value]) => {
    if (Array.isArray(value) || isConfigObject(value)) return [{ path: [...path, key], config: value }];
    return value !== null && typeof value === "object" && path.length === 1 ? configsOf(value, [...path, key]) : [];
  });
}

/** What the generator gathers: flat presets, eslintrc presets by name, named exports per module, and its sources. */
const table = { flat: [], eslintrc: new Map(), named: {}, sources: [`@eslint/js at eslint/eslint ${ESLINT_COMMIT} (packages/js/src/configs)`] };

/**
 * A flat preset at `path` of `module`; for one config object, its `rules` member too (`...js.configs.recommended.rules`
 * in a config's `rules`), as one entry for whatever files the object that spreads it is for.
 */
function addFlat(module, path, config) {
  table.flat.push({ module, path, entries: flatEntries(config) });
  if (!Array.isArray(config) && config.rules !== undefined) table.flat.push({ module, path: [...path, "rules"], entries: [{ rules: severities(config.rules) }] });
}

/** `eslint:recommended` and `eslint:all`, from `@eslint/js` at the pinned commit. */
async function addEslintJs() {
  for (const file of ["eslint-recommended.js", "eslint-all.js"]) {
    const config = await eslintJsConfig(file);
    const name = file.replace(/^eslint-|\.js$/gu, "");
    addFlat("@eslint/js", ["configs", name], config);
    table.eslintrc.set(`eslint:${name}`, { config, from: "@eslint/js" });
  }
}

/** Whether a plugin's config at `path` is one an eslintrc `plugin:` name gives: a top-level eslintrc or rules-only one. */
const isEslintrcPreset = (path, config) => path.length === 2 && (isEslintrc(config) || isRulesOnly(config));

/** A plugin's `configs`: each flat one at its member path, each eslintrc one by `plugin:` name. */
function addPluginConfigs(module, file, exported, plugin) {
  for (const { path, config } of configsOf(exported.configs)) {
    if (!isEslintrc(config)) addFlat(module, path, config);
    if (plugin === undefined || !isEslintrcPreset(path, config)) continue;
    for (const prefix of new Set([plugin, module])) table.eslintrc.set(`plugin:${prefix}/${path[1]}`, { config, from: file });
  }
}

/** One module of a pinned package, loaded from `<tools>`. */
async function addModule(module, plugin, wholeModule) {
  const file = require.resolve(module);
  const namespace = await import(pathToFileURL(file).href);
  const exported = namespace.default ?? namespace;
  table.named[module] = Object.keys(namespace).filter(key => key !== "default" && exported[key] !== undefined && namespace[key] === exported[key]).sort();
  if (wholeModule) addFlat(module, [], exported);
  else addPluginConfigs(module, file, exported, plugin);
}

async function main() {
  await addEslintJs();
  for (const [name, { version, plugin, modules }] of Object.entries(PACKAGES)) {
    const manifest = JSON.parse(readFileSync(join(tools, "node_modules", name, "package.json"), "utf8"));
    if (manifest.version !== version) throw new Error(`${name} ${manifest.version} found; the table is pinned to ${version}`);
    table.sources.push(`${name}@${version}`);
    for (const module of modules ?? [name]) await addModule(module, plugin, modules !== undefined);
  }
  const flatRows = table.flat.map(({ module, path, entries }) => ({ key: JSON.stringify([module, ...path]), entries }));
  const eslintrcRows = [...table.eslintrc].map(([key, { config, from }]) => ({ key, entries: eslintrcEntries(config, from, table.eslintrc) }));
  write(flatRows, eslintrcRows, table.named, table.sources);
}

/** The table as TypeScript: each distinct entry once, each preset key naming its entries by index. */
function write(flatRows, eslintrcRows, named, sources) {
  const entries = [];
  const indexOf = new Map();
  const index = entry => {
    const text = JSON.stringify(entry);
    if (!indexOf.has(text)) {
      indexOf.set(text, entries.length);
      entries.push(text);
    }
    return indexOf.get(text);
  };
  const keyed = rows => rows.sort((left, right) => left.key.localeCompare(right.key))
    .map(({ key, entries: list }) => `  [${JSON.stringify(key)}, [${list.map(index).join(", ")}]],`).join("\n");
  const flatText = keyed(flatRows);
  const eslintrcText = keyed(eslintrcRows);
  const text = `// Generated by scripts/generate/lint-config/eslint-presets.mjs; do not edit.
// From ${sources.join(", ")}.

/** One rule entry of a preset: its rules, limited to \`files\` less \`ignores\` where it has them (as ESLint matches them). */
export interface EslintPresetEntry {
  readonly files?: readonly unknown[];
  readonly ignores?: readonly unknown[];
  readonly rules: Readonly<Record<string, unknown>>;
}

/** Each distinct entry of the presets. */
const ENTRIES: readonly EslintPresetEntry[] = [
${entries.map(entry => `  ${entry},`).join("\n")}
];

/** A flat config preset by \`JSON.stringify([module, ...memberPath])\` of the module's default export, with its entries' indexes in order. */
const FLAT: readonly (readonly [string, readonly number[]])[] = [
${flatText}
];

/** An eslintrc \`extends\` name (\`eslint:recommended\`, \`plugin:react/recommended\`) with its entries' indexes in order. */
const ESLINTRC: readonly (readonly [string, readonly number[]])[] = [
${eslintrcText}
];

const presets = (rows: typeof FLAT): ReadonlyMap<string, readonly EslintPresetEntry[]> => new Map(rows.map(([key, list]) => [key, list.map(entry => ENTRIES[entry]!)]));
export const ESLINT_FLAT_PRESETS = presets(FLAT);
export const ESLINTRC_PRESETS = presets(ESLINTRC);
/** Per module, the named exports that are its default export's member of the same name (\`import { configs } from "typescript-eslint"\`). */
export const ESLINT_PRESET_NAMED_EXPORTS: ReadonlyMap<string, readonly string[]> = new Map(${JSON.stringify(Object.entries(named))});
`;
  writeFileSync(join(import.meta.dirname, "../../../src/engine/catalog/lint-config/generated/eslint-presets.generated.ts"), text);
}

await main();
