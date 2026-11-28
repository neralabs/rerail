const ESCAPES: Record<string, string> = { "<": "\\u003c", ">": "\\u003e", "&": "\\u0026", "\u2028": "\\u2028", "\u2029": "\\u2029" };

export function htmlEscapeJsonString(json: string) {
  return json.replace(/[<>&\u2028\u2029]/g, (c) => ESCAPES[c]);
}
