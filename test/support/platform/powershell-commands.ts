/**
 * What a case reads in, or puts before, a script Rerail hands Windows PowerShell. Windows PowerShell finds a cmdlet named
 * alone (`Get-Process`) by first reading every installed module its cache has not read yet, which took 46 s with 40
 * large ones; one named with its module (`Microsoft.PowerShell.Management\Get-Process`) it loads by that name.
 */

/**
 * Each cmdlet `script` calls by its name alone: a verb, a dash and a noun, each starting with a capital
 * (`ConvertTo-Json`), with no module before it. It misses aliases (`select`) and names written in lowercase, which
 * PowerShell finds the same way.
 */
export function cmdletsWithoutModule(script: string): string[] {
  return [...script.matchAll(/(?<![\w\\.-])[A-Z][A-Za-z]+-[A-Z]\w*/gu)].map(match => match[0]);
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
 */
export const MODULE_QUALIFIED_ONLY = "$PSModuleAutoLoadingPreference = 'ModuleQualified'; ";
