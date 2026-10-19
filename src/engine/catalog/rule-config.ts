/**
 * A repository's option values for a code rule by Rerail (a rule setting's `config`). A bundled document is stored
 * resolved, every `$option` reference already replaced by its option's value, so `config` is applied to the
 * document as written (`RuleCatalog.sources`) and that is resolved again, as the engine resolves options.
 */
import { ZodError } from "zod";
import { CodeRuleCompileError, EngineUnavailableError, storableCodeRule } from "../code-rule/code-rule-compile.js";
import { applyCodeRuleConfig, CodeRuleOptionError } from "../code-rule/code-rule-options.js";
import { codeRuleSchema, type CodeRule } from "../code-rule/code-rule-schema.js";
import { stableJsonText } from "../stable-json.js";
import type { RuleCatalog, RuleConfig } from "./repository-model.js";

export type ConfiguredCodeRule = { readonly ok: true; readonly rule: CodeRule } | { readonly ok: false; readonly reason: string };

/** Per rule catalog, `<rule id>\0<config as stable JSON>` → the outcome; a repository's config changes rarely. */
const configured = new WeakMap<RuleCatalog, Map<string, ConfiguredCodeRule>>();

/** `config.<option>...: <problem>` for a problem in the patched document, else the message. */
function reasonOf(error: unknown): string {
  if (error instanceof CodeRuleOptionError) {
    const at = error.path.lastIndexOf("config");
    return at < 0 ? error.message : `config.${error.path.slice(at + 1).join(".")}: ${error.message}`;
  }
  if (error instanceof ZodError) {
    const issue = error.issues[0];
    return issue === undefined ? error.message : `${issue.path.join(".")}: ${issue.message}`;
  }
  if (error instanceof CodeRuleCompileError) return `config: the engine refuses the rule with these values: ${error.reason}`;
  throw error;
}

/**
 * The configured document, compiled as the catalog compiles it: a value its declaration allows can still make a
 * clause the engine refuses (a pattern option that is no valid regex). Without an engine, the schema's reading stands;
 * the catalog cannot run the rule then either.
 */
function configuredRule(document: unknown): CodeRule {
  try {
    return storableCodeRule(document).rule;
  } catch (error) {
    if (error instanceof EngineUnavailableError) return codeRuleSchema.parse(document);
    throw error;
  }
}

/**
 * Bundled code rule `ruleId` with `config` (option name → value) set: its document as written with each value in the
 * `config` of the define declaring the option, then resolved, validated and compiled. Not ok when the rule is not a
 * code rule by Rerail, names an option the document does not declare, or a value its declaration or the engine does
 * not allow.
 */
export function configureBundledCodeRule(ruleCatalog: RuleCatalog, ruleId: string, config: RuleConfig): ConfiguredCodeRule {
  const key = `${ruleId}\0${stableJsonText(config) ?? ""}`;
  let outcomes = configured.get(ruleCatalog);
  if (outcomes === undefined) {
    outcomes = new Map();
    configured.set(ruleCatalog, outcomes);
  }
  let outcome = outcomes.get(key);
  if (outcome === undefined) {
    const source = ruleCatalog.sources.get(ruleId);
    if (source === undefined) {
      outcome = { ok: false, reason: `config applies to code rules by Rerail; ${ruleId} is not one` };
    } else {
      try {
        outcome = { ok: true, rule: configuredRule(applyCodeRuleConfig(source, { ...config })) };
      } catch (error) {
        outcome = { ok: false, reason: reasonOf(error) };
      }
    }
    outcomes.set(key, outcome);
  }
  return outcome;
}
