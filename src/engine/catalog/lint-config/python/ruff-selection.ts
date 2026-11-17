/**
 * The rule selection a chain of Ruff configs makes (`ruff-config-file.ts` reads the chain): each config's selectors
 * read as Ruff reads them, from the extended config to the root's, the per-file ignores Ruff applies, and the nearest
 * config's preview, `explicit-preview-rules` and docstring convention. A config Ruff refuses (a key that names no Ruff
 * option, a selector it does not know, a removed rule selected, an `extend` it fails to load) is not read at all.
 */
import type { LinterNote } from "../linter-config.js";
import { RUFF_CONVENTIONS, RUFF_RULES } from "../generated/ruff-rules.generated.js";
import type { ChainEnd, RuffConvention, RuffFile, RuffLayer, RuffPerFileIgnore } from "./ruff-config-file.js";
import { resolveSelector, ruffText, type RuffSelector } from "./ruff-selectors.js";

/** Ruff's rule selection in the config Ruff uses at the root. */
export interface RuffSelection {
  /** The config Ruff uses at the root. */
  readonly file: string;
  /** Each config's selection in the order Ruff applies them: the configs `extend` names first, the root's last. */
  readonly layers: readonly RuffLayer[];
  /** `per-file-ignores` and `extend-per-file-ignores`: a selector ignored in the files a pattern matches. */
  readonly perFileIgnores: readonly RuffPerFileIgnore[];
  /** Whether preview is on for the linter; undefined when no config read sets it and one not read may. */
  readonly preview: boolean | undefined;
  /** `explicit-preview-rules`; undefined as `preview`. */
  readonly explicitPreviewRules: boolean | undefined;
  /** `pydocstyle.convention`; null where no config sets it, undefined where one not read may. */
  readonly convention: RuffConvention | null | undefined;
  /** The extended config Rerail does not read, which may select or ignore any rule: how the person knows it. */
  readonly unread?: string;
}

/** The per-file ignores Ruff applies: those of the nearest config that sets `per-file-ignores`, and every extension. */
function perFileIgnores(chain: readonly RuffFile[]): RuffPerFileIgnore[] {
  return [...chain.find(file => file.perFileIgnores !== undefined)?.perFileIgnores ?? [], ...chain.flatMap(file => file.extendPerFileIgnores)];
}

/** The nearest config's value of a setting; `none` when none sets it and the chain is read whole, undefined otherwise. */
function chainValue<T, N>(values: readonly (T | undefined)[], whole: boolean, none: N): T | N | undefined {
  return values.find(value => value !== undefined) ?? (whole ? none : undefined);
}

/**
 * Why Ruff refuses selector `by` with preview `preview` (undefined where a config not read may set it), if it does
 * (`RuleSelection::resolve` and `as_rule_table`): one it does not know or takes only with preview (a category or a rule
 * name), without preview; a removed rule selected by its code. With preview, Ruff skips one it does not know.
 */
function refusal(by: RuffSelector, selects: boolean, preview: boolean | undefined): string | undefined {
  const resolved = resolveSelector(by.selector);
  if (selects && RUFF_RULES.get(resolved?.selector ?? "")?.status === "removed") return `${ruffText(by)} selects a removed rule`;
  if (preview === true || resolved?.previewOnly === false) return undefined;
  return resolved === undefined ? `${ruffText(by)} is not a selector Ruff knows` : `${ruffText(by)} selects by category or rule name, which Ruff takes only with preview`;
}

/** Why Ruff refuses the selection read, in every case it does. */
function refusals(chain: readonly RuffFile[], perFile: readonly RuffPerFileIgnore[], preview: boolean | undefined): string[] {
  const selects = chain.flatMap(file => [...file.layer.select ?? [], ...file.layer.extendSelect]);
  const others = [...chain.flatMap(file => file.layer.ignore), ...perFile.map(entry => entry.ignore)];
  const conventions = chain.flatMap(file => (file.convention === undefined || Object.hasOwn(RUFF_CONVENTIONS, file.convention.name) ? [] : [`${file.convention.because} is not a convention Ruff knows`]));
  return [
    ...selects.flatMap(by => refusal(by, true, preview) ?? []),
    ...others.flatMap(by => refusal(by, false, preview) ?? []),
    ...conventions,
  ];
}

/** Selector `by` as Ruff reads it; undefined, with a note, for one Ruff does not know (and skips, with preview), or a rule without a code. */
function resolved(by: RuffSelector, notes: LinterNote[]): RuffSelector | undefined {
  const reading = resolveSelector(by.selector);
  if (reading === undefined) notes.push({ linter: "ruff", text: `${by.file}: ${JSON.stringify(by.selector)} in ${by.key} is not a Ruff rule code or prefix.` });
  if (reading?.selector === undefined) return undefined;
  return reading.selector === by.selector ? by : { ...by, selector: reading.selector, written: by.selector };
}

/** Layer `layer` with each selector as Ruff reads it. */
function resolvedLayer(layer: RuffLayer, notes: LinterNote[]): RuffLayer {
  const each = (selectors: readonly RuffSelector[]): RuffSelector[] => selectors.flatMap(by => resolved(by, notes) ?? []);
  return { ...layer, select: layer.select === undefined ? undefined : each(layer.select), extendSelect: each(layer.extendSelect), ignore: each(layer.ignore) };
}

/**
 * The selection `chain` (the config Ruff uses at the root first, ending at `end`) makes, with each selector as Ruff
 * reads it; undefined, with a note, where Ruff refuses it.
 */
export function selectionOf(chain: readonly RuffFile[], end: ChainEnd, notes: LinterNote[]): RuffSelection | undefined {
  const unread = end.unread;
  const whole = unread === undefined;
  // Ruff combines `lint.preview` and `preview` over the chain apart, the linter's taking `lint.preview` first.
  const preview = chain.map(file => file.lintPreview).find(value => value !== undefined) ?? chainValue(chain.map(file => file.preview), whole, false);
  const perFile = perFileIgnores(chain);
  const refused = end.refused === undefined ? refusals(chain, perFile, preview) : [end.refused];
  if (refused.length > 0) {
    notes.push({ linter: "ruff", text: `${chain[0]!.name}: Ruff refuses this config (${refused.join("; ")}), so Rerail reads none of it: the rules Rerail ports from Ruff keep the pack's values.` });
    return undefined;
  }
  return {
    file: chain[0]!.name,
    layers: chain.map(file => resolvedLayer(file.layer, notes)).reverse(),
    perFileIgnores: perFile.flatMap(entry => {
      const ignore = resolved(entry.ignore, notes);
      return ignore === undefined ? [] : [{ ...entry, ignore }];
    }),
    preview,
    explicitPreviewRules: chainValue(chain.map(file => file.explicitPreviewRules), whole, false),
    convention: chainValue(chain.map(file => file.convention), whole, null),
    ...(unread === undefined ? {} : { unread }),
  };
}
