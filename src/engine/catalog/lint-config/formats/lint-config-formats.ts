/**
 * What the small readers set-up needs for a repository's own lint config (`repository-lint-config.ts`) share: each
 * format reader (`jsonc-format.ts`, `toml-format.ts`, `ini-format.ts`, `neon-format.ts`, `xml-format.ts`) refuses
 * what it cannot read rather than guess: it throws `LintConfigFormatError`, which `readFormat` turns into the reason a
 * file is not read.
 */

export class LintConfigFormatError extends Error {}

/** `read()`'s value, or why the file is not read when a format reader refuses it; any other error is a bug and propagates. */
export function readFormat<T>(read: () => T): { readonly ok: true; readonly value: T } | { readonly ok: false; readonly why: string } {
  try {
    return { ok: true, value: read() };
  } catch (error) {
    if (error instanceof LintConfigFormatError) return { ok: false, why: error.message };
    throw error;
  }
}
