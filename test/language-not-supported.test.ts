import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { createRerailApi } from "../src/api/http.js";
import { operations, type RerailApiStores } from "../src/api/operations.js";
import { NativeCodeRuleEnforcer } from "../src/code-rule-enforcer.js";
import type { AuditInput, AuditRecorder, AuditResult } from "../src/audit.js";
import { bootstrapCodeRuleDslCatalog } from "../src/code-rule-dsl-catalog.js";
import { assertCodeRuleCompiles, CodeRuleCompileError } from "../src/code-rule-compile.js";
import { LANGUAGE_NOT_SUPPORTED, codeRuleLanguageNotSupported } from "../src/code-rule-language-support.js";
import { CODE_RULE_PLANNED_LANGUAGES } from "../src/code-rule-languages.js";
import { codeRuleSchema, type CodeRule } from "../src/code-rule-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";

/** One forbid clause over an ast query of `language`, reporting every identifier of every file. */
function astCodeRule(id: string, language: string): CodeRule {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1", id, revision: 1, mode: "steer",
    scope: { include: ["**/*"], exclude: [] },
    clauses: [{
      id: "any",
      violation: { code: "ANY", anchor: "witness.span", message: "an identifier" },
      forbid: { ast: { language, rule: { kind: "identifier" } } },
    }],
  });
}

const reason = (language: string) => `/clauses/0/forbid/ast/language: \`${language}\` is not supported yet`;

let root: string;
let warnings: MockInstance<typeof process.stderr.write>;

beforeEach(async () => {
  root = await realpath(await mkdtemp(join(tmpdir(), "rerail-planned-")));
  warnings = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
});

afterEach(async () => {
  warnings.mockRestore();
  await rm(root, { recursive: true, force: true });
});

describe("a rule for a planned language", () => {
  it.each(CODE_RULE_PLANNED_LANGUAGES)("is a valid document that compiles to language-not-supported for %s, in the engine and the host alike", language => {
    const rule = astCodeRule(`planned:${language}`, language);
    let refused: unknown;
    try {
      assertCodeRuleCompiles(rule);
    } catch (error) {
      refused = error;
    }
    expect(refused).toBeInstanceOf(CodeRuleCompileError);
    expect(refused).toMatchObject({ code: LANGUAGE_NOT_SUPPORTED, reason: `${LANGUAGE_NOT_SUPPORTED}: ${reason(language)}` });
    expect(codeRuleLanguageNotSupported(rule)).toEqual({ code: LANGUAGE_NOT_SUPPORTED, reason: reason(language) });
  });

  it("is reported for a define of a planned language, and never for a supported one", () => {
    const rule = codeRuleSchema.parse({
      ...astCodeRule("planned:define", "python"),
      clauses: [{ id: "d", define: { language: "vue", utils: { u: { kind: "identifier" } } } }, astCodeRule("x", "python").clauses[0]],
    });
    expect(codeRuleLanguageNotSupported(rule)).toEqual({ code: LANGUAGE_NOT_SUPPORTED, reason: "/clauses/0/define/language: `vue` is not supported yet" });
    expect(codeRuleLanguageNotSupported(astCodeRule("supported", "typescript"))).toBeUndefined();
    expect(() => assertCodeRuleCompiles(astCodeRule("supported", "typescript"))).not.toThrow();
  });

  it("is stored, and the catalog keeps it inactive with its reason, unreported", () => {
    const databasePath = join(root, "code-rules.sqlite");
    const store = new SqliteRuleStore(databasePath);
    store.upsertCodeRule(astCodeRule("planned:vue", "vue"));
    store.upsertCodeRule(astCodeRule("supported:python", "python"));
    expect(store.list({ kind: "file" }).map(stored => stored.rule.id).toSorted()).toEqual(["planned:vue", "supported:python"]);
    store.close();
    const catalog = bootstrapCodeRuleDslCatalog({ databasePath });
    try {
      expect(catalog.codeRules().map(entry => entry.id)).toEqual(["supported:python"]);
      expect(catalog.quarantined()).toEqual([
        { ruleId: "planned:vue", kind: "file", owner: "project:unknown", reason: reason("vue"), code: LANGUAGE_NOT_SUPPORTED },
      ]);
      expect(warnings).not.toHaveBeenCalled();
    } finally {
      catalog.close();
    }
  });

  it("is shown inactive, with its reason, by the rule and activity APIs", async () => {
    const catalog = ScopedRuleCatalog.open(join(root, "catalog.sqlite"), { repositoryId: "repo", workspaceRoot: root });
    try {
      const app = createRerailApi({ rules: () => catalog, audits: { ruleActivity: () => new Map() }, reader: { steersByRule: () => new Map() } } as unknown as RerailApiStores, operations, { version: "0.0.0-test" });
      const inactive = { code: LANGUAGE_NOT_SUPPORTED, reason: reason("sql") };
      const put = await app.request("/api/rules/planned:sql", {
        method: "PUT",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ kind: "file", rule: astCodeRule("planned:sql", "sql") }),
      });
      expect(put.status).toBe(200);
      expect(((await put.json()) as { rule: { inactive?: unknown } }).rule.inactive).toEqual(inactive);
      catalog.upsertCodeRule(astCodeRule("supported:python", "python"));

      const listed = (await (await app.request("/api/rules")).json()) as { rules: { rule: { id: string }; inactive?: unknown }[]; quarantined: unknown[] };
      expect(listed.rules.map(stored => [stored.rule.id, stored.inactive])).toEqual([["planned:sql", inactive], ["supported:python", undefined]]);
      expect(listed.quarantined).toEqual([]);
      const fetched = (await (await app.request("/api/rules/planned:sql")).json()) as { rule: { inactive?: unknown } };
      expect(fetched.rule.inactive).toEqual(inactive);
      const activity = (await (await app.request("/api/rules/activity?since=0")).json()) as { rules: Record<string, { measured: boolean; inactive?: unknown }> };
      expect(activity.rules["planned:sql"]).toEqual(expect.objectContaining({ measured: false, inactive }));
      expect(activity.rules["supported:python"]).toEqual(expect.objectContaining({ measured: true }));
      expect(activity.rules["supported:python"]).not.toHaveProperty("inactive");
    } finally {
      catalog.close();
    }
  });

  it("makes a repository audit over .vue, .sql and .rs files evaluate nothing and fail nothing", async () => {
    const workspaceRoot = join(root, "workspace");
    for (const [path, text] of [["ui/App.vue", "<script>const a = 1</script>\n"], ["db/schema.sql", "select a from b;\n"], ["src/main.rs", "fn main() { let a = 1; }\n"]] as const) {
      await mkdir(dirname(join(workspaceRoot, path)), { recursive: true });
      await writeFile(join(workspaceRoot, path), text);
    }
    const databasePath = join(root, "code-rules.sqlite");
    const store = new SqliteRuleStore(databasePath);
    for (const language of CODE_RULE_PLANNED_LANGUAGES) store.upsertCodeRule(astCodeRule(`planned:${language}`, language));
    store.upsertCodeRule(astCodeRule("supported:python", "python"));
    store.close();
    const catalog = bootstrapCodeRuleDslCatalog({ databasePath });
    const audits = new Map<string, AuditInput & { result?: AuditResult }>();
    const recorder: AuditRecorder = {
      start(input) { const auditId = `audit-${audits.size + 1}`; audits.set(auditId, { ...input }); return auditId; },
      finish(auditId, result) { audits.get(auditId)!.result = result; },
      progress(auditId, result) { audits.get(auditId)!.result = { status: "completed", ...result }; },
      interrupt() {},
    };
    const enforcer = new NativeCodeRuleEnforcer(catalog, { workspaceRoot, steerDelivery: { requestSteer: async () => undefined }, audits: recorder });
    try {
      await enforcer.openWorkspace("planned");
      const [auditId] = await enforcer.audit("planned", { scope: "repository", mode: "log" });
      expect(audits.get(auditId!)?.result).toEqual(expect.objectContaining({ status: "completed", violations: [], failures: [] }));
      expect([...audits.values()].flatMap(audit => audit.result?.failures ?? [])).toEqual([]);
      expect(enforcer.snapshotStatus().failures).toEqual([]);
    } finally {
      await enforcer.close();
      catalog.close();
    }
  });
});
