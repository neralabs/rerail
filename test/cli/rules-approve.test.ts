import { describe, expect, it } from "vitest";
import { runRulesCommand } from "../../src/cli/project-cli.js";
import { defaultRerailDatabasePath, getLibraryRule, loadRuleCatalog, openRerailDatabase, upsertLibraryRule, type CodeRule } from "../../src/engine/index.js";
import { codeRule } from "../support/rule-configuration-builders.js";

const usage = () => "usage";

/** Runs `rerail rules <args>` against the test file's Rerail home and returns what it printed. */
function rules(...args: string[]): string {
  let printed = "";
  runRulesCommand(args, usage, { stdout: { write: text => { printed += text; } } });
  return printed;
}

function propose(rule: CodeRule, approve = false): void {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    upsertLibraryRule(database, loadRuleCatalog(), rule, "code", { approve });
  } finally {
    database.$client.close();
  }
}

function contentOf(id: string): string {
  const database = openRerailDatabase(defaultRerailDatabasePath());
  try {
    return JSON.stringify(getLibraryRule(database, id)?.document.rule);
  } finally {
    database.$client.close();
  }
}

describe("rerail rules list and approve", () => {
  it("lists a proposed rule, shows its whole content when approving it, and lists nothing after", () => {
    // A zero-width space hidden in the message must reach the person as an escape they can see.
    propose(codeRule("cli-proposed", { message: "Use the logger\u200b, not print" }));

    expect(rules("list")).toMatch(/cli-proposed .*created by you.*new/u);
    const shown = rules("approve", "cli-proposed");
    expect(shown).toContain("Use the logger\\u200b, not print");
    expect(shown).toMatch(/Approved cli-proposed/u);
    expect(rules("list")).toMatch(/No rules wait for approval/u);
  });

  it("names what an edit changes against the approved content", () => {
    propose(codeRule("cli-edited", { message: "approved words" }), true);
    propose(codeRule("cli-edited", { message: "edited words" }));

    expect(rules("list")).toMatch(/cli-edited .*edited: the approved content runs meanwhile/u);
    const shown = rules("approve", "cli-edited");
    expect(shown).toContain("edited words");
    expect(shown).toMatch(/changing: clauses/u);
    expect(contentOf("cli-edited")).toContain("edited words");
  });

  it("refuses to approve a rule that has nothing waiting", () => {
    propose(codeRule("cli-settled"), true);
    expect(() => rules("approve", "cli-settled")).toThrow(/nothing of rule cli-settled waits for approval/u);
    expect(() => rules("approve", "cli-unknown")).toThrow(/waits for approval/u);
  });
});
