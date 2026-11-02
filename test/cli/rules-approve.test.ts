import { writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { describe, expect, it } from "vitest";
import { runRulesCommand } from "../../src/cli/project-cli.js";
import { BLOCK_STEERS_NOTE } from "../../src/engine/index.js";
import {
  defaultRerailDatabasePath,
  getLibraryRule,
  listRepositoryRules,
  loadRuleCatalog,
  openRerailDatabase,
  upsertLibraryRule,
  type CodeRule,
} from "../../src/engine/index.js";
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

  it("says a rule on Block steers in this release, when listing and approving it", () => {
    propose(codeRule("cli-blocking", { mode: "block" }));

    expect(rules("list")).toContain(BLOCK_STEERS_NOTE);
    expect(rules("approve", "cli-blocking")).toContain(BLOCK_STEERS_NOTE);
    propose(codeRule("cli-steering", { mode: "steer" }));
    expect(rules("approve", "cli-steering")).not.toContain(BLOCK_STEERS_NOTE);
  });
});

describe("rerail rules import --approve", () => {
  const repositoryKey = "remote:github.com/acme/imported";

  /** A dump of the person's own: rules created by them, both added to a repository. */
  function dump(): string {
    const library = [codeRule("imp-new"), codeRule("imp-settled")].map(rule => ({
      kind: "code", rule, revision: rule.revision, copiedFromRuleId: null, createdAt: 1, updatedAt: 2,
    }));
    const attachments = library.map(({ rule }) => ({ folder: "", sourceKind: "library_rule", sourceId: rule.id }));
    const path = join(dirname(defaultRerailDatabasePath()), "imported-rules.json");
    writeFileSync(path, JSON.stringify({
      version: 1,
      exportedAt: new Date(0).toISOString(),
      library,
      repositories: [{ repositoryKey, displayName: "imported", aliases: [], configuredAt: 1, attachments, settings: [] }],
    }));
    return path;
  }

  function repositoryRules() {
    const database = openRerailDatabase(defaultRerailDatabasePath());
    try {
      return listRepositoryRules(database, loadRuleCatalog(), repositoryKey);
    } finally {
      database.$client.close();
    }
  }

  it("leaves imported rules waiting without it, and approves exactly the imported content that waits with it", () => {
    const file = dump();
    propose(codeRule("imp-settled"), true);
    // Proposed outside the dump: importing with --approve must not approve it.
    propose(codeRule("imp-other"));

    const plain = rules("import", file);
    expect(plain).not.toMatch(/Approved/u);
    expect(plain).toMatch(/wait for your approval/u);
    expect(repositoryRules().awaitingApproval).toBe(1);
    expect(repositoryRules().rules.map(rule => rule.document.rule.id)).toEqual(["imp-settled"]);

    const approved = rules("import", file, "--approve");
    expect(approved).toMatch(/Approved imp-new\b/u);
    expect(approved).not.toMatch(/Approved imp-settled/u);
    expect(approved).not.toMatch(/Approved imp-other/u);
    const view = repositoryRules();
    expect(view.awaitingApproval).toBe(0);
    expect(view.rules.map(rule => [rule.document.rule.id, rule.approval.state])).toEqual([["imp-new", "approved"], ["imp-settled", "approved"]]);
    expect(rules("list")).not.toMatch(/imp-new/u);
    expect(rules("list")).toMatch(/imp-other/u);
  });
});
