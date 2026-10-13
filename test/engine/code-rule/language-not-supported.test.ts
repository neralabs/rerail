import { mkdir, mkdtemp, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi, type MockInstance } from "vitest";
import { execFileSync } from "node:child_process";
import { createRerailApi, operations, DatabaseApiStores } from "../../../src/api/index.js";
import { NativeCodeRuleEnforcer } from "../../../src/engine/code-rule/code-rule-enforcer.js";
import type { AuditInput, AuditRecorder, AuditResult } from "../../../src/engine/audit/audit.js";
import { CheckoutCatalogs } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { assertCodeRuleCompiles, CodeRuleCompileError } from "../../../src/engine/code-rule/code-rule-compile.js";
import {
  LANGUAGE_NOT_SUPPORTED,
  codeRuleLanguageNotSupported,
} from "../../../src/engine/code-rule/code-rule-language-support.js";
import { EXEMPLAR } from "../support/exemplar-registry.js";
import { codeRuleSchema, type CodeRule } from "../../../src/engine/code-rule/code-rule-schema.js";
import { setUpTestRepository, testCodeRuleCatalog } from "../../support/repository-rules.js";

// The exemplar is a language only tests plan: the registry module lists it for this file, and the engine refuses it as a
// planned language (test/engine/support/exemplar-registry.ts, exemplar-engine.ts).
vi.mock("../../../src/engine/code-rule/code-rule-language-registry.generated.js", async () =>
  (await import("../support/exemplar-registry.js")).exemplarRegistryModule());
vi.mock("../../../src/engine/native/native.js", async importOriginal =>
  (await import("../support/exemplar-engine.js")).nativeRefusingExemplar(await importOriginal()));

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

/** A committed Git repository at `path`. */
function gitRepository(path: string): void {
  execFileSync("git", ["init", "--quiet", path], { stdio: "ignore" });
  execFileSync("git", ["-C", path, "-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "--allow-empty", "-m", "seed"], { stdio: "ignore" });
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
  it("is a valid document that compiles to language-not-supported for the exemplar, in the host", () => {
    const language = EXEMPLAR;
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
      clauses: [{ id: "d", define: { language: EXEMPLAR, utils: { u: { kind: "identifier" } } } }, astCodeRule("x", "python").clauses[0]],
    });
    expect(codeRuleLanguageNotSupported(rule)).toEqual({ code: LANGUAGE_NOT_SUPPORTED, reason: `/clauses/0/define/language: \`${EXEMPLAR}\` is not supported yet` });
    expect(codeRuleLanguageNotSupported(astCodeRule("supported", "typescript"))).toBeUndefined();
    expect(() => assertCodeRuleCompiles(astCodeRule("supported", "typescript"))).not.toThrow();
  });

  it("is stored in the library, and the checkout's catalog keeps it inactive with its reason, unreported", () => {
    const workspace = join(root, "workspace");
    gitRepository(workspace);
    const databasePath = join(root, "rerail.sqlite");
    setUpTestRepository(workspace, {
      library: [{ kind: "code", rule: astCodeRule(`planned:${EXEMPLAR}`, EXEMPLAR) }, { kind: "code", rule: astCodeRule("supported:python", "python") }],
    }, databasePath);
    const catalogs = new CheckoutCatalogs(databasePath);
    try {
      expect(catalogs.refresh(workspace)).toBe("ok");
      const catalog = catalogs.catalog(workspace);
      expect(catalog.snapshot().codeRules.map(entry => entry.id)).toEqual(["supported:python"]);
      expect(catalog.quarantined()).toEqual([
        { ruleId: `planned:${EXEMPLAR}`, kind: "code", owner: "library", reason: reason(EXEMPLAR), code: LANGUAGE_NOT_SUPPORTED },
      ]);
      expect(warnings).not.toHaveBeenCalled();
    } finally {
      catalogs.close();
    }
  });

  it("is shown inactive, with its reason, by the library, repository rule and activity APIs", async () => {
    const workspace = join(root, "workspace");
    gitRepository(workspace);
    const databasePath = join(root, "rerail.sqlite");
    setUpTestRepository(workspace, {}, databasePath);
    const stores = new DatabaseApiStores(databasePath, {
      health: () => ({ pid: process.pid, database: { path: databasePath, exists: true }, native: { ok: true } }),
      origin: () => "http://127.0.0.1:4737",
    } as unknown as ConstructorParameters<typeof DatabaseApiStores>[1]);
    try {
      const app = createRerailApi(stores, operations, { version: "0.0.0-test" });
      const inactive = { code: LANGUAGE_NOT_SUPPORTED, reason: reason(EXEMPLAR) };
      const put = async (rule: CodeRule) => app.request(`/api/library-rules/${rule.id}`, {
        method: "PUT",
        headers: { "content-type": "application/json", host: "127.0.0.1:4737" },
        body: JSON.stringify({ kind: "code", rule, checkout: workspace, attach: true }),
      });
      const stored = await put(astCodeRule(`planned:${EXEMPLAR}`, EXEMPLAR));
      expect(stored.status).toBe(200);
      expect(((await stored.json()) as { rule: { inactive?: unknown } }).rule.inactive).toEqual(inactive);
      expect((await put(astCodeRule("supported:python", "python"))).status).toBe(200);

      const get = async <T>(path: string) => (await (await app.request(path, { headers: { host: "127.0.0.1:4737" } })).json()) as T;
      const library = await get<{ rules: { rule: { id: string }; inactive?: unknown }[]; quarantined: unknown[] }>("/api/library-rules");
      expect(library.rules.map(entry => [entry.rule.id, entry.inactive])).toEqual([[`planned:${EXEMPLAR}`, inactive], ["supported:python", undefined]]);
      expect(library.quarantined).toEqual([]);
      const effective = await get<{ rules: { rule: { id: string }; inactive?: unknown }[] }>(`/api/repository/rules?${new URLSearchParams({ checkout: workspace })}`);
      expect(effective.rules.map(entry => [entry.rule.id, entry.inactive]).toSorted()).toEqual([[`planned:${EXEMPLAR}`, inactive], ["supported:python", undefined]]);
      const activity = await get<{ rules: Record<string, { measured: boolean; inactive?: unknown }> }>("/api/rules/activity?since=0");
      expect(activity.rules[`planned:${EXEMPLAR}`]).toEqual(expect.objectContaining({ measured: false, inactive }));
      expect(activity.rules["supported:python"]).toEqual(expect.objectContaining({ measured: true }));
      expect(activity.rules["supported:python"]).not.toHaveProperty("inactive");
    } finally {
      stores.close();
    }
  });

  it("makes a repository audit read .exemplar files as text: a planned language's rules evaluate nothing, a regex rule reads them, and nothing fails", async () => {
    const workspaceRoot = join(root, "workspace");
    for (const [path, text] of [["ui/app.exemplar", "name a = 1\n"], ["db/schema.exemplar", "select a from b;\n"]] as const) {
      await mkdir(dirname(join(workspaceRoot, path)), { recursive: true });
      await writeFile(join(workspaceRoot, path), text);
    }
    const catalog = testCodeRuleCatalog([
      astCodeRule(`planned:${EXEMPLAR}`, EXEMPLAR),
      astCodeRule("supported:python", "python"),
      codeRuleSchema.parse({
        format: "rerail.policy/v1", id: "text:a", revision: 1, mode: "steer",
        scope: { include: ["**/*"], exclude: [] },
        clauses: [{ id: "a", violation: { code: "A", anchor: "witness.span", message: "an a" }, forbid: { regex: { source: "file.content", pattern: "\\ba\\b" } } }],
      }),
    ]);
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
      const result = audits.get(auditId!)?.result;
      expect(result).toEqual(expect.objectContaining({ status: "completed", failures: [] }));
      expect((result?.violations ?? []).map(violation => [violation.path, violation.ruleId]).sort()).toEqual([["db/schema.exemplar", "text:a"], ["ui/app.exemplar", "text:a"]]);
      expect([...audits.values()].flatMap(audit => audit.result?.failures ?? [])).toEqual([]);
      expect(enforcer.snapshotStatus().failures).toEqual([]);
    } finally {
      await enforcer.close();
    }
  });
});
