/**
 * PHP_CodeSniffer ruleset files read as data (`xml-format.ts`), following the rulesets in the repository they
 * reference. `Ruleset::processRuleset` (src/Ruleset.php l.528-890) builds the sniffs from `<rule ref>`s, which
 * `expandRulesetReference` (l.955-1122) reads in its order, PHP_CodeSniffer run from the root: a ref starting with `.`
 * is relative to its ruleset's folder when that path exists, any other relative to the root; a file there is another
 * ruleset (whatever its name: `phpcs.xml.dist` too) or, named `…Sniff.php`, a sniff; else a bare name is a standard
 * (whose sniffs `phpcs-standards.generated.ts` lists), a folder its `ruleset.xml`, and a dotted name a category,
 * sniff or message code. A reference Rerail does not follow (outside the repository, an installed package, a sniff, a
 * standard PHP_CodeSniffer does not ship) is kept as unread, with why.
 */
import { posix } from "node:path";
import { LintConfigFormatError, readFormat } from "../formats/lint-config-formats.js";
import { NOT_A_FILE, readRootFile } from "../linter-config.js";
import { PHPCS_STANDARDS, type PhpcsStandard } from "../generated/phpcs-standards.generated.js";
import { isRepositoryFolder, repositoryPath } from "../repository-paths.js";
import { parseXml, xmlChildren, type XmlElement } from "../formats/xml-format.js";

/** How many rulesets deep `<rule ref>`s to other ruleset files are followed. */
const MAX_RULESET_DEPTH = 8;
/** A path in an installed package, which Rerail does not read. */
const INSTALLED = /(?:^|\/)vendor\//u;

/** A pattern that leaves out the files it matches, and where it is written, for the person. */
export interface Pattern {
  readonly pattern: string;
  readonly because: string;
}

/** A `<rule ref>` or `<exclude name>`, as PHP_CodeSniffer expands it (`Ruleset::expandRulesetReference`, l.955-1130). */
export type Reference =
  | { readonly kind: "standard"; readonly name: string; readonly standard: PhpcsStandard }
  /** A category, sniff or message code, of any standard: what it names follows from the name. */
  | { readonly kind: "name"; readonly name: string }
  | { readonly kind: "ruleset"; readonly name: string; readonly ruleset: PhpcsRuleset }
  | { readonly kind: "unread"; readonly name: string; readonly why: string };

export interface PhpcsRule {
  readonly ref: Reference;
  readonly excludes: readonly Reference[];
  readonly severity: number | undefined;
  readonly excludePatterns: readonly string[];
  readonly includePatterns: readonly string[];
  readonly properties: boolean;
}

export interface PhpcsRuleset {
  readonly file: string;
  readonly rules: readonly PhpcsRule[];
  /** Its `<arg>`s by name (a flag argument by its value), the first of a name winning. */
  readonly args: ReadonlyMap<string, string>;
  /** Its own `<exclude-pattern>`s, which leave files out for every sniff. */
  readonly ignores: readonly Pattern[];
}

/** Whether PHP_CodeSniffer (not phpcbf) processes `element` (`Ruleset::shouldProcessElement`, l.1386-1410). */
function forPhpcs(element: XmlElement): boolean {
  const limited = element.attributes.has("phpcs-only") || element.attributes.has("phpcbf-only");
  return !limited || element.attributes.get("phpcs-only") === "true";
}

const processed = (parent: XmlElement, name: string): XmlElement[] => xmlChildren(parent, name).filter(forPhpcs);

/** Reads ruleset files, following the ones in the repository that rulesets reference. */
export class RulesetReader {
  constructor(private readonly root: string) {}

  /** Ruleset file `file`; throws `LintConfigFormatError` when it is not XML Rerail reads, or not a ruleset. */
  ruleset(file: string, text: string, depth: number): PhpcsRuleset {
    const ruleset = parseXml(text);
    if (ruleset.localName !== "ruleset") throw new LintConfigFormatError("its root element is not <ruleset>");
    const rules = processed(ruleset, "rule").flatMap(rule => {
      const ref = rule.attributes.get("ref");
      return ref === undefined ? [] : [this.rule(file, rule, ref, depth)];
    });
    const args = new Map<string, string>();
    for (const arg of processed(ruleset, "arg")) {
      const name = arg.attributes.get("name") ?? `-${arg.attributes.get("value") ?? ""}`;
      if (!args.has(name)) args.set(name, arg.attributes.get("value") ?? "");
    }
    const ignores = processed(ruleset, "exclude-pattern").map(element => element.text.trim())
      .map(pattern => ({ pattern, because: `${file}: <exclude-pattern>${pattern}</exclude-pattern>` }));
    return { file, rules, args, ignores };
  }

  private rule(file: string, rule: XmlElement, ref: string, depth: number): PhpcsRule {
    const severity = processed(rule, "severity")[0]?.text.trim();
    return {
      ref: this.reference(file, ref, depth),
      excludes: processed(rule, "exclude").flatMap(exclude => {
        const name = exclude.attributes.get("name");
        return name === undefined ? [] : [this.reference(file, name, depth)];
      }),
      severity: severity === undefined ? undefined : Number.parseInt(severity, 10) || 0,
      excludePatterns: processed(rule, "exclude-pattern").map(pattern => pattern.text.trim()),
      includePatterns: processed(rule, "include-pattern").map(pattern => pattern.text.trim()),
      properties: processed(rule, "properties").length > 0,
    };
  }

  /**
   * What reference `name` in ruleset `file` is, as `expandRulesetReference` reads it with PHP_CodeSniffer run from the
   * root: a file there first, then a standard, a folder or a name.
   */
  private reference(file: string, name: string, depth: number): Reference {
    const path = this.referencedPath(file, name);
    if (path === undefined) return { kind: "unread", name, why: "it is outside the repository" };
    if (INSTALLED.test(path)) return { kind: "unread", name, why: "it is an installed package's ruleset" };
    const found = readRootFile(this.root, path);
    if (found.present && !isRepositoryFolder(this.root, path)) return this.fileReference(name, path, found.text, depth);
    return this.nameReference(name, path, depth);
  }

  /**
   * The path in the repository `name` names: one starting with `.` relative to the ruleset's folder when it exists
   * there (`realpath`), any other relative to the root, PHP_CodeSniffer's working directory; undefined outside it.
   */
  private referencedPath(file: string, name: string): string | undefined {
    if (name.startsWith("~/")) return undefined;
    const near = name.startsWith(".") ? repositoryPath(posix.dirname(file), name) : undefined;
    return near !== undefined && readRootFile(this.root, near).present ? near : repositoryPath("", name);
  }

  /** A file reference (its text undefined when it is not a regular file): a sniff (`…Sniff.php`), else another ruleset, whatever its name. */
  private fileReference(name: string, path: string, text: string | undefined, depth: number): Reference {
    if (path.endsWith("Sniff.php")) return { kind: "unread", name, why: "it is a sniff, which Rerail never runs" };
    return this.rulesetReference(name, path, text, depth);
  }

  /**
   * A reference that is no file: a standard PHP_CodeSniffer ships; a folder, its `ruleset.xml` or its sniffs (for a
   * bare name PHP_CodeSniffer looks up an installed standard first: Rerail takes a repository folder to be none); a
   * dotted name a category, sniff or message code; a bare name, a standard installed with PHP_CodeSniffer.
   */
  private nameReference(name: string, path: string, depth: number): Reference {
    const standard = Object.hasOwn(PHPCS_STANDARDS, name) ? PHPCS_STANDARDS[name] : undefined;
    if (standard !== undefined) return { kind: "standard", name, standard };
    if (isRepositoryFolder(this.root, path)) return this.folderReference(name, path, depth);
    if (!/[./]/u.test(name)) return { kind: "unread", name, why: "it is a coding standard PHP_CodeSniffer does not ship, which Rerail does not read" };
    return name.includes("/") ? { kind: "unread", name, why: "it does not exist" } : { kind: "name", name };
  }

  /** A folder: an external standard when it holds a `ruleset.xml`, else a folder of sniffs. */
  private folderReference(name: string, path: string, depth: number): Reference {
    const ruleset = posix.join(path, "ruleset.xml");
    const found = readRootFile(this.root, ruleset);
    return found.present ? this.rulesetReference(name, ruleset, found.text, depth) : { kind: "unread", name, why: "it is a folder of sniffs, which Rerail never runs" };
  }

  /** Ruleset file `file`, present in the repository with `text` (undefined when it is not a regular file), read in turn. */
  private rulesetReference(name: string, file: string, text: string | undefined, depth: number): Reference {
    if (depth >= MAX_RULESET_DEPTH) return { kind: "unread", name, why: `rulesets are followed ${MAX_RULESET_DEPTH} deep` };
    if (text === undefined) return { kind: "unread", name, why: NOT_A_FILE.replace(/\.$/u, "") };
    const parsed = readFormat(() => this.ruleset(file, text, depth + 1));
    return parsed.ok ? { kind: "ruleset", name, ruleset: parsed.value } : { kind: "unread", name, why: parsed.why };
  }
}
