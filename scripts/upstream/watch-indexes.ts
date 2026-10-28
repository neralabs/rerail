/**
 * The upstream source files the nightly report (scripts/upstream/watch.ts) reads to find a ported rule's code and the
 * tests upstream adds for it, where no path convention says: Ruff's rule registry (`codes.rs`, `registry.rs`) and test
 * lists (`#[test_case(Rule::X, Path::new("…"))]` in a plugin's `mod.rs`), and Clippy's lint index
 * (`clippy_lints/src/declared_lints.rs`) and test annotations (`//~ <lint>`). Text only; nothing upstream is built or run.
 */

/** Ruff's rule registry. */
export const RUFF_CODES = "crates/ruff_linter/src/codes.rs";
export const RUFF_REGISTRY = "crates/ruff_linter/src/registry.rs";
/** Where Ruff's plugins live: `<RUFF_RULES>/<plugin>/mod.rs` holds its tests, `<RUFF_RULES>/<plugin>/rules/` its rules. */
export const RUFF_RULES = "crates/ruff_linter/src/rules";
/** Where a Ruff plugin's test fixtures live: `<RUFF_FIXTURES>/<plugin>/<file>`, as its `mod.rs` builds the path. */
export const RUFF_FIXTURES = "crates/ruff_linter/resources/test/fixtures";
/** Clippy's lint index, and the directory every lint's module is under. */
export const CLIPPY_DECLARED_LINTS = "clippy_lints/src/declared_lints.rs";
export const CLIPPY_LINTS = "clippy_lints/src";

/**
 * Each Ruff linter's code prefix, from `registry.rs`'s `#[prefix = "B"] Flake8Bugbear,`; null for a linter with several
 * prefixes (pycodestyle's E and W), whose codes.rs entries spell the whole code.
 */
export function ruffLinterPrefixes(registry: string): Map<string, string | null> {
  const prefixes = new Map<string, string | null>();
  let pending: string[] = [];
  for (const line of registry.split("\n")) {
    const prefix = /^\s*#\[prefix = "([A-Z0-9]*)"\]\s*$/u.exec(line);
    if (prefix !== null) {
      pending = [...pending, prefix[1]!];
      continue;
    }
    const linter = /^\s*(\w+),\s*$/u.exec(line);
    if (linter !== null && pending.length > 0) {
      prefixes.set(linter[1]!, pending.length === 1 ? pending[0]! : null);
      pending = [];
    }
  }
  return prefixes;
}

/** A Ruff rule's place: its plugin module (`flake8_bugbear`) and its struct (`UnaryPrefixIncrementDecrement`). */
export interface RuffRule {
  plugin: string;
  struct: string;
}

/** Every rule code of `codes.rs` (`(Flake8Bugbear, "002") => rules::flake8_bugbear::rules::UnaryPrefixIncrementDecrement`), by its full code (`B002`). */
export function ruffCodes(codes: string, registry: string): Map<string, RuffRule> {
  const prefixes = ruffLinterPrefixes(registry);
  const rules = new Map<string, RuffRule>();
  for (const match of codes.matchAll(/^\s*\((\w+),\s*"([A-Z0-9]+)"\)\s*=>\s*(?:\([^,]*,\s*)?rules::(\w+)::rules::(\w+)/gmu)) {
    const [, linter, suffix, plugin, struct] = match as unknown as [string, string, string, string, string];
    const prefix = prefixes.get(linter) ?? null;
    rules.set(`${prefix ?? ""}${suffix}`, { plugin, struct });
  }
  return rules;
}

/** The `#[test_case(Rule::X, Path::new("file"))]` attributes of a Ruff plugin's `mod.rs`: each rule struct and fixture file, in file order. */
export function ruffTestCases(modRs: string): { struct: string; file: string }[] {
  return [...modRs.matchAll(/#\[test_case\(\s*Rule::(\w+)\s*,\s*Path::new\("([^"]+)"\)/gu)].map((match) => ({ struct: match[1]!, file: match[2]! }));
}

/** Every lint of `declared_lints.rs` (`crate::misc_early::ZERO_PREFIXED_LITERAL_INFO`), by name, with its module's path (`misc_early`). */
export function clippyDeclaredLints(text: string): Map<string, string> {
  const lints = new Map<string, string>();
  for (const match of text.matchAll(/\bcrate::((?:\w+::)+?)([A-Z0-9_]+)_INFO\b/gu)) lints.set(match[2]!.toLowerCase(), match[1]!.slice(0, -2).replaceAll("::", "/"));
  return lints;
}

/**
 * The lints a Clippy UI test's `//~` annotations name (`//~^ double_parens`, `//~ ERROR: clippy::double_parens`): the first
 * word after the marker, a level and `clippy::` skipped.
 */
export function clippyAnnotatedLints(text: string): Set<string> {
  const lints = new Set<string>();
  for (const match of text.matchAll(/\/\/~[\^v|]*\s*(?:(?:ERROR|WARN(?:ING)?|NOTE|HELP)\b:?\s*)?(?:clippy::)?([a-z][a-z0-9_]*)/gu)) lints.add(match[1]!);
  return lints;
}
