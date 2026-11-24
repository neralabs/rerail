/**
 * Where a checkout's `.rerail` rule applies: the folders its placement turns it on in, at which mode, and the files its
 * `.language` and `.scope` take. A written file starts a run only for the rules that take it, and a finding counts only
 * where its rule's placement puts the rule on.
 */
import { globPattern } from "../catalog/rule-pack-matcher.js";
import { type CodeRulePlacement, folderContains } from "../catalog/repository-model.js";
import type { RuleMode } from "../catalog/rule-mode.js";
import type { RerailRuleEntry } from "../code-rule/code-rule-dsl-catalog.js";
import { codeRuleDialectForPath, codeRuleLanguageDialects, isCodeRuleAstLanguage } from "../code-rule/code-rule-languages.js";

/**
 * The rule's mode at `path` (relative to the checkout): the deepest placement folder holding the path decides, as for
 * every code rule. Undefined outside every folder, and where that folder turns the rule off.
 */
export function modeAt(placement: CodeRulePlacement, path: string): RuleMode | undefined {
  const holding = placement.filter(entry => folderContains(entry.folder, path));
  const deepest = holding.sort((left, right) => right.folder.length - left.folder.length)[0];
  return deepest?.mode ?? undefined;
}

/** Whether one of `globs` matches `path`; undefined where a glob does not read here (the engine read it, so the caller runs the rule rather than skip it). */
function matchesAny(globs: readonly string[], path: string): boolean | undefined {
  try {
    return globPattern(globs).test(path);
  } catch {
    return undefined;
  }
}

/**
 * Whether a file of the rule's `.language` is at `path`: one of the language's dialects reads it. A rule of no
 * language, or of one the registry does not name, reads any file.
 */
function ofLanguage(language: string | null, path: string): boolean {
  if (language === null || !isCodeRuleAstLanguage(language)) return true;
  const dialect = codeRuleDialectForPath(path);
  return dialect !== undefined && codeRuleLanguageDialects(language).includes(dialect);
}

/**
 * Whether a `.scope text` glob of `text` reads `path` as text. The engine reads no hidden file (`.draft.html`) as text
 * (`corpus::scoped`), so a write of one starts no run.
 */
function readsAsText(text: readonly string[], path: string): boolean {
  if (text.length === 0 || path.slice(path.lastIndexOf("/") + 1).startsWith(".")) return false;
  return matchesAny(text, path) !== false;
}

/** Whether the rule reads `path` as a file: one of its `.language`, or one a `.scope text` glob reads as text. */
function readsAsFile(rule: Pick<RerailRuleEntry, "scope" | "language">, path: string): boolean {
  return ofLanguage(rule.language, path) || readsAsText(rule.scope.text, path);
}

/**
 * Whether the rule reads `path`, as its run selects files (`RerailScope`): one of its `include` globs matches (any, when
 * it has none), none of its `exclude` globs does, and it reads the file ({@link readsAsFile}).
 */
export function inRuleScope(rule: Pick<RerailRuleEntry, "scope" | "language">, path: string): boolean {
  const { include, exclude } = rule.scope;
  const included = include.length === 0 || (matchesAny(include, path) ?? true);
  return included && readsAsFile(rule, path) && matchesAny(exclude, path) !== true;
}

/** Whether a write of `path` is one for `rule` to judge: its placement turns it on there and its scope takes the file. */
export function takes(rule: RerailRuleEntry, path: string): boolean {
  return modeAt(rule.placement, path) !== undefined && inRuleScope(rule, path);
}

/** Whether `path` lies inside one of `folders` (relative to the same checkout): a checkout nested in it, for instance. */
export function insideAny(folders: readonly string[], path: string): boolean {
  return folders.some(folder => path.startsWith(`${folder}/`));
}
