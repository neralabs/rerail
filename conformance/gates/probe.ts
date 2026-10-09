/**
 * How check 8 finds the node a pattern's `context` selects: the engine reads the context as source and reports the
 * first node of the selector's kind. Ported from the rule validation ladder's V1 engine probe (nera-rule-validation
 * `src/validation/engine.ts`, `selectedText`), compiled as a `rerail.policy/v1` document in this checkout's engine.
 */
import { z } from "zod";
import { CODE_RULE_DIALECTS, codeRuleLanguageDialects, isCodeRuleAstLanguage } from "../../src/code-rule-languages.js";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../../src/native.js";
import { capabilitiesJson } from "../engine.js";

/**
 * The character the engine writes a metavariable's `$` with where `$` cannot start an identifier, as it reads a
 * pattern (native/src/code_rule_dsl/language.rs, from ast-grep-language 0.45's `impl_lang_expando!`). A context is
 * pattern text, so it is read the same way. TypeScript and JavaScript read `$A` as an identifier and need none. The
 * engine also rewrites php and blade patterns; no tool of theirs records source facts, so check 8 judges neither yet.
 */
const EXPANDO: Readonly<Record<string, string>> = { python: "µ" };

/**
 * The engine's `pre_process_pattern`: a run of `$` before `A`-`Z` or `_` (`$A`, `$$A`, `$$$A`), and a run of exactly
 * three (`$$$`), take the expando character, one for each `$`.
 */
export function withExpando(code: string, expando: string): string {
  let out = "";
  let dollars = 0;
  for (const character of code) {
    if (character === "$") {
      dollars += 1;
      continue;
    }
    out += (/[A-Z_]/u.test(character) || dollars === 3 ? expando : "$").repeat(dollars) + character;
    dollars = 0;
  }
  return out + (dollars === 3 ? expando : "$").repeat(dollars);
}

const evaluationSchema = z.object({
  violations: z.array(z.object({ span: z.object({ startByte: z.number().int().nonnegative(), endByte: z.number().int().nonnegative() }) })),
  failures: z.array(z.unknown()),
});

/**
 * The source text of the first (outermost first) node of kind `selector` in `code`, read as `language` reads its own
 * text, with metavariables spelled back with `$`. Undefined when the language is not one the engine reads, its grammar
 * has no such kind, the context does not parse, or nothing is selected: the gate then judges every word of the context.
 */
export function selectedText(code: string, selector: string, language: string): string | undefined {
  if (!isCodeRuleAstLanguage(language)) return undefined;
  const dialect = codeRuleLanguageDialects(language)[0]!;
  const probe = JSON.stringify({
    format: "rerail.policy/v1",
    id: "validation:selector-probe",
    revision: 1,
    mode: "log",
    scope: { include: ["**/*"], exclude: [] },
    clauses: [{ id: "selector", forbid: { ast: { language, rule: { kind: selector } } }, violation: { anchor: "witness.span", code: "SELECTOR", message: "selector" } }],
  });
  let compiled: NativeCompiledCodeRuleDsl;
  try {
    compiled = loadNativeBinding().CodeRuleDslRuntime.compile(probe, "json", capabilitiesJson());
  } catch (error) {
    // A selector the grammar has no node kind for selects nothing. Any other compile error means the probe no longer
    // fits the policy contract, which must stop the check rather than widen what it judges.
    if (error instanceof Error && error.message.startsWith("/clauses/0/forbid/ast/rule/kind:")) return undefined;
    throw error;
  }
  const expando = EXPANDO[language];
  const source = expando === undefined ? code : withExpando(code, expando);
  const snapshot = { format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files: [{ path: `context${CODE_RULE_DIALECTS[dialect].suffixes[0]!}`, language: dialect, source }] };
  const result = evaluationSchema.parse(JSON.parse(compiled.evaluate(JSON.stringify(snapshot), "report")));
  if (result.failures.length > 0) return undefined;
  const first = result.violations.map(violation => violation.span).sort((a, b) => a.startByte - b.startByte || b.endByte - a.endByte)[0];
  if (first === undefined) return undefined;
  const selected = Buffer.from(source, "utf8").subarray(first.startByte, first.endByte).toString("utf8");
  return expando === undefined ? selected : selected.replaceAll(expando, "$");
}
