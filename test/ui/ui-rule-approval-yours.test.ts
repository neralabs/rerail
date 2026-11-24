import { afterEach, describe, expect, it } from "vitest";
import { awaitingAgentApproval, awaitingApproval, awaitingYourApproval, isWaitingEdit } from "../../ui/derive.js";
import { toStoredRules } from "../../ui/poll.js";
import { data } from "../../ui/state.js";
import { firstToReview, othersLine, othersToApprove, waitingForYouLine, waitingForYouTitle, yourRulesToApprove } from "../../ui/views/overview.js";
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

  it("count that edited copy as an edit that waits, asked for with the rules your coding agent added", () => {
    const copy = rule("user", { approved: false, subject: subject("rule") }, "typescript-eslint-no-explicit-any");
    const approved = rule("user", { approved: true, subject: subject("rule") }, "typescript-eslint-no-explicit-any");

    expect(isWaitingEdit(copy)).toBe(true);
    expect(awaitingAgentApproval(copy)).toBe(true);
    expect(isWaitingEdit(approved)).toBe(false);
    expect(awaitingAgentApproval(rule("user", { approved: false, subject: subject("rule") }))).toBe(true);
    // What someone else wrote keeps its own banner.
    expect(awaitingAgentApproval(rule("pack", { approved: false, subject: subject("pack") }))).toBe(false);
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

  it("leaves out the rules your coding agent added and the edits it made, which have their own banner", () => {
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

describe("Overview's banner for what your coding agent added and edited", () => {
  it("says how many were added or edited, and counts both when both wait", () => {
    expect(waitingForYouTitle(1, 0)).toBe("Your coding agent added a rule. It’s on Log only until you approve it.");
    expect(waitingForYouTitle(3, 0)).toBe("Your coding agent added 3 rules. They’re on Log only until you approve them.");
    expect(waitingForYouTitle(0, 1)).toBe("Your coding agent edited a rule. The edit is on Log only until you approve it.");
    expect(waitingForYouTitle(0, 2)).toBe("Your coding agent edited 2 rules. The edits are on Log only until you approve them.");
    expect(waitingForYouTitle(3, 1)).toBe("4 rules wait for your approval: 3 added, 1 edited. They’re on Log only until you approve them.");
  });

  it("says an edited rule's original keeps running, and that Review opens the next one waiting", () => {
    expect(waitingForYouLine(1, 0)).toBe("Open it to see what it checks and what it tells the coding agent, then approve it on its page.");
    expect(waitingForYouLine(0, 1)).toBe("The original rule keeps running until then. Open it to see what it checks and what it tells the coding agent, then approve it on its page.");
    expect(waitingForYouLine(3, 0)).toBe("Review opens the next one waiting. Each is approved on its own page.");
    expect(waitingForYouLine(3, 1)).toBe("An edited rule’s original keeps running until then. Review opens the next one waiting. Each is approved on its own page.");
  });
});

describe("Review next rule", () => {
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

  it("opens an edit that waits too, in its turn", () => {
    data.loaded = true;
    data.rules = toStoredRules([waiting("a-rule", {}, "Alpha"), waiting("no-console-copy", { copiedFromRuleId: "no-console" }, "No console")]);
    const activity = (distinctViolations: number) => ({ measured: true, distinctViolations, loggedViolations: distinctViolations, rerails: 0, sessions: 0, lastSeenAt: null, audits: 1 });

    expect(yourRulesToApprove().map(stored => stored.rule.id)).toEqual(["a-rule", "no-console-copy"]);
    data.activityAll = { "no-console-copy": activity(2) } as never;
    expect(firstToReview(yourRulesToApprove())?.rule.id).toBe("no-console-copy");
  });
});

describe("the Approve button of a rule your coding agent added", () => {
  it("reads Approve and steer for a rule set to steer and held on Log only by the approval, else Approve", () => {
    expect(approveLabel({ modeSource: { by: "awaiting-approval" } })).toBe("Approve and steer");
    expect(approveLabel({ modeSource: { by: "rule-default" } })).toBe("Approve");
    expect(approveLabel({})).toBe("Approve");
  });
});
