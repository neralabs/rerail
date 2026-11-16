/**
 * The INI files Bandit's `.bandit` and SQLFluff's `.sqlfluff`, `setup.cfg`, `tox.ini` and `pep8.ini` are, read as
 * Python's `configparser.ConfigParser` reads them (CPython 3.12, `Lib/configparser.py` `RawConfigParser._read`):
 * `[section]` headers, `key = value` options, full-line `#` and `;` comments, values continued on lines indented
 * deeper than their option, blank lines kept inside a value, a section or an option set twice refused (`strict`), and
 * the `[DEFAULT]` section's options given to every section, as `items(section)` gives them. What it cannot read as
 * configparser would, it refuses: it throws `LintConfigFormatError`.
 */
import { LintConfigFormatError } from "./lint-config-formats.js";

/** How one tool's `ConfigParser` is set up. */
export interface IniDialect {
  /** What separates an option from its value: `=` and `:` by default; SQLFluff passes `delimiters="="`. */
  readonly delimiters: readonly string[];
  /** Whether option names are lower-cased, configparser's default `optionxform`; SQLFluff keeps their case. */
  readonly lowerCaseOptions: boolean;
  /**
   * Whether values go through `BasicInterpolation` (`%(name)s`, `%%`), the default; SQLFluff passes
   * `interpolation=None`. Rerail does not expand it: a value with a `%` is refused.
   */
  readonly interpolation: boolean;
}

/** Each section's options, `[DEFAULT]`'s included, as `items(section)` gives them; `[DEFAULT]` itself is no section. */
export type IniSections = ReadonlyMap<string, ReadonlyMap<string, string>>;

const DEFAULT_SECTION = "DEFAULT";
const COMMENT_PREFIXES = ["#", ";"];
/** configparser's `SECTCRE`, matched at the start of the stripped line: the header runs to the last `]`. */
const SECTION_HEADER = /^\[(.+)\]/u;

/** `text` as configparser with `dialect` reads it; throws `LintConfigFormatError` (with the line) where it would fail. */
export function readIni(text: string, dialect: IniDialect): IniSections {
  return new IniReader(dialect).read(text);
}

/** The state of one read: configparser's `_read` loop, one line at a time. */
class IniReader {
  private readonly sections = new Map<string, Map<string, string[]>>();
  private readonly defaults = new Map<string, string[]>();
  /** `section NUL option` for every option read, so one set twice is refused (`strict`). */
  private readonly added = new Set<string>();
  private readonly option: RegExp;
  private section: { name: string; options: Map<string, string[]> } | undefined;
  private optionName: string | undefined;
  private indentLevel = 0;

  constructor(private readonly dialect: IniDialect) {
    const delimiters = dialect.delimiters.map(delimiter => delimiter.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&")).join("|");
    // configparser's OPTCRE: the option up to the first delimiter, spaces around it dropped.
    this.option = new RegExp(`^(.*?)\\s*(?:${delimiters})\\s*(.*)$`, "u");
  }

  read(text: string): IniSections {
    if (text.startsWith("\uFEFF")) throw new LintConfigFormatError("it starts with a byte order mark, which configparser reads as text");
    text.split(/\r\n|\r|\n/u).forEach((line, index) => this.readLine(line, index + 1));
    return this.joined();
  }

  private readLine(line: string, lineNumber: number): void {
    const value = line.trim();
    if (COMMENT_PREFIXES.some(prefix => value.startsWith(prefix))) return;
    const indent = line.length - line.trimStart().length;
    if (this.continuesValue(value, indent)) return;
    this.indentLevel = indent;
    const header = SECTION_HEADER.exec(value);
    if (header === null) this.readOption(value, lineNumber);
    else this.openSection(header[1]!, lineNumber);
  }

  /**
   * Whether the line is part of the value being read, adding it if so: a line indented deeper than its option, or a
   * blank line (kept; trailing ones are dropped when the value is joined). A blank line outside a value is skipped.
   */
  private continuesValue(value: string, indent: number): boolean {
    if (this.optionName === undefined) return value === "";
    if (value !== "" && indent <= this.indentLevel) return false;
    this.section!.options.get(this.optionName)!.push(value);
    return true;
  }

  private openSection(name: string, lineNumber: number): void {
    // configparser lets [DEFAULT] open again; any other section set twice is refused (`strict`).
    let options = this.defaults;
    if (name !== DEFAULT_SECTION) {
      if (this.sections.has(name)) throw new LintConfigFormatError(`line ${lineNumber}: section [${name}] is set twice`);
      options = new Map<string, string[]>();
      this.sections.set(name, options);
    }
    this.section = { name, options };
    this.optionName = undefined;
  }

  private readOption(value: string, lineNumber: number): void {
    if (this.section === undefined) throw new LintConfigFormatError(`line ${lineNumber}: an option comes before any [section]`);
    const match = this.option.exec(value);
    if (match === null || match[1] === "") {
      throw new LintConfigFormatError(`line ${lineNumber}: ${JSON.stringify(value)} is not a section header or a "key ${this.dialect.delimiters[0]} value" option`);
    }
    const name = this.dialect.lowerCaseOptions ? match[1]!.toLowerCase() : match[1]!;
    const key = `${this.section.name}\u0000${name}`;
    if (this.added.has(key)) throw new LintConfigFormatError(`line ${lineNumber}: option ${name} in [${this.section.name}] is set twice`);
    this.added.add(key);
    this.section.options.set(name, [match[2]!]);
    this.optionName = name;
  }

  /** Each section with `[DEFAULT]`'s options under its own, every value's lines joined and trailing space dropped. */
  private joined(): IniSections {
    const join = (options: Map<string, string[]>): Map<string, string> => new Map([...options].map(([name, lines]) => [name, this.value(name, lines)]));
    const defaults = join(this.defaults);
    return new Map([...this.sections].map(([name, options]) => [name, new Map([...defaults, ...join(options)])]));
  }

  private value(name: string, lines: readonly string[]): string {
    const value = lines.join("\n").trimEnd();
    if (this.dialect.interpolation && value.includes("%")) {
      throw new LintConfigFormatError(`the value of ${name} has a "%", which configparser interpolates and Rerail does not`);
    }
    return value;
  }
}
