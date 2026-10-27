// Where fixture data lives (Jeff's structure metric #3) and what each fixtures/<family>/<feature> tests (#10).
//
// #3: a tracked file is fixture data when
//   - a directory in its path is named `fixtures` or `__fixtures__`,
//   - it is a recording (`*.expected.json`) or a case list (`*.cases.json`),
//   - it is a code-rule document (a JSON object with `format: "rerail.policy/v1"` and `clauses`),
//   - it sits in a test root (`test/`, `integration/`, `conformance/`, `benchmark/`) and is not TypeScript or
//     JavaScript source or Markdown: test roots hold code, their data is fixtures, or
//   - it sits in `test/` or `integration/` and its name says fixture (`hook-fixtures.ts`).
// It may live only under `fixtures/` or `packs/<id>/upstream/` (a pack's tool recordings and their inputs). Declared
// exemptions: `packs/<id>/candidates/` (ports still scored, never shipped, with their hand-labelled probe packs: pack
// data), `gate-pins` (the conformance gate's lists: `conformance/languages/ungated-rules.json` and
// `conformance/languages/pending.json`, the not-gated rules and pending tool-packs it reads;
// `conformance/recorders/port-readiness/runtimes/**`, the manifests that pin each upstream runtime; and the Python
// and PHP workers under `conformance/recorders/port-readiness/`, code the recorder runs inside those runtimes) and, for code-rule
// documents only, the shipped rule directories `packs/<id>/rules/` and `code-rule-dsl/rules/`.
//
// #10: every directory fixtures/<family>/<feature>/ names a top-level module of the engine
// (native/src/code_rule_dsl/<feature>.rs or <feature>/), or a documented non-engine group: `upstream` (a language's
// tool recordings ahead of its pack) or `licenses` (third-party records). `fixtures/host/<area>/` (host-test data) and
// `fixtures/packs/<pack>/` (pack-validator inputs) are non-engine families: their children are areas, not features.
//
// Usage: node scripts/check/check-structure/fixtures.mjs   (prints the table; exits 1 on data outside or an unmapped feature)

import { pathToFileURL } from "node:url";
import { compare, loadRepo } from "./repo.mjs";

/** Groups a fixtures/<family>/<feature> may name instead of an engine module. */
export const NON_ENGINE_GROUPS = Object.freeze(["upstream", "licenses"]);
/** Families whose children are areas of non-engine test data, not engine features. */
export const NON_ENGINE_FAMILIES = Object.freeze(["host", "packs"]);

const ENGINE = "native/src/code_rule_dsl/";
/** Files of the engine's root module that are no feature module. */
const NOT_MODULES = new Set(["mod", "test_support"]);
const TEST_ROOTS = ["test/", "integration/", "conformance/", "benchmark/"];
const NAMED_FIXTURE_ROOTS = ["test/", "integration/"];
const SOURCE = /\.(?:[cm]?ts|[cm]?js|md)$|^test_\w+\.py$/u;
const FIXTURE_DIRECTORY = new Set(["fixtures", "__fixtures__"]);
const FIXTURE_NAME = /(?:^|[-_.])fixtures?(?:[-_.]|$)/iu;
const RECORDING = /\.(?:expected|cases)\.json$/u;
const RULE_FORMAT = "rerail.policy/v1";

/** The engine's top-level modules: `native/src/code_rule_dsl/<name>.rs` and `<name>/` directories holding Rust. */
export function engineModules(repo) {
  const modules = new Set();
  for (const path of repo.files) {
    if (!path.startsWith(ENGINE) || !path.endsWith(".rs")) continue;
    const [first, ...rest] = path.slice(ENGINE.length).split("/");
    const name = rest.length === 0 ? first.slice(0, -".rs".length) : first;
    if (!NOT_MODULES.has(name)) modules.add(name);
  }
  return [...modules].sort(compare);
}

function isRuleDocument(repo, path) {
  if (!path.endsWith(".json")) return false;
  try {
    const document = JSON.parse(repo.text(path));
    return document !== null && typeof document === "object" && document.format === RULE_FORMAT && Array.isArray(document.clauses);
  } catch {
    return false;
  }
}

/** Which predicate makes `path` fixture data, or undefined. */
function fixtureKind(repo, path) {
  const segments = path.split("/");
  const name = segments.at(-1);
  if (segments.slice(0, -1).some(segment => FIXTURE_DIRECTORY.has(segment))) return "directory";
  if (RECORDING.test(name)) return "recording";
  if (TEST_ROOTS.some(root => path.startsWith(root)) && !SOURCE.test(name)) return "test-root data";
  if (NAMED_FIXTURE_ROOTS.some(root => path.startsWith(root)) && FIXTURE_NAME.test(name)) return "named";
  if (isRuleDocument(repo, path)) return "rule";
  return undefined;
}

/** Exemption `gate-pins`: the conformance gate's lists and the recorder's runtimes and Python and PHP workers (see the header). */
function isGatePin(path) {
  if (path === "conformance/languages/ungated-rules.json" || path === "conformance/languages/pending.json") return true;
  if (path.startsWith("conformance/recorders/port-readiness/runtimes/")) return true;
  return path.startsWith("conformance/recorders/port-readiness/") && (path.endsWith(".py") || path.endsWith(".php"));
}

function allowed(path, kind) {
  const segments = path.split("/");
  if (segments[0] === "fixtures") return true;
  if (kind === "test-root data" && isGatePin(path)) return true;
  if (segments[0] === "packs" && segments.length > 3 && (segments[2] === "upstream" || segments[2] === "candidates")) return true;
  if (kind !== "rule") return false;
  return (segments[0] === "packs" && segments[2] === "rules") || path.startsWith("code-rule-dsl/rules/");
}

/**
 * `outside`: tracked fixture data outside its places (#3 counts them). `features`: every fixtures/<family>/<feature>
 * directory with the engine module or non-engine group it names. `unmapped`: `fixtures/<family>/<feature>` that name
 * neither (#10 counts them). All sorted.
 */
export function fixtureLayout(repo) {
  const outside = repo.files.filter(path => {
    const kind = fixtureKind(repo, path);
    return kind !== undefined && !allowed(path, kind);
  });
  const modules = new Set(engineModules(repo));
  const seen = new Map();
  for (const path of repo.files) {
    const segments = path.split("/");
    if (segments[0] !== "fixtures" || segments.length < 4) continue;
    const [, family, feature] = segments;
    const key = `fixtures/${family}/${feature}`;
    if (seen.has(key)) continue;
    let group = null;
    let module = null;
    if (NON_ENGINE_FAMILIES.includes(family)) group = family;
    else if (modules.has(feature)) {
      group = "engine";
      module = feature;
    } else if (NON_ENGINE_GROUPS.includes(feature)) group = feature;
    seen.set(key, { family, feature, module, group });
  }
  const features = [...seen.entries()].sort(([a], [b]) => compare(a, b)).map(([, entry]) => entry);
  const unmapped = features.filter(entry => entry.group === null).map(({ family, feature }) => `fixtures/${family}/${feature}`);
  return { outside: outside.sort(compare), features, unmapped };
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const { outside, features, unmapped } = fixtureLayout(loadRepo(process.cwd()));
  console.log("| Feature directory | Engine module | Group |");
  console.log("|---|---|---|");
  for (const { family, feature, module, group } of features) {
    console.log(`| fixtures/${family}/${feature}/ | ${module === null ? "—" : `native/src/code_rule_dsl/${module}`} | ${group ?? "UNMAPPED"} |`);
  }
  console.log(`\n#3 fixture data outside fixtures/ and packs/*/upstream/: ${outside.length}`);
  for (const path of outside) console.log(`  ${path}`);
  console.log(`#10 feature directories naming no engine module or group: ${unmapped.length}`);
  for (const path of unmapped) console.log(`  ${path}`);
  process.exitCode = outside.length + unmapped.length === 0 ? 0 : 1;
}
