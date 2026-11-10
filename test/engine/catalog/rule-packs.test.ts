import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { afterEach, describe, expect, it } from "vitest";
import {
  createRerailApi,
  operations,
  rulePackListSchema,
  rulePackRecommendationsSchema,
  DatabaseApiStores,
} from "../../../src/api/index.js";
import { globPattern, scanRepository, signalEvidence } from "../../../src/engine/catalog/rule-pack-matcher.js";
import {
  cachedRecommendRulePacks,
  RECOMMENDATION_CACHE_MS,
  readRulePacks,
  recommendRulePacks,
  type RulePack,
} from "../../../src/engine/catalog/rule-packs.js";
import { loadPackRuleDocuments } from "../../../src/engine/catalog/rule-documents.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { MOCK_CONVERSATION_RULES } from "../../../src/cli/index.js";
import { installPack, removeInstalledPacks } from "../../support/installed-packs.js";
import { repositoryPath } from "../../support/oracles.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  for (const directory of temporaryDirectories.splice(0)) await rm(directory, { recursive: true, force: true });
});

async function temporaryDirectory(prefix: string): Promise<string> {
  const directory = await mkdtemp(join(tmpdir(), prefix));
  temporaryDirectories.push(directory);
  return directory;
}

function writeFiles(root: string, files: Record<string, string>): void {
  for (const [path, content] of Object.entries(files)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), content);
  }
}

const complexityDirectory = repositoryPath("packs", "complexity");
/** A code rule of this repository's complexity pack, which the synthetic packs below reuse under their own ids. */
const typescriptRule = loadPackRuleDocuments([complexityDirectory]).find(document => document.rule.id === "complexity-cyclomatic-typescript")!;

/**
 * A pack directory (`<root>/<id>/pack.json`, documents in `<id>/rules/`) written from a definition and its documents.
 */
function writePack(root: string, definition: { readonly id: string } & Readonly<Record<string, unknown>>, documents: Record<string, object>): string {
  const directory = join(root, definition.id);
  writeFiles(directory, {
    "pack.json": JSON.stringify(definition),
    ...Object.fromEntries(Object.entries(documents).map(([path, document]) => [path, JSON.stringify(document)])),
  });
  return directory;
}

/**
 * A TypeScript language pack, `lang-ts`, matched as a TypeScript pack is, and with `repository` a neutral repository
 * pack, `acme-api`; each has one example rule.
 */
async function withTestPacks(repository = false): Promise<readonly RulePack[]> {
  const root = await temporaryDirectory("rerail-test-packs-");
  const typescript = writePack(root, {
    format: "rerail.rule-pack/v1", id: "lang-ts", name: "TypeScript", category: "language",
    covers: "TypeScript sources.", reason: "matches TypeScript",
    match: {
      any: [
        { dependencies: { ecosystem: "npm", names: ["typescript"] } },
        { files: ["**/*.ts", "**/*.mts", "**/*.cts"], exclude: ["**/*.d.ts"], label: "*.ts files" },
        { files: ["**/*.tsx"], label: "*.tsx files" },
      ],
    },
    rules: [{ id: "lang-ts-cyclomatic", document: "rules/lang-ts-cyclomatic.json", title: "Keep functions simple", group: "TypeScript" }],
  }, { "rules/lang-ts-cyclomatic.json": { ...typescriptRule.rule, id: "lang-ts-cyclomatic" } });
  const acme = writePack(root, {
    format: "rerail.rule-pack/v1", id: "acme-api", name: "acme-api", category: "repository",
    covers: "The conventions of the acme API.", reason: "matches acme-api",
    match: { any: [{ files: ["api/acme_api/**"], label: "the acme_api package" }] },
    rules: [{ id: "acme-api-cyclomatic", document: "rules/acme-api-cyclomatic.json", title: "Keep functions simple", group: "api" }],
  }, { "rules/acme-api-cyclomatic.json": { ...typescriptRule.rule, id: "acme-api-cyclomatic" } });
  return readRulePacks([typescript, ...(repository ? [acme] : [])]);
}

describe("rule pack definitions", () => {
  const conversation = { ...MOCK_CONVERSATION_RULES[0]!, id: "talk" };
  const codeRule = { ...typescriptRule.rule, id: "cyclomatic" };
  async function packFolder(definitions: readonly { id: string; rules: readonly { id: string; document: string }[] }[]): Promise<string[]> {
    const directory = await temporaryDirectory("rerail-rule-pack-def-");
    const documents: Record<string, object> = { cyclomatic: codeRule, talk: conversation };
    return definitions.map(definition =>
      writePack(directory, definition, Object.fromEntries(definition.rules.flatMap(rule => (documents[rule.id] === undefined ? [] : [[rule.document, documents[rule.id]!]])))),
    );
  }
  const definition = (id: string, ruleId: string, rule: object = {}) => ({
    format: "rerail.rule-pack/v1", id, name: id, category: "language", covers: "c", reason: "r", match: { any: [{ files: ["**/*.ts"], label: "*.ts files" }] },
    rules: [{ id: ruleId, document: `rules/${ruleId}.json`, title: "t", group: "g", ...rule }],
  });

  it("reject a rule listed by two packs", async () => {
    const directories = await packFolder([definition("one", "cyclomatic"), definition("two", "cyclomatic")]);
    expect(() => readRulePacks(directories)).toThrow("Rule cyclomatic is in both rule packs one and two");
  });

  it("reject a rule whose document does not exist, or carries another id", async () => {
    const missing = await packFolder([definition("one", "missing")]);
    expect(() => readRulePacks(missing)).toThrow("Rule pack one lists missing, whose document rules/missing.json does not exist");
    const otherId = await packFolder([definition("one", "cyclomatic")]);
    writeFileSync(join(otherId[0]!, "rules", "cyclomatic.json"), JSON.stringify(conversation));
    expect(() => readRulePacks(otherId)).toThrow("Rule pack one lists cyclomatic, but rules/cyclomatic.json has id talk");
  });

  it("reject a rule id in another style, or a document not named for its id", async () => {
    const colon = await packFolder([definition("one", "cyclomatic")]);
    const listed = definition("one", "one:cyclomatic");
    writeFileSync(join(colon[0]!, "pack.json"), JSON.stringify({ ...listed, rules: [{ ...listed.rules[0]!, document: "rules/cyclomatic.json" }] }));
    expect(() => readRulePacks(colon)).toThrow("must be lowercase letters, digits and dashes");
    const filePack = definition("one", "cyclomatic");
    const misnamed = await packFolder([{ ...filePack, rules: [{ ...filePack.rules[0]!, document: "rules/talk.json" }] }]);
    expect(() => readRulePacks(misnamed)).toThrow("Rule pack one lists cyclomatic, whose document must be rules/cyclomatic.json, not rules/talk.json");
  });

  it("reject a conversation rule example that names a file", async () => {
    const examples = { caught: { path: "a.ts", text: "a" }, passes: { path: "a.ts", text: "b" } };
    const directories = await packFolder([definition("one", "talk", { examples })]);
    expect(() => readRulePacks(directories)).toThrow("Conversation rule talk in rule pack one takes no example path");
  });

  it("reject a pack whose globs do not compile, naming the pack, when the packs are read", async () => {
    const nested = { ...definition("one", "cyclomatic"), match: { any: [{ files: ["**/*.{ts,{js,jsx}}"], label: "code" }] } };
    const directories = await packFolder([nested]);
    expect(() => readRulePacks(directories)).toThrow("Rule pack one: Nested { in glob **/*.{ts,{js,jsx}}: braces do not nest");
  });

  it("list a rule in the mode its pack gives it, and otherwise in its document's mode", async () => {
    const directories = await packFolder([definition("one", "talk", { mode: "log", on: false }), definition("two", "cyclomatic")]);
    const loaded = readRulePacks(directories);
    expect(loaded.find(found => found.id === "one")!.rules[0]).toMatchObject({ kind: "conversation", mode: "log", on: false });
    expect(loaded.find(found => found.id === "two")!.rules[0]).toMatchObject({ kind: "code", mode: typescriptRule.rule.mode, on: true });
  });
});

describe("globPattern", () => {
  it.each([
    ["**/*.ts", "index.ts", true],
    ["**/*.ts", "web/src/index.ts", true],
    ["**/*.ts", "web/src/index.tsx", false],
    ["*.ts", "web/index.ts", false],
    ["**/*.{js,jsx}", "src/App.jsx", true],
    ["**/*.{js,jsx}", "src/App.json", false],
    ["src/**", "src/a/b.py", true],
    ["src/**", "lib/a.py", false],
    ["api/acme_api/**", "api/acme_api/routes/a.py", true],
    ["api/acme_api/**", "web/api/acme_api/a.py", false],
    ["**/*.config.{js,mjs}", "eslint.config.mjs", true],
    ["**/.*rc.{js,cjs}", "web/.eslintrc.cjs", true],
    ["**/a?c.ts", "x/abc.ts", true],
    ["**.ts", "a.ts", true],
    ["**.ts", "src/a.ts", false],
    ["{src/**/,}*.py", "src/a/b.py", true],
    ["{src/**/,}*.py", "b.py", true],
  ])("%s on %s is %s", (glob, path, expected) => {
    expect(globPattern([glob]).test(path)).toBe(expected);
  });
});

describe("the Python pack's match", () => {
  const pack = JSON.parse(readFileSync(fileURLToPath(new URL("../../../packs/python/pack.json", import.meta.url)), "utf8"));
  const [signal] = pack.match.any;
  const scan = (files: string[]) => ({ root: "/", files, dependencies: new Map() });

  it.each([
    ["src/app/models.py", true],
    ["app/views.py", true],
    ["stubs/pkg/__init__.pyi", true],
    ["tests/test_models.py", false],
    ["src/app/models_test.py", false],
    ["conftest.py", false],
    ["scripts/release.py", false],
    ["script/bootstrap.py", false],
    ["tools/gen.py", false],
    ["ci/check.py", false],
    [".github/actions/lint/main.py", false],
    ["setup.py", false],
    ["noxfile.py", false],
    ["docs/conf.py", false],
    ["docs/source/conf.py", false],
    ["doc/source/conf.py", false],
    ["docs/_ext/directive.py", false],
  ])("counts %s as Python source: %s", (path, expected) => {
    expect(signalEvidence(signal, scan([path]))).toEqual(expected ? [path] : []);
  });
});

describe("dependency manifests", () => {
  it("reads npm, PyPI and Composer dependencies, each named as its ecosystem compares names, and a manifest it cannot read names nothing", async () => {
    const root = await temporaryDirectory("rerail-manifests-");
    writeFiles(root, {
      "web/package.json": JSON.stringify({ devDependencies: { typescript: "^5.9.0" } }),
      "web/composer.json": JSON.stringify({ require: { "Laravel/Framework": "^11.0", php: "^8.3" }, "require-dev": { "pestphp/pest": "^3" } }),
      "api/requirements-dev.txt": "# tools\n-r requirements.txt\n--index-url https://example.invalid\nDjango_REST.framework>=3.15 ; python_version > '3.9'  # api\n\n",
      "api/pyproject.toml": "[project]\nname = \"api\"\ndependencies = [\n  \"FastAPI>=0.115\",  # web\n]\n\n[project.optional-dependencies]\ntest = [\"pytest\"]\n",
      "worker/pyproject.toml": "[tool.poetry.dependencies]\npython = \"^3.12\"\ncelery = { version = \"^5.4\", extras = [\"redis\"] }\n\n[tool.poetry.group.dev.dependencies]\n\"Ruff\" = \"*\"\n",
      // Poetry before 1.2 keeps development tools here.
      "legacy/pyproject.toml": "[tool.poetry.dependencies]\npython = \"^3.9\"\n\n[tool.poetry.dev-dependencies]\nDjango-Stubs = \"^4.2\"\n",
      "broken/pyproject.toml": "[project\ndependencies = [\"numpy\"]\n",
    });

    const { dependencies } = scanRepository(root);
    expect(Object.fromEntries(dependencies.get("pypi")!)).toEqual({
      "django-rest-framework": ["api/requirements-dev.txt"],
      fastapi: ["api/pyproject.toml"],
      pytest: ["api/pyproject.toml"],
      celery: ["worker/pyproject.toml"],
      ruff: ["worker/pyproject.toml"],
      "django-stubs": ["legacy/pyproject.toml"],
    });
    expect(Object.fromEntries(dependencies.get("composer")!)).toEqual({
      "laravel/framework": ["web/composer.json"], php: ["web/composer.json"], "pestphp/pest": ["web/composer.json"],
    });
    expect(Object.fromEntries(dependencies.get("npm")!)).toEqual({ typescript: ["web/package.json"] });
  });

  it("matches a dependency signal by the name its ecosystem compares, naming the manifest, at that manifest's folder", async () => {
    const root = await temporaryDirectory("rerail-pypi-match-");
    writeFiles(root, { "api/requirements.txt": "django-rest-framework==3.15.2\n", "api/app.py": "app = None\n" });
    const directory = writePack(await temporaryDirectory("rerail-pypi-pack-"), {
      format: "rerail.rule-pack/v1", id: "drf", name: "Django REST framework", category: "framework", covers: "c", reason: "r",
      match: { any: [{ dependencies: { ecosystem: "pypi", names: ["Django_REST_Framework"] } }] },
      rules: [{ id: "drf-cyclomatic", document: "rules/drf-cyclomatic.json", title: "t", group: "g" }],
    }, { "rules/drf-cyclomatic.json": { ...typescriptRule.rule, id: "drf-cyclomatic" } });

    expect(recommendRulePacks(root, readRulePacks([directory])).map(({ reason, folder }) => [reason, folder]))
      .toEqual([["matches the Django_REST_Framework dependency in api/requirements.txt", "api"]]);
  });
});

describe("the complexity pack's match", () => {
  const [complexity] = readRulePacks([complexityDirectory]);
  it.each([
    [{ "app/grade.py": "def grade(): pass\n" }, "matches *.py files in app", "app"],
    [{ "src/index.ts": "export {};\n", "lib/util.js": "module.exports = {};\n" }, "matches *.ts files in src and *.js files in lib", ""],
    [{ "app/Grade.php": "<?php\n", "resources/views/grade.blade.php": "<p></p>\n" }, "matches *.php files in app and *.blade.php files in resources/views", ""],
    [{ "crates/core/src/lib.rs": "pub fn f() {}\n" }, "matches *.rs files in crates/core/src", "crates/core/src"],
    [{ "web/src/App.vue": "<template></template>\n" }, "matches *.vue files in web/src", "web/src"],
  ])("fits Python, TypeScript, JavaScript, PHP, Blade, Rust and Vue source: %j", async (files, reason, folder) => {
    const root = await temporaryDirectory("rerail-complexity-match-");
    writeFiles(root, files);
    expect(recommendRulePacks(root, [complexity!]).map(found => [found.reason, found.folder])).toEqual([[reason, folder]]);
  });

  it("does not fit a repository whose only such files are tests", async () => {
    const root = await temporaryDirectory("rerail-complexity-tests-");
    writeFiles(root, {
      "tests/test_grade.py": "def test(): pass\n",
      "src/index.test.ts": "export {};\n",
      "tests/Feature/GradeTest.php": "<?php\n",
      "app/GradeTest.php": "<?php\n",
      "tests/views/grade.blade.php": "<p></p>\n",
      "tests/grade.rs": "fn f() {}\n",
      "src/__tests__/Grade.vue": "<template></template>\n",
      "main.go": "package main\n",
    });
    expect(recommendRulePacks(root, [complexity!])).toEqual([]);
  });
});

describe("recommendRulePacks", () => {
  const reasons = (root: string, from: readonly RulePack[]) =>
    recommendRulePacks(root, from).map(({ pack: found, ruleCount, reason, folder }) => [found.name, ruleCount, reason, folder]);

  it("picks a TypeScript pack for a TypeScript repository from its files and package.json, with the reason", async () => {
    const root = await temporaryDirectory("rerail-acme-web-");
    writeFiles(root, {
      "package.json": JSON.stringify({ name: "acme-web", dependencies: { react: "^19.0.0" }, devDependencies: { typescript: "^5.9.0" } }),
      "web/src/App.tsx": "export function App() { return null; }\n",
      "src/server.ts": "export const port = 3000;\n",
      "tests/fixtures/legacy.py": "print(1)\n",
      "node_modules/left-pad/index.ts": "export {};\n",
    });
    execFileSync("git", ["init", "-q", root]);

    expect(reasons(root, await withTestPacks())).toEqual([["TypeScript", 1, "matches the typescript dependency in package.json and *.ts files in src", ""]]);
  });

  it("picks a repository pack only for the repository it matches, after the language packs, for the whole repository", async () => {
    const root = await temporaryDirectory("rerail-acme-like-");
    writeFiles(root, {
      "api/acme_api/app.py": "app = None\n",
      "api/acme_api/routes/users.py": "router = None\n",
      "web/src/main.tsx": "export {};\n",
    });
    const withAcme = await withTestPacks(true);

    // A language pack goes where its evidence is; a repository pack covers the whole repository.
    expect(reasons(root, withAcme)).toEqual([
      ["TypeScript", 1, "matches *.tsx files in web/src", "web/src"],
      ["acme-api", 1, "matches the acme_api package in api/acme_api", ""],
    ]);
    expect(reasons(root, await withTestPacks())).toEqual([["TypeScript", 1, "matches *.tsx files in web/src", "web/src"]]);
  });

  it("counts only the rules a pack starts on, and never recommends a pack whose rules all start off", async () => {
    const root = await temporaryDirectory("rerail-rules-off-");
    writeFiles(root, { "src/server.ts": "export const port = 3000;\n" });
    const directory = await temporaryDirectory("rerail-rules-off-packs-");
    const rule = (id: string, on: boolean) => ({ id, document: `rules/${id}.json`, title: "Keep functions simple", group: "TypeScript", on });
    const definition = (id: string, rules: ReturnType<typeof rule>[]) =>
      writePack(directory, {
        format: "rerail.rule-pack/v1", id, name: id, category: "language", covers: "c", reason: "r",
        match: { any: [{ files: ["**/*.ts"], label: "*.ts files" }] }, rules,
      }, Object.fromEntries(rules.map(listed => [listed.document, { ...typescriptRule.rule, id: listed.id }])));
    const mixed = definition("mixed", [rule("mixed-on", true), rule("mixed-off", false)]);
    const off = definition("off", [rule("off-one", false), rule("off-two", false)]);

    expect(reasons(root, readRulePacks([mixed, off]))).toEqual([["mixed", 1, "matches *.ts files in src", "src"]]);
  });

  it("places a pack at the folder holding every manifest that matched, naming the shallowest in the reason", async () => {
    const root = await temporaryDirectory("rerail-two-manifests-");
    const typescriptManifest = JSON.stringify({ devDependencies: { typescript: "^5.9.0" } });
    writeFiles(root, {
      "packages/web/admin/package.json": typescriptManifest,
      "packages/web/package.json": typescriptManifest,
      "packages/web/README.md": "web\n",
      "api/app.py": "print(1)\n",
    });

    const from = await withTestPacks();
    expect(reasons(root, from)).toEqual([["TypeScript", 1, "matches the typescript dependency in packages/web/package.json", "packages/web"]]);
    writeFiles(root, { "tools/build.ts": "export {};\n" });
    expect(recommendRulePacks(root, from).map(({ folder }) => folder)).toEqual([""]);
  });

  it("recommends nothing for a stack no matching pack covers, outside Git and ignoring dependency folders", async () => {
    const root = await temporaryDirectory("rerail-python-");
    writeFiles(root, { "app/main.py": "print(1)\n", "pyproject.toml": "[project]\n", ".venv/lib/site.ts": "export {};\n", "go.mod": "module shop\n" });
    expect(recommendRulePacks(root, await withTestPacks())).toEqual([]);
  });

  it("ignores tracked files deleted from the worktree", async () => {
    const root = await temporaryDirectory("rerail-deleted-ts-");
    writeFiles(root, { "app/main.py": "print(1)\n", "legacy/index.ts": "export {};\n" });
    execFileSync("git", ["init", "-q", root]);
    execFileSync("git", ["-C", root, "add", "."]);
    rmSync(join(root, "legacy/index.ts"));

    expect(recommendRulePacks(root, await withTestPacks())).toEqual([]);
  });

  it("ignores tracked files a sparse checkout leaves off disk, and counts them once the checkout includes them", async () => {
    const root = await temporaryDirectory("rerail-sparse-");
    writeFiles(root, { "README.md": "hi\n", "api/acme_api/app.py": "app = None\n", "web/src/main.ts": "export {};\n" });
    const withAcme = await withTestPacks(true);
    const git = (...args: string[]) => execFileSync("git", ["-C", root, ...args], { stdio: "pipe" });
    git("init", "-q");
    git("add", ".");
    git("-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "-m", "base");
    git("sparse-checkout", "set", "--no-cone", "/README.md");

    expect(recommendRulePacks(root, withAcme)).toEqual([]);
    git("sparse-checkout", "set", "--no-cone", "/README.md", "/api/");
    expect(recommendRulePacks(root, withAcme).map(({ pack: found }) => found.id)).toEqual(["acme-api"]);
  });

  it("never recommends a pack without rules, even where its match fits", async () => {
    const root = await temporaryDirectory("rerail-empty-pack-");
    writeFiles(root, { "web/src/App.jsx": "export const App = () => null;\n" });
    const empty = writePack(await temporaryDirectory("rerail-empty-pack-def-"), {
      format: "rerail.rule-pack/v1", id: "acme-jsx", name: "acme-jsx", category: "framework",
      covers: "No rules yet: every port is a candidate.", reason: "matches JSX",
      match: { any: [{ files: ["**/*.jsx"], label: "*.jsx files" }] },
      rules: [],
    }, {});
    const loaded = readRulePacks([empty]);

    expect(loaded.map(found => [found.id, found.rules.length])).toEqual([["acme-jsx", 0]]);
    expect(recommendRulePacks(root, loaded)).toEqual([]);
  });

  it("lets the daemon reuse a repository's recommendations for a minute, then scans it again", async () => {
    const root = await temporaryDirectory("rerail-recommend-cache-");
    writeFiles(root, { "app/main.py": "print(1)\n" });
    const first = cachedRecommendRulePacks(root, 0);
    expect(cachedRecommendRulePacks(root, RECOMMENDATION_CACHE_MS - 1)).toBe(first);
    const rescanned = cachedRecommendRulePacks(root, RECOMMENDATION_CACHE_MS);
    expect(rescanned).not.toBe(first);
    expect(cachedRecommendRulePacks(root, 2 * RECOMMENDATION_CACHE_MS - 1)).toBe(rescanned);
  });

  it("scans again within the minute once a pack is installed", async () => {
    const root = await temporaryDirectory("rerail-recommend-installed-");
    writeFiles(root, { "app/main.ts": "export const one = 1;\n" });
    try {
      expect(cachedRecommendRulePacks(root, 0)).toEqual([]);
      installPack("typescript");
      expect(cachedRecommendRulePacks(root, 1).map(found => found.pack.id)).toEqual(["typescript"]);
    } finally {
      removeInstalledPacks();
    }
  });

  it("reads no package.json that is a symlink, outside the repository or to a device, and none larger than 1 MiB", async () => {
    const parent = await temporaryDirectory("rerail-manifest-guard-");
    const typescriptManifest = JSON.stringify({ devDependencies: { typescript: "5.9.0" } });
    writeFiles(parent, { "outside/package.json": typescriptManifest });
    const root = join(parent, "repo");
    writeFiles(root, {
      "app/main.py": "print(1)\n",
      // Valid JSON that names typescript, padded past the cap.
      "big/package.json": `${typescriptManifest.slice(0, -1)}, "padding": "${"x".repeat(1024 * 1024)}"}`,
    });
    mkdirSync(join(root, "linked"));
    mkdirSync(join(root, "device"));
    symlinkSync(join(parent, "outside/package.json"), join(root, "linked/package.json"));
    symlinkSync("/dev/zero", join(root, "device/package.json"));
    execFileSync("git", ["init", "-q", root]);
    execFileSync("git", ["-C", root, "add", "."]);

    const from = await withTestPacks();
    expect(recommendRulePacks(root, from)).toEqual([]);
    writeFileSync(join(root, "package.json"), typescriptManifest);
    expect(recommendRulePacks(root, from).map(({ pack: found, reason }) => [found.id, reason])).toEqual([["lang-ts", "matches the typescript dependency in package.json"]]);
  });
});

describe("rule pack API", () => {
  it("lists the installed packs with their versions, recommends an installed TypeScript pack for a TypeScript checkout, and tags their rules with their pack", async () => {
    const root = realpathSync(await temporaryDirectory("rerail-rule-pack-api-"));
    const checkout = join(root, "web-app");
    writeFiles(checkout, { "src/main.ts": "export {};\n" });
    execFileSync("git", ["init", "-q", checkout]);
    const databasePath = join(root, "rerail.sqlite");
    const database = openRerailDatabase(databasePath);
    let repositoryKey: string;
    try {
      repositoryKey = recordCheckout(database, checkout).repositoryKey;
    } finally {
      database.$client.close();
    }
    const stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    const rerailRules = z.object({ rules: z.array(z.object({ rule: z.object({ id: z.string() }), packId: z.string().nullable() })) });
    const get = async <T>(url: string, schema: z.ZodType<T>): Promise<T> => schema.parse(await (await app.request(url)).json());
    const recommendations = `/api/rule-packs/recommendations?checkout=${encodeURIComponent(checkout)}`;
    try {
      // Core ships no packs: before any is installed there is nothing to list, recommend or run.
      expect(await get("/api/rule-packs", rulePackListSchema)).toEqual({ packs: [], skipped: [] });
      expect((await get(recommendations, rulePackRecommendationsSchema)).recommendations).toEqual([]);
      expect((await get("/api/rerail-rules", rerailRules)).rules).toEqual([]);

      installPack("typescript");
      installPack("complexity");
      const [typescript, complexity] = ["typescript", "complexity"].map(id => readRulePacks([repositoryPath("packs", id)])[0]!);
      const version = (id: string): string => (JSON.parse(readFileSync(repositoryPath("packs", id, "package.json"), "utf8")) as { version: string }).version;

      const listed = await get("/api/rule-packs", rulePackListSchema);
      expect(listed.skipped).toEqual([]);
      expect(listed.packs.map(found => [found.id, found.version, found.rules.length]))
        .toEqual([["complexity", version("complexity"), complexity!.rules.length], ["typescript", version("typescript"), typescript!.rules.length]]);
      const cyclomatic = complexity!.rules.find(rule => rule.id === "complexity-cyclomatic-typescript")!;
      expect(listed.packs[0]!.rules.find(rule => rule.id === "complexity-cyclomatic-typescript"))
        .toEqual({ id: "complexity-cyclomatic-typescript", kind: "code", title: cyclomatic.title, group: "Complexity", mode: "steer", on: true, examples: cyclomatic.examples });

      expect(await get(recommendations, rulePackRecommendationsSchema)).toEqual({
        repositoryKey, checkoutPath: checkout,
        recommendations: [
          { packId: "complexity", ruleCount: complexity!.rules.filter(rule => rule.on).length, reason: "matches *.ts files in src", folder: "src" },
          { packId: "typescript", ruleCount: typescript!.rules.filter(rule => rule.on).length, reason: "matches *.ts files in src", folder: "src" },
        ],
      });

      const { rules } = await get("/api/rerail-rules", rerailRules);
      const packOf = Object.fromEntries(rules.map(listedRule => [listedRule.rule.id, listedRule.packId]));
      expect(packOf["complexity-cyclomatic-typescript"]).toBe("complexity");
      expect(packOf[typescript!.rules[0]!.id]).toBe("typescript");
    } finally {
      stores.close();
    }
  });
});
