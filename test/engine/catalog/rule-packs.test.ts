import { execFileSync } from "node:child_process";
import { copyFileSync, cpSync, mkdirSync, readFileSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
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
import { languageFor } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import { loadNativeBinding } from "../../../src/engine/native/native.js";
import { globPattern } from "../../../src/engine/catalog/rule-pack-matcher.js";
import {
  cachedRecommendRulePacks,
  loadRulePacks,
  RECOMMENDATION_CACHE_MS,
  readRulePacks,
  recommendRulePacks,
  type RulePack,
} from "../../../src/engine/catalog/rule-packs.js";
import { corePackDirectories, loadBundledRules } from "../../../src/engine/catalog/bundled-rule-documents.js";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import { MOCK_CONVERSATION_RULES } from "../../../src/cli/index.js";

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

const packs = loadRulePacks();
const bundled = loadBundledRules();
const pack = (id: string): RulePack => packs.find(candidate => candidate.id === id)!;
const ids = (found: RulePack) => found.rules.map(rule => rule.id).sort();
const complexity = pack("complexity");
const complexityIds = ["complexity:cyclomatic-javascript", "complexity:cyclomatic-python", "complexity:cyclomatic-typescript"];
/** A shipped file rule the synthetic packs below reuse under their own ids. */
const typescriptRule = bundled.find(document => document.rule.id === "complexity:cyclomatic-typescript")!;

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
 * The shipped packs plus a TypeScript language pack, `lang-ts`, matched as a TypeScript pack is, and with
 * `repository` a neutral repository pack, `acme-api`; each has one example rule. No pack core ships has a match
 * definition, so the recommendation tests bring their own.
 */
async function withTestPacks(repository = false): Promise<readonly RulePack[]> {
  const root = await temporaryDirectory("rerail-test-packs-");
  const typescript = writePack(root, {
    format: "rerail.rule-pack/v1", id: "lang-ts", name: "TypeScript", category: "language",
    covers: "TypeScript sources.", reason: "matches TypeScript",
    match: {
      any: [
        { npmDependencies: ["typescript"] },
        { files: ["**/*.ts", "**/*.mts", "**/*.cts"], exclude: ["**/*.d.ts"], label: "*.ts files" },
        { files: ["**/*.tsx"], label: "*.tsx files" },
      ],
    },
    rules: [{ id: "lang-ts:cyclomatic", document: "rules/cyclomatic.json", title: "Keep functions simple", group: "TypeScript" }],
  }, { "rules/cyclomatic.json": { ...typescriptRule.rule, id: "lang-ts:cyclomatic" } });
  const acme = writePack(root, {
    format: "rerail.rule-pack/v1", id: "acme-api", name: "acme-api", category: "repository",
    covers: "The conventions of the acme API.", reason: "matches acme-api",
    match: { any: [{ files: ["api/acme_api/**"], label: "the acme_api package" }] },
    rules: [{ id: "acme-api:cyclomatic", document: "rules/cyclomatic.json", title: "Keep functions simple", group: "api" }],
  }, { "rules/cyclomatic.json": { ...typescriptRule.rule, id: "acme-api:cyclomatic" } });
  return readRulePacks([...corePackDirectories(), typescript, ...(repository ? [acme] : [])]);
}

const capabilities = readFileSync(fileURLToPath(new URL("../../../native/src/code_rule_dsl/capabilities.json", import.meta.url)), "utf8");
const evaluationSchema = z.object({ violations: z.array(z.object({ ruleId: z.string() })).default([]), failures: z.array(z.unknown()).default([]) });

/** Whether the file rule `ruleId`, evaluated on `text` at `path`, reports a violation. */
function fires(ruleId: string, path: string, text: string): boolean {
  const document = bundled.find(candidate => candidate.rule.id === ruleId)!;
  const compiled = loadNativeBinding().CodeRuleDslRuntime!.compile(JSON.stringify(document.rule), "json", capabilities);
  expect(compiled.inScope(path), `${ruleId} example ${path} is outside the rule's scope`).toBe(true);
  const input = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: [{ language: languageFor(path) ?? "text", path, source: text }] });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations.some(violation => violation.ruleId === ruleId);
}

describe("shipped rule packs", () => {
  it("are the complexity pack alone, with no match definition, so none is ever recommended", () => {
    expect(packs.map(candidate => candidate.id)).toEqual(["complexity"]);
    expect(ids(complexity)).toEqual(complexityIds);
    expect(bundled.map(document => document.rule.id).sort()).toEqual(complexityIds);
    expect(complexity.match).toBeUndefined();
  });

  it("start every rule in the mode its own document has", () => {
    for (const rule of packs.flatMap(found => found.rules)) {
      expect(rule.mode, rule.id).toBe(bundled.find(document => document.rule.id === rule.id)!.rule.mode);
    }
  });

  const exampled = packs.flatMap(found => found.rules).filter(rule => rule.examples !== undefined && rule.kind === "file");
  it.each(exampled.map(rule => [rule.id, rule] as const))("%s catches its caught example and lets its passing example pass", (id, rule) => {
    const { caught, passes } = rule.examples!;
    expect(fires(id, caught.path!, caught.text)).toBe(true);
    expect(fires(id, passes.path!, passes.text)).toBe(false);
  });

  it("are the packs core ships, whose rules are exactly the documents seeded disabled", () => {
    expect(corePackDirectories()).toEqual([fileURLToPath(new URL("../../../packs/complexity", import.meta.url))]);
    expect(packs.flatMap(found => found.rules).every(rule => bundled.some(document => document.rule.id === rule.id))).toBe(true);
  });
});

describe("rule pack definitions", () => {
  const conversation = { ...MOCK_CONVERSATION_RULES[0]!, id: "test:talk" };
  const fileRule = { ...typescriptRule.rule, id: "test:cyclomatic" };
  async function packFolder(definitions: readonly { id: string; rules: readonly { id: string; document: string }[] }[]): Promise<string[]> {
    const directory = await temporaryDirectory("rerail-rule-pack-def-");
    const documents: Record<string, object> = { "test:cyclomatic": fileRule, "test:talk": conversation };
    return definitions.map(definition =>
      writePack(directory, definition, Object.fromEntries(definition.rules.flatMap(rule => (documents[rule.id] === undefined ? [] : [[rule.document, documents[rule.id]!]])))),
    );
  }
  const definition = (id: string, ruleId: string, rule: object = {}) => ({
    format: "rerail.rule-pack/v1", id, name: id, category: "language", covers: "c", reason: "r", match: { any: [{ files: ["**/*.ts"], label: "*.ts files" }] },
    rules: [{ id: ruleId, document: `rules/${ruleId.replace(/^test:/u, "")}.json`, title: "t", group: "g", ...rule }],
  });

  it("reject a rule listed by two packs", async () => {
    const directories = await packFolder([definition("one", "test:cyclomatic"), definition("two", "test:cyclomatic")]);
    expect(() => readRulePacks(directories)).toThrow("Rule test:cyclomatic is in both rule packs one and two");
  });

  it("reject a rule whose document does not exist, or carries another id", async () => {
    const missing = await packFolder([definition("one", "test:missing")]);
    expect(() => readRulePacks(missing)).toThrow("Rule pack one lists test:missing, whose document rules/missing.json does not exist");
    const filePack = definition("one", "test:cyclomatic");
    const misnamed = await packFolder([{ ...filePack, rules: [{ ...filePack.rules[0]!, document: "rules/talk.json" }] }]);
    writeFileSync(join(misnamed[0]!, "rules", "talk.json"), JSON.stringify(conversation));
    expect(() => readRulePacks(misnamed)).toThrow("Rule pack one lists test:cyclomatic, but rules/talk.json has id test:talk");
  });

  it("reject a conversation rule example that names a file", async () => {
    const examples = { caught: { path: "a.ts", text: "a" }, passes: { path: "a.ts", text: "b" } };
    const directories = await packFolder([definition("one", "test:talk", { examples })]);
    expect(() => readRulePacks(directories)).toThrow("Conversation rule test:talk in rule pack one takes no example path");
  });

  it("reject a pack whose globs do not compile, naming the pack, when the packs are read", async () => {
    const nested = { ...definition("one", "test:cyclomatic"), match: { any: [{ files: ["**/*.{ts,{js,jsx}}"], label: "code" }] } };
    const directories = await packFolder([nested]);
    expect(() => readRulePacks(directories)).toThrow("Rule pack one: Nested { in glob **/*.{ts,{js,jsx}}: braces do not nest");
  });

  it("list a rule in the mode its pack gives it, and otherwise in its document's mode", async () => {
    const directories = await packFolder([definition("one", "test:talk", { mode: "log", on: false }), definition("two", "test:cyclomatic")]);
    const loaded = readRulePacks(directories);
    expect(loaded.find(found => found.id === "one")!.rules[0]).toMatchObject({ kind: "conversation", mode: "log", on: false });
    expect(loaded.find(found => found.id === "two")!.rules[0]).toMatchObject({ kind: "file", mode: typescriptRule.rule.mode, on: true });
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
    // Of the packs core ships, none fits even a TypeScript repository.
    expect(reasons(root, packs)).toEqual([]);
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

  it("recommends nothing for a stack no matching pack covers, not even the hand-picked complexity pack, outside Git and ignoring dependency folders", async () => {
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
  it("lists the packs, recommends none of them for a TypeScript checkout, and tags By Rerail rules with their pack", async () => {
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
    try {
      const listed = await get("/api/rule-packs", rulePackListSchema);
      expect(listed.packs.map(found => [found.id, found.category, found.rules.length]))
        .toEqual([["complexity", "language", complexityIds.length]]);
      const cyclomatic = complexity.rules.find(rule => rule.id === "complexity:cyclomatic-typescript")!;
      expect(listed.packs.find(found => found.id === "complexity")!.rules.find(rule => rule.id === "complexity:cyclomatic-typescript"))
        .toEqual({ id: "complexity:cyclomatic-typescript", kind: "file", title: cyclomatic.title, group: "Complexity", mode: "steer", on: true, examples: cyclomatic.examples });
      expect(cyclomatic.examples).toBeDefined();

      const recommended = await get(`/api/rule-packs/recommendations?checkout=${encodeURIComponent(checkout)}`, rulePackRecommendationsSchema);
      expect(recommended).toEqual({
        repositoryKey, checkoutPath: checkout,
        recommendations: [],
      });

      const { rules } = await get("/api/rerail-rules", rerailRules);
      const packOf = Object.fromEntries(rules.map(listedRule => [listedRule.rule.id, listedRule.packId]));
      expect(packOf["complexity:cyclomatic-typescript"]).toBe("complexity");
    } finally {
      stores.close();
    }
  });
});
