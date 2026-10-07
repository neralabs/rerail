import { mkdirSync } from "node:fs";
import { join } from "node:path";
import { defaultRerailDatabasePath } from "../src/database.js";
import { openScopedRuleCatalog } from "../src/scoped-rule-catalog.js";

/**
 * The example executable policy the kernel/harness contract tests exercise: an api-scoped rule that flags
 * `cursor.execute(` with the CURSOR_EXECUTE code the assertions look for. The tests own it rather than
 * depending on which rules ship as bundled defaults; its compliant form (a transaction helper) carries no
 * `cursor.execute(`, so it isolates the mechanism these tests measure.
 */
export const CONTRACT_CURSOR_EXECUTE_POLICY = {
  format: "rerail.policy/v1",
  id: "contract:cursor-execute",
  revision: 1,
  mode: "steer",
  scope: { include: ["api/**/*"], exclude: [] },
  clauses: [
    {
      id: "cursor-execute",
      forbid: { regex: { pattern: "cursor\\.execute\\s*\\(", source: "file.content" } },
      violation: { anchor: "witness.span", code: "CURSOR_EXECUTE", message: "Direct cursor access bypasses the typed API and backend portability." },
    },
  ],
};

/**
 * Seed the contract policy into `<rerailHome>/rerail.sqlite` for `workspaceRoot`, before a runtime or harness
 * seeds the bundled defaults. `seed()` keeps this id (it is not bundled), so it survives that pass.
 */
export function seedContractPolicy(rerailHome: string, workspaceRoot: string): void {
  mkdirSync(rerailHome, { recursive: true });
  const catalog = openScopedRuleCatalog(workspaceRoot, defaultRerailDatabasePath({ ...process.env, RERAIL_HOME: rerailHome }));
  try {
    // Global applicability so the rule applies whatever repository context a runtime, daemon, or harness
    // resolves for its workspace — the contract tests use several, and a global rule sidesteps matching them.
    catalog.upsertPolicy(CONTRACT_CURSOR_EXECUTE_POLICY, { applicability: "global", enabled: true });
  } finally {
    catalog.close();
  }
}

/** Convenience for the common layout where `RERAIL_HOME` is `<root>/home` and the workspace is `<root>`. */
export function seedContractPolicyForRoot(root: string): void {
  seedContractPolicy(join(root, "home"), root);
}

/** Seed into an explicit home (e.g. a shared isolated RERAIL_HOME) for a given workspace root. */
export function seedContractPolicyForHome(rerailHome: string, workspaceRoot: string): void {
  seedContractPolicy(rerailHome, workspaceRoot);
}
