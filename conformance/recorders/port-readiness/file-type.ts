import { extname } from "node:path";

import { CODE_RULE_DIALECTS } from "../../../src/engine/index.js";

/** Each supported dialect suffix (src/engine's CODE_RULE_DIALECT_SUFFIXES). */
const DIALECT_SUFFIXES: readonly string[] = Object.values(CODE_RULE_DIALECTS).flatMap((dialect) => dialect.suffixes);

/**
 * A path's file type: the longest dialect suffix of the language registry it ends with when that is longer than its
 * extension (`.blade.php`, a Blade template rather than PHP, as the engine routes it), or else its extension. A parser
 * verdict is named by it (recording/verdicts.ts), so a template's verdict is never a PHP file's.
 */
export function fileTypeOf(path: string): string {
  const name = path.slice(path.lastIndexOf("/") + 1);
  const lower = name.replace(/[A-Z]+/g, (upper) => upper.toLowerCase());
  let found = extname(name);
  for (const suffix of DIALECT_SUFFIXES) {
    if (lower.length > suffix.length && lower.endsWith(suffix) && suffix.length > found.length) found = name.slice(-suffix.length);
  }
  return found;
}
