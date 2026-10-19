/** Where a hardcoded word sits in a policy document. */
export interface PolicyLiteral {
  token: string;
  pointer: string;
  /**
   * `pattern`: code the engine matches or emits; `regex`: a regular expression; `context`: a pattern's context code,
   * whose words matter only inside the selected node; `value`: a name the DSL takes literally (resolved globals and
   * imports, argument slots, static values, declared globals).
   */
  role: "pattern" | "regex" | "context" | "value";
  text: string;
  /** For `context` words: the selector kind and the query language, so the gate can find the selected node. */
  selector?: string;
  language?: string;
  /** For `regex` words: whether the regex reads comment text (file content, or comment nodes). */
  commentText?: boolean;
}

/** One regex field of a policy, whatever words it spells. */
export interface PolicyRegex {
  pointer: string;
  text: string;
  /** True for a `file.content`/`file.path` regex, which runs over whole text; false for a regex on one node's text. */
  wholeText: boolean;
  /** Whether the regex reads comment text (file content, or comment nodes). */
  commentText: boolean;
}

const METAVARIABLE = /\$\$?\$?[A-Z_][A-Z0-9_]*/gu;
const CODE_WORD = /[A-Za-z_][\w$]*/gu;

/**
 * The keys whose string values carry code, by the role they play, read from rerail's policy schema at the engine pin
 * (src/engine/code-rule/code-rule-schema.ts). Every other string is DSL vocabulary: grammar node kinds, enums, capture
 * and utility names, anchors and violation metadata.
 */
const ROLE_BY_KEY: Record<string, PolicyLiteral["role"]> = {
  pattern: "pattern",
  fix: "pattern",
  template: "pattern",
  by: "pattern",
  commute: "pattern",
  context: "context",
  regex: "regex",
  notRegex: "regex",
  replace: "regex",
  path: "regex",
  module: "regex",
  moduleName: "regex",
  package: "regex",
  eq: "value",
  startsWith: "value",
  signature: "value",
  absent: "value",
  global: "value",
  import: "value",
  imported: "value",
  jsxPragma: "value",
  jsxFragmentName: "value",
  typingModules: "value",
  extendGenerics: "value",
};

/** Argument slots name keyword arguments only in Python; TypeScript and JavaScript fill slots by position. */
const PYTHON_ONLY_KEYS = new Set(["signature", "absent"]);

/** Objects whose keys are code names: `languageOptions.globals` declares globals by name. */
const NAME_KEYED_OBJECTS = new Set(["globals"]);

/** Words of a regex after removing escapes, character classes, group syntax and inline flags. */
export function regexWords(pattern: string): string[] {
  const stripped = pattern
    // An escaped backslash before a letter matches a source escape sequence such as `\n`, not a word.
    .replace(/\\\\(?:u\{[0-9A-Fa-f]+\}|u[0-9A-Fa-f]{4}|x[0-9A-Fa-f]{2}|[A-Za-z])/gu, " ")
    .replace(/\[(?:\\.|[^\]\\])*\]/gu, " ")
    .replace(/\(\?(?:P?<[A-Za-z_]\w*>|[:=!]|<[=!]|[a-z]+[:)])/gu, " ")
    .replace(/\\k<[A-Za-z_]\w*>/gu, " ")
    .replace(/\\(?:x\{[0-9A-Fa-f]+\}|x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f]+\}|u[0-9A-Fa-f]{4}|[pP]\{[^}]*\}|.)/gu, " ");
  // An unescaped `$` is an anchor, so regex words start with a letter or underscore.
  return stripped.match(/[A-Za-z_]\w*/gu) ?? [];
}

/**
 * The regex with each group of plain alternatives written between word characters replaced by the whole words it
 * spells: `__(eq|ne)__` spells `__eq__` and `__ne__`, `(get|set)attr` spells `getattr` and `setattr`. Only words are
 * read from the result, so the gate judges the names a rule matches rather than their fragments.
 */
export function affixedAlternations(pattern: string): string {
  return pattern.replace(
    /(?<=^|[^\w\\]|\\[bBAz])([A-Za-z_]\w*)?\((?:\?:)?(\w+(?:\|\w+)*)\)(?![?*+{])(\w+)?/gu,
    (whole, prefix: string | undefined, alternatives: string, suffix: string | undefined) => (prefix === undefined && suffix === undefined
      ? whole
      : ` ${alternatives.split("|").map((alternative) => `${prefix ?? ""}${alternative}${suffix ?? ""}`).join(" ")} `),
  );
}

/** Words of code with its metavariables (wildcards such as `$A` and `$$$B`) removed. */
export function codeWords(text: string): string[] {
  return text.replace(METAVARIABLE, " ").match(CODE_WORD) ?? [];
}

function escapePointer(token: string): string {
  return token.replaceAll("~", "~0").replaceAll("/", "~1");
}

interface Field {
  pointer: string;
  key: string;
  value: string;
  parent: Record<string, unknown> | undefined;
  language: string | undefined;
  /** Whether the node this field describes is a comment: some `kind` on the same node (through `all`/`any`) says so. */
  onComment: boolean;
}

/** Whether a rule says, on the node it describes (directly or through `all`/`any`), that the node is of `kind`. */
function describesKind(record: Record<string, unknown>, kind: string): boolean {
  return record.kind === kind || ["all", "any"].some((key) => Array.isArray(record[key])
    && (record[key] as unknown[]).some((item) => item !== null && typeof item === "object" && describesKind(item as Record<string, unknown>, kind)));
}

/** Relations that describe another node; `all`, `any` and `not` still describe the same one. */
const NODE_MOVES = new Set(["has", "inside", "follows", "precedes"]);

/**
 * Every string field under the clauses, with its key, parent object and the query language it sits in. A `parse` reads
 * a literal's text as a tree of its own, so every node under it is part of that literal, and part of a comment's text
 * when the literal is a comment, whatever relation reaches it; its `host` is the literal again. The `value` of a parsed
 * grammar's `error` node is the grammar's own message (for `percentFormat`, Ruff's), not code, so its fields are left out.
 */
function stringFields(document: { clauses?: unknown }): { fields: Field[]; nameKeys: { pointer: string; key: string }[] } {
  const fields: Field[] = [];
  const nameKeys: { pointer: string; key: string }[] = [];
  interface Place {
    language: string | undefined;
    /** The node is part of a comment's text. */
    onComment: boolean;
    /** For each enclosing `parse`, innermost last, whether the literal it reads is a comment. */
    parsed: boolean[];
    /** The node is a parsed grammar's `error` node. */
    onError: boolean;
  }
  const visit = (value: unknown, pointer: string, key: string | undefined, parent: Record<string, unknown> | undefined, place: Place): void => {
    if (Array.isArray(value)) {
      value.forEach((item, index) => visit(item, `${pointer}/${index}`, key, parent, place));
      return;
    }
    if (value !== null && typeof value === "object") {
      const record = value as Record<string, unknown>;
      const language = typeof record.language === "string" ? record.language : place.language;
      const comment = place.onComment || describesKind(record, "comment");
      const error = place.parsed.length > 0 && (place.onError || describesKind(record, "error"));
      for (const [childKey, child] of Object.entries(record)) {
        if (error && childKey === "value") continue;
        const childPointer = `${pointer}/${escapePointer(childKey)}`;
        if (key !== undefined && NAME_KEYED_OBJECTS.has(key)) nameKeys.push({ pointer: childPointer, key: childKey });
        if (childKey === "parse") {
          visit(child, childPointer, childKey, record, { language, onComment: comment, parsed: [...place.parsed, comment], onError: false });
        } else if (childKey === "host" && place.parsed.length > 0) {
          visit(child, childPointer, childKey, record, { language, onComment: place.parsed.at(-1)!, parsed: place.parsed.slice(0, -1), onError: false });
        } else {
          const moves = NODE_MOVES.has(childKey);
          visit(child, childPointer, childKey, record, { language, onComment: moves ? place.parsed.at(-1) ?? false : comment, parsed: place.parsed, onError: moves ? false : error });
        }
      }
      return;
    }
    if (typeof value === "string" && key !== undefined) fields.push({ pointer, key, value, parent, language: place.language, onComment: place.onComment });
  };
  visit(document.clauses, "/clauses", undefined, undefined, { language: undefined, onComment: false, parsed: [], onError: false });
  return { fields, nameKeys };
}

/**
 * Whether a signature entry names a slot that cannot change what the rule matches: the engine reads only the slots the
 * arguments bind or list as absent (and a keyword reaches only the slot of its own name), so in any mode a slot the
 * arguments neither bind nor list as absent only holds a position, and its name is a placeholder.
 */
function placeholderSlot(field: Field): boolean {
  const args = field.parent;
  if (field.key !== "signature" || args === undefined) return false;
  const bound = args.bind !== null && typeof args.bind === "object" ? Object.values(args.bind as Record<string, unknown>) : [];
  const absent = Array.isArray(args.absent) ? args.absent : [];
  return ![...bound, ...absent].includes(field.value);
}

/** A transform's pattern or replacement text: `/clauses/<clause>/.../transform/<variable>/replace/{replace,by}`. */
const TRANSFORM_TEXT = /^\/clauses\/(\d+)\/(?:.*\/)?transform\/([A-Z0-9_]+)\/replace\/(?:replace|by)$/u;
const FIX_VARIABLE = /\$(?:\$\$)?([A-Z0-9_]+)/gu;

/**
 * For each clause, the transform variables whose text reaches code: a fix interpolates it (`$NAME`) or a `compare`
 * reads it, directly or through the transforms that read it as their source. A transform only a message reads builds
 * violation text, which is not code.
 */
function codeTransforms(document: { clauses?: unknown }): Map<number, Set<string>> {
  const byClause = new Map<number, Set<string>>();
  if (!Array.isArray(document.clauses)) return byClause;
  document.clauses.forEach((clause, index) => {
    const sources = new Map<string, string>();
    const read: string[] = [];
    const visit = (value: unknown, key: string | undefined): void => {
      if (Array.isArray(value)) return value.forEach((item) => visit(item, key));
      if (value === null || typeof value !== "object") {
        if (key === "fix" || key === "template") read.push(...[...String(value).matchAll(FIX_VARIABLE)].map((match) => match[1]!));
        return;
      }
      const record = value as Record<string, unknown>;
      if (key === "transform") {
        for (const [name, transformation] of Object.entries(record)) {
          const body = transformation !== null && typeof transformation === "object" ? Object.values(transformation as Record<string, unknown>)[0] : undefined;
          const source = body !== null && typeof body === "object" ? (body as Record<string, unknown>).source : undefined;
          if (typeof source === "string") sources.set(name, source.replace(/^\$+/u, ""));
        }
      }
      if (key === "compare") read.push(...[record.left, record.right].filter((name): name is string => typeof name === "string"));
      // A fix's `template` names variables as a fix string does; elsewhere a `template` is an option's, not a fix's.
      for (const [childKey, child] of Object.entries(record)) visit(child, childKey === "template" && key !== "fix" ? undefined : childKey);
    };
    visit(clause, undefined);
    const reached = new Set<string>();
    for (let name = read.pop(); name !== undefined; name = read.pop()) {
      if (reached.has(name)) continue;
      reached.add(name);
      const source = sources.get(name);
      if (source !== undefined) read.push(source);
    }
    byClause.set(index, reached);
  });
  return byClause;
}

function roleOf(field: Field): PolicyLiteral["role"] | undefined {
  if (field.key === "pattern" && field.parent !== undefined && "source" in field.parent) return "regex";
  if (PYTHON_ONLY_KEYS.has(field.key) && field.language !== "python") return undefined;
  return ROLE_BY_KEY[field.key];
}

function readsComments(field: Field): boolean {
  return (field.parent !== undefined && field.parent.source === "file.content") || field.onComment;
}

/** Every regex field of a policy, including regexes that spell no word, so the gate can judge what each matches. */
export function policyRegexes(document: { clauses?: unknown }): PolicyRegex[] {
  return stringFields(document).fields
    .filter((field) => roleOf(field) === "regex")
    .map((field) => ({
      pointer: field.pointer,
      text: field.value,
      wholeText: field.parent !== undefined && "source" in field.parent,
      commentText: readsComments(field),
    }));
}

/** Every identifier-like word a rule's clauses hardcode in a field that carries code, with the role it plays. */
export function policyLiterals(document: { clauses?: unknown }): PolicyLiteral[] {
  const literals: PolicyLiteral[] = [];
  const add = (literal: Omit<PolicyLiteral, "token">, tokens: string[]) => {
    for (const token of new Set(tokens)) literals.push({ token, ...literal });
  };
  const { fields, nameKeys } = stringFields(document);
  const reachesCode = codeTransforms(document);
  for (const field of fields) {
    const role = roleOf(field);
    if (role === undefined) continue;
    if (placeholderSlot(field)) continue;
    const transform = TRANSFORM_TEXT.exec(field.pointer);
    if (transform !== null && !reachesCode.get(Number(transform[1]))?.has(transform[2]!)) continue;
    const { pointer, value } = field;
    // A one-letter regex fragment (`n$` for a BigInt suffix) is syntax; the gate judges regexes by what they match.
    if (role === "regex") add({ pointer, role, text: value, commentText: readsComments(field) }, regexWords(affixedAlternations(value)).filter((word) => word.length > 1));
    else if (role === "context") {
      const selector = typeof field.parent?.selector === "string" ? field.parent.selector : undefined;
      add({ pointer, role, text: value, ...(selector === undefined ? {} : { selector }), ...(field.language === undefined ? {} : { language: field.language }) }, codeWords(value));
    } else add({ pointer, role, text: value }, codeWords(value));
  }
  for (const { pointer, key } of nameKeys) add({ pointer, role: "value", text: key }, codeWords(key));
  return literals;
}
