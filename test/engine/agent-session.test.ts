import { execFileSync } from "node:child_process";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { AGENT_SESSION_VARIABLES, agentSession, runRuleApproveCommand, runRulesCommand } from "../../src/cli/index.js";
import { CLEARED_AGENT_SESSION_VARIABLES } from "../support/environment/agent-session-variables.js";
import { defaultRerailDatabasePath, loadRuleCatalog, openRerailDatabase, recordCheckout, setUpRepository, upsertLibraryRule } from "../../src/engine/index.js";
import { removeFolder } from "../support/platform/remove-folder.js";

const folders: string[] = [];

afterEach(async () => {
  await Promise.all(folders.splice(0).map(folder => removeFolder(folder)));
});

async function repository(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-agent-session-"));
  folders.push(root);
  execFileSync("git", ["init", "-q", "-b", "main"], { cwd: root });
  return root;
}

describe("a coding agent's session", () => {
  it("is read from the variable each coding agent sets in its shell", () => {
    expect(agentSession({ CLAUDECODE: "1" })).toEqual({ agent: "Claude Code", variable: "CLAUDECODE" });
    expect(agentSession({ CODEX_THREAD_ID: "019a-thread" })).toEqual({ agent: "Codex", variable: "CODEX_THREAD_ID" });
    expect(agentSession({ CURSOR_AGENT: "1" })).toEqual({ agent: "Cursor", variable: "CURSOR_AGENT" });
    expect(agentSession({ PI_CODING_AGENT: "true", AI_AGENT: "pi" })).toEqual({ agent: "Pi", variable: "PI_CODING_AGENT" });
    expect(agentSession({ AI_AGENT: "some-agent" })).toEqual({ agent: "a coding agent", variable: "AI_AGENT" });
  });

  it("is cleared for every test, so the suite reads as a person's terminal whoever runs it", () => {
    expect(CLEARED_AGENT_SESSION_VARIABLES).toEqual(AGENT_SESSION_VARIABLES);
    expect(agentSession()).toBeUndefined();
  });

  it("is not a person's terminal, where none is set or one is empty", () => {
    expect(agentSession({})).toBeUndefined();
    expect(agentSession({ CLAUDECODE: "", AI_AGENT: "" })).toBeUndefined();
    // The integrated terminal of Cursor's editor sets this for a person too.
    expect(agentSession({ CURSOR_TRACE_ID: "abc" })).toBeUndefined();
  });

  it("is refused rules approve of a named rule or pack, and told where a person approves it, but lists what waits", async () => {
    const repo = await repository();
    // A rule created by you, added here and not approved, so it waits.
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      upsertLibraryRule(database, loadRuleCatalog(), {
        format: "rerail.policy/v1", id: "acme:no-console-log", revision: 1, mode: "steer", title: "No console.log",
        scope: { include: ["src/**/*.ts"], exclude: [] },
        clauses: [{ id: "c", forbid: { regex: { pattern: String.raw`console\.log`, source: "file.content" } }, violation: { anchor: "witness.span", code: "X", message: "Use the logger." } }],
      }, "code");
      setUpRepository(database, recordCheckout(database, repo).repositoryKey, { packs: [], rules: [{ sourceKind: "library_rule", sourceId: "acme:no-console-log", folder: "" }] });
    } finally {
      database.$client.close();
    }
    let listed = "";
    const write = { stdout: { write: (text: string) => (listed += text) } };

    expect(() => runRuleApproveCommand(["acme:no-console-log", "--repo", repo], () => "usage", write, undefined, { CLAUDECODE: "1" })).toThrow(
      "rules approve is a person's, and Claude Code runs this command (CLAUDECODE is set). A person reads and approves acme:no-console-log on its "
        + "page in the dashboard, which rerail manage opens, or with rerail rules approve in a terminal of their own.",
    );
    expect(() => runRuleApproveCommand(["a", "b", "--repo", repo], () => "usage", write, undefined, { AI_AGENT: "some-agent" })).toThrow(
      "rules approve is a person's, and a coding agent runs this command (AI_AGENT is set). A person reads and approves a, b on their page",
    );
    expect(runRuleApproveCommand(["--repo", repo], () => "usage", write, undefined, { CLAUDECODE: "1" })).toBe(0);
    expect(listed).toBe([
      "Waiting for approval; each runs in Log, and steers no one, until a person approves it:",
      "  rule acme:no-console-log  (created by you)",
      "A person reads each and approves it on its page in the dashboard, which rerail manage opens.",
      "",
    ].join("\n"));
  });

  it("is refused rules import, which replaces your rules, approved ones included", () => {
    expect(() => runRulesCommand(["import", "rules.json", "--approve"], () => "usage", {}, { CURSOR_AGENT: "1" })).toThrow(
      "rules import is a person's, and Cursor runs this command (CURSOR_AGENT is set): it replaces your rules, approved ones included. Run it in a terminal of your own.",
    );
  });
});
