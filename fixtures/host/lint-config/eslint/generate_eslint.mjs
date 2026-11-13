// Records eslint.expected.json: for each config of corpus.json, the ported ESLint rules the real ESLint runs on each
// probe file, as the oracle of set-up's reading of ESLint configs (src/engine/catalog/lint-config/eslint-*.ts).
// Run by hand, not in CI, against the pinned tools:
//   npm install --prefix <tools> --legacy-peer-deps eslint@10.11.0 @eslint/js@10.0.1 globals@16.5.0 \
//     @eslint/eslintrc@3.3.7 typescript-eslint@8.70.1 @typescript-eslint/eslint-plugin@8.70.1 \
//     @typescript-eslint/parser@8.70.1 typescript@6.0.3 eslint-plugin-react@7.37.5 eslint-plugin-react-hooks@7.1.1 \
//     eslint-plugin-unicorn@76.0.0 eslint-plugin-n@18.4.1 @next/eslint-plugin-next@16.3.8 eslint-plugin-vue@10.11.1 \
//     vue-eslint-parser@10.4.1 @vitest/eslint-plugin@1.6.27
//   node fixtures/host/lint-config/eslint/generate_eslint.mjs <tools>
// The versions are the packs' pins (packs/<pack>/upstream/<tool>/gates.yaml); any other installed version is refused.
//
// Each case's files are written to <tools>/cases/<name>/ (so its imports resolve to <tools>/node_modules) and read
// there. A flat config is ESLint 10.11.0's own: `new ESLint({ cwd }).calculateConfigForFile(probe)`, which looks the
// config up from the probe's folder. ESLint 10 reads no eslintrc, so an eslintrc case is read by the eslintrc loader
// ESLint 9 and earlier ran, @eslint/eslintrc 3.3.7's CascadingConfigArrayFactory, with ESLint 10.11.0's rules and
// @eslint/js's configs. A rule is recorded under the name a port gives it by the rule it is, not the id the config
// names it by: `ts/no-explicit-any` is typescript-eslint's `@typescript-eslint/no-explicit-any` when `ts` is bound to
// typescript-eslint's plugin. Per probe, the ported rules on (severity warn or error), sorted and space-joined; every
// other ported rule is off there.
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";

const HERE = import.meta.dirname;
const PACKS = join(HERE, "../../../../packs");
const PINS = {
  eslint: "10.11.0",
  "@eslint/js": "10.0.1",
  "@eslint/eslintrc": "3.3.7",
  "typescript-eslint": "8.70.1",
  "@typescript-eslint/eslint-plugin": "8.70.1",
  "eslint-plugin-react": "7.37.5",
  "eslint-plugin-react-hooks": "7.1.1",
  "eslint-plugin-unicorn": "76.0.0",
  "eslint-plugin-n": "18.4.1",
  "@next/eslint-plugin-next": "16.3.8",
  "eslint-plugin-vue": "10.11.1",
  "@vitest/eslint-plugin": "1.6.27",
};
/** The plugins the packs port, by the namespace the ports name their rules under. */
const PLUGINS = {
  "@typescript-eslint": "@typescript-eslint/eslint-plugin",
  react: "eslint-plugin-react",
  "react-hooks": "eslint-plugin-react-hooks",
  unicorn: "eslint-plugin-unicorn",
  n: "eslint-plugin-n",
  "@next/next": "@next/eslint-plugin-next",
  vue: "eslint-plugin-vue",
  vitest: "@vitest/eslint-plugin",
};
const FLAT_CONFIGS = ["eslint.config.js", "eslint.config.mjs", "eslint.config.cjs", "eslint.config.ts", "eslint.config.mts", "eslint.config.cts"];

const tools = process.argv[2];
if (tools === undefined) throw new Error("usage: node fixtures/host/lint-config/eslint/generate_eslint.mjs <tools>");
const require = createRequire(join(tools, "node_modules", "noop.js"));
for (const [name, version] of Object.entries(PINS)) {
  const found = JSON.parse(readFileSync(join(tools, "node_modules", name, "package.json"), "utf8")).version;
  if (found !== version) throw new Error(`${name} ${found} found; the oracle is pinned to ${version}`);
}
const load = async name => {
  const module = await import(pathToFileURL(require.resolve(name)).href);
  return module.default ?? module;
};

/** Every ESLint rule name the packs' ports give in `suppress.eslint`. */
function portedNames() {
  const names = new Set();
  const visit = value => {
    if (Array.isArray(value)) value.forEach(visit);
    else if (value !== null && typeof value === "object") {
      for (const name of value.suppress?.eslint?.rules ?? []) names.add(name);
      Object.values(value).forEach(visit);
    }
  };
  for (const pack of readdirSync(PACKS, { withFileTypes: true }).filter(entry => entry.isDirectory())) {
    const rules = join(PACKS, pack.name, "rules");
    let files = [];
    try {
      files = readdirSync(rules).filter(file => file.endsWith(".json"));
    } catch {
      continue;
    }
    for (const file of files) visit(JSON.parse(readFileSync(join(rules, file), "utf8")));
  }
  return names;
}

/** Each ported rule's implementation → the name the ports give it. */
async function portedRules(names) {
  const { builtinRules } = await load("eslint/use-at-your-own-risk");
  const byRule = new Map();
  for (const [name, rule] of builtinRules) if (names.has(name)) byRule.set(rule, name);
  for (const [namespace, module] of Object.entries(PLUGINS)) {
    for (const [name, rule] of Object.entries((await load(module)).rules)) {
      if (names.has(`${namespace}/${name}`)) byRule.set(rule, `${namespace}/${name}`);
    }
  }
  return byRule;
}

/** A rule id as ESLint splits it (`parseRuleId`, lib/config/config.js). */
function ruleIdParts(id) {
  if (!id.includes("/")) return { namespace: "@", name: id };
  const namespace = id.slice(0, id.startsWith("@") ? id.lastIndexOf("/") : id.indexOf("/"));
  return { namespace, name: id.slice(namespace.length + 1) };
}

const isOn = value => {
  const level = Array.isArray(value) ? value[0] : value;
  return level === 1 || level === 2 || level === "warn" || level === "error";
};

/** The ported rules on in `rules` (rule id → setting), each found through `ruleOf`. */
function onRules(rules, ruleOf, byRule) {
  const on = new Set();
  for (const [id, value] of Object.entries(rules ?? {})) {
    const ported = isOn(value) ? byRule.get(ruleOf(id)) : undefined;
    if (ported !== undefined) on.add(ported);
  }
  return [...on].sort().join(" ");
}

/** A flat config's ported rules on at `probe`, as ESLint 10.11.0 calculates its config. */
async function flatProbe(directory, probe, byRule, builtinRules) {
  const { ESLint } = await load("eslint");
  const config = await new ESLint({ cwd: directory }).calculateConfigForFile(join(directory, probe));
  if (config === undefined) return "";
  return onRules(config.rules, id => {
    const { namespace, name } = ruleIdParts(id);
    return namespace === "@" ? builtinRules.get(name) : config.plugins?.[namespace]?.rules?.[name];
  }, byRule);
}

/**
 * `@eslint/js`'s `recommended` and `all` as the eslintrc files `eslint:recommended` and `eslint:all` load: their rules
 * alone (the flat configs' `name` is not an eslintrc key).
 */
async function eslintJsConfigFiles(root) {
  const js = await load("@eslint/js");
  const paths = {};
  for (const name of ["recommended", "all"]) {
    paths[name] = join(root, `eslint-${name}.json`);
    writeFileSync(paths[name], JSON.stringify({ rules: js.configs[name].rules }));
  }
  return paths;
}

/** An eslintrc's ported rules on at `probe`, as @eslint/eslintrc's cascading loader extracts its config. */
async function eslintrcProbe(directory, probe, byRule, builtinRules) {
  const { Legacy } = await load("@eslint/eslintrc");
  const js = await eslintJsConfigFiles(dirname(directory));
  const factory = new Legacy.CascadingConfigArrayFactory({
    cwd: directory,
    resolvePluginsRelativeTo: tools,
    builtInRules: builtinRules,
    eslintRecommendedPath: js.recommended,
    eslintAllPath: js.all,
  });
  const file = join(directory, probe);
  const config = factory.getConfigArrayForFile(file).extractConfig(file);
  return onRules(config.rules, id => {
    const { namespace, name } = ruleIdParts(id);
    return namespace === "@" ? builtinRules.get(name) : config.plugins[namespace]?.definition?.rules?.[name];
  }, byRule);
}

async function main() {
  const { builtinRules } = await load("eslint/use-at-your-own-risk");
  const byRule = await portedRules(portedNames());
  const { cases } = JSON.parse(readFileSync(join(HERE, "corpus.json"), "utf8"));
  const expected = {};
  const root = join(tools, "cases");
  for (const { name, files, probes } of cases) {
    const directory = join(root, name);
    rmSync(directory, { recursive: true, force: true });
    for (const [path, text] of Object.entries(files)) {
      mkdirSync(dirname(join(directory, path)), { recursive: true });
      writeFileSync(join(directory, path), text);
    }
    const probe = FLAT_CONFIGS.some(file => Object.hasOwn(files, file)) ? flatProbe : eslintrcProbe;
    expected[name] = {};
    for (const path of probes) expected[name][path] = await probe(directory, path, byRule, builtinRules);
  }
  rmSync(root, { recursive: true, force: true });
  writeFileSync(join(HERE, "eslint.expected.json"), `${JSON.stringify(expected, undefined, 1)}\n`);
}

await main();
