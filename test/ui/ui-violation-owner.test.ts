import { describe, expect, it } from "vitest";
import { violationOwner } from "../../ui/format.js";

describe("who a violation is credited to", () => {
  it("is the session it was found in, which its button opens", () => {
    expect(violationOwner({ sessionId: "5f1c0a2e-0b7d-4c1e-9a3f-29b2d96d0c11", host: "claude-code" }))
      .toEqual({ session: "5f1c0a2e-0b7d-4c1e-9a3f-29b2d96d0c11", label: "claude-29b2d96d", title: "Open this session" });
  });

  it("is the repository, with no session to open, for one its own audit recorded", () => {
    expect(violationOwner({ sessionId: null, host: null }))
      .toEqual({ session: null, label: "Repository", title: "Found in another worktree; no session made this change" });
  });
});
