import { realpathSync, writeFileSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { defaultPolicy, loadPolicy, parsePolicy, POLICY_PATH } from "../../../src/engine/merge-assist/policy.js";
import { commit, git } from "../../support/git.js";

const DEFAULTS = { version: 1, dimmer: { interruptCost: 300, stopGrowthPerHour: 50, checkpointsPerDay: 3 } };

const problems = (text: string): string[] => {
  const parsed = parsePolicy(text);
  return parsed.ok ? [] : parsed.problems;
};

describe("parsePolicy", () => {
  it("fills the dimmer's defaults from a bare version", () => {
    expect(defaultPolicy()).toEqual(DEFAULTS);
    expect(parsePolicy("version: 1\n")).toEqual({ ok: true, policy: DEFAULTS });
  });

  it("reads the dimmer's settings, defaulting the ones left out", () => {
    expect(parsePolicy("version: 1\ndimmer:\n  interruptCost: 120\n  checkpointsPerDay: 0\n"))
      .toEqual({ ok: true, policy: { version: 1, dimmer: { interruptCost: 120, stopGrowthPerHour: 50, checkpointsPerDay: 0 } } });
  });

  it("refuses any key besides version and dimmer, as the earlier review settings were", () => {
    expect(problems("version: 1\nsteps: [hard-checks, panel]")).toEqual([expect.stringContaining("steps")]);
    expect(problems("version: 1\npanel: { judges: 3 }")).toEqual([expect.stringContaining("panel")]);
    expect(problems("version: 1\ndimmer:\n  loud: true\n")).toEqual([expect.stringContaining("dimmer")]);
  });

  it("refuses another version, duplicate keys, out-of-range settings and text that is not YAML", () => {
    expect(problems("version: 2")).toEqual([expect.stringContaining("version")]);
    expect(problems("dimmer: {}")).toEqual([expect.stringContaining("version")]);
    expect(problems("version: 1\nversion: 1")[0]).toMatch(/^not YAML/u);
    expect(problems("version: 1\ndimmer:\n  interruptCost: 0\n")).toEqual([expect.stringContaining("dimmer.interruptCost")]);
    expect(problems("version: 1\ndimmer:\n  checkpointsPerDay: 1.5\n")).toEqual([expect.stringContaining("dimmer.checkpointsPerDay")]);
    expect(problems("version: [1")[0]).toMatch(/^not YAML/u);
  });
});

describe("loadPolicy", () => {
  const roots: string[] = [];
  afterEach(async () => {
    await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true })));
  });

  async function repository(policy: string | null): Promise<string> {
    const root = realpathSync(await mkdtemp(join(tmpdir(), "rerail-policy-")));
    roots.push(root);
    git(root, "init", "--quiet", "-b", "main");
    commit(root, { "README.md": "app\n", ...(policy === null ? {} : { [POLICY_PATH]: policy }) }, "start");
    return root;
  }

  it("reads the policy at the revision, not the working tree", async () => {
    const root = await repository("version: 1\ndimmer:\n  checkpointsPerDay: 1\n");
    writeFileSync(join(root, POLICY_PATH), "version: 1\ndimmer:\n  checkpointsPerDay: 9\n");
    expect(loadPolicy(root, "main")).toEqual({ source: "file", policy: { version: 1, dimmer: { interruptCost: 300, stopGrowthPerHour: 50, checkpointsPerDay: 1 } } });
  });

  it("falls back to the defaults without a file, and reports an invalid one", async () => {
    expect(loadPolicy(await repository(null), "main")).toEqual({ source: "default", policy: DEFAULTS });
    expect(loadPolicy(await repository("version: 1\ncriteria: {}\n"), "main")).toEqual({ source: "invalid", problems: [expect.stringContaining("criteria")] });
  });

  it("refuses a revision that could be read as an option or a path", async () => {
    const root = await repository(null);
    expect(loadPolicy(root, "--output=x")).toEqual({ source: "invalid", problems: ["not a revision: --output=x"] });
    expect(loadPolicy(root, "main:README.md")).toEqual({ source: "invalid", problems: ["not a revision: main:README.md"] });
  });
});
