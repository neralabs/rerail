import { describe, expect, expectTypeOf, it } from "vitest";
import type { z } from "zod";
import { presentationReadSchema } from "../../src/api/presentation-contracts.js";
import { presentationResponseSchema } from "../../src/api/session-contracts.js";

/** A full `/api/presentation` answer as the daemon's contract describes it. */
const answer = {
  presence: "compact",
  repositoryState: "started",
  scope: { repositoryKey: "clone:git:repository", checkoutPath: "/work/repo" },
  context: {
    daemon: { pid: 4242, database: { path: "/home/.rerail/rerail.sqlite", exists: true }, native: { ok: true }, workspaceRoot: "/work/repo" },
    session: {
      session: { sessionId: "s1", host: "claude-code", cwd: "/work/repo", repositoryId: "clone:git:repository", startedAt: 1, endedAt: null, eventCount: 3 },
      hud: { activeRuleCount: 417, steeringCount: 2, overlays: ["worktree"], awaitingApproval: 1 },
      audit: {
        workspaceRoot: "/work/repo", scope: "ai-output", mode: "steer", status: "failed", violationCount: 0, failureCount: 1,
        unlistedFailureCount: 0, blindFileCount: 0, gapCount: 0, outOfWorkRuleCount: 0, fileWorkStoppedRuleCount: 0,
      },
    },
  },
};

describe("the footer's reading of /api/presentation", () => {
  it("reads every answer the daemon's contract allows, keeping its fields", () => {
    const served = presentationResponseSchema.parse(answer);
    expect(presentationReadSchema.parse(served)).toEqual(served);
    const withoutSession = presentationResponseSchema.parse({ ...answer, context: { ...answer.context, session: null } });
    expect(presentationReadSchema.parse(withoutSession)).toEqual(withoutSession);
    const before = presentationResponseSchema.parse({ presence: "silent", repositoryState: "not-set-up" });
    expect(presentationReadSchema.parse(before)).toEqual(before);
  });

  it("accepts the type of every answer the daemon's contract allows", () => {
    expectTypeOf<z.infer<typeof presentationResponseSchema>>().toExtend<z.input<typeof presentationReadSchema>>();
  });

  it("refuses an answer missing what the footer reads", () => {
    const { audit: _audit, ...withoutAudit } = answer.context.session;
    expect(presentationReadSchema.safeParse({ ...answer, context: { ...answer.context, session: withoutAudit } }).success).toBe(false);
    expect(presentationReadSchema.safeParse({ ...answer, presence: "loud" }).success).toBe(false);
  });
});
