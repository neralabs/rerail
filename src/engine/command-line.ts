/**
 * Command lines Rerail writes for a shell to run, and the rules for quoting a word in each shell.
 */

/** The five characters PowerShell reads as a single quote: ' ‘ ’ ‚ ‛ (U+0027 and U+2018 to U+201B). */
export const POWERSHELL_QUOTES = "'‘’‚‛";

const POWERSHELL_QUOTE = /['‘-‛]/gu;

/**
 * `value` as one PowerShell single-quoted literal, which expands nothing: `$`, backticks, `%` and spaces stay as they
 * are. PowerShell ends the literal at any of {@link POWERSHELL_QUOTES} and reads one written twice as itself, so each
 * is written twice.
 */
export function powershellLiteral(value: string): string {
  return `'${value.replace(POWERSHELL_QUOTE, "$&$&")}'`;
}

/**
 * What an instruction tells an agent to do to `what`, a value it puts inside PowerShell single quotes itself, as
 * {@link powershellLiteral} does: `each ' ‘ ’ ‚ ‛ in <what> written twice`.
 */
export function powershellQuotesTwice(what: string): string {
  return `each ${[...POWERSHELL_QUOTES].join(" ")} in ${what} written twice`;
}
