/**
 * Command lines Rerail writes for a shell to run, the rules for quoting a word in each shell, and the readers of a
 * quoted word, so a rule and its reader change together. A caller names the shell the line runs in, never a platform:
 * Claude Code runs commands in Git Bash on Windows, a POSIX shell, and Codex and Cursor in PowerShell
 * (`windowsCommandShell`). A hook command is a stored format that registration recognises by its exact text, so
 * `harness-registration.ts` writes and reads those itself, with the quoted words read here.
 */

/** A shell Rerail writes a command line for: a POSIX shell (sh, bash, zsh, or Git Bash on Windows), or PowerShell. */
export type Shell = "posix" | "powershell";

/** The five characters PowerShell reads as a single quote: ' ‘ ’ ‚ ‛ (U+0027 and U+2018 to U+201B). */
export const POWERSHELL_QUOTES = "'‘’‚‛";

const POWERSHELL_QUOTE = new RegExp(`[${POWERSHELL_QUOTES}]`, "gu");

/** A quote of {@link POWERSHELL_QUOTES} written twice, as {@link powershellLiteral} writes each. */
const DOUBLED_POWERSHELL_QUOTE = new RegExp(`([${POWERSHELL_QUOTES}])\\1`, "gu");

/**
 * What {@link powershellLiteral} writes between its two `'`, as RegExp source for the `u` flag: any character but a
 * quote, or one quote written twice. It holds no capture group, so a reader puts it inside a pattern of its own,
 * captures and back-references included, with the ends it expects around it.
 */
export const POWERSHELL_LITERAL_BODY = `(?:[^${POWERSHELL_QUOTES}]|${[...POWERSHELL_QUOTES].map(quote => quote.repeat(2)).join("|")})*`;

/** The value a {@link POWERSHELL_LITERAL_BODY} holds: each quote written twice, read once. */
export function powershellLiteralValue(body: string): string {
  return body.replace(DOUBLED_POWERSHELL_QUOTE, "$1");
}

/** How a POSIX single-quoted word writes a `'`: it ends the quotes, writes the `'` escaped, and opens them again. */
const POSIX_QUOTE_ESCAPE = String.raw`'\''`;

/**
 * What {@link shellWord} writes between the single quotes of a POSIX word, as RegExp source for the `u` flag: any
 * character but `'`, or a `'` written `'\''`. Like {@link POWERSHELL_LITERAL_BODY} it holds no capture group.
 */
export const POSIX_QUOTED_BODY = String.raw`(?:[^']|'\\'')*`;

/** The value a {@link POSIX_QUOTED_BODY} holds: each `'\''` read as the `'` it stands for. */
export function posixQuotedValue(body: string): string {
  return body.replaceAll(POSIX_QUOTE_ESCAPE, "'");
}

/**
 * What a POSIX shell reads as one plain word: no space, quote, `$`, backtick, glob or other character it gives a
 * meaning to, and no `~` first, which it would expand to the home folder.
 */
const POSIX_SAFE = /^[\w@%+=:,./-][\w@%+=:,./~-]*$/u;

/** What PowerShell reads as one plain word: no `@`, `,`, `$`, quote or other character it gives a meaning to. */
const POWERSHELL_SAFE = /^[\w./:\\-]+$/u;

/**
 * `value` as one PowerShell single-quoted literal, which expands nothing: `$`, backticks, `%` and spaces stay as they
 * are. PowerShell ends the literal at any of {@link POWERSHELL_QUOTES} and reads one written twice as itself, so each
 * is written twice.
 */
export function powershellLiteral(value: string): string {
  return `'${value.replace(POWERSHELL_QUOTE, "$&$&")}'`;
}

/**
 * `value` as one word of `shell`: as it is where the shell reads it as plain text, else quoted so the shell reads it
 * back unchanged. A POSIX shell gets single quotes, each `'` in the value written `'\''`; PowerShell gets a literal
 * ({@link powershellLiteral}).
 */
export function shellWord(shell: Shell, value: string): string {
  if (shell === "powershell") return POWERSHELL_SAFE.test(value) ? value : powershellLiteral(value);
  return POSIX_SAFE.test(value) ? value : `'${value.replaceAll("'", POSIX_QUOTE_ESCAPE)}'`;
}

/**
 * `words`, the program and then its arguments, as one line `shell` runs, each word a {@link shellWord}. PowerShell's
 * line starts with the call operator `&`, since PowerShell takes a line that starts with a quoted path for a string
 * and refuses the words after it.
 */
export function commandLine(shell: Shell, words: readonly string[]): string {
  const line = words.map(word => shellWord(shell, word)).join(" ");
  return shell === "powershell" ? `& ${line}` : line;
}

/**
 * Where an instruction has an agent write a value of its own into a line for `shell`, such as `<packs>`: as it is for a
 * POSIX shell, where the agent quotes the value as it needs, and inside single quotes for PowerShell, which would read a
 * bare `a@1,b@2` as two arguments and give `@`, `$` and more a meaning of their own.
 */
export function placeholderWord(shell: Shell, placeholder: string): string {
  return shell === "powershell" ? `'${placeholder}'` : placeholder;
}

/**
 * What an instruction tells an agent to do to `what`, a value it puts inside PowerShell single quotes itself, as
 * {@link powershellLiteral} does: `each ' ‘ ’ ‚ ‛ in <what> written twice`.
 */
export function powershellQuotesTwice(what: string): string {
  return `each ${[...POWERSHELL_QUOTES].join(" ")} in ${what} written twice`;
}

/**
 * {@link placeholderWord} for text the person types as they like, such as a folder name, which may hold a quote. For
 * PowerShell it adds what the agent does to each quote in that text ({@link powershellQuotesTwice}): the single quotes
 * around the placeholder would end at one, and the rest of the line would not parse.
 */
export function textPlaceholder(shell: Shell, placeholder: string): string {
  const word = placeholderWord(shell, placeholder);
  return shell === "powershell" ? `${word}, with ${powershellQuotesTwice(placeholder)}` : word;
}
