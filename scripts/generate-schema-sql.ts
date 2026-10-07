import { writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { generateSQLiteDrizzleJson, generateSQLiteMigration } from "drizzle-kit/api";
import {
  checkoutStarts,
  catalogRules,
  events,
  presentationPreferences,
  policyRepositoryDiagnostics,
  policyRunResultPages,
  policyRuns,
  policyScanRequests,
  policySessionOwners,
  policySettings,
  ruleCatalogIdentities,
  ruleProjectRepositories,
  ruleProjects,
  ruleRepositories,
  repositoryStarts,
  rules,
  sessions,
  workLinks,
} from "../src/schema.js";

export const SCHEMA_SQL_PATH = resolve(fileURLToPath(import.meta.url), "..", "..", "src", "schema-sql.ts");

/**
 * Render the DDL for the Drizzle tables in `src/schema.ts` as an idempotent
 * statement list, applied verbatim every time a database is opened.
 */
export async function renderRerailSchemaStatements(): Promise<string[]> {
  const empty = await generateSQLiteDrizzleJson({});
  // Explicit order: module namespace key order differs between loaders.
  const current = await generateSQLiteDrizzleJson({
  sessions,
  events,
  rules,
  workLinks,
  policyRuns,
  policyRunResultPages,
  presentationPreferences,
  policySettings,
  policyRepositoryDiagnostics,
  policyScanRequests,
  policySessionOwners,
  ruleProjects,
  ruleRepositories,
  repositoryStarts,
  checkoutStarts,
  ruleCatalogIdentities,
  ruleProjectRepositories,
  catalogRules,
  });
  const statements = await generateSQLiteMigration(empty, current);
  return statements.map(statement =>
    statement
      .trim()
      .replace(/^CREATE TABLE `/u, "CREATE TABLE IF NOT EXISTS `")
      .replace(/^CREATE (UNIQUE )?INDEX `/u, "CREATE $1INDEX IF NOT EXISTS `")
      .replace(/;$/u, ""),
  );
}

export async function renderRerailSchemaModule(): Promise<string> {
  const statements = await renderRerailSchemaStatements();
  return [
    "// Generated from src/schema.ts by `npm run generate:schema`. Do not edit.",
    "",
    "/** DDL statements applied, in order, when a Rerail database is opened. */",
    "export const RERAIL_SCHEMA_STATEMENTS: readonly string[] = [",
    ...statements.map(statement => `  ${JSON.stringify(statement)},`),
    "];",
    "",
    "export const RERAIL_SCHEMA_SQL = `${RERAIL_SCHEMA_STATEMENTS.join(\";\\n\\n\")};\\n`;",
    "",
  ].join("\n");
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  writeFileSync(SCHEMA_SQL_PATH, await renderRerailSchemaModule());
  process.stdout.write(`Wrote ${SCHEMA_SQL_PATH}\n`);
}
