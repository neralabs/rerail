/**
 * The `<arg>`s PHP_CodeSniffer takes from the rulesets, and what each that Rerail reads changes: a ruleset's own
 * arguments win over those of the rulesets it references (src/Ruleset.php l.623-700); `--exclude` and `--sniffs` leave
 * sniffs out (`File::addMessage`, src/Files/File.php l.896-905); `ignore` leaves files out for every sniff; and a
 * severity argument or the `-n` flag changes the reporting threshold, which Rerail does not read, so a port keeps the
 * pack's value unless the config turns it off by name.
 */
import type { Leaf } from "./phpcs-leaf-replay.js";
import type { Pattern, PhpcsRuleset } from "./phpcs-ruleset.js";

/** The arguments that change the reporting threshold (src/Config.php `CLI_FLAGS_TO_SETTING_NAME`), and the `-n` flag. */
const THRESHOLD_ARGUMENTS = ["severity", "error-severity", "warning-severity"];

/** The read config: the ruleset, the arguments in force, and the patterns that leave files out for every sniff. */
export interface PhpcsRead {
  readonly ruleset: PhpcsRuleset;
  /** The arguments, from the highest-level ruleset that sets each. */
  readonly args: ReadonlyMap<string, { readonly value: string; readonly because: string }>;
  /** Every ruleset's own `<exclude-pattern>`s, and the `ignore` argument's (src/Config.php, `--ignore`). */
  readonly ignores: readonly Pattern[];
}

/** The patterns of the `ignore` argument: comma-separated, `\,` a comma in a pattern. */
export function argumentIgnores(args: PhpcsRead["args"]): Pattern[] {
  const ignore = args.get("ignore");
  const patterns = ignore === undefined ? [] : ignore.value.split(/(?<!\\),/u).map(pattern => pattern.trim()).filter(pattern => pattern !== "");
  return patterns.map(pattern => ({ pattern, because: ignore!.because }));
}

/** The arguments in force: a ruleset's own over those of the rulesets it references (l.623-700). */
export function argsOf(ruleset: PhpcsRuleset, args = new Map<string, { value: string; because: string }>()): Map<string, { value: string; because: string }> {
  for (const [name, value] of ruleset.args) if (!args.has(name)) args.set(name, { value, because: `${ruleset.file}: <arg ${name.startsWith("-") ? "" : `name="${name}" `}value="${value}"/>` });
  for (const rule of ruleset.rules) if (rule.ref.kind === "ruleset") argsOf(rule.ref.ruleset, args);
  return args;
}

/** The argument that changes the reporting threshold, if any: a severity argument or the `-n` flag. */
export function thresholdArgument(read: PhpcsRead): string | undefined {
  const named = THRESHOLD_ARGUMENTS.map(name => read.args.get(name)).find(arg => arg !== undefined);
  return (named ?? [...read.args].find(([name]) => name.startsWith("-") && name.includes("n"))?.[1])?.because;
}

/** The `--exclude` or `--sniffs` argument that leaves the leaf's sniff out, if any (`File::addMessage`, l.896-905). */
export function argumentOff(read: PhpcsRead, leaf: Leaf): string | undefined {
  const sniff = leaf.sniff.toLowerCase();
  const lists = (arg: { value: string }): boolean => arg.value.toLowerCase().split(",").map(listed => listed.trim()).includes(sniff);
  const exclude = read.args.get("exclude");
  if (exclude !== undefined && lists(exclude)) return exclude.because;
  const sniffs = read.args.get("sniffs");
  return sniffs !== undefined && !lists(sniffs) ? sniffs.because : undefined;
}
