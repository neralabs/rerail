/**
 * Whether Ruff runs one rule, decided as the pinned Ruff decides it (`as_rule_table` in ruff_workspace's
 * `configuration.rs`): every selector read as Ruff reads it (`ruff-selectors.ts`), Ruff's default selection
 * (`DEFAULT_SELECTORS` in ruff_linter's `settings/mod.rs`, or the default categories with preview), then each config's
 * selectors from the extended one to the root's, then the docstring convention and the pairs of rules Ruff never runs
 * together. A code a selector decides is explicit; one none decides follows the default, an implicit on or off. A
 * preview rule runs only with preview (and, with `explicit-preview-rules`, only when selected by its code). Where an
 * extended config Rerail does not read may set preview or a convention, the rule is decided in every mode it may run
 * in, and is unknown where they differ.
 */
import type { PortDecision } from "../linter-config.js";
import { RUFF_CONVENTIONS, RUFF_INCOMPATIBLE, RUFF_RULES, UPSTREAM_VERSIONS, type RuffRuleInfo } from "../generated/ruff-rules.generated.js";
import type { RuffLayer } from "./ruff-config-file.js";
import type { RuffSelection } from "./ruff-selection.js";
import { ruffSelects, ruffSpecificity, ruffText, type RuffSelector } from "./ruff-selectors.js";

/** The Ruff release whose rules and defaults the generated tables hold (packs/python/upstream/ruff/gates.yaml). */
export const RUFF_VERSION = UPSTREAM_VERSIONS["python/ruff"]!;

/** The mode Ruff's selection runs in: preview, whether preview rules need their own code, and the docstring convention. */
interface RuffMode {
  readonly preview: boolean;
  readonly explicitPreviewRules: boolean;
  readonly convention: string | undefined;
}

/**
 * Per status, whether a selector (exactly the rule's code, or not) selects a rule under a mode (`RuleSelector::rules`
 * in `rule_selector.rs`): a preview rule only with preview, and then by its code where preview rules need it; a
 * removed one by its code (Ruff then refuses the config). Ruff 0.16.9 has no deprecated rule, so the status type has
 * no `deprecated`: a Ruff that brings one back fails type-checking here until its arm (selected only by its code,
 * and only without preview) is added.
 */
const STATUS_ALLOWS: Readonly<Record<RuffRuleInfo["status"], (exact: boolean, mode: RuffMode) => boolean>> = {
  stable: () => true,
  preview: (exact, mode) => mode.preview && (exact || !mode.explicitPreviewRules),
  removed: exact => exact,
};

/** Whether Ruff runs a rule, and the selector that decided it; `by` is undefined for a rule a `select` leaves out. */
interface RuffDecision {
  readonly enabled: boolean;
  readonly by: RuffSelector | undefined;
  /** The config whose `select` left the rule out. */
  readonly leftOutBy?: RuffLayer;
}

/** One config's selectors that name `code` as Ruff applies them, least specific first, selects before ignores. */
function layerUpdates(layer: RuffLayer, carried: readonly RuffSelector[], code: string, applies: (selector: string) => boolean): { enabled: boolean; by: RuffSelector }[] {
  const updates = [
    ...[...(layer.select ?? []), ...layer.extendSelect].map(by => ({ enabled: true, by })),
    ...[...layer.ignore, ...carried].map(by => ({ enabled: false, by })),
  ].filter(({ by }) => ruffSelects(by.selector, code) && applies(by.selector));
  // A stable sort keeps selects before ignores of the same specificity.
  return updates.sort((left, right) => ruffSpecificity(left.by.selector) - ruffSpecificity(right.by.selector));
}

/** The ignores `select = []` with ignores and no `extend-select` carries into the next config. */
const carriedIgnores = (layer: RuffLayer): readonly RuffSelector[] =>
  (layer.select?.length === 0 && layer.extendSelect.length === 0 ? layer.ignore : []);

/**
 * Whether the configs select `code` by its code since the last `select`, which keeps the docstring convention from
 * turning it off (`docstring_overrides` in `as_rule_table`): a config that sets `select` starts it afresh.
 */
function selectedByCode(exact: boolean, layer: RuffLayer, code: string, selects: boolean): boolean {
  const named = selects && [...layer.select ?? [], ...layer.extendSelect].some(by => by.selector === code);
  return layer.select === undefined ? exact || named : named;
}

/**
 * What Ruff's rule selection decides for rule `code` under `mode`, replaying `as_rule_table` over the configs in order.
 * Within one config the most specific selector that names the code decides. A config that sets `select` replaces the
 * earlier configs' result (a code it does not select is off, decided by no selector); one that does not changes only
 * the codes its selectors name; `select = []` with ignores and no `extend-select` carries its ignores into the next
 * config. `decided` is undefined when no selector decides the code: Ruff's default selection does.
 */
function ruffDecision(selection: RuffSelection, code: string, info: RuffRuleInfo, mode: RuffMode): { decided: RuffDecision | undefined; exact: boolean } {
  const allows = STATUS_ALLOWS[info.status];
  let decided: RuffDecision | undefined;
  let carried: readonly RuffSelector[] = [];
  let exact = false;
  for (const layer of selection.layers) {
    const last = layerUpdates(layer, carried, code, selector => allows(selector === code, mode)).at(-1);
    carried = carriedIgnores(layer);
    exact = selectedByCode(exact, layer, code, allows(true, mode));
    const replaced = layer.select === undefined ? undefined : { enabled: false, by: undefined, leftOutBy: layer };
    decided = last ?? replaced ?? decided;
  }
  return { decided, exact };
}

/** Ruff's default selection under `mode`: the default categories with preview, less preview rules where they need their code. */
function byDefault(info: RuffRuleInfo, mode: RuffMode): boolean {
  if (!mode.preview) return info.byDefault;
  return info.byPreviewDefault && !(mode.explicitPreviewRules && info.status === "preview");
}

const defaultText = (mode: RuffMode): string => `Ruff ${RUFF_VERSION}'s default rule selection${mode.preview ? " with preview" : ""}`;

/** What the selectors and Ruff's default selection decide for `code` under `mode`, and whether a config selects it by its code. */
function selectedDecision(selection: RuffSelection, code: string, info: RuffRuleInfo, mode: RuffMode): { decision: PortDecision; exact: boolean } {
  const { decided, exact } = ruffDecision(selection, code, info, mode);
  const decision = ((): PortDecision => {
    if (decided?.by !== undefined) return { on: decided.enabled, explicit: true, because: ruffText(decided.by) };
    if (decided?.leftOutBy !== undefined) return { on: false, explicit: false, because: `${decided.leftOutBy.file}: ${decided.leftOutBy.selectKey!} does not select it` };
    if (selection.unread !== undefined) return { on: "unknown", because: `${selection.unread} which Rerail does not read, may select it` };
    return byDefault(info, mode)
      ? { on: true, explicit: false, because: `${selection.file}: ${defaultText(mode)}` }
      : { on: false, explicit: false, because: `${selection.file} does not select it, nor does ${defaultText(mode)}` };
  })();
  return { decision, exact };
}

/** Where the docstring convention turns `code` off: it is on, in the convention's list, and not selected by its code. */
function conventionOff(selection: RuffSelection, code: string, mode: RuffMode, exact: boolean): PortDecision | undefined {
  if (mode.convention === undefined || exact || !RUFF_CONVENTIONS[mode.convention]!.includes(code)) return undefined;
  return { on: false, explicit: false, because: `${selection.convention?.because ?? `pydocstyle.convention = ${JSON.stringify(mode.convention)}`} turns it off` };
}

/** Where Ruff turns `code` off as the second of a pair of rules it never runs together, the first being on. */
function incompatibleOff(selection: RuffSelection, code: string, mode: RuffMode): PortDecision | undefined {
  const preferred = RUFF_INCOMPATIBLE.find(([, expendable]) => expendable === code)?.[0];
  if (preferred === undefined) return undefined;
  const decision = modeDecision(selection, preferred, RUFF_RULES.get(preferred)!, mode);
  return decision.on === true ? { on: false, explicit: false, because: `Ruff never runs it with ${preferred}, which is on (${decision.because})` } : undefined;
}

/** Ruff's decision for `code` under one `mode`: its selection, then the docstring convention and the incompatible pairs. */
function modeDecision(selection: RuffSelection, code: string, info: RuffRuleInfo, mode: RuffMode): PortDecision {
  const { decision, exact } = selectedDecision(selection, code, info, mode);
  if (decision.on !== true) return decision;
  return conventionOff(selection, code, mode, exact) ?? incompatibleOff(selection, code, mode) ?? decision;
}

/** The modes the selection may run in: one where every config is read, every one an unread config may set otherwise. */
function modesOf(selection: RuffSelection): RuffMode[] {
  const previews = selection.preview === undefined ? [false, true] : [selection.preview];
  const explicits = selection.explicitPreviewRules === undefined ? [false, true] : [selection.explicitPreviewRules];
  const conventions = selection.convention === undefined ? [undefined, ...Object.keys(RUFF_CONVENTIONS)] : [selection.convention?.name];
  return previews.flatMap(preview => explicits.flatMap(explicitPreviewRules => conventions.map(convention => ({ preview, explicitPreviewRules, convention }))));
}

/**
 * Ruff's decision for `code` in every mode the selection may run in; unknown where they differ. The selection runs in
 * more than one mode only where a config it extends is not read, which then may set preview or a convention.
 */
export function codeDecision(selection: RuffSelection, code: string, info: RuffRuleInfo): PortDecision {
  const decisions = modesOf(selection).map(mode => modeDecision(selection, code, info, mode));
  if (decisions.every(decision => decision.on === decisions[0]!.on)) return decisions[0]!;
  return { on: "unknown", because: `${selection.unread!} which Rerail does not read, may set preview or a docstring convention, which decide it` };
}
