/**
 * typescript-eslint-no-extraneous-class must not steer a class that carries a decorator (#247).
 *
 * typescript-eslint 8.70.1 reports a decorated class like any other unless `allowWithDecorator` is set, and its default
 * is `false`. A decorated class is registered with a framework (Angular's `@Component`, `@NgModule`, `@Injectable`,
 * NestJS, TypeORM), which needs the class to exist even when its body is empty or holds only a constructor, so none of
 * the rule's three steers can be followed there. The port's `allowWithDecorator` defaults to `true`; set to `false`
 * it reports what upstream reports at its default.
 *
 * The decorators of an exported class stand before `export` (`@NgModule({}) export class AppModule {}`). In ESTree they
 * belong to the class; in tree-sitter they are children of the `export_statement`, so the port looks there too, as
 * upstream does with `allowWithDecorator: true`.
 *
 * Every expected position below is what typescript-eslint 8.70.1 reports on this source with the same option value.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "typescript-eslint-no-extraneous-class";
const written = await readFile(repositoryPath("packs", "typescript", "rules", `${RULE_ID}.json`), "utf8");

const documentSchema = z.object({ clauses: z.array(z.object({ id: z.string(), define: z.record(z.string(), z.unknown()).optional() }).passthrough()) }).passthrough();
const evaluationSchema = z.object({
  violations: z
    .array(z.object({ span: z.object({ startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

/** The rule compiled with `config` as its options' values (none: the document's defaults). */
function compile(config: Record<string, unknown> = {}): NativeCompiledCodeRuleDsl {
  if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
  const document = documentSchema.parse(JSON.parse(written));
  const options = document.clauses.find(clause => clause.id === "options");
  if (options?.define === undefined) throw new Error(`${RULE_ID} has no options define`);
  options.define.config = config;
  return runtime.compile(JSON.stringify(document), "json", capabilities);
}

/** The sorted 1-based line:column positions the rule reports in `source`, as ESLint prints them. */
function reported(compiled: NativeCompiledCodeRuleDsl, source: string): string[] {
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language: "typescript", path: "src/app/app.component.ts", source }],
  });
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations
    .map(violation => [violation.span.startLine + 1, violation.span.startColumn + 1] as const)
    .sort((a, b) => a[0] - b[0] || a[1] - b[1])
    .map(([line, column]) => `${line}:${column}`);
}

const lines = (...source: string[]) => [...source, ""].join("\n");

const REPRO = lines(
  "@Component({ standalone: true, template: `<span>…</span>`, changeDetection: ChangeDetectionStrategy.OnPush })",
  "class Test {}",
);

/** Decorated classes: empty, constructor-only and static-only; exported, default-exported, abstract, an expression. */
const DECORATED = lines(
  "@Component({ standalone: true })",
  "class Test {}",
  "@NgModule({ imports: [] })",
  "export class AppModule {}",
  "@Injectable()",
  "export class Loader {",
  "  constructor(http: HttpClient) {",
  "    http.get('/');",
  "  }",
  "}",
  "@Injectable() export default class Store {}",
  "@Entity() abstract class Base {}",
  "@Pipe({ name: 'p' }) class Tokens { static readonly key = 'k'; }",
  "export @Directive() class AfterExport {}",
  "const Expr = @Injectable() class {};",
);
/** Where upstream reports DECORATED at its default (`allowWithDecorator: false`). */
const DECORATED_AT_UPSTREAM_DEFAULT = ["2:7", "4:14", "6:14", "11:36", "12:26", "13:28", "14:27", "15:14"];

/** Classes upstream reports whatever `allowWithDecorator` is: no decorator on the class itself. */
const UNDECORATED = lines(
  "class Empty {}",
  "export class Ctor {",
  "  constructor() {}",
  "}",
  "export class Statics {",
  "  @Memo() static total() { return 0; }",
  "}",
);
const UNDECORATED_REPORTED = ["1:7", "2:14", "5:14"];

/** Classes no option value makes upstream report: subclasses, and a decorated class with an instance member. */
const NEVER = lines(
  "@Component({}) class Child extends Parent {}",
  "@Injectable() export class Service { load() { return 1; } }",
  "class Plain extends Base {}",
);

const EVERY_CONFIG = [{}, { allowWithDecorator: true }, { allowWithDecorator: false }];

describe(RULE_ID, () => {
  it("does not report an empty class carrying a framework decorator (issue repro)", () => {
    expect(reported(compile(), REPRO)).toEqual([]);
  });

  it("does not report decorated classes, exported or not, at its default", () => {
    expect(reported(compile(), DECORATED)).toEqual([]);
  });

  it("does not report decorated classes with allowWithDecorator true, a decorator before export included", () => {
    expect(reported(compile({ allowWithDecorator: true }), DECORATED)).toEqual([]);
  });

  it("reports decorated classes as upstream's default does with allowWithDecorator false", () => {
    expect(reported(compile({ allowWithDecorator: false }), REPRO)).toEqual(["2:7"]);
    expect(reported(compile({ allowWithDecorator: false }), DECORATED)).toEqual(DECORATED_AT_UPSTREAM_DEFAULT);
  });

  it("still reports undecorated empty, constructor-only and static-only classes, a decorated member not counting", () => {
    for (const config of EVERY_CONFIG) expect(reported(compile(config), UNDECORATED)).toEqual(UNDECORATED_REPORTED);
  });

  it("still leaves subclasses and a class with an instance member alone", () => {
    for (const config of EVERY_CONFIG) expect(reported(compile(config), NEVER)).toEqual([]);
  });
});
