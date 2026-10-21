import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createRerailApi, DatabaseApiStores, operations } from "../../src/api/index.js";
import { type RuleCatalog, PACKAGE_ROOT, readRuleCatalog, recordCheckout, ruleSettings, setUpRepository } from "../../src/engine/index.js";
import { toStoredRules } from "../../ui/poll.js";
import { isRunning, onText, optionsText } from "../../ui/derive.js";
import { data, matches, runningCount, summary, ui } from "../../ui/state.js";

const RULE = "typescript-eslint-consistent-type-definitions";
const typescriptPack = readRuleCatalog([join(PACKAGE_ROOT, "packs", "typescript")]);

/** The API over a machine whose rule catalog is the TypeScript pack, whose rules declare options. */
class TypeScriptPackStores extends DatabaseApiStores {
  override ruleCatalog(): RuleCatalog {
    return typescriptPack;
  }
}

type RulesBody = { rules: Array<{ rule: { id: string }; on: boolean; configError: string | null }> };

describe("a rule whose stored options no longer apply, on the page", () => {
  let root: string;
  let stores: TypeScriptPackStores;
  let body: RulesBody;

  beforeEach(async () => {
    root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-ui-not-running-")));
    const checkout = join(root, "app");
    mkdirSync(checkout, { recursive: true });
    execFileSync("git", ["init", "-q", "-b", "main"], { cwd: checkout, stdio: "ignore" });
    const databasePath = join(root, "rerail.sqlite");
    stores = new TypeScriptPackStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    const repositoryKey = recordCheckout(stores.database(), checkout).repositoryKey;
    setUpRepository(stores.database(), repositoryKey, { packs: [{ packId: "typescript", folder: "" }] });
    // A setting stored before the rule's document dropped the option (or imported from a dump): the API would refuse
    // it now, so it is written as the database holds it.
    stores.database().insert(ruleSettings).values({
      ownerKind: "repository", ownerId: repositoryKey, folder: "", targetKind: "rule", targetId: RULE, on: true, config: { opton: "type" }, updatedAt: 1,
    }).run();
    const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
    body = await (await app.request(`/api/repository/rules?${new URLSearchParams({ repository: repositoryKey })}`)).json() as RulesBody;
    data.rules = toStoredRules(body.rules);
    ui.filter = "all";
  });

  afterEach(async () => {
    ui.filter = "all";
    data.rules = new Map();
    stores.close();
    await rm(root, { recursive: true, force: true });
  });

  it("keeps the API's reason, shows the rule switched on but not running, and counts it as off", () => {
    const entry = body.rules.find(candidate => candidate.rule.id === RULE)!;
    expect(entry).toMatchObject({ on: true, configError: expect.stringMatching(/`opton` is not an option this document declares/u) });

    const stored = toStoredRules(body.rules).get(RULE)!;
    expect(stored).toMatchObject({ enabled: true, configError: entry.configError });
    expect(isRunning(stored)).toBe(false);
    expect(onText(stored)).toBe(`On · Set for the repository · not running: ${entry.configError}`);
    expect(optionsText(stored)).toBe(`opton: "type" · Set for the repository · does not apply: ${entry.configError}`);

    const running = body.rules.filter(candidate => candidate.on && candidate.configError === null).length;
    expect(running).toBeGreaterThan(0);
    expect(runningCount()).toBe(running);
    expect(summary().on).toHaveLength(running);
    ui.filter = "on";
    expect(matches(stored)).toBe(false);
    ui.filter = "off";
    expect(matches(stored)).toBe(true);
  });

  it("keeps the reason when the rule is added at more than one folder", () => {
    const entry = body.rules.find(candidate => candidate.rule.id === RULE)!;
    const stored = toStoredRules([{ ...entry, folder: "web", configError: null }, entry]).get(RULE)!;
    expect(stored.configError).toBe(entry.configError);
    expect(isRunning(stored)).toBe(false);
  });
});
