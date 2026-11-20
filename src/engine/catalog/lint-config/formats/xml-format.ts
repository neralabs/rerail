/**
 * The XML that Psalm's `psalm.xml` and PHP_CodeSniffer's rulesets are, read as data: elements, attributes, character
 * data, comments, CDATA sections and processing instructions; the five predefined entities and character references.
 * A document type declaration, and so any other entity, is refused, as is anything not well-formed: it throws
 * `LintConfigFormatError` with the line. Namespace prefixes are resolved against the `xmlns` attributes in scope.
 */
import { LintConfigFormatError } from "./lint-config-formats.js";

/** One element, as written. */
export interface XmlElement {
  /** Its name without the prefix. */
  readonly localName: string;
  /** The namespace its prefix (or the default namespace) is bound to; undefined in none. */
  readonly namespace: string | undefined;
  readonly attributes: ReadonlyMap<string, string>;
  readonly children: readonly XmlElement[];
  /** Its own character data (CDATA included), not its children's, as one string. */
  readonly text: string;
  /** The line its start tag is on, from 1. */
  readonly line: number;
}

/** The root element of XML document `text`; throws `LintConfigFormatError` on what it does not read. */
export function parseXml(text: string): XmlElement {
  return new XmlReader(text).read();
}

/** The child elements of `element` named `localName`, in document order. */
export function xmlChildren(element: XmlElement, localName: string): XmlElement[] {
  return element.children.filter(child => child.localName === localName);
}

/** The predefined entities XML gives every document. */
const PREDEFINED_ENTITIES: Readonly<Record<string, string>> = { lt: "<", gt: ">", amp: "&", apos: "'", quot: "\"" };
/** A name, as near XML's `Name` production as Rerail needs: no markup, quote, space or reference characters. */
const NAME = /[^\s<>/=!?"'&;]+/uy;
const SPACE = /[ \t\r\n]*/uy;
const MAX_DEPTH = 64;

/** The in-scope prefix bindings at an element: `""` for the default namespace. */
type Scope = ReadonlyMap<string, string>;

class XmlReader {
  #index = 0;

  constructor(private readonly text: string) {
    if (text.startsWith("\uFEFF")) this.#index = 1;
  }

  read(): XmlElement {
    this.misc();
    if (this.text.startsWith("<!DOCTYPE", this.#index)) this.fail("a document type declaration is not read");
    if (this.text[this.#index] !== "<") this.fail("no root element");
    const root = this.element(new Map(), 0);
    this.misc();
    if (this.#index < this.text.length) this.fail("content after the root element");
    return root;
  }

  /** Skips whitespace, comments and processing instructions (the XML declaration included). */
  private misc(): void {
    for (;;) {
      this.space();
      if (this.text.startsWith("<!--", this.#index)) this.comment();
      else if (this.text.startsWith("<?", this.#index)) this.instruction();
      else return;
    }
  }

  private space(): void {
    SPACE.lastIndex = this.#index;
    SPACE.exec(this.text);
    this.#index = SPACE.lastIndex;
  }

  private comment(): void {
    const end = this.text.indexOf("-->", this.#index + 4);
    if (end < 0) this.fail("an unterminated comment");
    this.#index = end + 3;
  }

  private instruction(): void {
    const end = this.text.indexOf("?>", this.#index + 2);
    if (end < 0) this.fail("an unterminated processing instruction");
    this.#index = end + 2;
  }

  private name(): string {
    NAME.lastIndex = this.#index;
    const match = NAME.exec(this.text);
    if (match === null) this.fail("a name was expected");
    this.#index = NAME.lastIndex;
    return match[0];
  }

  /** The element whose `<` is at the current index, its end tag read. */
  private element(outer: Scope, depth: number): XmlElement {
    if (depth >= MAX_DEPTH) this.fail(`elements nested over ${MAX_DEPTH} deep`);
    const line = this.line();
    this.#index += 1;
    const name = this.name();
    const attributes = this.attributes();
    const scope = scopeOf(outer, attributes);
    const children: XmlElement[] = [];
    const text: string[] = [];
    if (this.text.startsWith("/>", this.#index)) this.#index += 2;
    else {
      this.#index += 1;
      this.content(name, scope, depth, children, text);
    }
    return { ...qualified(name, scope, this), attributes, children, text: text.join(""), line };
  }

  /** The attributes of a start tag, up to (not past) its `>` or `/>`. */
  private attributes(): Map<string, string> {
    const attributes = new Map<string, string>();
    for (;;) {
      this.space();
      if (this.text.startsWith("/>", this.#index) || this.text[this.#index] === ">") return attributes;
      const name = this.name();
      if (attributes.has(name)) this.fail(`attribute ${name} set twice`);
      this.space();
      if (this.text[this.#index] !== "=") this.fail(`attribute ${name} has no value`);
      this.#index += 1;
      this.space();
      attributes.set(name, this.attributeValue());
    }
  }

  private attributeValue(): string {
    const quote = this.text[this.#index];
    if (quote !== "\"" && quote !== "'") this.fail("an attribute value must be quoted");
    const end = this.text.indexOf(quote, this.#index + 1);
    if (end < 0) this.fail("an unterminated attribute value");
    const raw = this.text.slice(this.#index + 1, end);
    if (raw.includes("<")) this.fail("an attribute value holds <");
    this.#index = end + 1;
    // XML normalizes each whitespace character in an attribute value to a space.
    return this.decode(raw.replace(/[\t\r\n]/gu, " "));
  }

  /** An element's content up to and past its end tag `</name>`. */
  private content(name: string, scope: Scope, depth: number, children: XmlElement[], text: string[]): void {
    for (;;) {
      const next = this.text.indexOf("<", this.#index);
      if (next < 0) this.fail(`element ${name} is not closed`);
      text.push(this.decode(this.text.slice(this.#index, next)));
      this.#index = next;
      if (this.text.startsWith("</", next)) return this.endTag(name);
      this.markup(scope, depth, children, text);
    }
  }

  /** A comment, CDATA section, processing instruction or child element in content. */
  private markup(scope: Scope, depth: number, children: XmlElement[], text: string[]): void {
    if (this.text.startsWith("<!--", this.#index)) this.comment();
    else if (this.text.startsWith("<![CDATA[", this.#index)) text.push(this.cdata());
    else if (this.text.startsWith("<?", this.#index)) this.instruction();
    else if (this.text.startsWith("<!", this.#index)) this.fail("a declaration is not read");
    else children.push(this.element(scope, depth + 1));
  }

  private cdata(): string {
    const start = this.#index + "<![CDATA[".length;
    const end = this.text.indexOf("]]>", start);
    if (end < 0) this.fail("an unterminated CDATA section");
    this.#index = end + 3;
    return this.text.slice(start, end);
  }

  private endTag(name: string): void {
    this.#index += 2;
    const closing = this.name();
    if (closing !== name) this.fail(`</${closing}> closes <${name}>`);
    this.space();
    if (this.text[this.#index] !== ">") this.fail(`</${closing}> is not closed`);
    this.#index += 1;
  }

  /** Character data with its references replaced. */
  private decode(raw: string): string {
    return raw.replace(/&([^;&]*);?/gu, (whole, reference: string) => {
      if (!whole.endsWith(";")) this.fail("an & that is not a reference");
      return referenceText(reference) ?? this.fail(`the reference &${reference}; is not read`);
    });
  }

  line(): number {
    return this.text.slice(0, this.#index).split("\n").length;
  }

  fail(why: string): never {
    throw new LintConfigFormatError(`not XML Rerail reads: ${why} (line ${this.line()})`);
  }
}

/** The text of reference `&reference;`: a predefined entity or a character reference; undefined for another entity. */
function referenceText(reference: string): string | undefined {
  const predefined = PREDEFINED_ENTITIES[reference];
  if (predefined !== undefined) return predefined;
  const code = /^#x([0-9a-fA-F]+)$/u.exec(reference)?.[1] ?? /^#([0-9]+)$/u.exec(reference)?.[1];
  if (code === undefined) return undefined;
  const point = Number.parseInt(code, reference.startsWith("#x") ? 16 : 10);
  return point <= 0x10FFFF ? String.fromCodePoint(point) : undefined;
}

/** The bindings in scope at an element: the outer ones with its own `xmlns` and `xmlns:prefix` attributes over them. */
function scopeOf(outer: Scope, attributes: ReadonlyMap<string, string>): Scope {
  const declared = [...attributes].filter(([name]) => name === "xmlns" || name.startsWith("xmlns:"));
  if (declared.length === 0) return outer;
  return new Map([...outer, ...declared.map(([name, value]): [string, string] => [name === "xmlns" ? "" : name.slice(6), value])]);
}

/** Element name `name` split into its local name and the namespace its prefix is bound to. */
function qualified(name: string, scope: Scope, reader: XmlReader): { localName: string; namespace: string | undefined } {
  const colon = name.indexOf(":");
  if (colon < 0) return { localName: name, namespace: scope.get("") || undefined };
  const namespace = scope.get(name.slice(0, colon));
  if (namespace === undefined) reader.fail(`the prefix of ${name} is not bound`);
  return { localName: name.slice(colon + 1), namespace };
}
