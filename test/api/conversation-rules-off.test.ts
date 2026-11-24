import { execFileSync } from "node:child_process";
import { mkdirSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import { experimentOffMessage, experimentVariable, lookupCheckout, openRerailDatabase, type ConversationRule } from "../../src/engine/index.js";
import { onRuleCount } from "../../src/runtime/index.js";
import { isRunning, notRunningReason } from "../../ui/derive.js";
import { enforcementCounts } from "../../ui/pack-model.js";
import { toStoredRules } from "../../ui/poll.js";
import { ruleCountsOf } from "../../ui/repo-rules.js";
import { installPackDirectory, removeInstalledPacks, versionOf, withConversationRule } from "../support/installed-packs.js";
import { repositoryPath } from "../support/oracles.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { setUpTestRepository } from "../support/repository-rules.js";

const COMPLEXITY = repositoryPath("packs", "complexity");
const RULE: ConversationRule = {
  id: "complexity-no-force-push",
  title: "Push without force",
  mode: "steer",
  match: { field: "input.command", regex: "git push --force" },
  sample: { source: "tool:*", boundary: "tool-call" },
  action: { content: "Push without --force." },
  repeat: { mode: "always" },
};
const INACTIVE = { code: "experimental-off", reason: experimentOffMessage("CONVERSATION_RULES", "start the Rerail daemon with") };

type Entry = { rule: { id: string }; on: boolean; mode: string; inactive?: unknown };
type PackRule = { id: string; on: boolean; inactive?: unknown };

let root: string;

beforeEach(async () => {
  removeInstalledPacks();
  root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-conversation-off-")));
  // Installed as the registry installs Rerail's own packs, so its rules steer without waiting for an approval.
  installPackDirectory(withConversationRule(COMPLEXITY, join(root, "pack"), RULE), "complexity", undefined, versionOf(COMPLEXITY));
});

afterEach(async () => {
  vi.unstubAllEnvs();
  removeInstalledPacks();
  await removeFolder(root);
});

describe("a pack's conversation rule switched on", () => {
  let checkout: string;
  let databasePath: string;

  beforeEach(() => {
    checkout = join(root, "app");
    mkdirSync(checkout);
    execFileSync("git", ["init", "--quiet"], { cwd: checkout });
    databasePath = join(root, "rerail.sqlite");
    setUpTestRepository(checkout, { packs: ["complexity"] }, databasePath);
  });

  /** What the APIs say of the rule, and the rules on as `rerail` and the dashboard's Overview, Settings and pack page count them. */
  async function surfaces(conversationRules: "0" | "1") {
    vi.stubEnv(experimentVariable("CONVERSATION_RULES"), conversationRules);
    const stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    try {
      const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
      const recorded = lookupCheckout(stores.database(), checkout)!;
      const rules = (await (await app.request(`/api/repository/rules?${new URLSearchParams({ repository: recorded.repositoryKey })}`)).json() as { rules: Entry[] }).rules;
      const packs = (await (await app.request("/api/rule-packs")).json() as { packs: { id: string; rules: PackRule[] }[] }).packs;
      const activity = (await (await app.request(`/api/rules/activity?${new URLSearchParams({ since: "0", repository: recorded.repositoryKey })}`)).json() as { rules: Record<string, unknown> }).rules;
      const database = openRerailDatabase(databasePath);
      let cli: number;
      try {
        cli = onRuleCount(database, recorded);
      } finally {
        database.$client.close();
      }
      const stored = toStoredRules(rules);
      return {
        entry: rules.find(entry => entry.rule.id === RULE.id),
        packRule: packs.find(pack => pack.id === "complexity")!.rules.find(rule => rule.id === RULE.id),
        activity: activity[RULE.id],
        stored: stored.get(RULE.id)!,
        cli,
        overview: [...stored.values()].filter(isRunning).length,
        settings: ruleCountsOf(rules).on,
        steer: Number(/(\d+) steer/u.exec(enforcementCounts([...stored.values()]))?.[1] ?? 0),
      };
    } finally {
      stores.close();
    }
  }

  it("is inactive while conversation rules are off, so no surface counts it as on or steering", async () => {
    const off = await surfaces("0");
    const on = await surfaces("1");

    // Its settings are kept: on, in Steer. The APIs say it does not run, and why.
    expect(off.entry).toMatchObject({ on: true, mode: "steer", inactive: INACTIVE });
    expect(off.packRule).toMatchObject({ on: true, inactive: INACTIVE });
    expect(off.activity).toMatchObject({ measured: false, inactive: INACTIVE });
    expect(isRunning(off.stored)).toBe(false);
    expect(notRunningReason(off.stored)).toBe(INACTIVE.reason);

    // With the experiment on, it runs and every surface counts it once more.
    expect(on.entry).not.toHaveProperty("inactive");
    expect(on.packRule).not.toHaveProperty("inactive");
    expect(on.activity).not.toHaveProperty("inactive");
    expect(isRunning(on.stored)).toBe(true);
    expect(off.overview).toBe(on.overview - 1);
    expect(off.steer).toBe(on.steer - 1);

    // Overview's Rules on, Settings' repository row and `rerail`'s own count agree, the experiment on or off.
    for (const counts of [off, on]) expect([counts.settings, counts.cli]).toEqual([counts.overview, counts.overview]);
  });
});
