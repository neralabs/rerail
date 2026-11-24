import { afterEach, describe, expect, it } from "vitest";
import { awaitingApproval, awaitingYourApproval } from "../../ui/derive.js";
import { toStoredRules } from "../../ui/poll.js";
import { data } from "../../ui/state.js";
import { firstToReview, othersLine, othersToApprove, yourRulesToApprove } from "../../ui/views/overview.js";
import { approveLabel } from "../../ui/render/rule-sheet-model.js";

const subject = (kind: string) => ({ kind, id: "x", digest: "sha256:0" });
const rule = (origin: string, approval: { approved: boolean; subject: { kind: string } } | null, copiedFromRuleId: string | null = null) =>
  ({ origin, approval, copiedFromRuleId });

describe("rules waiting for your approval", () => {
  it("are rules created by you that are not approved yet", () => {
    expect(awaitingYourApproval(rule("user", { approved: false, subject: subject("rule") }))).toBe(true);
  });

  it("leave out approved rules and rules that need no approval", () => {
    expect(awaitingYourApproval(rule("user", { approved: true, subject: subject("rule") }))).toBe(false);
    expect(awaitingYourApproval(rule("user", null))).toBe(false);
  });

  it("leave out what someone else wrote, which still waits for approval on its own page", () => {
    // A worktree's own rule, and a rule of a pack Rerail did not publish.
    const worktree = rule("pack", { approved: false, subject: subject("rule") });
    const thirdParty = rule("pack", { approved: false, subject: subject("pack") });

    expect(awaitingYourApproval(worktree)).toBe(false);
    expect(awaitingYourApproval(thirdParty)).toBe(false);
    expect(awaitingApproval(worktree)).toBe(true);
    expect(awaitingApproval(thirdParty)).toBe(true);
  });

  it("leave out your edited copy of a rule Rerail ships, whose approval turns the original off", () => {
    expect(awaitingYourApproval(rule("user", { approved: false, subject: subject("rule") }, "typescript-eslint-no-explicit-any"))).toBe(false);
  });

  it("read which rule a copy was made from off the rules answer", () => {
    // Two entries of `GET /api/repository/rules`, as the daemon sends them: a copy names its original, any other rule null.
    const entry = (id: string, copiedFromRuleId: string | null) => ({
      kind: "code", rule: { id, mode: "steer", clauses: [] }, folder: "", on: true, mode: "log",
      source: { kind: "library_rule", id, packId: null }, onSource: { by: "rule-default" }, modeSource: { by: "awaiting-approval" },
      suppressions: "honour", suppressionsSource: { by: "rule-default" }, config: null, configSource: {}, configError: null,
      revision: 2, payloadHash: "sha256:0", addedAt: 1, updatedAt: 1, copiedFromRuleId,
      approval: { subject: { kind: "rule", id, digest: "sha256:0" }, approved: false },
    });
    const stored = toStoredRules([entry("no-console-copy", "no-console"), entry("agents-rule", null)]);

    expect(awaitingYourApproval(stored.get("no-console-copy")!)).toBe(false);
    expect(awaitingYourApproval(stored.get("agents-rule")!)).toBe(true);
  });
});

/** One waiting entry of `GET /api/repository/rules`, as the daemon sends it; `fields` override the entry's. */
const waiting = (id: string, fields: Record<string, unknown> = {}, title?: string) => ({
  kind: "code", rule: { id, mode: "steer", clauses: [{ violation: { message: `Message of ${id}.` } }], ...(title === undefined ? {} : { title }) },
  folder: "", on: true, mode: "log",
  source: { kind: "library_rule", id, packId: null }, onSource: { by: "rule-default" }, modeSource: { by: "awaiting-approval" },
  suppressions: "honour", suppressionsSource: { by: "rule-default" }, config: null, configSource: {}, configError: null,
  revision: 1, payloadHash: "sha256:0", addedAt: 1, updatedAt: 1, copiedFromRuleId: null,
  approval: { subject: { kind: "rule", id, digest: `sha256:${id}` }, approved: false },
  ...fields,
});
const worktree = { source: { kind: "pack", id: "worktree" }, onSource: { by: "pack-start" }, modeSource: { by: "awaiting-approval" } };

describe("Overview's banner for rules someone else wrote", () => {
  afterEach(() => {
    data.loaded = false;
    data.rules = new Map();
    data.packs = [];
    data.activityAll = {};
  });

  it("leaves out edited copies, which their original's sheet tells of, and rules your coding agent added", () => {
    data.loaded = true;
    data.rules = toStoredRules([
      waiting("storefront-no-legacy-client", worktree, "No legacy API client"),
      waiting("typescript-eslint-no-explicit-any-copy", { copiedFromRuleId: "typescript-eslint-no-explicit-any" }),
      waiting("storefront-no-raw-fetch", {}, "No raw fetch"),
    ]);

    expect(othersToApprove()).toEqual([{ kind: "rule", label: "No legacy API client", act: "detail", target: "storefront-no-legacy-client" }]);
  });

  it("labels a rule with no title of its own and no pack Review rule, not its violation message", () => {
    data.loaded = true;
    data.rules = toStoredRules([waiting("storefront-untitled", worktree)]);

    expect(othersToApprove().map(item => item.label)).toEqual(["Review rule"]);
  });

  it("names a rule with no title of its own by its pack's title for it", () => {
    data.loaded = true;
    data.packs = [{ id: "acme", name: "Acme", category: "language", covers: "", rules: [{ id: "acme-no-eval", title: "No eval", group: "Security", mode: "steer", on: true }] }] as never;
    data.rules = toStoredRules([waiting("acme-no-eval", { source: { kind: "pack", id: "acme" }, onSource: { by: "pack-start" } })]);

    expect(othersToApprove().map(item => item.label)).toEqual(["No eval"]);
  });

  it("says Rerail did not write them and where each is approved, without a slash command", () => {
    expect(othersLine(1)).toBe("Rerail did not write it. Read what it tells the coding agent, then approve it on its page.");
    expect(othersLine(3)).toBe("Rerail did not write them. Read what each tells the coding agent, then approve each on their page.");
    expect(`${othersLine(1)} ${othersLine(2)}`).not.toContain("/rerail-create");
  });
});

describe("Review in Created by you", () => {
  afterEach(() => {
    data.loaded = false;
    data.rules = new Map();
    data.activityAll = {};
  });

  it("opens the waiting rule that caught the most, else the first by title", () => {
    data.loaded = true;
    data.rules = toStoredRules([waiting("a-rule", {}, "Alpha"), waiting("b-rule", {}, "Beta"), waiting("c-rule", {}, "Gamma")]);
    const activity = (distinctViolations: number) => ({ measured: true, distinctViolations, loggedViolations: distinctViolations, rerails: 0, sessions: 0, lastSeenAt: null, audits: 1 });

    expect(firstToReview(yourRulesToApprove())?.rule.id).toBe("a-rule");
    data.activityAll = { "a-rule": activity(1), "b-rule": activity(7), "c-rule": activity(3) } as never;
    expect(firstToReview(yourRulesToApprove())?.rule.id).toBe("b-rule");
  });
});

describe("the Approve button of a rule your coding agent added", () => {
  it("reads Approve and steer for a rule set to steer and held on Log only by the approval, else Approve", () => {
    expect(approveLabel({ modeSource: { by: "awaiting-approval" } })).toBe("Approve and steer");
    expect(approveLabel({ modeSource: { by: "rule-default" } })).toBe("Approve");
    expect(approveLabel({})).toBe("Approve");
  });
});
