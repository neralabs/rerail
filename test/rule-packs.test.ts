import { execFileSync } from "node:child_process";
import { copyFileSync, cpSync, mkdirSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { z } from "zod";
import { afterEach, describe, expect, it } from "vitest";
import { createRerailApi } from "../src/api/http.js";
import { operations, type RerailApiStores } from "../src/api/operations.js";
import { rulePackListSchema, rulePackRecommendationsSchema } from "../src/api/rule-pack-contracts.js";
import { globPattern } from "../src/rule-pack-matcher.js";
import {
  cachedRecommendRulePacks,
  loadRulePacks,
  RECOMMENDATION_CACHE_MS,
  readRulePacks,
  recommendRulePacks,
  rulePacksDirectory,
  type RulePack,
} from "../src/rule-packs.js";
import { bundledRulesDirectory, loadBundledRules } from "../src/rule-bootstrap.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import { MOCK_COSINE_RULES } from "../src/seed.js";

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
const tslint = pack("tslint");
const optInIds = bundled.filter(document => document.enabledByDefault === false).map(document => document.rule.id).sort();

/**
 * The shipped rules and packs plus a neutral repository pack, `acme-api`, of one example rule: core ships no
 * repository pack, so the repository-pack tests bring their own.
 */
async function withRepositoryPack(): Promise<readonly RulePack[]> {
  const root = await temporaryDirectory("rerail-repository-pack-");
  const rules = join(root, "rules");
  const packFolder = join(root, "packs");
  cpSync(bundledRulesDirectory(), rules, { recursive: true });
  const debuggerRule = bundled.find(document => document.rule.id === "tslint:no-debugger")!;
  writeFileSync(join(rules, "acme-no-debugger.json"), JSON.stringify({ ...debuggerRule.rule, id: "acme-api:no-debugger" }));
  mkdirSync(packFolder);
  copyFileSync(join(rulePacksDirectory(), "tslint.json"), join(packFolder, "tslint.json"));
  writeFileSync(join(packFolder, "acme-api.json"), JSON.stringify({
    format: "rerail.rule-pack/v1", id: "acme-api", name: "acme-api", category: "repository",
    covers: "The conventions of the acme API.", reason: "matches acme-api",
    match: { any: [{ files: ["api/acme_api/**"], label: "the acme_api package" }] },
    rules: [{ id: "acme-api:no-debugger", title: "No debugger statements", group: "api" }],
  }));
  return readRulePacks(packFolder, rules);
}

describe("shipped rule packs", () => {
  it("are the TSLint pack of the golden TSLint ports, and nothing else", () => {
    expect(packs.map(candidate => candidate.id)).toEqual(["tslint"]);
    expect(ids(tslint)).toEqual(optInIds);
    expect(bundled.map(document => document.rule.id).sort()).toEqual(optInIds);
    expect(optInIds.every(id => id.startsWith("tslint:"))).toBe(true);
  });

  it("start every rule in the mode its own document has", () => {
    for (const rule of packs.flatMap(found => found.rules)) {
      expect(rule.mode, rule.id).toBe(bundled.find(document => document.rule.id === rule.id)!.rule.mode);
    }
  });

  it("are read from the shipped folder, apart from the bundled rule documents, and add no rule documents of their own", () => {
    expect(rulePacksDirectory()).not.toBe(bundledRulesDirectory());
    expect(packs.flatMap(found => found.rules).every(rule => bundled.some(document => document.rule.id === rule.id))).toBe(true);
  });
});

describe("rule pack definitions", () => {
  const conversation = { ...MOCK_COSINE_RULES[0]!, id: "test:talk" };
  const debuggerRule = bundled.find(document => document.rule.id === "tslint:no-debugger")!;
  const fileRule = { ...debuggerRule.rule, id: "test:no-debugger" };
  async function packFolder(definitions: readonly { id: string }[]): Promise<{ directory: string; bundledDirectory: string }> {
    const directory = await temporaryDirectory("rerail-rule-pack-def-");
    const bundledDirectory = join(directory, "bundled");
    mkdirSync(bundledDirectory);
    writeFileSync(join(bundledDirectory, "a.json"), JSON.stringify(fileRule));
    writeFileSync(join(bundledDirectory, "b.json"), JSON.stringify(conversation));
    for (const definition of definitions) writeFileSync(join(directory, `${definition.id}.json`), JSON.stringify(definition));
    return { directory, bundledDirectory };
  }
  const definition = (id: string, ruleId: string, rule: object = {}) => ({
    format: "rerail.rule-pack/v1", id, name: id, category: "language", covers: "c", reason: "r", match: { any: [{ files: ["**/*.ts"], label: "*.ts files" }] },
    rules: [{ id: ruleId, title: "t", group: "g", ...rule }],
  });

  it("reject a rule listed by two packs", async () => {
    const { directory, bundledDirectory } = await packFolder([definition("one", "test:no-debugger"), definition("two", "test:no-debugger")]);
    expect(() => readRulePacks(directory, bundledDirectory)).toThrow("Rule test:no-debugger is in both rule packs one and two");
  });

  it("reject a rule id that resolves to no bundled rule", async () => {
    const { directory, bundledDirectory } = await packFolder([definition("one", "test:missing")]);
    expect(() => readRulePacks(directory, bundledDirectory)).toThrow("Rule pack one lists test:missing, which is not a bundled rule");
  });

  it("reject a conversation rule example that names a file", async () => {
    const examples = { caught: { path: "a.ts", text: "a" }, passes: { path: "a.ts", text: "b" } };
    const { directory, bundledDirectory } = await packFolder([definition("one", "test:talk", { examples })]);
    expect(() => readRulePacks(directory, bundledDirectory)).toThrow("Conversation rule test:talk in rule pack one takes no example path");
  });

  it("reject a pack whose globs do not compile, naming the pack, when the packs are read", async () => {
    const nested = { ...definition("one", "test:no-debugger"), match: { any: [{ files: ["**/*.{ts,{js,jsx}}"], label: "code" }] } };
    const { directory, bundledDirectory } = await packFolder([nested]);
    expect(() => readRulePacks(directory, bundledDirectory)).toThrow("Rule pack one: Nested { in glob **/*.{ts,{js,jsx}}: braces do not nest");
  });

  it("list a rule in the mode its pack gives it, and otherwise in its document's mode", async () => {
    const { directory, bundledDirectory } = await packFolder([definition("one", "test:talk", { mode: "log", on: false }), definition("two", "test:no-debugger")]);
    const loaded = readRulePacks(directory, bundledDirectory);
    expect(loaded.find(found => found.id === "one")!.rules[0]).toMatchObject({ kind: "conversation", mode: "log", on: false });
    expect(loaded.find(found => found.id === "two")!.rules[0]).toMatchObject({ kind: "file", mode: debuggerRule.rule.mode, on: true });
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
  const reasons = (root: string, from: readonly RulePack[] = packs) =>
    recommendRulePacks(root, from).map(({ pack: found, ruleCount, reason }) => [found.name, ruleCount, reason]);

  it("picks TSLint for a TypeScript repository from its files and package.json, with the reason", async () => {
    const root = await temporaryDirectory("rerail-acme-web-");
    writeFiles(root, {
      "package.json": JSON.stringify({ name: "acme-web", dependencies: { react: "^19.0.0" }, devDependencies: { typescript: "^5.9.0" } }),
      "web/src/App.tsx": "export function App() { return null; }\n",
      "src/server.ts": "export const port = 3000;\n",
      "tests/fixtures/legacy.py": "print(1)\n",
      "node_modules/left-pad/index.ts": "export {};\n",
    });
    execFileSync("git", ["init", "-q", root]);

    expect(reasons(root)).toEqual([["TSLint", tslint.rules.length, "matches the typescript dependency in package.json and *.ts files in src"]]);
  });

  it("picks a repository pack only for the repository it matches, after the language packs", async () => {
    const root = await temporaryDirectory("rerail-acme-like-");
    writeFiles(root, {
      "api/acme_api/app.py": "app = None\n",
      "api/acme_api/routes/users.py": "router = None\n",
      "web/src/main.tsx": "export {};\n",
    });
    const withAcme = await withRepositoryPack();

    expect(reasons(root, withAcme)).toEqual([
      ["TSLint", tslint.rules.length, "matches *.tsx files in web/src"],
      ["acme-api", 1, "matches the acme_api package in api/acme_api"],
    ]);
    expect(reasons(root)).toEqual([["TSLint", tslint.rules.length, "matches *.tsx files in web/src"]]);
  });

  it("recommends nothing for a stack no pack covers, outside Git and ignoring dependency folders", async () => {
    const root = await temporaryDirectory("rerail-python-");
    writeFiles(root, { "app/main.py": "print(1)\n", "pyproject.toml": "[project]\n", ".venv/lib/site.ts": "export {};\n", "go.mod": "module shop\n" });
    expect(recommendRulePacks(root)).toEqual([]);
  });

  it("ignores tracked files deleted from the worktree", async () => {
    const root = await temporaryDirectory("rerail-deleted-ts-");
    writeFiles(root, { "app/main.py": "print(1)\n", "legacy/index.ts": "export {};\n" });
    execFileSync("git", ["init", "-q", root]);
    execFileSync("git", ["-C", root, "add", "."]);
    rmSync(join(root, "legacy/index.ts"));

    expect(recommendRulePacks(root)).toEqual([]);
  });

  it("ignores tracked files a sparse checkout leaves off disk, and counts them once the checkout includes them", async () => {
    const root = await temporaryDirectory("rerail-sparse-");
    writeFiles(root, { "README.md": "hi\n", "api/acme_api/app.py": "app = None\n", "web/src/main.ts": "export {};\n" });
    const withAcme = await withRepositoryPack();
    const git = (...args: string[]) => execFileSync("git", ["-C", root, ...args], { stdio: "pipe" });
    git("init", "-q");
    git("add", ".");
    git("-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", "commit", "-q", "-m", "base");
    git("sparse-checkout", "set", "--no-cone", "/README.md");

    expect(recommendRulePacks(root, withAcme)).toEqual([]);
    git("sparse-checkout", "set", "--no-cone", "/README.md", "/api/");
    expect(recommendRulePacks(root, withAcme).map(({ pack: found }) => found.id)).toEqual(["acme-api"]);
  });

  it("lets the daemon reuse a repository's recommendations for a minute, then scans it again", async () => {
    const root = await temporaryDirectory("rerail-recommend-cache-");
    writeFiles(root, { "app/main.py": "print(1)\n" });
    const ids = (now: number) => cachedRecommendRulePacks(root, now).map(({ pack: found }) => found.id);
    expect(ids(0)).toEqual([]);
    writeFiles(root, { "src/main.ts": "export {};\n" });
    expect(ids(RECOMMENDATION_CACHE_MS - 1)).toEqual([]);
    expect(ids(RECOMMENDATION_CACHE_MS)).toEqual(["tslint"]);
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

    expect(recommendRulePacks(root)).toEqual([]);
    writeFileSync(join(root, "package.json"), typescriptManifest);
    expect(recommendRulePacks(root).map(({ pack: found, reason }) => [found.id, reason])).toEqual([["tslint", "matches the typescript dependency in package.json"]]);
  });
});

describe("rule pack API", () => {
  it("lists the packs, recommends packs for the daemon's repository, and tags listed rules with their pack", async () => {
    const root = await temporaryDirectory("rerail-rule-pack-api-");
    writeFiles(root, { "src/main.ts": "export {};\n" });
    const catalog = ScopedRuleCatalog.open(join(root, "rerail.sqlite"), { repositoryId: "repo-a", workspaceRoot: root });
    catalog.seed(bundled.filter(document => document.rule.id === "tslint:triple-equals"));
    catalog.upsertCosine({ ...MOCK_COSINE_RULES[0]!, id: "ours" });
    const app = createRerailApi({ rules: () => catalog } as unknown as RerailApiStores, operations, { version: "0.0.0-test" });
    const listedRules = z.object({ rules: z.array(z.object({ rule: z.object({ id: z.string() }), packId: z.string().nullable() })) });
    const get = async <T>(url: string, schema: z.ZodType<T>): Promise<T> => schema.parse(await (await app.request(url)).json());
    try {
      const listed = await get("/api/rule-packs", rulePackListSchema);
      expect(listed.packs.map(found => [found.id, found.category, found.rules.length]))
        .toEqual([["tslint", "language", tslint.rules.length]]);
      expect(listed.packs[0]!.rules.find(rule => rule.id === "tslint:triple-equals"))
        .toEqual({ id: "tslint:triple-equals", kind: "file", title: tslint.rules.find(rule => rule.id === "tslint:triple-equals")!.title, group: "TSLint", mode: "steer", on: true });

      const recommended = await get("/api/rule-packs/recommendations", rulePackRecommendationsSchema);
      expect(recommended).toEqual({ repositoryId: "repo-a", recommendations: [{ packId: "tslint", ruleCount: tslint.rules.length, reason: "matches *.ts files in src" }] });

      const { rules } = await get("/api/rules", listedRules);
      expect(Object.fromEntries(rules.map(stored => [stored.rule.id, stored.packId])))
        .toEqual({ "tslint:triple-equals": "tslint", ours: null });
    } finally {
      catalog.close();
    }
  });
});
