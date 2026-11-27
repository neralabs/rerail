/**
 * A repository's option values for a code rule by Rerail (a rule setting's `config`). A bundled document is stored
 * resolved, every `$option` reference already replaced by its option's value, so `config` is applied to the
 * document as written (`RuleCatalog.sources`) and that is resolved again, as the engine resolves options. A rule
 * written in `.rerail` is not rewritten: each value is checked against its `.options` declaration, and the run hands
 * the values to the engine as `option` facts (decision 0720).
 */
import { ZodError } from "zod";
import { CodeRuleCompileError, EngineUnavailableError, storableCodeRule } from "../code-rule/code-rule-compile.js";
import { applyCodeRuleConfig, checkCodeRuleOptionValue, CodeRuleOptionError } from "../code-rule/code-rule-options.js";
import { codeRuleSchema, type CodeRule } from "../code-rule/code-rule-schema.js";
import { stableJsonText } from "../stable-json.js";
import type { RerailRule } from "./rerail-rules/rerail-rule.js";
import type { RuleCatalog, RuleConfig } from "./repository-model.js";

export type ConfiguredCodeRule = { readonly ok: true; readonly rule: CodeRule | RerailRule } | { readonly ok: false; readonly reason: string };

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
    outcome = configuredOutcome(ruleCatalog, ruleId, config);
    outcomes.set(key, outcome);
  }
  return outcome;
}

/** `rule` with `config`: each value one its `.options` declares, of a value the declaration allows. The rule is unchanged. */
function configuredRerail(rule: RerailRule, config: RuleConfig): ConfiguredCodeRule {
  try {
    for (const [name, value] of Object.entries(config)) {
      // Only the rule's own declarations: `toString` is no option of a rule that declares none.
      const declaration = Object.hasOwn(rule.options, name) ? rule.options[name] : undefined;
      if (declaration === undefined) throw new CodeRuleOptionError(["config", name], `\`${name}\` is not an option this rule declares`);
      checkCodeRuleOptionValue(declaration, value, ["config", name]);
    }
    return { ok: true, rule };
  } catch (error) {
    return { ok: false, reason: reasonOf(error) };
  }
}

/** {@link configureBundledCodeRule}, worked out. */
function configuredOutcome(ruleCatalog: RuleCatalog, ruleId: string, config: RuleConfig): ConfiguredCodeRule {
  const document = ruleCatalog.documents.get(ruleId);
  if (document?.kind === "rerail") return configuredRerail(document.rule, config);
  const source = ruleCatalog.sources.get(ruleId);
  if (source === undefined) return { ok: false, reason: `config applies to code rules by Rerail; ${ruleId} is not one` };
  try {
    return { ok: true, rule: configuredRule(applyCodeRuleConfig(source, { ...config })) };
  } catch (error) {
    return { ok: false, reason: reasonOf(error) };
  }
}
