/**
 * A decision's signals: short literal code fragments whose presence in new code means the decision is at stake. A
 * guard compares them with the lines an agent adds; a hit is a reason to show the decision, not a violation.
 */
import { z } from "zod";

/** The most signals one decision carries. */
export const MAX_SIGNALS = 8;
const MIN_SIGNAL_LENGTH = 2;
/** The longest signal, in characters, after trimming. */
export const MAX_SIGNAL_LENGTH = 40;

/** A signal: a literal code fragment, 2–40 chars after trimming, no newline; matched case-sensitively as a substring. */
export const signalSchema: z.ZodType<string> = z.string()
  .trim()
  .min(MIN_SIGNAL_LENGTH, `a signal is at least ${MIN_SIGNAL_LENGTH} characters`)
  .max(MAX_SIGNAL_LENGTH, `a signal is at most ${MAX_SIGNAL_LENGTH} characters`)
  .refine(signal => !/[\r\n]/u.test(signal), "a signal is one line");

/** The signals that occur in `text`, in their order. */
export function signalHits(signals: readonly string[], text: string): string[] {
  return signals.filter(signal => text.includes(signal));
}

/** Words of the language's syntax: never a name the example declares, never a call head on their own. */
const KEYWORDS = new Set([
  "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "debugger", "def", "default", "delete",
  "do", "elif", "else", "enum", "except", "export", "extends", "false", "finally", "for", "from", "function", "if",
  "implements", "import", "in", "instanceof", "interface", "lambda", "let", "new", "null", "of", "raise", "return",
  "static", "super", "switch", "this", "throw", "true", "try", "type", "typeof", "undefined", "var", "void", "while",
  "with", "yield",
]);

const IDENTIFIER = String.raw`[A-Za-z_$][\w$]*`;
const CHAIN = String.raw`${IDENTIFIER}(?:\.${IDENTIFIER})*`;
/** The most fragments {@link mineSignals} returns. */
const MAX_MINED = 32;
/** A prefix {@link generalizeSignal} offers is at least this long. */
const MIN_PREFIX_LENGTH = 4;

/** `text` without comments and with string literals emptied, so their words are not read as code. */
function codeOnly(text: string): string {
  let code = "";
  for (let index = 0; index < text.length;) {
    const char = text[index]!;
    const next = text[index + 1];
    if (char === "/" && next === "/") {
      const end = text.indexOf("\n", index);
      index = end === -1 ? text.length : end;
    } else if (char === "/" && next === "*") {
      const end = text.indexOf("*/", index + 2);
      index = end === -1 ? text.length : end + 2;
      code += " ";
    } else if (char === "\"" || char === "'") {
      let end = index + 1;
      while (end < text.length && text[end] !== char && text[end] !== "\n") end += text[end] === "\\" ? 2 : 1;
      code += `${char}${char}`;
      index = end + 1;
    } else {
      code += char;
      index += 1;
    }
  }
  return code;
}

/** The binding names a list declares: `a`, `a = 1`, `...rest`, `a: Type` (parameters) or `key: a` (destructuring). */
function bindings(list: string, renamedAfterColon: boolean): string[] {
  return list.split(",").flatMap(part => {
    let binding = part.split("=")[0]!.trim().replace(/^\.\.\./u, "");
    if (binding.includes(":")) binding = renamedAfterColon ? binding.slice(binding.indexOf(":") + 1) : binding.slice(0, binding.indexOf(":"));
    const name = new RegExp(`^\\s*(${IDENTIFIER})`, "u").exec(binding.replace(/^\s*type\s+/u, ""))?.[1];
    return name === undefined ? [] : [name];
  });
}

/** The names `code` declares: variables, functions, classes and types, their parameters, and import bindings. */
function declaredNames(code: string): Set<string> {
  const names = new Set<string>();
  const each = (pattern: string, take: (match: RegExpExecArray) => string[]): void => {
    for (const match of code.matchAll(new RegExp(pattern, "gu"))) for (const name of take(match)) names.add(name);
  };
  each(String.raw`\b(?:const|let|var|class|interface|type|enum|def|function)(?:\s*\*\s*|\s+)(${IDENTIFIER})`, match => [match[1]!]);
  each(String.raw`\b(?:const|let|var)\s*[{[]([^}\]]*)[}\]]`, match => bindings(match[1]!, true));
  each(String.raw`\b(?:function|def)\b[^(]*\(([^)]*)\)`, match => bindings(match[1]!, false));
  each(String.raw`\(([^()]*)\)\s*(?::\s*[^=;{]+)?=>`, match => bindings(match[1]!, false));
  each(String.raw`(${IDENTIFIER})\s*=>`, match => [match[1]!]);
  each(String.raw`${IDENTIFIER}\s*\(([^()]*)\)\s*(?::\s*[^{;]+)?\{`, match => bindings(match[1]!, false));
  each(String.raw`\bimport\s+(?:type\s+)?(${IDENTIFIER})`, match => [match[1]!]);
  each(String.raw`\*\s*as\s+(${IDENTIFIER})`, match => [match[1]!]);
  each(String.raw`\bimport\s+(?:type\s+)?(?:${IDENTIFIER}\s*,\s*)?\{([^}]*)\}`, match =>
    match[1]!.split(",").flatMap(part => {
      const words = part.trim().replace(/^type\s+/u, "").split(/\s+as\s+/u);
      const name = words.at(-1)?.trim() ?? "";
      return new RegExp(`^${IDENTIFIER}$`, "u").test(name) ? [name] : [];
    }));
  for (const keyword of KEYWORDS) names.delete(keyword);
  return names;
}

/**
 * Fragments a correction removed: code in `before` that is not in `after`, read by syntax alone. They are member-access
 * chains (`a.b`, `a.b.c`), call heads (`name(`, `a.b(`), tagged-template heads (``tag` ``) and the keyword phrases
 * `export default`, `throw new`, `throw`, `new X(` and `await X(`. A fragment headed by a name the example declares
 * (a variable, function, class, parameter or import binding) is left out, as are comments and string contents. At
 * most {@link MAX_MINED}, in the order they occur; they are candidates for {@link signalSchema}, not yet checked by it.
 */
export function mineSignals(example: { before: string; after: string }): string[] {
  const code = codeOnly(example.before);
  const declared = declaredNames(`${code}\n${codeOnly(example.after)}`);
  const found: Array<{ index: number; token: string }> = [];
  const named = (chain: string): boolean => {
    const head = chain.split(".")[0]!;
    return !KEYWORDS.has(head) && !declared.has(head);
  };
  for (const match of code.matchAll(new RegExp(String.raw`(?<![\w$.])${CHAIN}`, "gu"))) {
    const chain = match[0];
    if (!named(chain)) continue;
    const next = code[match.index + chain.length];
    if (next === "(" || next === "`") found.push({ index: match.index, token: `${chain}${next}` });
    else if (chain.includes(".")) found.push({ index: match.index, token: chain });
  }
  for (const match of code.matchAll(/\bexport\s+default\b/gu)) found.push({ index: match.index, token: "export default" });
  for (const match of code.matchAll(/\bthrow\s+new\b/gu)) found.push({ index: match.index, token: "throw new" });
  for (const match of code.matchAll(/\bthrow\b/gu)) found.push({ index: match.index, token: "throw" });
  for (const match of code.matchAll(new RegExp(String.raw`\b(new|await)\s+(${CHAIN})\s*\(`, "gu"))) {
    if (named(match[2]!)) found.push({ index: match.index, token: `${match[1]} ${match[2]}(` });
  }
  const mined = new Set<string>();
  for (const { token } of found.sort((left, right) => left.index - right.index)) {
    if (mined.size >= MAX_MINED) break;
    if (example.before.includes(token) && !example.after.includes(token)) mined.add(token);
  }
  return [...mined];
}

/**
 * `signal` and its prefixes at syntax boundaries, most specific first and most general last: before each `(` or
 * backtick; after the last `.` of its head (the part before any `(`), and before it when what is left is still a
 * member access; and at a space between two keywords. `console.log(` → `console.log`, `console.`;
 * `process.env.API_URL` → `process.env.`, `process.env`; `throw new Error(` → `throw new Error`, `throw`. A prefix is
 * at least four characters and holds an identifier character.
 */
export function generalizeSignal(signal: string): string[] {
  const prefixes = new Set<string>();
  for (let index = 0; index < signal.length; index += 1) {
    if (signal[index] === "(" || signal[index] === "`") prefixes.add(signal.slice(0, index));
  }
  const head = signal.split(/[(`]/u)[0]!;
  const dot = head.lastIndexOf(".");
  if (dot >= 0) {
    prefixes.add(head.slice(0, dot + 1));
    const object = head.slice(0, dot);
    if (object.slice(object.lastIndexOf(" ") + 1).includes(".")) prefixes.add(object);
  }
  for (const space of head.matchAll(/\s+/gu)) {
    const before = head.slice(0, space.index).split(/\s+/u).at(-1) ?? "";
    const after = head.slice(space.index + space[0].length).split(/[^\w$]/u)[0] ?? "";
    if (KEYWORDS.has(before) && KEYWORDS.has(after)) prefixes.add(head.slice(0, space.index));
  }
  const general = [...prefixes]
    .map(prefix => prefix.trim())
    .filter(prefix => prefix !== signal && prefix.length >= MIN_PREFIX_LENGTH && /[\w$]/u.test(prefix))
    .sort((left, right) => right.length - left.length);
  return [signal, ...new Set(general)];
}
