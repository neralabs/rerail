/**
 * Ruff's selectors, as the pinned Ruff reads them (`RuleSelector` in ruff_linter's `rule_selector.rs`): a remapped
 * one, `TCH` or `TRY200`, as its target; a code prefix within one linter; `ALL`, the legacy `C` and `T`, and, only with
 * preview, a category or a rule name. What each selects, and how specific it is, follow from the generated tables of
 * Ruff's rules, so the selection and its replay both read selectors here and nowhere else.
 */
import {
  RUFF_CATEGORIES,
  RUFF_LINTER_PREFIXES,
  RUFF_NAMES_WITHOUT_CODE,
  RUFF_REDIRECTS,
  RUFF_RULES,
} from "../generated/ruff-rules.generated.js";

const CATEGORIES = new Set(RUFF_CATEGORIES);
/** Each rule's name to its code; undefined for a rule without one. */
const NAMES = new Map<string, string | undefined>([
  ...[...RUFF_RULES].map(([code, info]): [string, string] => [info.name, code]),
  ...RUFF_NAMES_WITHOUT_CODE.map((name): [string, undefined] => [name, undefined]),
]);
const LINTERS_LONGEST_FIRST = [...RUFF_LINTER_PREFIXES].sort((left, right) => right.length - left.length);
/** The linter whose code prefix starts `selector`, the longest that does, as Ruff's `Linter::parse_code` finds it. */
const linterOf = (selector: string): string | undefined => LINTERS_LONGEST_FIRST.find(prefix => selector.startsWith(prefix));
/** Every code selector Ruff takes: each prefix of a rule's code from its linter's prefix on (`E`, `E7`, `E71`, `E711`; `PL`, `PLR`). */
const CODE_SELECTORS = new Set([...RUFF_RULES.keys()].flatMap(code => {
  const linter = linterOf(code)!;
  return Array.from({ length: code.length - linter.length + 1 }, (_, extra) => code.slice(0, linter.length + extra));
}));
/** The legacy selectors `C` and `T` (`RuleSelector::C` and `T` in ruff_linter's `rule_selector.rs`) and the linters each selects. */
const LEGACY_GROUPS: Readonly<Record<string, readonly string[]>> = { C: ["C4", "C90"], T: ["T10", "T20"] };

/** A Ruff selector in one of the rule selection options, with the file and key path that hold it. */
export interface RuffSelector {
  readonly file: string;
  readonly key: string;
  /** The selector as Ruff reads it: a remapped one as its target, a rule name as its code. */
  readonly selector: string;
  /** The selector as the config writes it, where Ruff reads it as another. */
  readonly written?: string;
}

/** What Ruff reads selector `written` as: the selector it selects by (undefined for a rule without a code), and whether Ruff takes it only with preview; undefined for one Ruff does not know. */
export function resolveSelector(written: string): { readonly selector: string | undefined; readonly previewOnly: boolean } | undefined {
  if (written === "ALL" || Object.hasOwn(LEGACY_GROUPS, written)) return { selector: written, previewOnly: false };
  const redirected = RUFF_REDIRECTS.get(written) ?? written;
  if (CODE_SELECTORS.has(redirected)) return { selector: redirected, previewOnly: false };
  if (CATEGORIES.has(written)) return { selector: written, previewOnly: true };
  return NAMES.has(written) ? { selector: NAMES.get(written), previewOnly: true } : undefined;
}

export const ruffText = ({ file, key, selector, written }: RuffSelector): string =>
  `${file}: ${JSON.stringify(written ?? selector)} in ${key}${written === undefined ? "" : ` (Ruff reads it as ${JSON.stringify(selector)})`}`;

/**
 * Whether Ruff selector `selector` (as Ruff reads it) names rule `code`: `ALL` every rule; a category the rules in it;
 * `C` and `T` their linters; otherwise a prefix of the code within the same linter, so `E` selects `E711` but not
 * `ERA001`, and `PL` selects Pylint's `PLR0913`. Whether Ruff then runs the rule also depends on its status.
 */
export function ruffSelects(selector: string, code: string): boolean {
  if (selector === "ALL") return true;
  if (CATEGORIES.has(selector)) return RUFF_RULES.get(code)?.category === selector;
  if (Object.hasOwn(LEGACY_GROUPS, selector)) return LEGACY_GROUPS[selector]!.includes(linterOf(code)!);
  return code.startsWith(selector) && linterOf(selector) === linterOf(code);
}

/**
 * How specific a Ruff selector is, as Ruff orders them (`Specificity` in ruff_linter's `rule_selector.rs`): `ALL`, a
 * category, the legacy `C` and `T`, a linter (`E`, `PL`), a prefix by its length (`E7`, `PLR09`), a rule (`E711`). Two
 * code selectors that select the same code are both prefixes of it in one linter, so the longer is the more specific.
 */
export function ruffSpecificity(selector: string): number {
  if (selector === "ALL") return 0;
  if (CATEGORIES.has(selector)) return 1;
  return Object.hasOwn(LEGACY_GROUPS, selector) ? 2 : 2 + selector.length;
}
