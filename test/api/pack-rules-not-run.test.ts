import { execFileSync } from "node:child_process";
import { mkdirSync, readFileSync, realpathSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createRerailApi } from "../../src/api/http.js";
import { operations } from "../../src/api/operations.js";
import { DatabaseApiStores } from "../../src/api/stores.js";
import {
  CheckoutCatalogs,
  lookupCheckout,
  NativeCodeRuleEnforcer,
  openRerailDatabase,
  type AuditInput,
  type AuditRecorder,
  type AuditResult,
} from "../../src/engine/index.js";
import { checkoutRulesNotRun, onRuleCount } from "../../src/runtime/index.js";
import { isRunning, notRunningReason } from "../../ui/derive.js";
import { toStoredRules } from "../../ui/poll.js";
import { installPackDirectory, removeInstalledPacks, withOlderTokenRule } from "../support/installed-packs.js";
import { repositoryPath } from "../support/oracles.js";
import { removeFolder } from "../support/platform/remove-folder.js";
import { setUpTestRepository } from "../support/repository-rules.js";

const COMPLEXITY = repositoryPath("packs", "complexity");
const VERSION = (JSON.parse(readFileSync(join(COMPLEXITY, "package.json"), "utf8")) as { version: string }).version;
const RULE = "complexity-cyclomatic-python";
const REASON = `This Rerail no longer runs the rule as Function complexity ${VERSION} writes it: it tests a token that parsed files now leave out. The pack's next version runs it again.`;
const INACTIVE = { code: "reads-left-out-token", reason: REASON };

let root: string;

function recordingAudits(): { recorder: AuditRecorder; audits: (AuditInput & { result?: AuditResult })[] } {
  const audits: (AuditInput & { result?: AuditResult })[] = [];
  return {
    recorder: {
      start(input) {
        audits.push({ ...input });
        return `run-${audits.length}`;
      },
      finish(auditId, result) { audits[Number(auditId.slice(4)) - 1]!.result = result; },
      progress(auditId, result) { audits[Number(auditId.slice(4)) - 1]!.result = { status: "completed", ...result }; },
      interrupt() {},
    },
    audits,
  };
}

beforeEach(async () => {
  removeInstalledPacks();
  root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-not-run-")));
  installPackDirectory(withOlderTokenRule(COMPLEXITY, join(root, "pack"), RULE), "complexity");
});

afterEach(async () => {
  removeInstalledPacks();
  await removeFolder(root);
});

describe("an installed pack's rule this Rerail no longer runs", () => {
  let checkout: string;
  let databasePath: string;

  beforeEach(() => {
    checkout = join(root, "app");
    mkdirSync(checkout);
    execFileSync("git", ["init", "--quiet"], { cwd: checkout });
    databasePath = join(root, "rerail.sqlite");
    setUpTestRepository(checkout, { packs: ["complexity"] }, databasePath);
  });

  it("is kept inactive, so the opening audit does not fail", async () => {
    const catalogs = new CheckoutCatalogs(databasePath);
    const { recorder, audits } = recordingAudits();
    const enforcer = new NativeCodeRuleEnforcer(catalogs.catalog(checkout), {
      workspaceRoot: checkout,
      steerDelivery: { requestSteer: async () => undefined },
      audits: recorder,
      mode: "log",
    });
    try {
      expect(catalogs.catalog(checkout).quarantined()).toEqual([{ ruleId: RULE, kind: "code", owner: "rerail", reason: REASON, code: "reads-left-out-token" }]);
      await enforcer.openWorkspace("session");
      expect(enforcer.snapshotStatus().failures).toEqual([]);
      expect(audits.filter(audit => audit.result?.status === "failed")).toEqual([]);
    } finally {
      await enforcer.close();
      catalogs.close();
    }
  });

  it("is listed as inactive by the rules and packs APIs, keeps its pack's start, and is not counted as on", async () => {
    const stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    });
    try {
      const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
      const repositoryKey = lookupCheckout(stores.database(), checkout)!.repositoryKey;
      type Entry = { rule: { id: string }; on: boolean; inactive?: unknown };
      const rules = (await (await app.request(`/api/repository/rules?${new URLSearchParams({ repository: repositoryKey })}`)).json() as { rules: Entry[] }).rules;
      expect(rules.find(entry => entry.rule.id === RULE)).toMatchObject({ on: true, inactive: INACTIVE });
      expect(rules.filter(entry => entry.inactive !== undefined).map(entry => entry.rule.id)).toEqual([RULE]);

      type PackRule = { id: string; on: boolean; inactive?: unknown };
      const packs = (await (await app.request("/api/rule-packs")).json() as { packs: { id: string; rules: PackRule[] }[] }).packs;
      const packRules = packs.find(pack => pack.id === "complexity")!.rules;
      expect(packRules.find(rule => rule.id === RULE)).toMatchObject({ on: true, inactive: INACTIVE });
      expect(packRules.filter(rule => rule.inactive !== undefined)).toHaveLength(1);

      // The count set-up and `rerail start` say is the one the session note says: the compiled rules.
      const running = rules.filter(entry => entry.on && entry.inactive === undefined).length;
      const database = openRerailDatabase(databasePath);
      try {
        const recorded = lookupCheckout(database, checkout)!;
        expect(onRuleCount(database, recorded)).toBe(running);
        // Set-up names the rules on that do not run, which add up with the count to the rules switched on.
        expect(checkoutRulesNotRun(database, recorded)).toEqual([{ name: "Function complexity", version: VERSION, ruleCount: 1 }]);
        expect(running + 1).toBe(rules.filter(entry => entry.on).length);
      } finally {
        database.$client.close();
      }
      const catalogs = new CheckoutCatalogs(databasePath);
      try {
        expect(catalogs.catalog(checkout).snapshot().codeRules).toHaveLength(running);
      } finally {
        catalogs.close();
      }

      // The dashboard shows it switched on but not running, with why, and leaves it out of its counts.
      const stored = toStoredRules(rules).get(RULE)!;
      expect(stored).toMatchObject({ enabled: true, inactive: INACTIVE });
      expect(isRunning(stored)).toBe(false);
      expect(notRunningReason(stored)).toBe(REASON);
      expect([...toStoredRules(rules).values()].filter(isRunning)).toHaveLength(running);
    } finally {
      stores.close();
    }
  });
});
