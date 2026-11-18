import { describe, expect, it } from "vitest";
import { awaitingApproval, awaitingYourApproval } from "../../ui/derive.js";

const subject = (kind: string) => ({ kind, id: "x", digest: "sha256:0" });
const rule = (origin: string, approval: { approved: boolean; subject: { kind: string } } | null) => ({ origin, approval });

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
});
