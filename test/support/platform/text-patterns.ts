/**
 * `text` as regular-expression source that matches it exactly: a Windows path's backslashes and a command's quotes and
 * dots stay literal. Only the syntax characters are escaped, so the source is valid with the `u` flag.
 */
export function literal(text: string): string {
  return text.replace(/[\\^$.*+?()[\]{}|/]/gu, "\\$&");
}
