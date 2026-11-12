import { describe, expect, it } from "vitest";
import type { ConflictDelivery, MergeAssistStatus } from "../../src/engine/index.js";
import {
  budgetText, conflictSections, dismissToast, githubText, hookFailure, hunkLines, judgeText, openConflictCount,
  readinessChecks, safePeerUrl, scanSummary, scannedWhat, settingsChange, settingsDraft, settingsSummary, writeError,
} from "../../ui/merge-model.js";
import { conflict, dismissed } from "./support/merge-conflicts.js";

const NOW = Date.parse("2026-11-02T12:00:00Z");
const at = (minutesAgo: number) => new Date(NOW - minutesAgo * 60_000).toISOString();

const status = (over: Partial<MergeAssistStatus> = {}): MergeAssistStatus => ({
  repositoryKey: "remote:github.com/acme/storefront",
  checkout: "/work/storefront",
  branch: "tbelanger/export-api",
  baseBranch: "main",
  enabled: true,
  settings: { delivery: "both", hook: null },
  readiness: {
    judge: { available: true, runner: "claude", problem: null },
    github: { state: "signed-in", login: "tbelanger", repository: "acme/storefront" },
  },
  budget: { used: 3, limit: 40, windowMinutes: 60 },
  scanning: false,
  lastScan: { at: at(4), peers: { branches: 3, pullRequests: 2 }, problems: [] },
  conflicts: [],
  ...over,
});

describe("a peer's link", () => {
  const onGithub = { repositoryKey: "remote:github.com/acme/storefront", repository: "acme/storefront" };
  const onEnterprise = { repositoryKey: "remote:github.example.com/acme/storefront", repository: "acme/storefront" };

  it("opens a pull request of this repository on github.com or on its own GitHub Enterprise host", () => {
    expect(safePeerUrl("https://github.com/acme/storefront/pull/12", onGithub)).toBe("https://github.com/acme/storefront/pull/12");
    expect(safePeerUrl("https://github.example.com/acme/storefront/pull/12", onEnterprise)).toBe("https://github.example.com/acme/storefront/pull/12");
  });

  it.each([
    "javascript:alert(1)",
    "http://github.com/acme/storefront/pull/12",
    "https://evil.example/acme/storefront/pull/12",
    "https://github.com.evil.example/acme/storefront/pull/12",
    "https://github.example.com/acme/storefront/pull/12",
    "https://notgithub.com/acme/storefront/pull/12",
    "https://user:token@github.com/acme/storefront/pull/12",
    "https://github.com:8443/acme/storefront/pull/12",
    "https://github.com/someone-else/storefront/pull/12",
    " https://github.com/acme/storefront/pull/12",
    "data:text/html,<script>alert(1)</script>",
    null,
    42,
  ])("refuses %s for a github.com repository", url => {
    expect(safePeerUrl(url, onGithub)).toBeNull();
  });

  it("accepts only github.com when the repository key names no GitHub host, as with an ssh alias", () => {
    const aliased = { repositoryKey: "remote:github-work/acme/storefront", repository: "acme/storefront" };
    expect(safePeerUrl("https://github.com/acme/storefront/pull/3", aliased)).toBe("https://github.com/acme/storefront/pull/3");
    expect(safePeerUrl("https://github.example.com/acme/storefront/pull/3", aliased)).toBeNull();
  });
});

describe("the status strip", () => {
  it("names the summarising CLI, or says none was found with why", () => {
    expect(judgeText({ available: true, runner: "codex", problem: null })).toEqual({ ok: true, text: "Codex" });
    expect(judgeText({ available: false, runner: null, problem: "Neither claude nor codex is on PATH." }))
      .toEqual({ ok: false, text: "No agent CLI found", detail: "Neither claude nor codex is on PATH." });
  });

  it("says gh's state with the login it is signed in as", () => {
    expect(githubText({ state: "signed-in", login: "tbelanger", repository: "acme/storefront" })).toEqual({ ok: true, text: "Signed in as tbelanger" });
    expect(githubText({ state: "signed-out", login: null, repository: null }).ok).toBe(false);
  });

  it("says what a scan read and the budget used this hour", () => {
    expect(scannedWhat({ at: at(1), peers: { branches: 1, pullRequests: 2 }, problems: [] })).toBe("1 local branch and 2 open pull requests");
    expect(scannedWhat({ at: at(1), peers: { branches: 0, pullRequests: 0 }, problems: [] })).toBe("no other branches");
    expect(budgetText({ used: 12, limit: 40, windowMinutes: 60 })).toEqual({ text: "12 of 40 this hour", spent: false });
    expect(budgetText({ used: 40, limit: 40, windowMinutes: 60 }).spent).toBe(true);
  });

  it("sums up the open conflicts a scan found that the page did not show before, not the totals", () => {
    const conflicts = [conflict("a"), conflict("b"), dismissed("c", at(3))];
    expect(scanSummary(null, status({ conflicts }))).toBe("Scanned 3 local branches and 2 open pull requests. New: 2 conflicts.");
    // Nothing changed: the conflicts that are still open are not news.
    expect(scanSummary(status({ conflicts }), status({ conflicts }))).toBe("Scanned 3 local branches and 2 open pull requests: nothing new.");
    expect(scanSummary(status({ conflicts }), status({ conflicts: [...conflicts, conflict("d")] })))
      .toBe("Scanned 3 local branches and 2 open pull requests. New: 1 conflict.");
    expect(scanSummary(null, status({ lastScan: { at: at(0), peers: { branches: 1, pullRequests: 0 }, problems: ["gh: rate limited"] } })))
      .toBe("Scanned 1 local branch: nothing new. 1 thing could not be read; see Last scan.");
  });

  it("says a scan request found another scan running, and that the results are the last ones", () => {
    expect(scanSummary(status(), status({ scanning: true }))).toBe("Another scan is running; showing the last results.");
  });
});

describe("the readiness checklist", () => {
  const text = (segments: Array<string | { code: string }>) => segments.map(s => (typeof s === "string" ? s : s.code)).join("");

  it("says the agent CLI is run for a one-line summary of each conflict, and names the supported CLIs when none is found", () => {
    const ready = readinessChecks(status().readiness);
    expect(ready.map(c => [c.key, c.ok])).toEqual([["judge", true], ["github", true]]);
    expect(text(ready[0]!.text)).toBe("Your agent CLI: Claude Code. Rerail runs it for a one-line summary of each conflict.");
    const missing = readinessChecks({ ...status().readiness, judge: { available: false, runner: null, problem: null } });
    expect(missing[0]!.ok).toBe(false);
    expect(text(missing[0]!.text)).toContain("need Claude Code or Codex on your PATH");
  });

  it("says whether gh is signed in, and how to sign in when it is not", () => {
    expect(text(readinessChecks(status().readiness)[1]!.text)).toContain("gh is signed in as tbelanger");
    const signedOut = readinessChecks({ ...status().readiness, github: { state: "signed-out", login: null, repository: null } });
    expect(signedOut[1]!.ok).toBe(false);
    expect(text(signedOut[1]!.text)).toContain("Run gh auth login");
  });
});

describe("a failed write's reason", () => {
  it("is the API's message when it answered one, else the error as sent", () => {
    const refused = new Error('POST /api/merge-assist/conflicts/dismiss answered 409: {"error":"invalid-state","message":"This conflict is no longer open."}');
    expect(writeError(refused)).toBe("This conflict is no longer open.");
    expect(writeError(new Error("POST /api/merge-assist/scan answered 500: oops"))).toBe("POST /api/merge-assist/scan answered 500: oops");
    expect(writeError(new Error("network down"))).toBe("network down");
  });
});

describe("conflicts", () => {
  const conflicts = [
    conflict("logged-new", { level: "logged", detectedAt: at(1) }),
    conflict("ambient-old", { detectedAt: at(60) }),
    dismissed("dismissed-1", at(2)),
    conflict("stop", { level: "stop", detectedAt: at(90) }),
    conflict("ambient-new", { detectedAt: at(5) }),
    dismissed("dismissed-2", at(20)),
  ];

  it("lists open conflicts loudest first, each level newest first; dismissed ones as the status orders them", () => {
    const sections = conflictSections(status({ conflicts }));
    expect(sections.open.map(c => c.id)).toEqual(["stop", "ambient-new", "ambient-old", "logged-new"]);
    expect(sections.dismissed.map(c => c.id)).toEqual(["dismissed-1", "dismissed-2"]);
  });

  it("drops a conflict being dismissed from the open list at once", () => {
    const { open } = conflictSections(status({ conflicts }), new Set(["ambient-old"]));
    expect(open.map(c => c.id)).toEqual(["stop", "ambient-new", "logged-new"]);
  });

  it("counts every open conflict in the sidebar, logged ones included, and none while off or being dismissed", () => {
    expect(openConflictCount(status({ conflicts }))).toBe(4);
    expect(openConflictCount(status({ conflicts }), new Set(["stop"]))).toBe(3);
    expect(openConflictCount(status({ conflicts, enabled: false }))).toBe(0);
    expect(openConflictCount(null)).toBe(0);
  });

  it("says what Dismiss did, and that it failed", () => {
    expect(dismissToast(conflict("c"))).toEqual({
      done: "Marked “Shared matchers c” as not a conflict. It stays hidden for PR #398.",
      fail: "Couldn’t mark “Shared matchers c” as not a conflict",
    });
  });

  it("colours a hunk's added, removed and header lines", () => {
    expect(hunkLines("@@ -1 +1 @@\n-old\n+new\n same\n").map(l => l.kind)).toEqual(["meta", "del", "add", "ctx"]);
  });

  it("says a conflict hook failed with its event and why, and nothing when it ran fine or never ran", () => {
    const hook = { event: "detected" as const, ok: false, error: "exit code 2", note: null, status: null, at: at(1) };
    expect(hookFailure(conflict("c", { hook }))).toBe("Conflict hook failed (detected): exit code 2");
    expect(hookFailure(conflict("c", { hook: { ...hook, ok: true, error: null } }))).toBeNull();
    expect(hookFailure(conflict("c"))).toBeNull();
  });
});

describe("the settings", () => {
  const draft = (command: string, timeout = "10", delivery: ConflictDelivery = "terminal") => ({ delivery, command, timeout });

  it("fills the form from the saved settings: the command as JSON, the timeout, and 10 seconds without a hook", () => {
    expect(settingsDraft({ delivery: "chat", hook: { command: ["./hook.sh", "--quiet"], timeoutSeconds: 30 } }))
      .toEqual({ delivery: "chat", command: "[\"./hook.sh\",\"--quiet\"]", timeout: "30" });
    expect(settingsDraft({ delivery: "both", hook: null })).toEqual({ delivery: "both", command: "", timeout: "10" });
    expect(settingsSummary({ delivery: "terminal", hook: { command: ["./hook.sh"], timeoutSeconds: 10 } })).toBe("Settings: delivery terminal, conflict hook: ./hook.sh");
    expect(settingsSummary({ delivery: "both", hook: null })).toBe("Settings: delivery both, no conflict hook");
  });

  it("saves the delivery and the hook command as an argv with its timeout; an empty command removes the hook", () => {
    expect(settingsChange(draft(" [\"./hook.sh\", \"a b\"] ", "45"))).toEqual({ body: { delivery: "terminal", hook: { command: ["./hook.sh", "a b"], timeoutSeconds: 45 } } });
    expect(settingsChange(draft("[\"./hook.sh\"]", " "))).toEqual({ body: { delivery: "terminal", hook: { command: ["./hook.sh"], timeoutSeconds: 10 } } });
    expect(settingsChange(draft("  ", "nonsense", "chat"))).toEqual({ body: { delivery: "chat", hook: null } });
  });

  it("refuses a command that is not a JSON array of non-empty strings, and a timeout outside 1 to 60 seconds", () => {
    const argv = "The hook command must be a JSON array of non-empty strings, like [\"./scripts/on-conflict.sh\", \"--quiet\"].";
    for (const command of ["./hook.sh --quiet", "\"./hook.sh\"", "[]", "[\"\"]", "[\"./hook.sh\", 3]", "{\"command\":[\"x\"]}"]) {
      expect(settingsChange(draft(command))).toEqual({ problem: argv, field: "command" });
    }
    expect(settingsChange(draft(JSON.stringify(Array.from({ length: 65 }, () => "x"))))).toEqual({ problem: "The hook command takes at most 64 arguments.", field: "command" });
    for (const timeout of ["0", "61", "2.5", "ten"]) {
      expect(settingsChange(draft("[\"./hook.sh\"]", timeout))).toEqual({ problem: "The timeout must be a whole number of seconds from 1 to 60.", field: "timeout" });
    }
  });
});
