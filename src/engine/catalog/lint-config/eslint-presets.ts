/**
 * The shared ESLint configs ("presets") set-up recognises, from the releases the packs port
 * (`eslint-presets.generated.ts`): `@eslint/js`'s `recommended` and `all` (eslintrc `eslint:recommended`, `eslint:all`),
 * typescript-eslint's configs, and those of eslint-plugin-react, eslint-plugin-react-hooks, eslint-plugin-unicorn,
 * eslint-plugin-n, @next/eslint-plugin-next and eslint-config-next, eslint-plugin-vue and @vitest/eslint-plugin. A flat
 * config names one by the module it imports and the member path it reads (`js.configs.recommended`), an eslintrc by its
 * `extends` name (`plugin:react/recommended`). Each is its rule entries, severities only, in the order ESLint applies
 * them.
 */
import { ESLINT_FLAT_PRESETS, ESLINT_PRESET_NAMED_EXPORTS, type EslintPresetEntry } from "./generated/eslint-presets.generated.js";

/** A namespace import's path (`ns.default.configs`, `ns.configs`) as one from the default export, where it is one. */
function namespacePath(named: readonly string[], path: readonly string[]): readonly string[] | undefined {
  if (path[0] === "default") return path.slice(1);
  return path[0] !== undefined && named.includes(path[0]) ? path : undefined;
}

/** The member path from module `module`'s default export a binding imported as `imported` (`default`, `*`, a name) reaches with `path`. */
function defaultExportPath(module: string, imported: string, path: readonly string[]): readonly string[] | undefined {
  if (imported === "default") return path;
  const named = ESLINT_PRESET_NAMED_EXPORTS.get(module) ?? [];
  if (imported === "*") return namespacePath(named, path);
  return named.includes(imported) ? [imported, ...path] : undefined;
}

/**
 * The preset a flat config reads at member `path` of the binding it imports as `imported` from `module`; undefined
 * for anything else. A config object's `rules` member is a preset of one entry for every file.
 */
export function flatPreset(module: string, imported: string, path: readonly string[]): readonly EslintPresetEntry[] | undefined {
  const member = defaultExportPath(module, imported, path);
  return member === undefined ? undefined : ESLINT_FLAT_PRESETS.get(JSON.stringify([module, ...member]));
}
