/**
 * What a case reads in, or puts before, a script Rerail hands Windows PowerShell. Windows PowerShell finds a cmdlet named
 * alone (`Get-Process`) by first reading every installed module its cache has not read yet, which took 46 s with 40
 * large ones; one named with its module (`Microsoft.PowerShell.Management\Get-Process`) it loads by that name.
 */

/**
 * The aliases Windows PowerShell 5.1 defines for the commands of Microsoft.PowerShell.Core, Microsoft.PowerShell.Management
 * and Microsoft.PowerShell.Utility, as `Get-Alias` lists them in `powershell.exe -NoProfile` on Windows 11 (build 26300);
 * Microsoft.PowerShell.Security defines none. PowerShell finds an alias's command as it finds one named alone.
 */
export const POWERSHELL_ALIASES: readonly string[] = [
  "%", "?", "ac", "asnp", "cat", "cd", "CFS", "chdir", "clc", "clhy", "cli", "clp", "clv", "cnsn", "compare", "copy",
  "cp", "cpi", "cpp", "curl", "cvpa", "dbp", "del", "diff", "dir", "dnsn", "ebp", "echo", "epal", "epcsv", "epsn",
  "erase", "etsn", "exsn", "fc", "fhx", "fl", "foreach", "ft", "fw", "gal", "gbp", "gc", "gci", "gcm", "gcs", "gdr",
  "ghy", "gi", "gjb", "gl", "gm", "gmo", "gp", "gps", "gpv", "group", "gsn", "gsnp", "gsv", "gu", "gv", "gwmi", "h",
  "history", "icm", "iex", "ihy", "ii", "ipal", "ipcsv", "ipmo", "ipsn", "irm", "iwmi", "iwr", "kill", "lp", "ls",
  "measure", "mi", "mount", "move", "mp", "mv", "nal", "ndr", "ni", "nmo", "npssc", "nsn", "nv", "ogv", "oh", "popd",
  "ps", "pushd", "pwd", "r", "rbp", "rcjb", "rcsn", "rd", "rdr", "ren", "ri", "rjb", "rm", "rmdir", "rmo", "rni",
  "rnp", "rp", "rsn", "rsnp", "rujb", "rv", "rvpa", "rwmi", "sajb", "sal", "saps", "sasv", "sbp", "sc", "select",
  "set", "shcm", "si", "sl", "sleep", "sls", "sort", "sp", "spjb", "spps", "spsv", "start", "sujb", "sv", "swmi",
  "tee", "trcm", "type", "wget", "where", "wjb", "write",
];

/** A verb, a dash and a noun, in any case, with no module before it. */
const NAME_ALONE = /(?<![\w\\.-])[a-z]+-[a-z]\w*/giu;
/** Here-strings and quoted strings: text, not commands, as the C# the Credential Manager script hands `Add-Type`. */
const QUOTED = /@"[\s\S]*?\n"@|@'[\s\S]*?\n'@|"(?:[^"`]|`[\s\S])*"|'[^']*'/gu;
/** An alias where a command starts: a line's start, after `;`, a pipe, `{`, `(`, `=` or `&`. */
const ALIAS_AT_COMMAND = new RegExp(
  String.raw`(?:^|[;|{(=&])[ \t]*(${POWERSHELL_ALIASES.map(alias => alias.replace(/[?]/gu, "\\$&")).join("|")})(?=[\s;|)}]|$)`,
  "gimu",
);

/**
 * Each cmdlet `script` calls by its name alone, in the order the script names them: a verb, a dash and a noun in any
 * case (`ConvertTo-Json`, `out-null`), with no module before it, anywhere in the script, since one may run from a
 * string; and an alias from {@link POWERSHELL_ALIASES} in any case where a command starts, outside quoted strings and
 * here-strings, where a short word is only text. `foreach` counts after a pipe: at a statement's start it is
 * PowerShell's loop keyword. A hashtable key that is an alias (`@{ sort = 1 }`) reads as one.
 */
export function cmdletsWithoutModule(script: string): string[] {
  const found: Array<[number, string]> = [...script.matchAll(NAME_ALONE)].map(match => [match.index, match[0]]);
  const code = script.replace(QUOTED, text => text.replace(/[^\n]/gu, " "));
  for (const match of code.matchAll(ALIAS_AT_COMMAND)) {
    const alias = match[1] ?? "";
    const at = match.index + match[0].length - alias.length;
    if (alias.toLowerCase() !== "foreach" || match[0].startsWith("|")) found.push([at, script.slice(at, at + alias.length)]);
  }
  return found.sort(([a], [b]) => a - b).map(([, name]) => name);
}

/** The script that follows `-Command` in `args`, or undefined when none does. */
export function commandScript(args: readonly string[]): string | undefined {
  const at = args.indexOf("-Command");
  return at < 0 ? undefined : args[at + 1];
}

/** `args` with `prefix` put first in the script that follows `-Command`. */
export function withCommandPrefix(args: readonly string[], prefix: string): string[] {
  return args.map((arg, at) => (args[at - 1] === "-Command" ? `${prefix}${arg}` : arg));
}

/**
 * PowerShell's own setting for a machine whose modules it would have to read: autoloading limited to module-qualified
 * names, so a cmdlet named alone, from a module not loaded yet, is not found, and one named with its module runs.
 * Windows PowerShell 5.1 started with `-NoProfile -NonInteractive -Command` has no module loaded when the command
 * starts; process-liveness.test.ts shows `Get-Process` named alone is not found under it.
 */
export const MODULE_QUALIFIED_ONLY = "$PSModuleAutoLoadingPreference = 'ModuleQualified'; ";
