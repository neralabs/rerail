/**
 * Internal testing: a declared pack's `config` in `.rerail/packs.json` as option values for its rules. Each option
 * applies to every rule of the pack that declares it, as a rule setting's `config` for that rule over the whole
 * repository (docs/rule-scoping.md), and is checked here against each of them: the file is refused, naming itself, the
 * pack and the option, when no rule of the pack declares an option or a rule does not allow its value. Any reader of
 * the file (a headless scan, later a session) runs the pack's rules with the values this returns.
 */
import type { CodeRuleDslValue } from "../../code-rule/code-rule.js";
import { codeRuleDeclaredOptions } from "../../code-rule/code-rule-options.js";
import type { RuleCatalog, RuleConfig } from "../repository-model.js";
import { configureBundledCodeRule } from "../rule-config.js";
import { declaredPackLabel, PacksFileError, type DeclaredPack } from "./packs-file.js";

/** One rule's option values from its pack's `config`: what a rule setting's `config` for the rule would hold. */
export interface DeclaredRuleConfig {
  readonly ruleId: string;
  readonly config: RuleConfig;
}

/**
 * The option values `pack`'s `config` gives each rule of the pack it loaded as (`packId` in `catalog`); empty without
 * `config`. `label` names the file in messages (`.rerail/packs.json`, or `<ref>:.rerail/packs.json`). Throws
 * `PacksFileError` for an option no rule of the pack declares, or a value a rule that declares the option does not
 * allow (its declaration, or the rule's checks with the value in place, refuse it).
 */
export function declaredPackRuleConfigs(catalog: RuleCatalog, pack: DeclaredPack, packId: string, label: string): DeclaredRuleConfig[] {
  if (pack.config === undefined) return [];
  const name = declaredPackLabel(pack);
  const rules = (catalog.packs.get(packId)?.rules ?? []).map(rule => {
    const source = catalog.sources.get(rule.id);
    return { id: rule.id, declared: source === undefined ? new Set<string>() : codeRuleDeclaredOptions(source) };
  });
  const configs = new Map<string, Record<string, CodeRuleDslValue>>();
  for (const [option, value] of Object.entries(pack.config)) {
    const declaring = rules.filter(rule => rule.declared.has(option));
    if (declaring.length === 0) {
      const known = [...new Set(rules.flatMap(rule => [...rule.declared]))].sort();
      const declared = known.length === 0 ? "its rules declare no option" : `its rules declare ${known.join(", ")}`;
      throw new PacksFileError(`${label} sets config.${option} for ${name}, which no rule of that pack declares (${declared})`);
    }
    for (const rule of declaring) {
      const outcome = configureBundledCodeRule(catalog, rule.id, { [option]: value });
      if (!outcome.ok) throw new PacksFileError(`${label} sets config.${option} for ${name} to ${JSON.stringify(value)}, which ${rule.id} does not allow: ${outcome.reason}`);
      configs.set(rule.id, { ...configs.get(rule.id), [option]: value });
    }
  }
  return [...configs].map(([ruleId, config]) => {
    // Values each allowed on its own can still make a rule the engine refuses together.
    const outcome = configureBundledCodeRule(catalog, ruleId, config);
    if (!outcome.ok) {
      const options = Object.keys(config).map(option => `config.${option}`).join(", ");
      throw new PacksFileError(`${label} sets ${options} for ${name}, which ${ruleId} does not allow together: ${outcome.reason}`);
    }
    return { ruleId, config };
  });
}
