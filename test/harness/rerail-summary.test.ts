import { describe, expect, it } from "vitest";
import { renderRerailSummary, type RerailSummaryInput } from "../../src/harness/rerail-summary.js";

type SessionPresentation = NonNullable<RerailSummaryInput["presentation"]>;

const summary = (status: SessionPresentation["status"]): string =>
  renderRerailSummary({
    presentation: { status } as SessionPresentation,
    repositoryState: null,
    ruleCount: null,
    awaitingApproval: null,
    rulesUrl: null,
    repositoryName: "acme-web",
    sigil: "/",
  } satisfies RerailSummaryInput);

describe("/rerail for a session Rerail has not read", () => {
  it("says a starting session is not recorded yet", () => {
    expect(summary("starting")).toContain("This session is not recorded yet; ask again after your next message.");
  });

  it("says an unavailable session could not be read, not that it is still starting", () => {
    const text = summary("unavailable");
    expect(text).toContain("This session could not be read.");
    expect(text).not.toContain("not recorded yet");
  });

  it("says a daemon that did not answer in time is running, and gives no advice to start it", () => {
    const noSession = renderRerailSummary({
      presentation: null, repositoryState: "not-answering", ruleCount: null, awaitingApproval: null, rulesUrl: null, repositoryName: "acme-web", sigil: "/",
    });
    for (const text of [summary("not-answering"), noSession]) {
      expect(text.split("\n").slice(0, 2)).toEqual([
        "● Rerail is running, but its daemon did not answer in time.",
        "  Ask again in a moment; if it keeps happening, run `rerail status` in a terminal.",
      ]);
      expect(text).not.toContain("rerail daemon start");
    }
  });
});
