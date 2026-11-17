/**
 * The decision Biome 2.5.15 comes to for one rule from the `linter` sections it merges for some files: each section
 * merged over the ones before it, as far as the rule goes, then what its `enabled`, presets, groups, rule entries and
 * domains do to the rule. Which rules a preset or a domain holds comes from Biome's own rule metadata
 * (`biome-rules.generated.ts`); a domain's recommended rules also turn on for the dependencies of a `package.json`,
 * which Rerail does not read, so they keep the pack's values in a repository with one.
 */
import { BIOME_RULES, type BiomeRuleDeclaration } from "../generated/biome-rules.generated.js";
import type { PortDecision } from "../linter-config.js";
import type { DomainValue, Fact, GroupLayer, Level, LinterLayer, Preset, RuleEntry, RulesLayer } from "./biome-config-file.js";

/** Biome turns nursery rules on only by name or by the group: no preset, domain or dependency does. */
const NURSERY = "nursery";
const OPTIONS_NOT_MAPPED = "Biome's rule options are not read";

/** What an extended config Rerail does not read may have set: any value, or none. */
export const UNREAD = Symbol("unread");
export type Unread = typeof UNREAD;
/** A value as far as Rerail knows it: set, unset, or possibly set by a config it does not read. */
type Known<T> = Fact<T> | Unread | undefined;

const isFact = <T>(known: Known<T>): known is Fact<T> => known !== undefined && known !== UNREAD;

/** A Biome rule as its config knows it, from its metadata (`lint_group_structs!` takes the last analyzer's declaration). */
interface BiomeRule {
  readonly group: string;
  readonly name: string;
  readonly nursery: boolean;
  /** In the recommended preset: recommended and in no domain. */
  readonly presetRecommended: boolean;
  /** In no domain: a group set to a severity turns it on. */
  readonly nonDomain: boolean;
  readonly declarations: readonly BiomeRuleDeclaration[];
}

/** Biome rule `lint/<group>/<rule>`; undefined when Biome 2.5.15 has no such rule. */
function biomeRule(name: string): BiomeRule | undefined {
  const [, group = "", rule = ""] = name.split("/");
  const declarations = BIOME_RULES[`${group}/${rule}`] ?? [];
  const last = declarations.at(-1);
  if (last === undefined) return undefined;
  const [recommended, domains] = last;
  return { group, name: rule, nursery: group === NURSERY, presetRecommended: recommended && domains.length === 0, nonDomain: domains.length === 0, declarations };
}

/** A group as the merged config has it for one rule: a severity, or its preset settings and the rule's entry. */
type GroupState =
  | { readonly kind: "plain"; readonly level: Fact<Level> }
  | { readonly kind: "object"; readonly recommended: Known<boolean>; readonly preset: Known<Preset>; readonly entry: Known<RuleEntry> };

/** The merged config, as far as one rule goes. */
interface Folded {
  /** Whether any `linter` sets `rules` (`recommended_enabled` reads it, crates/biome_service/src/settings.rs l.877-890). */
  readonly rulesSet: boolean | Unread;
  readonly recommended: Known<boolean>;
  readonly preset: Known<Preset>;
  readonly group: GroupState | Unread | undefined;
  readonly domains: ReadonlyMap<string, Fact<DomainValue>>;
  /** An extended config Rerail does not read may set any domain not set after it. */
  readonly domainsUnread: boolean;
  readonly enabled: Known<boolean>;
}

const EMPTY: Folded = { rulesSet: false, recommended: undefined, preset: undefined, group: undefined, domains: new Map(), domainsUnread: false, enabled: undefined };

/** A group object merged over `previous`, as `SeverityOrGroup::merge_with` turns a severity into a group of that severity for every rule. */
function objectBase(previous: GroupState | Unread | undefined): Extract<GroupState, { kind: "object" }> {
  if (previous === undefined) return { kind: "object", recommended: undefined, preset: undefined, entry: undefined };
  if (previous === UNREAD) return { kind: "object", recommended: UNREAD, preset: UNREAD, entry: UNREAD };
  if (previous.kind === "object") return previous;
  return { kind: "object", recommended: undefined, preset: undefined, entry: { value: { level: previous.level.value, options: false }, by: previous.level.by } };
}

/** A rule entry merged over `previous`: a severity alone keeps earlier options (`RuleConfiguration::merge_with`). */
function mergeEntry(previous: Known<RuleEntry>, next: Fact<RuleEntry> | undefined): Known<RuleEntry> {
  if (next === undefined) return previous;
  const options = next.value.options || (isFact(previous) && previous.value.options);
  return { value: { level: next.value.level, options }, by: next.by };
}

/** Group `layer` merged over `previous`, for rule `name`. */
function mergeGroup(previous: GroupState | Unread | undefined, layer: GroupLayer | undefined, name: string): GroupState | Unread | undefined {
  if (layer === undefined) return previous;
  if (layer.kind === "plain") return layer;
  const base = objectBase(previous);
  return { kind: "object", recommended: layer.recommended ?? base.recommended, preset: layer.preset ?? base.preset, entry: mergeEntry(base.entry, layer.rules.get(name)) };
}

/** `rules` merged over `state`, for `rule`. */
function foldRules(state: Folded, rules: RulesLayer | undefined, rule: BiomeRule): Folded {
  if (rules === undefined) return state;
  return {
    ...state,
    rulesSet: true,
    recommended: rules.recommended ?? state.recommended,
    preset: rules.preset ?? state.preset,
    group: mergeGroup(state.group, rules.groups.get(rule.group), rule.name),
  };
}

/**
 * `layer` merged over `state`, as Biome merges an extended config, the config, then the overrides for a file. An
 * extended config Rerail does not read may set any rule, preset and domain; it is taken not to turn the linter off.
 */
function foldLayer(state: Folded, layer: LinterLayer | Unread, rule: BiomeRule): Folded {
  if (layer === UNREAD) {
    return { ...state, rulesSet: state.rulesSet === true || UNREAD, recommended: UNREAD, preset: UNREAD, group: UNREAD, domains: new Map(), domainsUnread: true };
  }
  const merged = foldRules(state, layer.rules, rule);
  return { ...merged, domains: new Map([...merged.domains, ...layer.domains]), enabled: layer.enabled ?? merged.enabled };
}

/** What one part of the config does to a rule: turns it on or off, surely or only possibly (an unread config). */
interface Effect {
  readonly on: boolean;
  readonly sure: boolean;
  readonly explicit: boolean;
  readonly because: string;
  readonly options?: boolean;
}

/** What reading a config for a verdict needs: the file Biome takes, why some of it is not known, and whether the repository has a `package.json`. */
export interface Context {
  readonly file: string;
  readonly unread: string;
  readonly manifests: boolean;
}

const unsure = (context: Context): Effect[] => [
  { on: true, sure: false, explicit: true, because: context.unread },
  { on: false, sure: false, explicit: true, because: context.unread },
];

/** The preset `rules` gives every group that does not set its own (`Rules::preset`, rules.rs l.2569-2577). */
function topPreset(folded: Folded, context: Context): Known<Preset> {
  if (isFact(folded.recommended) && !folded.recommended.value) return { value: "none", by: folded.recommended.by };
  if (folded.recommended === UNREAD || folded.preset === UNREAD) return UNREAD;
  return folded.preset ?? { value: "recommended", by: `${context.file}: recommended` };
}

/**
 * A group's own preset, else its parent's (`collect_preset_rules`, crates/biome_configuration_macros/src/group_struct.rs l.294-311).
 * An unread config sets the group's `preset` and `recommended` to unread together, so a group without a preset has a
 * `recommended` that is set or unset, never unread.
 */
function groupPreset(group: Extract<GroupState, { kind: "object" }>, parent: Known<Preset>): Known<Preset> {
  if (group.preset !== undefined) return group.preset;
  if (!isFact(group.recommended)) return parent;
  return { value: group.recommended.value ? "recommended" : "none", by: group.recommended.by };
}

/** What a preset does to `rule`: `all` names every rule; `recommended` holds the recommended rules in no domain. */
function presetEffects(preset: Known<Preset>, rule: BiomeRule, context: Context): Effect[] {
  if (preset === UNREAD) return unsure(context).slice(0, 1);
  if (preset === undefined || preset.value === "none") return [];
  if (preset.value === "all") return [{ on: true, sure: true, explicit: true, because: preset.by }];
  return rule.presetRecommended ? [{ on: true, sure: true, explicit: false, because: preset.by }] : [];
}

/** What a group set to a severity does: on turns on its rules in no domain, off turns off all of them (`SeverityOrGroup`, mod.rs l.1105-1141). */
function plainEffects(level: Fact<Level>, rule: BiomeRule): Effect[] {
  if (level.value === "off") return [{ on: false, sure: true, explicit: true, because: level.by }];
  return rule.nonDomain ? [{ on: true, sure: true, explicit: true, because: level.by }] : [];
}

function entryEffects(entry: Known<RuleEntry>, context: Context): Effect[] {
  if (entry === undefined) return [];
  if (entry === UNREAD) return unsure(context);
  return [{ on: entry.value.level !== "off", sure: true, explicit: true, because: entry.by, options: entry.value.options }];
}

/** What `rules` does to `rule` (`Rules::as_enabled_rules` and `as_disabled_rules`, rules.rs l.2581-2667). */
function ruleEffects(folded: Folded, rule: BiomeRule, context: Context): Effect[] {
  const group = folded.group;
  if (group === UNREAD) return unsure(context);
  if (group === undefined) return rule.nursery ? [] : presetEffects(topPreset(folded, context), rule, context);
  if (group.kind === "plain") return plainEffects(group.level, rule);
  return [...entryEffects(group.entry, context), ...rule.nursery ? [] : presetEffects(groupPreset(group, topPreset(folded, context)), rule, context)];
}

/** Whether a rule's entry turns it on. */
function entryOn(entry: Known<RuleEntry>): boolean | Unread {
  if (entry === UNREAD) return UNREAD;
  return entry !== undefined && entry.value.level !== "off";
}

/** Whether `rules` names the rule on, by itself or its group's severity (`as_explicitly_enabled_rules`), which a domain set to none does not undo. */
function explicitlyOn(folded: Folded, rule: BiomeRule): boolean | Unread {
  const group = folded.group;
  if (group === UNREAD) return UNREAD;
  if (group === undefined) return false;
  if (group.kind === "plain") return group.level.value !== "off" && rule.nonDomain;
  return entryOn(group.entry);
}

function domainValue(folded: Folded, domain: string): Known<DomainValue> {
  return folded.domains.get(domain) ?? (folded.domainsUnread ? UNREAD : undefined);
}

/** What one configured domain does to a rule of it (`record_rule_from_domains`, crates/biome_service/src/file_handlers/mod.rs l.1712-1764). */
function domainEffects(value: Known<DomainValue>, recommended: boolean, context: Context): Effect[] {
  if (value === undefined) return [];
  if (value === UNREAD) return unsure(context);
  if (value.value === "none") return [{ on: false, sure: true, explicit: true, because: value.by }];
  if (value.value === "all") return [{ on: true, sure: true, explicit: true, because: value.by }];
  return recommended ? [{ on: true, sure: true, explicit: false, because: value.by }] : [];
}

/** What the domains do to one analyzer's rule: any domain turning it on wins over one set to none, which a rule named on undoes. */
function declarationDomainEffects(folded: Folded, [recommended, domains]: BiomeRuleDeclaration, named: boolean | Unread, context: Context): Effect[] {
  const effects = domains.flatMap(domain => domainEffects(domainValue(folded, domain), recommended, context));
  const ons = effects.filter(effect => effect.on);
  if (named === true || ons.some(effect => effect.sure)) return ons;
  return [...ons, ...effects.filter(effect => !effect.on).map(effect => (named === UNREAD ? { ...effect, sure: false } : effect))];
}

/**
 * Whether Biome turns on recommended domain rules for the dependencies in `package.json` (`recommended_enabled`). An
 * unread config sets `recommended` and `preset` to unread together, so with `recommended` unset `preset` is not unread.
 */
function recommendedEnabled(folded: Folded): boolean | Unread {
  if (folded.rulesSet !== true) return folded.rulesSet === false || UNREAD;
  if (folded.recommended !== undefined) return folded.recommended === UNREAD ? UNREAD : folded.recommended.value;
  return isFact(folded.preset) && folded.preset.value === "recommended";
}

/**
 * A recommended rule of a domain the config does not set turns on when the nearest `package.json` depends on one of
 * the domain's packages (`ManifestVisitor`, file_handlers/mod.rs l.2002-2063): Rerail does not tell, so it may, in a
 * repository with a `package.json`.
 */
function manifestEffects(folded: Folded, rule: BiomeRule, context: Context): Effect[] {
  if (rule.nursery || !context.manifests || recommendedEnabled(folded) === false) return [];
  const domains = rule.declarations.flatMap(([recommended, ruleDomains]) => (recommended ? ruleDomains : []));
  const open = domains.find(domain => domainValue(folded, domain) === undefined);
  if (open === undefined) return [];
  return [{ on: true, sure: false, explicit: false, because: `${context.file}: Biome turns the recommended ${JSON.stringify(open)} rules on when package.json depends on the domain's packages` }];
}

/** Explicit effects first, so `because` names what the config says about the rule itself. */
const explicitFirst = (effects: readonly Effect[]): Effect[] => [...effects.filter(effect => effect.explicit), ...effects.filter(effect => !effect.explicit)];

/** Where `effects` that turn nothing surely off leave the rule: on when one surely turns it on and nothing may turn it off. */
function onDecision(effects: readonly Effect[], context: Context): PortDecision {
  const ons = explicitFirst(effects.filter(effect => effect.on));
  const unsureOff = effects.find(effect => !effect.on);
  const on = ons.find(effect => effect.sure);
  if (unsureOff !== undefined && ons.length > 0) return { on: "unknown", because: unsureOff.because };
  if (on !== undefined) return { on: true, explicit: on.explicit, because: on.because, ...on.options === true ? { config: { ok: false, reason: OPTIONS_NOT_MAPPED } } : {} };
  if (ons[0] !== undefined) return { on: "unknown", because: ons[0].because };
  return { on: false, explicit: false, because: `${context.file} does not turn it on` };
}

/** The decision `effects` come to: a rule disabled anywhere is off; one enabled anywhere is on; otherwise Biome leaves it off. */
function decide(effects: readonly Effect[], context: Context): PortDecision {
  const off = explicitFirst(effects.filter(effect => !effect.on && effect.sure))[0];
  if (off !== undefined) return { on: false, explicit: off.explicit, because: off.because };
  return onDecision(effects, context);
}

/** What the merged config decides for `rule`. */
function foldedDecision(folded: Folded, rule: BiomeRule, context: Context): PortDecision {
  if (isFact(folded.enabled) && !folded.enabled.value) return { on: false, explicit: false, because: folded.enabled.by };
  const named = explicitlyOn(folded, rule);
  const domainRule = rule.nursery ? [] : rule.declarations.flatMap(declaration => declarationDomainEffects(folded, declaration, named, context));
  return decide([...ruleEffects(folded, rule, context), ...domainRule, ...manifestEffects(folded, rule, context)], context);
}

/** One `linter` section Biome merges for a file; `some` (what it is, for the person) where it may or may not. */
export interface Step {
  readonly layer: LinterLayer | Unread;
  readonly some?: string;
}

/** A merged config a file may get, and the last step for only some files it took. */
interface Possible {
  readonly folded: Folded;
  readonly via: string | undefined;
}

/** `possible`, `step` merged over each; a step for only some files keeps each as it was too. Alike merged configs are kept once, the first. */
function foldStep(possible: readonly Possible[], step: Step, rule: BiomeRule): Possible[] {
  const merged = possible.map(state => ({ folded: foldLayer(state.folded, step.layer, rule), via: step.some ?? state.via }));
  if (step.some === undefined) return merged;
  const seen = new Set<string>();
  return [...possible, ...merged].filter(state => {
    const key = JSON.stringify(state.folded, (_, value: unknown) => (value instanceof Map ? [...value] : value === UNREAD ? "unread" : value));
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}

/**
 * The state `decisions`, each for some files, share: named by the config (explicit) only where each names it, as the
 * files that do not take what names it decide alone. Undefined where their states differ.
 */
export function sharedDecision(decisions: readonly PortDecision[]): PortDecision | undefined {
  const [first, ...others] = decisions;
  if (others.some(decision => decision.on !== first!.on)) return undefined;
  return decisions.find(decision => decision.on !== "unknown" && !decision.explicit) ?? first;
}

/** What the merged `steps` decide for `rule`: unknown where a step for only some files may decide it otherwise. */
function ruleDecision(steps: readonly Step[], rule: BiomeRule, context: Context): PortDecision {
  const possible = steps.reduce((states, step) => foldStep(states, step, rule), [{ folded: EMPTY, via: undefined }] as Possible[]);
  const decisions = possible.map(state => foldedDecision(state.folded, rule, context));
  // The first merged config takes no step for only some files; one deciding otherwise takes one.
  const differing = possible[decisions.findIndex(decision => decision.on !== decisions[0]!.on)];
  return sharedDecision(decisions) ?? { on: "unknown", because: `${differing!.via!} is for only some files and may set it` };
}

/** A port's decision from its Biome rules' (all of whose checks it ports, at least one): on when any is on, unknown when any may be. */
export function portDecision(decisions: readonly PortDecision[]): PortDecision {
  const on = decisions.filter(decision => decision.on === true);
  return on.find(decision => decision.on === true && decision.explicit)
    ?? on[0]
    ?? decisions.find(decision => decision.on === "unknown")
    ?? decisions.find(decision => decision.on === false && decision.explicit)
    ?? decisions[0]!;
}

/** Rule `name` (`lint/<group>/<rule>`) as Biome 2.5.15 has it; unknown when it has no such rule. */
export function nameDecision(steps: readonly Step[], name: string, context: Context): PortDecision {
  const rule = biomeRule(name);
  return rule === undefined ? { on: "unknown", because: `${name} is not a Biome 2.5.15 lint rule` } : ruleDecision(steps, rule, context);
}

