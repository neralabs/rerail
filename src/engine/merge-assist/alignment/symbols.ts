/**
 * The declarations a source file makes, read line by line for TypeScript, JavaScript, Python, Go and Rust, and the
 * names a text refers to. Alignment anchors units to these names and compares an exported declaration's signature at
 * two revisions. A declaration-head scan, not a parse: it needs no grammar per language and reads any revision Git
 * holds, including ones that no longer build; it misses what only a parser sees (nested and generated declarations),
 * which costs a unit, never a wrong one.
 */

export type SourceLanguage = "typescript" | "javascript" | "python" | "go" | "rust";

export type DeclarationKind = "function" | "class" | "type" | "constant" | "method" | "module" | "macro";

export interface Declaration {
  /** The declared name; a method's is qualified by its owner (`Store.put`, `Tape.write`). */
  readonly name: string;
  readonly kind: DeclarationKind;
  /** Visible outside its module: `export`, `pub`, a capitalised Go name, a Python name without a leading `_`. */
  readonly exported: boolean;
  /** The declaration head (up to the body), whitespace collapsed; a constant's whole first statement line. */
  readonly signature: string;
  /** 1-based. */
  readonly line: number;
}

const EXTENSIONS: ReadonlyArray<readonly [RegExp, SourceLanguage]> = [
  [/\.(?:ts|tsx|mts|cts)$/u, "typescript"],
  [/\.(?:js|jsx|mjs|cjs)$/u, "javascript"],
  [/\.(?:py|pyi)$/u, "python"],
  [/\.go$/u, "go"],
  [/\.rs$/u, "rust"],
];

/** The source language of `path` by its extension, or null for any other file. */
export function sourceLanguageOf(path: string): SourceLanguage | null {
  for (const [pattern, language] of EXTENSIONS) if (pattern.test(path)) return language;
  return null;
}

/** The longest signature kept, in characters. */
const MAX_SIGNATURE = 400;
/** A declaration head spans at most this many lines before it is cut. */
const MAX_HEAD_LINES = 12;

interface HeadPattern {
  readonly pattern: RegExp;
  readonly kind: DeclarationKind;
  /** The capture group holding the name. */
  readonly name: number;
  readonly exported: (match: RegExpExecArray, indent: number) => boolean;
  /** Only at the file's top level (no indentation). */
  readonly topLevel: boolean;
}

const ECMA_PATTERNS: readonly HeadPattern[] = [
  { pattern: /^(export\s+(?:default\s+)?)?(?:declare\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][\w$]*)/u, kind: "function", name: 2, exported: match => match[1] !== undefined, topLevel: true },
  { pattern: /^(export\s+(?:default\s+)?)?(?:declare\s+)?(?:abstract\s+)?class\s+([A-Za-z_$][\w$]*)/u, kind: "class", name: 2, exported: match => match[1] !== undefined, topLevel: true },
  { pattern: /^(export\s+)?(?:declare\s+)?(?:interface|type|(?:const\s+)?enum|namespace)\s+([A-Za-z_$][\w$]*)/u, kind: "type", name: 2, exported: match => match[1] !== undefined, topLevel: true },
  { pattern: /^(export\s+)?(?:declare\s+)?(?:const|let|var)\s+([A-Za-z_$][\w$]*)/u, kind: "constant", name: 2, exported: match => match[1] !== undefined, topLevel: true },
];

const PYTHON_PATTERNS: readonly HeadPattern[] = [
  { pattern: /^(?:async\s+)?def\s+([A-Za-z_]\w*)/u, kind: "function", name: 1, exported: match => !match[1]!.startsWith("_"), topLevel: false },
  { pattern: /^class\s+([A-Za-z_]\w*)/u, kind: "class", name: 1, exported: match => !match[1]!.startsWith("_"), topLevel: true },
  { pattern: /^([A-Z][A-Z0-9_]+)\s*(?::[^=]+)?=(?!=)/u, kind: "constant", name: 1, exported: () => true, topLevel: true },
];

const GO_PATTERNS: readonly HeadPattern[] = [
  { pattern: /^func\s+(?:\(\s*\w*\s*\*?\s*([A-Za-z_]\w*)(?:\[[^\]]*\])?\s*\)\s*)?([A-Za-z_]\w*)/u, kind: "function", name: 2, exported: match => /^[A-Z]/u.test(match[2]!), topLevel: true },
  { pattern: /^type\s+([A-Za-z_]\w*)/u, kind: "type", name: 1, exported: match => /^[A-Z]/u.test(match[1]!), topLevel: true },
  { pattern: /^(?:const|var)\s+([A-Za-z_]\w*)/u, kind: "constant", name: 1, exported: match => /^[A-Z]/u.test(match[1]!), topLevel: true },
];

const RUST_VISIBILITY = String.raw`(pub(?:\([^)]*\))?\s+)?`;
const RUST_PATTERNS: readonly HeadPattern[] = [
  { pattern: new RegExp(String.raw`^${RUST_VISIBILITY}(?:default\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:extern\s+"[^"]*"\s+)?fn\s+([A-Za-z_]\w*)`, "u"), kind: "function", name: 2, exported: match => match[1] !== undefined, topLevel: false },
  { pattern: new RegExp(String.raw`^${RUST_VISIBILITY}(?:unsafe\s+)?(?:struct|enum|trait|union|type)\s+([A-Za-z_]\w*)`, "u"), kind: "type", name: 2, exported: match => match[1] !== undefined, topLevel: false },
  { pattern: new RegExp(String.raw`^${RUST_VISIBILITY}(?:const|static)\s+(?:mut\s+)?([A-Za-z_]\w*)\s*:`, "u"), kind: "constant", name: 2, exported: match => match[1] !== undefined, topLevel: false },
  { pattern: new RegExp(String.raw`^${RUST_VISIBILITY}mod\s+([A-Za-z_]\w*)`, "u"), kind: "module", name: 2, exported: match => match[1] !== undefined, topLevel: true },
  { pattern: /^()macro_rules!\s*([A-Za-z_]\w*)/u, kind: "macro", name: 2, exported: () => true, topLevel: true },
];

const PATTERNS: Record<SourceLanguage, readonly HeadPattern[]> = {
  typescript: ECMA_PATTERNS,
  javascript: ECMA_PATTERNS,
  python: PYTHON_PATTERNS,
  go: GO_PATTERNS,
  rust: RUST_PATTERNS,
};

/** `export { a, b as c }` and `export type { T }`: names a module exports by list. */
const EXPORT_LIST = /^export\s+(?:type\s+)?\{([^}]*)\}/u;

function collapse(text: string): string {
  return text.replace(/\s+/gu, " ").trim();
}

/** The head of the declaration starting at `lines[start]`: through its parameters, cut before its body. */
function headOf(lines: readonly string[], start: number, language: SourceLanguage, kind: DeclarationKind): string {
  if (kind === "constant") return collapse(lines[start]!).slice(0, MAX_SIGNATURE);
  let depth = 0;
  let head = "";
  for (let index = start; index < Math.min(lines.length, start + MAX_HEAD_LINES); index += 1) {
    const line = lines[index]!;
    for (let position = 0; position < line.length; position += 1) {
      const char = line[position]!;
      if (char === "(" || char === "[" || char === "<") depth += 1;
      else if ((char === ")" || char === "]" || char === ">") && depth > 0 && line[position - 1] !== "=") depth -= 1;
      const bodyStarts = depth === 0 && (char === "{" || char === ";" || (language === "python" && char === ":" && line.slice(position + 1).trim() === ""));
      if (bodyStarts) return collapse(`${head} ${line.slice(0, position)}`).slice(0, MAX_SIGNATURE);
    }
    head = `${head} ${line}`;
    if (depth === 0 && /=>\s*$|=\s*$/u.test(line)) break;
  }
  return collapse(head).slice(0, MAX_SIGNATURE);
}

/**
 * The declarations `text` makes, in order, for a file in `language`. Python methods are qualified by the enclosing
 * top-level class, Go methods by their receiver type, Rust associated functions by the `impl` they sit in.
 */
export function declarations(language: SourceLanguage, text: string): Declaration[] {
  const lines = text.split(/\r?\n/u);
  const found: Declaration[] = [];
  const exportedByList = new Set<string>();
  let owner: { name: string; indent: number } | null = null;
  for (let index = 0; index < lines.length; index += 1) {
    const raw = lines[index]!;
    const trimmed = raw.trimStart();
    if (trimmed === "") continue;
    const indent = raw.length - trimmed.length;
    if (owner !== null && indent <= owner.indent && !trimmed.startsWith("}") && !trimmed.startsWith(")")) owner = null;
    if (language === "typescript" || language === "javascript") {
      const list = indent === 0 ? EXPORT_LIST.exec(trimmed) : null;
      if (list !== null) {
        for (const entry of list[1]!.split(",")) {
          const name = entry.trim().split(/\s+as\s+/u).pop()?.replace(/^type\s+/u, "").trim();
          if (name !== undefined && /^[A-Za-z_$][\w$]*$/u.test(name)) exportedByList.add(name);
        }
        continue;
      }
    }
    if (language === "rust") {
      const impl = /^impl\b(?:<[^>]*>)?\s*(?:[\w:<>, ]+\s+for\s+)?([A-Za-z_]\w*)/u.exec(trimmed);
      if (impl !== null) {
        owner = { name: impl[1]!, indent };
        continue;
      }
    }
    for (const head of PATTERNS[language]) {
      if (head.topLevel && indent > 0) continue;
      const match = head.pattern.exec(trimmed);
      if (match === null) continue;
      let name = match[head.name]!;
      let kind = head.kind;
      if (language === "python" && head.kind === "function" && indent > 0) {
        if (owner === null) break;
        name = `${owner.name}.${name}`;
        kind = "method";
      } else if (language === "go" && head.kind === "function" && match[1] !== undefined) {
        name = `${match[1]}.${name}`;
        kind = "method";
      } else if (language === "rust" && indent > 0) {
        if (owner === null || head.kind !== "function") break;
        name = `${owner.name}.${name}`;
        kind = "method";
      }
      found.push({ name, kind, exported: head.exported(match, indent), signature: headOf(lines, index, language, kind), line: index + 1 });
      if (language === "python" && head.kind === "class") owner = { name, indent };
      break;
    }
  }
  if (exportedByList.size === 0) return found;
  return found.map(declaration => (exportedByList.has(declaration.name) && !declaration.exported ? { ...declaration, exported: true } : declaration));
}

/** The last segment of a qualified name: what code that uses it writes (`put` for `Store.put`). */
export function shortName(name: string): string {
  return name.slice(name.lastIndexOf(".") + 1);
}

const IDENTIFIER_TOKEN = /[A-Za-z_$][\w$]*/gu;
/** Shorter identifiers are too common to anchor anything. */
const MIN_IDENTIFIER = 3;
const COMMON_WORDS = new Set([
  "the", "and", "for", "not", "but", "with", "this", "that", "from", "into", "none", "null", "true", "false", "self",
  "return", "const", "let", "var", "function", "class", "type", "interface", "export", "import", "async", "await",
  "pub", "use", "mod", "impl", "struct", "enum", "trait", "def", "elif", "else", "func", "package", "string", "number",
  "boolean", "void", "undefined", "new", "throw", "try", "catch", "finally", "while", "break", "continue", "match",
  "Some", "Ok", "Err", "Vec", "String", "Option", "Result", "Box", "len", "str", "int", "bool", "usize", "u32", "u64",
  "i32", "i64", "f64", "err", "nil", "ctx",
]);

/** The identifiers `text` names, at least three characters and not a keyword or a type every file uses. */
export function identifiers(text: string): Set<string> {
  const names = new Set<string>();
  for (const match of text.matchAll(IDENTIFIER_TOKEN)) {
    const name = match[0];
    if (name.length >= MIN_IDENTIFIER && !COMMON_WORDS.has(name)) names.add(name);
  }
  return names;
}

/** SQL table names a text creates, alters or drops (also inside string literals, as embedded schemas write them). */
export function sqlTables(text: string): string[] {
  const names = new Set<string>();
  const pattern = /\b(?:CREATE\s+(?:VIRTUAL\s+|TEMP(?:ORARY)?\s+)?TABLE|ALTER\s+TABLE|DROP\s+TABLE|CREATE\s+(?:UNIQUE\s+)?INDEX\s+(?:IF\s+NOT\s+EXISTS\s+)?\w+\s+ON)\s+(?:IF\s+(?:NOT\s+)?EXISTS\s+)?["`[]?([A-Za-z_][\w.]*)/giu;
  for (const match of text.matchAll(pattern)) names.add(match[1]!);
  return [...names];
}
