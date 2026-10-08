import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { defaultRerailDatabasePath } from "../src/database.js";
import { openScopedRuleCatalog } from "../src/scoped-rule-catalog.js";

/**
 * The example code rule the engine/host contract tests exercise: an api-scoped rule that flags
 * `cursor.execute(`; assertions identify its steer by the rule id in the steer header and by its message.
 * The tests own it rather than depending on which rules ship as bundled defaults; its compliant form (a
 * transaction helper) carries no `cursor.execute(`, so it isolates the mechanism these tests measure.
 */
export const CONTRACT_CURSOR_EXECUTE_RULE_ID = "contract:cursor-execute";
export const CONTRACT_CURSOR_EXECUTE_MESSAGE = "Direct cursor access bypasses the typed API and backend portability.";
export const CONTRACT_CURSOR_EXECUTE_CODE_RULE = {
  format: "rerail.policy/v1",
  id: CONTRACT_CURSOR_EXECUTE_RULE_ID,
  revision: 1,
  mode: "steer",
  scope: { include: ["api/**/*"], exclude: [] },
  clauses: [
    {
      id: "cursor-execute",
      forbid: { regex: { pattern: "cursor\\.execute\\s*\\(", source: "file.content" } },
      violation: { anchor: "witness.span", code: "CURSOR_EXECUTE", message: CONTRACT_CURSOR_EXECUTE_MESSAGE },
    },
  ],
};

/**
 * Seed the contract code rule into `<rerailHome>/rerail.sqlite` for `workspaceRoot`, before a runtime or host
 * seeds the bundled defaults. `seed()` keeps this id (it is not bundled), so it survives that pass.
 */
export function seedContractCodeRule(rerailHome: string, workspaceRoot: string): void {
  mkdirSync(rerailHome, { recursive: true });
  const catalog = openScopedRuleCatalog(workspaceRoot, defaultRerailDatabasePath({ ...process.env, RERAIL_HOME: rerailHome }));
  try {
    // Global applicability so the rule applies whatever repository context a runtime, daemon, or host
    // resolves for its workspace — the contract tests use several, and a global rule sidesteps matching them.
    catalog.upsertCodeRule(CONTRACT_CURSOR_EXECUTE_CODE_RULE, { applicability: "global", enabled: true });
  } finally {
    catalog.close();
  }
}

/** Convenience for the common layout where `RERAIL_HOME` is `<root>/home` and the workspace is `<root>`. */
export function seedContractCodeRuleForRoot(root: string): void {
  seedContractCodeRule(join(root, "home"), root);
}

/** Seed into an explicit home (e.g. a shared isolated RERAIL_HOME) for a given workspace root. */
export function seedContractCodeRuleForHome(rerailHome: string, workspaceRoot: string): void {
  seedContractCodeRule(rerailHome, workspaceRoot);
}
