import { describe, expect, it } from "vitest";
import { awaitingApproval, awaitingYourApproval } from "../../ui/derive.js";
import { toStoredRules } from "../../ui/poll.js";

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
