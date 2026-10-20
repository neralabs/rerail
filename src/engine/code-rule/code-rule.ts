import type { AuditFailure, AuditMode, AuditScope } from "../audit/audit.js";
import type { RuleMode } from "../catalog/rule-mode.js";
import type { CodeRuleDialectLabel, CodeRuleLanguageId } from "./code-rule-languages.js";
import type { CodeRuleDslSemanticAstRuleFields, CodeRuleDslSemanticCompareText, CodeRuleDslSuppress } from "./code-rule-semantic.js";
import { steerBlock } from "./rerail-steer.js";
import { steerOrigin, type SteerRuleSource } from "./steer-origin.js";

export type CodeRuleDslScalar = boolean | number | string | null;
export type CodeRuleDslValue = CodeRuleDslScalar | CodeRuleDslValue[] | { [key: string]: CodeRuleDslValue };

export interface CodeRuleDslDocument {
  format: "rerail.policy/v1";
  id: string;
  revision: number;
  mode: RuleMode;
  description?: string;
  scope?: CodeRuleDslScope;
  clauses: CodeRuleDslClause[];
}

export interface CodeRuleDslScope {
  include: string[];
  exclude: string[];
}

/**
 * Where a violation's `span` points: the whole witness, a zero-width point at its start or end, or
 * a capture: an ast metavariable, a named group (`(?<NAME>…)`) of an ast `regex` that tests the
 * witness node itself, or a named group of a regex query. Fact clauses (`require_each`, `budget`)
 * accept only `witness.span`. The violation id always derives from the witness span.
 */
export type CodeRuleDslViolationAnchor = "witness.span" | "witness.start" | "witness.end" | "file.start" | `capture.${string}`;

export interface CodeRuleDslViolationSpec {
  code: string;
  anchor: CodeRuleDslViolationAnchor;
  /**
   * `$NAME` / `$$$NAME` interpolate captured text, as in ast-grep rule messages; a name the query cannot bind is an
   * error. `\$` is a literal `$`, and a name starting with a digit (`$100`) stays literal text. `${NAME}` and
   * `$$${NAME}` (a Rerail extension; ast-grep renders them literally) end the name at the brace, so a variable can sit
   * before name characters (`${A}_ID`); `\${NAME}` is a backslash before the variable.
   */
  message: string;
  /**
   * Suggested replacement for the witness span: an ast-grep fix template over the rule's
   * metavariables, or `$NAME` named-group substitution for a `file.content` regex. Never applied.
   * `${NAME}` and `$$${NAME}` name a variable next to name characters, as in messages.
   * An ast query's fix may instead replace a capture's range inside the witness: `{ template, range:
   * "capture.NAME" }` (a node capture, a `$$$` capture from its first node to its last, a regex group of
   * the witness, or a capture a parsed sub-tree exported). Where the capture is unbound or lies outside
   * the witness the violation carries no fix. `range: "witness.span"` is the string form.
   */
  fix?: string | { template: string; range: "witness.span" | `capture.${string}` };
  /**
   * An ast query's single-node capture (name without `$`): one violation per distinct binding of it at a witness (a
   * Rerail extension), where the rule reaches several (a relation's candidates, `any` branches). Each violation carries
   * its binding in `captures`, and its id includes the binding's range; without `each` a witness is one violation, with
   * the id it always had. A match that leaves the capture unbound is one violation, only where no match binds it. Capped
   * by `maxWitnesses`. A capture a parsed tree exports (`parse`, `formatArgs`' `missing`) and one `formatArgs`'
   * `unused` binds enumerate too, by their ranges in the host: one violation per placeholder. A named group of a `regex`
   * that tests the witness itself (no metavariable of that name) enumerates each match of the regex in the witness
   * text that binds it (non-overlapping, left to right), with that match's groups in the message, anchor and fix.
   */
  each?: string;
  /**
   * Another linter's suppression comments, honoured as that linter honours them (a Rerail extension, opt-in per violation;
   * the engine never reads them otherwise). A violation a comment suppresses is not reported. An ast query only.
   */
  suppress?: CodeRuleDslSuppress;
  /** Reserved: author-supplied violation variables are not supported yet, so only `{}` is accepted. */
  variables?: Record<string, never>;
}


export interface CodeRuleDslClauseBase {
  id: string;
  violation: CodeRuleDslViolationSpec;
  define?: never;
}

/**
 * A clause: one that reports violations (`forbid`, `require_each`, `budget`, `llm`), or a `define`, which reports
 * nothing and holds what the others share.
 */
export type CodeRuleDslClause = CodeRuleDslReportingClause | CodeRuleDslDefineClause;

/** A define clause (a Rerail extension): definitions the document's other clauses share. It has no violation. */
export interface CodeRuleDslDefineClause {
  id: string;
  define: CodeRuleDslDefine;
  violation?: never;
  forbid?: never;
  require_each?: never;
  budget?: never;
  llm?: never;
  relational?: never;
}

export interface CodeRuleDslDefine {
  /** Report all findings together only above this count. Failures remain visible. */
  report_when?: { count: { gt: number } };

  /** The language `utils` compile for; required with them. */
  language?: CodeRuleLanguageId;
  /**
   * Utilities every ast query of `language` in the document may name with `matches`, as if it listed them in its own
   * `utils` (an id may not be in both). Each compiles with the calling query's `languageOptions` and `transform`, and
   * may name only other shared utilities. Unused ones are checked too.
   */
  utils?: Record<string, CodeRuleDslUtility>;
  /**
   * Typed options with defaults, document-wide (a name is declared once). The document reads an option's value with a
   * reference, resolved before it is validated: `{ "$option": "name" }` is the value; with `cases` (and `default`) the
   * value selects one (a string by its text, anything else by its JSON text); `escape: "regex"`, `map` (a template per
   * item), `join` and `template` (`{}` is the value) make text; `spread: true` splices an array into the array holding
   * the reference. An ast query's `when.option` gates it on options. Names start with a letter or `_` and hold letters,
   * digits, `_`, `-` and `.`.
   */
  options?: Record<string, CodeRuleDslOptionDeclaration>;
  /** The configuration: values for options the document declares (a variant's patch); unset ones take their default. */
  config?: Record<string, CodeRuleDslValue>;
}

export interface CodeRuleDslOptionDeclaration {
  type: "boolean" | "string" | "number" | "integer" | "array";
  /** An array option's items (default `string`). */
  items?: "string" | "number" | "integer" | "boolean";
  /** The values allowed (an array option's items). */
  enum?: CodeRuleDslValue[];
  nullable?: boolean;
  default: CodeRuleDslValue;
  description?: string;
}

/** A condition on an option's value: the value itself, `{ in: [...] }`, `{ includes: item }` (an array option) or `{ not: ... }`. */
export type CodeRuleDslOptionCondition =
  | CodeRuleDslScalar
  | CodeRuleDslScalar[]
  | { in: CodeRuleDslValue[] }
  | { includes: CodeRuleDslScalar }
  | { not: CodeRuleDslOptionCondition };

/**
 * A utility: a rule that shares its caller's captures, or (a Rerail extension) a rule with `params`, capture names
 * (without `$`) a call binds to its own captures (`matches`), which runs in a frame of its own.
 */
export type CodeRuleDslUtility = CodeRuleDslAstRule | { params: string[]; rule: CodeRuleDslAstRule };

export type CodeRuleDslReportingClause = CodeRuleDslClauseBase & (
  | {
      forbid: CodeRuleDslQuery;
      require_each?: never;
      budget?: never;
      llm?: never;
      relational?: never;
    }
  | {
      forbid?: never;
      require_each: {
        subject: CodeRuleDslRelation;
        evidence: CodeRuleDslRelation;
      };
      budget?: never;
      llm?: never;
      relational?: never;
    }
  | {
      forbid?: never;
      require_each?: never;
      budget: {
        query: CodeRuleDslRelation;
        group_by?: string[];
        count: { lte: number; distinct?: string[] };
      };
      llm?: never;
      relational?: never;
    }
  | {
      forbid?: never;
      require_each?: never;
      budget?: never;
      llm: unknown;
      relational?: never;
    }
  | { forbid?: never; require_each?: never; budget?: never; llm?: never; relational: CodeRuleDslRelational }
);

/** Finite SQL joins and recursive walks over projected native evidence tables. */
export interface CodeRuleDslRelational {
  /** Authored file-start diagnostic locations, including files absent from the input inventory. */
  locations?: string[];
  /** Evaluate independent inputs and SQL stages per file, caching final local findings. */
  partitionBy?: "file" | "repository";
  inputs: Record<string, CodeRuleDslRelation>;
  references?: { language: "python"; siblingLinks?: NonNullable<CodeRuleDslAstRule["calls"]>["siblingLinks"]; constructorFields?: NonNullable<CodeRuleDslAstRule["calls"]>["constructorFields"] };
  stages?: ({ name: string } & (
    | { query: string; indexes?: string[][]; deferred?: Record<string, string>; reachability?: never }
    | { reachability: { roots: string; edges: string; batchSize?: number; graph?: "directed" | "forest" }; query?: never; indexes?: never; deferred?: never }
  ))[];
  query: string;
  key?: string[];
  code?: string;
}


export type CodeRuleDslQuery =
  | CodeRuleDslRegexQuery
  | CodeRuleDslAstQuery
  | CodeRuleDslSelectWhenQuery;

export interface CodeRuleDslRegexQuery {
  regex: {
    source: "file.content" | "file.path";
    /** Named groups (`(?<NAME>…)`) become the witness's captures. */
    pattern: string;
    /** Filters over each match's text: all include patterns hold and no exclude pattern holds. */
    include?: string[];
    exclude?: string[];
    /** Conditions over the complete current file, before line scanning or match filters. Content queries only. */
    content?: { include?: string[]; exclude?: string[] };
    /** Scan independent Unicode lines, optionally skipping lines or reporting only their first match. Content only. */
    lines?: {
      include?: string[]; exclude?: string[]; first?: boolean; terminators?: "python" | "lf";
      /** Process transitions before line filters, emitting only from the listed starting states. */
      state?: { initial: string; emit: string[]; transitions: Array<{
        from: string; to: string; pattern: string; skip?: boolean;
        count?: { min?: number; max?: number; parity?: "odd" | "even" };
      }> };
      /** Suppress deeper indentation after a marker, until reset; ignored lines do not update the marker. */
      indentation?: { excludeUnder: string; reset: string; ignore: string[] };
    };
    /** Restrict this clause's files within the document scope. */
    paths?: CodeRuleDslScope;
  };
}

export interface CodeRuleDslAstQuery {
  ast: {
    /** Gate on whole-file syntax. Invalid permits matches inside error regions. */
    syntax?: "valid" | "invalid";
    language: CodeRuleLanguageId;
    rule: CodeRuleDslAstRule;
    /**
     * Named rules that `rule`, `constraints`, and other utilities reference with `matches` (ast-grep's local
     * utilities). A utility shares the caller's metavariables and may recurse through a relation. A `define` clause
     * of the query's language shares more; an id may not be both.
     */
    utils?: Record<string, CodeRuleDslUtility>;
    /**
     * Metavariable name (without `$`) to a rule the node it captured must also match, checked after `rule` matches
     * (ast-grep's `constraints`). Applies to single-node captures; an unbound metavariable is not checked.
     */
    constraints?: Record<string, CodeRuleDslAstRule>;
    /**
     * Modelled on ESLint's `languageOptions`: which names are globals for `resolves` (`global`) and, for TypeScript,
     * what top-level `this` is (`this`). With them a `global` name must also be a declared global (TypeScript) or a
     * builtin of `pythonVersion` (Python). Omitted keys take ESLint's defaults (`"latest"`, no extra globals) and Ruff's
     * (`"3.10"`). Without a `sourceType`, each file has its own module system (see `sourceType`).
     */
    languageOptions?: CodeRuleDslLanguageOptions;
    /**
     * The configurations the query applies to: `pythonVersion` (Python, `"3.7"` to `"3.15"`) or `ecmaVersion`
     * (TypeScript, as in `languageOptions`) ranges, both bounds inclusive, and for TypeScript `sourceType` and
     * `ecmaFeatures`. It reads the query's own `languageOptions`, else the defaults (`"3.10"`; `"latest"`, the file's
     * own module system, no features). Every key must hold; otherwise the query matches nothing, which is sound, not
     * unknown (Ruff's B905 applies from 3.10; ESLint's strict picks its mode by sourceType and ecmaFeatures).
     */
    when?: CodeRuleDslAstWhen;
    /**
     * Python only: conditions on the file's module path (the host's `module`, else the file's stem, as `resolves.import`
     * reads it). Every key set must hold, or the query does not apply to the file. Stateless and workspace evaluation
     * alike; a new `module` for unchanged source is re-evaluated.
     */
    file?: CodeRuleDslFileCondition;
    /**
     * ast-grep 0.45.3's `transform`: variables (names without `$`) whose text is computed from a capture (`source`,
     * `$NAME` or `$$$NAME`) or from another transform variable, after the rule and its constraints matched. Messages
     * and fixes interpolate them as `$NAME`, and a `compare` reads them as text (`source` or `unquoted`) where the
     * captures they derive from are bound, a parsed tree's rule included; a comparison of one runs after the terms that
     * bind those captures. A capture a parsed tree exports or `formatArgs` binds is a source like any other. A source
     * left unbound gives the empty text and runs no operation, as upstream, unless the transformation sets `default`
     * (a Rerail extension): the text an unbound source reads as, which the operation then transforms (`default: ""` with
     * `replace: "^$"` supplies a fallback). `value: true` (a Rerail extension) reads a capture's value (as `value` does,
     * not followed) instead of its source text, converted to a string as the host language does: JavaScript's
     * `String(v)` of a primitive or a RegExp (`'\x61'` is `a`, `1e2` is `100`; an identifier otherwise reads as its
     * name, escapes decoded), Python's `str(v)` of a `str`, `int`, `float` (`1e2` is `100.0`), `bool`, `None` or
     * `...`. A value that is not known reads as an unbound source where there is a `default`; otherwise the variable is
     * the empty text and a comparison of it is unknown. The object form only; `rewrite` is not supported.
     */
    transform?: Record<string, CodeRuleDslTransformation>;
  };
}

/**
 * One transform variable: `replace` (every match of the regex replaced by `by`, which expands `$1` and `${name}`),
 * `substring` (characters from `startChar` to `endChar`, a negative index from the end), or `convert` (to a case,
 * splitting words at `separatedBy`, by default all of them).
 *
 * Rerail's, which read a capture's node rather than its text (a transform variable is no source for them): `line`, the
 * 1-based line of the capture's start (`edge: "end"`: its end), with the line terminators of the linter a port follows
 * (ESLint's for TypeScript, Ruff's for Python), as unified-signatures' "the one on line N"; `lines`, the number of lines
 * it spans; `count`, the number of nodes a `$$$` capture holds (1 for a single capture), as max-params' count. A `$$$`
 * capture spans its first node to its last; a capture a parsed tree exports, its host range. Unbound: the `default`,
 * else the empty text.
 */
export type CodeRuleDslTransformation =
  | { replace: { source: string; default?: string; value?: boolean; replace: string; by: string } }
  | { substring: { source: string; default?: string; value?: boolean; startChar?: number; endChar?: number } }
  | {
      convert: {
        source: string;
        default?: string;
        value?: boolean;
        toCase: "lowerCase" | "upperCase" | "capitalize" | "camelCase" | "snakeCase" | "kebabCase" | "pascalCase";
        separatedBy?: Array<"caseChange" | "dash" | "dot" | "slash" | "space" | "underscore">;
      };
    }
  | { line: { source: string; default?: string; edge?: "start" | "end" } }
  | { lines: { source: string; default?: string } }
  | { count: { source: string; default?: string } };

/** A text condition: `regex` (unanchored) must match, `notRegex` must not; at least one. */
export interface CodeRuleDslTextCondition {
  regex?: string;
  notRegex?: string;
}

export interface CodeRuleDslFileCondition {
  /** Every language: over the path the host sends (the snapshot or request file's `path`). */
  path?: CodeRuleDslTextCondition;
  /**
   * TypeScript only: whether the path names a declaration file, typescript-eslint 8.70.1's `isDefinitionFile` over the
   * lower-cased path (`.d.ts`, `.d.mts`, `.d.cts`, `.d.css.ts`).
   */
  declaration?: boolean;
  /**
   * Python only. Over the module path's names as on disk joined by `.` (`pkg.sub.mod`, `pkg.__init__`, `json.d.__init__`, or the stem,
   * dots and all). No module: no match, `notRegex` included.
   */
  module?: CodeRuleDslTextCondition;
  /** Python only. Over the module path's last name, the file's stem (`__init__` for a package's `__init__.py`, `my.mod`). */
  moduleName?: CodeRuleDslTextCondition;
  /**
   * Python: over the first name, the top package's directory as on disk (`json.d`; Ruff's N999 for `__init__.py`). Only
   * in a package. TypeScript: conditions on the file's nearest `package.json` (see `CodeRuleDslPackageCondition`).
   */
  package?: CodeRuleDslTextCondition | CodeRuleDslPackageCondition;
  /** Python only. Whether the file is in a package: the host sent its module path, rather than the stem fallback (Ruff's INP001). */
  inPackage?: boolean;
  /**
   * TypeScript only: the file's JSX runtime is one of these. A `@jsxImportSource` pragma comment naming React or a React
   * wrapper (`react`, `@emotion/react`, `theme-ui`, `@theme-ui/core`, or a subpath) gives `react`, one naming another JSX
   * library (`preact`, `solid-js`, `vue`, `@stencil/core`, `hono`, `@builder.io/qwik`, `inferno`, `mithril`,
   * `hyperapp`, or a subpath) `other`; a `@jsx` pragma naming `React` or `React.…` gives `react`; any other pragma is no
   * evidence. Then a top-level static import of
   * `h` or `createElement` from `@stencil/core`, `preact`, `vue`, `solid-js`, `solid-js/h`, `hyperapp` or `mithril`
   * (`other`); else the `compilerOptions.jsxImportSource` of the `tsconfig.json` (else `jsconfig.json`) beside the
   * nearest `package.json`, through relative `extends` to `tsconfig*.json` or `jsconfig*.json` files, where it names
   * React's or another library's as a pragma would;
   * else that manifest's dependencies: `react` for React's packages (`react`, `react-dom`,
   * `react-native`, `next`) without another JSX library's (`vue`, `@stencil/core`, `preact`, `solid-js`,
   * `@builder.io/qwik`, `inferno`, `mithril`), `other` for the reverse, `unknown` for both, neither or no manifest.
   */
  jsxRuntime?: Array<"react" | "other" | "unknown">;
}

/**
 * TypeScript's `file.package`: conditions on the file's nearest `package.json` at or above it, never above the workspace
 * root (a regular file of at most 1 MiB; one that cannot be read has no fields). Every key set must hold.
 */
export interface CodeRuleDslPackageCondition {
  /**
   * Whether the file is one of the manifest's `bin` targets (the string, or each string value of the object), as
   * eslint-plugin-n's `isBinFile` resolves them against the manifest's folder: the target is the file's path, or that
   * path without `.js`, or without `.js` and a last `/index`. Without a manifest neither `true` nor `false` holds.
   */
  bin?: boolean;
  /**
   * The file's configured Node range is a subset of `subsetOf` (node-semver's `subset`). The configured range is
   * eslint-plugin-n's: `engines.node`, else the `devEngines.runtime` entry named `node`'s `version`, else `>=16.0.0`
   * (also without a manifest); one that is no valid range falls through to the next.
   */
  nodeVersion?: { subsetOf: string };
  /** The manifest lists one of these in `dependencies`, `devDependencies`, `peerDependencies` or `optionalDependencies`. Needs a manifest. */
  dependsOn?: string[];
  /** Over the file's path relative to the manifest's folder. Needs a manifest, `notRegex` too. */
  path?: CodeRuleDslTextCondition;
  /**
   * The manifest names `next` and the file is in its Next App Router server module graph: a server file convention
   * (`page`, `layout`, `template`, `default`, `loading`, `not-found`, `forbidden`, `unauthorized`, `global-not-found`,
   * `route`, or a metadata image or route file) under the package's `app/` (else `src/app/`, which Next ignores beside a
   * root `app/`), outside private `_` folders, or a module such a file reaches by static imports, `export … from`, and
   * `import()` or `require()` of a string, passing no module whose directive prologue holds `"use client"` (that module
   * and what only it imports are client code). Specifiers resolve as Next resolves them in the package: relative paths
   * with Next's extensions and `index` files, then the package's `tsconfig.json` (else `jsconfig.json`) `paths` and
   * `baseUrl` through relative `extends` to `tsconfig*.json` or `jsconfig*.json` files. A module of another package (a
   * bare specifier, a file below a nested `package.json`) or in `node_modules` or a hidden folder is no edge; a package
   * of more than 20,000 source files has no graph, on every path, nor has one read on disk whose source or folder cannot
   * be read. Entries take Next's default page extensions (`.tsx`, `.ts`, `.jsx`, `.js`); `paths` maps through its first
   * 64 patterns (by pattern text) and the first 16 targets of each. `false` holds for a file of a package naming `next`
   * outside the graph; without such a manifest neither holds.
   */
  serverGraph?: boolean;
}

/** An inclusive version range; at least one bound. */
export interface CodeRuleDslVersionRange<T> {
  min?: T;
  max?: T;
}

export interface CodeRuleDslAstWhen {
  pythonVersion?: CodeRuleDslVersionRange<string>;
  ecmaVersion?: CodeRuleDslVersionRange<number | "latest">;
  /** TypeScript only: the query's `sourceType` (default the file's own, see `languageOptions.sourceType`) is this one or one of these. */
  sourceType?: "script" | "module" | "commonjs" | Array<"script" | "module" | "commonjs">;
  /** TypeScript only: each flag set equals the query's `parserOptions.ecmaFeatures` flag (default false). At least one. */
  ecmaFeatures?: { globalReturn?: boolean; impliedStrict?: boolean };
  /** The document's options (a define's `options`): each named option's value meets its condition; `{}` always holds. */
  option?: Record<string, CodeRuleDslOptionCondition>;
}

/** ESLint's global settings, with its aliases: `true`/`"true"`/`"writeable"` are writable; `false`/`"false"`/`"readable"`/`null` read-only. */
export type CodeRuleDslGlobalSetting = "readonly" | "writable" | "off" | "readable" | "writeable" | "true" | "false" | boolean | null;

export interface CodeRuleDslLanguageOptions {
  /**
   * Selects the ES builtin globals (ESLint 10.11.0's table): `"latest"` (2026), 3, 5, 6 to 17, or 2015 to 2026.
   */
  ecmaVersion?: number | "latest";
  /**
   * `script`: top-level declarations join the global scope and top-level `this` is the global object. `module`: a module
   * scope; `this` is `undefined`. `commonjs`: a function-like module scope that adds `require`, `module`, `exports` and
   * `global`. Omitted, each file has its own: `.cjs` and `.cts` files (`.d.cts` included) are `commonjs`; `.mjs`,
   * `.mts`, `.tsx` and other declaration files `module`. A `.js`, `.jsx` or `.ts` file follows the `type` of its nearest
   * `package.json` (`module`
   * or `commonjs`), as Node runs `.ts` files like `.js` ones; where that sets no valid `type`, Node's syntax detection
   * decides: a top-level `import` or `export` declaration that type stripping keeps (not `import type`, `export type`,
   * an exported interface, type alias or `declare`, nor `import x = require(…)` or `export =`), `import.meta`, a
   * top-level `await` (outside functions), or a top-level `const`, `let` or `class` binding `require`, `module`,
   * `exports`, `__dirname` or `__filename` makes a module, anything else `commonjs`. Without a `package.json` it is a
   * module (ESLint's default).
   */
  sourceType?: "script" | "module" | "commonjs";
  /** Identifiers to declare global (`readonly`, `writable`) or to remove (`off`). At most 16384. */
  globals?: Record<string, CodeRuleDslGlobalSetting>;
  /**
   * ESLint's `parserOptions`, of which only `ecmaFeatures.globalReturn` and `ecmaFeatures.impliedStrict` are read (both
   * default false). `globalReturn` makes the top level a function scope: it is no global scope (`this: "globalScope"`,
   * `scriptDeclarations`), and its `this` is the global object only where the top level is not strict. `impliedStrict`
   * makes all code strict from ES5.
   */
  parserOptions?: { ecmaFeatures?: { globalReturn?: boolean; impliedStrict?: boolean } };
  /**
   * typescript-eslint's `jsxPragma` (default `"React"`; `null` for none): the first JSX element or fragment whose scope
   * sees a variable of this name reads it, for `used` and `referenced`. Base ESLint has no pragma: JavaScript ports set
   * `null`.
   */
  jsxPragma?: string | null;
  /** typescript-eslint's `jsxFragmentName` (default none): the same for the first fragment (`<>`). */
  jsxFragmentName?: string | null;
  /**
   * typescript-eslint's `parserOptions.lib` (default none): TypeScript libs (`"es2022"`, `"dom"`, `"lib"`; lower-cased,
   * each one typescript-eslint 8.70.1's scope manager knows) whose global variables, and those of the libs they include,
   * are declared in their namespaces: a type reference reads a lib type, a value reference a lib value. `resolves.global`,
   * `resolves.unresolved` and `binds.builtin` read them; neither `off` removes one.
   */
  lib?: string[];
  /**
   * Python only: the target version, `"3.7"` to `"3.15"` (Ruff 0.16.9's). It selects Ruff's builtins and magic
   * globals of that version, and from `"3.14"` reads every annotation after the module (PEP 649), as
   * `from __future__ import annotations` does. The keys before it are TypeScript's.
   */
  pythonVersion?: string;
  /**
   * Python only: Ruff's `lint.typing-modules` (default none), dotted module names whose members are read as `typing`'s,
   * as Ruff 0.16.9 reads them: `compat.Literal[...]` is a `Literal` (its strings are no type definitions),
   * `compat.Optional["T"]` a generic whose slice is one, `compat.cast`, `compat.TypeVar` and the other typing callables
   * read their type arguments, and `X: compat.TypeAlias = "T"` is an alias. A relative import resolves against the
   * file's module path (`from .typical import Literal` in `foo.bar` is `foo.typical.Literal`). It changes which strings
   * `parse` reads as type definitions, what `resolves`, `binds` and the usage filters see in them, and what a stub reads
   * after the module. A listed `typing.<member>` in `resolves.import` or `resolves.imported` also matches that member
   * of a typing module (`std.overload` for `typing.overload`), as Ruff's `match_typing_expr`. At most 256 names.
   */
  typingModules?: string[];
  /**
   * Python only: Ruff's `lint.pyflakes.extend-generics` (default none), qualified names of further generics whose
   * subscript's slice is a type definition (`django.db.models.ForeignKey`, so `ForeignKey["Path"]` reads `Path`). At
   * most 256 names.
   */
  extendGenerics?: string[];
}

/**
 * Compares two single-node captures' texts (a Rerail extension; ast-grep has none). It runs last in its rule object and
 * reads the bindings made so far; both must be bound or it does not match. Inside a relation's rule the walk goes on to
 * the first node whose captures compare.
 */
export interface CodeRuleDslCompare {
  /** Metavariable name without `$`. */
  left: string;
  op: "lt" | "le" | "eq" | "ne" | "gt" | "ge";
  right: string;
  /**
   * `ordinal` (default): UTF-16 code units, as JavaScript's `<`. `fold-lower` / `fold-upper`: after `toLowerCase()` /
   * `toUpperCase()`. `lowercase-first`: after swapping the case of ASCII letters. `natural`: the natural-compare
   * package's order, digit runs by number (ESLint sort-keys `natural`); `natural-fold-lower`: after `toLowerCase()`.
   */
  collation?: "ordinal" | "fold-lower" | "fold-upper" | "lowercase-first" | "natural" | "natural-fold-lower";
  /**
   * `source` (default): the capture's text. `unquoted`: without one pair of enclosing quotes; escapes stay. `value`:
   * what the captured expressions evaluate to, as `value` reads them (Python `==` and order; JavaScript `Object.is` for
   * `eq` and `ne`, and its relational operators for the orders, converting primitives as `<` does: `null <= 0`,
   * `'1' < 2`, NaN never; the collation applies where both are strings); unknown where either is not known, or where an
   * order would convert an object, an array or a regex. `key` (TypeScript): the static property name of a key or of the
   * property, method, field, signature or member access that holds one (ESLint's `getStaticPropertyName`); a computed
   * key's expression read by itself (`` `a` `` in `` [`a`] ``) is named as its property. A key without one (`a[b]`, where
   * ESLint's is null) equals no key, another nameless one included, so `eq` is false and `ne` true; an order against it
   * is unknown. `member` (TypeScript): the name typescript-eslint's `getStaticMemberAccessValue` reads from the same nodes:
   * a non-computed identifier's or private name's (`#a` is `a`), otherwise `String(v)` of what the key folds to with
   * `value`'s folding, bindings followed (`['f' + '']`, and `[k]` after `const k = 'f'`, are `f`); unknown where the key's
   * value is not known, never nameless. `name` (TypeScript): what ESTree names the captured node itself, an identifier's
   * name with its escapes decoded (a private name's without `#`) or a string literal's value, so the string key `'ab'`
   * equals the function name `a\u0062` (func-name-matching, no-useless-rename); any other node is nameless, as for `key`.
   * `reference` (TypeScript): ESLint's `isSameReference`, for `eq` and `ne` under the ordinal collation: parentheses and
   * optional chains aside, the same `this` or `super`, the same identifier or private name, literals equal as
   * `equalLiteralValue` has them (a regex by its text, a BigInt by value, anything else by `===`: `0x10` and `16`), or
   * member accesses of the same reference, where one with a static name (`getStaticPropertyName`) is the same as one of
   * that name (`a.b` and `a['b']`); anything else (calls, templates, `a!`) never. `reference-no-static-keys`: its
   * `disableStaticComputedKey` (operator-assignment's), where a computed access and a static one differ.
   * PHP (`key`, `member`, `name`, `reference`, `reference-no-static-keys`): `key` is the name of the property, method,
   * class constant, array key or named argument a node holds or is (`$o->a`, `$o?->a()`, `C::$a`, `C::A`, `['a' => 1]`,
   * `$a['a']`, `f(a: 1)`, a member declaration), a computed name being what PHP converts its constant value to
   * (`$o->{'a' . 'b'}` is `ab`, `[1.5 => x]` is `1`, `$a['1']` is `1`); a dynamic name (`$o->$x`) is nameless. `member`
   * is the same with a computed name's variables followed through their binding; unknown where its value is not known.
   * `name` is what PHP looks a symbol up by: functions, classes, methods and namespaces folded to ASCII lower case,
   * variables (without `$`), properties and constants as written, a string literal's value. `reference`: parentheses
   * aside and `?->` as `->`, the same variable, constant, literal (`===`) or access chain of the same object, class
   * (`self` is not `static`) or array by the same key, keys converted as PHP converts them (`$a[1]` is `$a['1']`, `$o->a`
   * is `$o->{'a'}`); `reference-no-static-keys` compares a written name only with a written one and literal keys by
   * `===`.
   * `tokens`: the leaf tokens, comments and white space aside (ESLint's `equalTokens`), for `eq` and `ne`
   * under the ordinal collation only. `structure` (Python, PHP): the syntax trees, parentheses, comments and trivia
   * aside, literals by value and type (`1 == 0x1`, `'a' == "a"`, `1 != 1.0 != True`), as Ruff's `ComparableExpr`
   * (Python) or as nikic/PHP-Parser's trees with their attributes stripped (PHP: `array()` is `[]`, `<>` is `!=`,
   * `(integer)` is `(int)`, `exit` is `die`, a single statement is a block of it); `structure-hash`: numbers folded by
   * value (`1 == 1.0 == True`) and constant tuples element by element, as Ruff's `HashableExpr` (Python), or a constant
   * expression by the array key PHP coerces it to (`1 == '1' == 1.5 == true`, PHP); `structure-parts` (Python):
   * implicitly concatenated strings part by part (`"a" "b" != "ab" ""`), as Ruff's `ComparableLiteral`; PHP has no
   * implicit concatenation and refuses it. The structural modes are for `eq` and `ne` under the ordinal collation, and
   * unknown where a capture does not decode. `node`: whether the two captures are the same node (range and kind), not
   * merely the same text: two identical loops in one scope differ; `eq` and `ne` under the ordinal collation. `index`:
   * each capture's
   * position among the elements of the list that holds it (ESTree's element index: the commas before it, so array holes
   * count and comments do not; parentheses are transparent), compared as numbers under the ordinal collation. A node
   * outside a list (array, array pattern, arguments, parameters, object, tuple type, Python displays, argument lists,
   * parameters and patterns) has none, and compares false. Pairs two lists by position: no-self-assign's `[a, b] = [a, c]`.
   */
  text?: "source" | "unquoted" | "tokens" | "node" | "index" | CodeRuleDslSemanticCompareText;
  /**
   * `tokens` only: binary operators (their operator text, `["||", "&&"]`) whose operands compare in either order,
   * recursively and parentheses aside, as ESLint no-dupe-else-if's `equal` does: `a || b` equals `b || a`, and
   * `(a && b) || c` equals `c || (b && a)`. Anything else compares by its tokens.
   */
  commute?: string[];
}


/**
 * A cross-file relation projected from an AST query's matches in every file of the code rule's
 * scope (narrowed by `paths`). Each match is one row of the `project` fields: `capture.NAME`
 * (the capture's source text), `capture.NAME.text` (decoded exported text or host text),
 * `capture.NAME.startByte`, `capture.NAME.endByte`, `capture.NAME.startLine`,
 * `capture.NAME.kind` (host AST node kind), `file.path`, `file.directory`, `file.name`, `file.stem` (the name
 * without extensions), `file.module` or `file.package` (Python). A file whose query failed or
 * was read around syntax errors has unknown rows: no absence or upper bound ignores it.
 */
export interface CodeRuleDslEvidenceQuery {
  evidence: {
    as: string;
    paths?: { include?: string[]; exclude?: string[] };
    query: CodeRuleDslAstQuery;
    project: Record<string, string | { source: string; regex: string; group: number }>;
    where?: CodeRuleDslPredicate;
    /** Ordered, cycle-safe name closures over raw projected fields. */
    /** Enumerate bindings before projecting rows. */
    each?: string;
    /** Report a selected capture's location; defaults to witness.span. */
    anchor?: string;
    closure?: { name: string; seeds?: string[]; seed_field?: string; edges?: { from: string; to: string };
      extends?: string[]; partition_by?: string[]; contains: Record<string, string> }[];
  };
}

/** A `require_each` or `budget` relation projected from AST evidence. */
export type CodeRuleDslRelation = CodeRuleDslEvidenceQuery;

export interface CodeRuleDslSelectWhenQuery {
  select_when: {
    select: CodeRuleDslQuery;
    exists: CodeRuleDslQuery;
  };
}

export type CodeRuleDslPredicate =
  | { all: CodeRuleDslPredicate[] }
  | { any: CodeRuleDslPredicate[] }
  | { not: CodeRuleDslPredicate }
  | { eq: [CodeRuleDslValue, CodeRuleDslValue] }
  | { lt: [CodeRuleDslValue, CodeRuleDslValue] }
  | { le: [CodeRuleDslValue, CodeRuleDslValue] }
  | { gt: [CodeRuleDslValue, CodeRuleDslValue] }
  | { ge: [CodeRuleDslValue, CodeRuleDslValue] }
  | { ne: [CodeRuleDslValue, CodeRuleDslValue] };

export type CodeRuleDslAstPattern = string | { context: string; selector: string };
/**
 * A relation's sub-rule plus how far it walks: `neighbor` (default), `end`, or a rule object. With a rule, the walk
 * ends at the first node that rule matches; that node is still tested (ast-grep's `stopBy`). A stop rule matches in its
 * own environment, so it neither reads nor binds the caller's metavariables.
 *
 * `field` (`inside` and `has` only, not in a `count`; ast-grep 0.45.3's): a field name of the grammar. `inside` tests
 * only the ancestors the walk enters through that field (`inside: { kind: "function_definition", field: "body",
 * stopBy: "end" }`); `has` starts at the node's child in that field: `neighbor` tests it alone, `end` it and its
 * descendants, and a stop rule it and then, unless the stop rule matches it, its children only (upstream's one level).
 * A parsed tree's rule (`parse`) refuses it.
 */
export type CodeRuleDslAstRelation = CodeRuleDslAstRule & { stopBy?: "neighbor" | "end" | CodeRuleDslAstRule; field?: string };

/**
 * `nthChild` (ast-grep 0.45.3's): the node's 1-based position among its parent's named children (comments included), as a
 * number or CSS's `An+B` (`"2n+1"`, `"-n+3"`). The object form counts only the siblings `ofRule` matches (each probed
 * with the caller's bindings; the node's own bindings under `ofRule` are kept) and, with `reverse`, from the last.
 * With `ofRule` it is ordered among a rule object's binding terms, so `ofRule` may compare what a sibling relation binds.
 * A parsed tree's rule (`parse`) refuses it.
 */
export type CodeRuleDslNthChild = number | string | { position: number | string; ofRule?: CodeRuleDslAstRule; reverse?: boolean };

export interface CodeRuleDslAstRule extends CodeRuleDslSemanticAstRuleFields {
  pattern?: CodeRuleDslAstPattern;
  kind?: string;
  regex?: string;
  nthChild?: CodeRuleDslNthChild;
  /** File path condition that can also scope a call-graph seed. */
  filePath?: CodeRuleDslTextCondition;
  all?: CodeRuleDslAstRule[];
  any?: CodeRuleDslAstRule[];
  not?: CodeRuleDslAstRule;
  /**
   * A utility rule of the query's `utils` or a define of its language: its id, or for a utility with `params` a call
   * mapping each parameter to a capture, `{ "id": { "P": "$C" } }` (a Rerail extension).
   */
  matches?: string | Record<string, Record<string, string>>;
  inside?: CodeRuleDslAstRelation;
  has?: CodeRuleDslAstRelation;
  precedes?: CodeRuleDslAstRelation;
  follows?: CodeRuleDslAstRelation;
  compare?: CodeRuleDslCompare;
  /** How many nodes one relation reaches that match its rule (a Rerail extension). */
  count?: CodeRuleDslAstCount;
}

/** A `calls` term: its inner rule's keys, plus how chains pass and what they bind. */
export type CodeRuleDslCalls = CodeRuleDslAstRule & { through?: "sync" | "any"; chain?: string;
  graph?: "calls" | "references"; at?: "call" | "definition";
  siblingLinks?: { decorator: string; name: { replace: string; by: string } }[];
  constructorFields?: { fieldToParameter: { replace: string; by: string } } };

/** A count of lines or bytes: exactly `eq`, or from `min` to `max` inclusive (either may be left out). */
export type CodeRuleDslLineCount = { eq: number } | { min?: number; max?: number };

/**
 * The lines on one side of the node, up to its neighbour there (a Rerail extension). Lines end at the terminators of the
 * linter a port follows: ESLint's (`\r\n`, `\r`, `\n`, U+2028, U+2029) for TypeScript, Ruff's (`\r\n`, `\r`, `\n`)
 * for Python.
 *
 * - `to`: `"token"`, the nearest token (a leaf of the tree, punctuation included), as ESLint's `getTokenBefore` and
 *   `getTokenAfter`; `"sibling"`, the nearest named sibling; `"capture.NAME"`, a single-node capture the rule bound
 *   before (it must lie wholly on that side of the node). A zero-width token (a missing one) is passed over.
 * - `comments` (token and sibling): a comment counts as the neighbour; by default it is passed over, as
 *   `getTokenBefore` does without `includeComments`.
 * - `edge` (token and sibling): with no neighbour on that side, the file's start (`before`) or end (`after`) stands in;
 *   by default the gap then does not hold.
 * - `breaks`: the line breaks between the neighbour's end and the node's start (`before`), or the node's end and the
 *   neighbour's start (`after`): 0 when they share a line.
 * - `blank`: the lines between them that hold no token and no comment (a line inside a multi-line comment or string is
 *   not blank), as ESLint's sort-keys counts them.
 */
export interface CodeRuleDslLayoutGap {
  to: "token" | "sibling" | `capture.${string}`;
  comments?: boolean;
  edge?: boolean;
  breaks?: CodeRuleDslLineCount;
  blank?: CodeRuleDslLineCount;
}

/**
 * `span`: the line breaks inside the node (0 on one line). `start`: its first byte's offset in the file (0 at the file's
 * start: Ruff's file-level diagnostics). `before` and `after`: see `CodeRuleDslLayoutGap`. Every key listed must hold. It
 * binds nothing, and reads a capture only through `to`. A gap reads outside the node, as `precedes` does: negated, it
 * is not evaluated in a file with syntax errors. A parsed tree's rule (`parse`) refuses it.
 */
export interface CodeRuleDslLayout {
  /** Regex on the complete first line of the node, or the preceding line. */
  line?: string;
  previousLine?: string;
  /** Regex over a bounded window around the first line, optionally ending at the first matching line, inclusive. */
  context?: { before?: number; after?: number; through?: string; regex: string };
  span?: CodeRuleDslLineCount;
  start?: CodeRuleDslLineCount;
  before?: CodeRuleDslLayoutGap;
  after?: CodeRuleDslLayoutGap;
}

/**
 * The arguments of a format call and the fields of its template, as Ruff 0.16.9's Pyflakes rules read them (F504,
 * F505, F507, F522, F523, F524). It reads two captures the rule bound before it: `string`, the template's string literal
 * (parsed by `braceFormat` or `percentFormat`; read through parentheses, as Ruff's AST has none), and `args`, a
 * `.format` call's argument list (a `$$$` capture) or the right operand of `%` (or a logging call's argument list).
 * Every key listed must find a node.
 *
 * - `unused` is matched on the host arguments no field consumes, and may bind: `.format`'s positional arguments by
 *   index (a starred one never), its keywords by name; `%`'s dict display pairs whose string key no placeholder names
 *   (with no positional placeholder), a tuple display's elements past the placeholders, an operand of a known type
 *   other than a tuple as one value, and, with no placeholder at all, any operand Ruff does not type as a tuple.
 * - `missing` is matched on the template's fields (`.format`) or placeholders and stars (`%`) no argument fills; its
 *   captures are exported as `parse`'s are.
 *
 * Unknown (no violation, negated or not): a template the grammar refuses, and a string that is no plain `str` literal
 * (an f-string or bytes, which Ruff's rules skip); for `.format`, a keyword past `**kw` and any missing field past
 * `*args` or `**kw`; for `%`, a `**` splat or a key that is no string literal, a starred element, a format that mixes
 * keys and positions, an operand Ruff types as a tuple (`(1,) + (2,)`), and an operand of unknown type with
 * placeholders.
 *
 * Style `printf` (php) reads a printf-family call as PHP 8.5.11 and PHPStan 2.2.16's `argument.sprintf` read it:
 * `string` captures the format (an argument a pattern's `$F` captures, or its expression, through parentheses; parsed
 * by `printfFormat`): a quoted string, heredoc or nowdoc, or any expression whose value PHPStan reads as a constant
 * string, decoded as `value` with `follow` decodes it (a concatenation, a cast, a variable's one binding, a `const`, a
 * top-level `define` or a class constant this file declares), its tree's nodes then mapped to the whole expression.
 * `args` the values: a `$$$` capture of the arguments after the format (`sprintf`, `printf`, `fprintf`) or one capture
 * of the array literal a `v*printf` call passes. `missing` is matched on the placeholders and stars that number a
 * value past the last passed (PHP's `ArgumentCountError`); `unused` on the values past the highest one any field
 * numbers (an `argument`, or an `array_element_initializer`), which PHP ignores; a value a numbered placeholder skips
 * (`%2$s` with two values) is not unused. Unknown: a format PHP refuses (a `ValueError`), a format whose value is not
 * known (an interpolated string, a variable several bindings reach, a constant another file may declare), an `...`
 * unpacking or named argument, and values that are no array literal, unpack one, or have a key not known (a
 * `v*printf` array's entries count after PHP's key coercion and replacement). `$$$A` matches no node after a
 * comma: a call passing the format alone needs a pattern of its own (`sprintf($F)`). Style `scanf` (php) reads
 * `sscanf`/`fscanf` as PHP 8.5.11 validates them, the format read as `printf` reads it: `args` is the variables after
 * the format; `missing` is matched on the placeholders numbering a variable past the last passed, `unused` on the
 * variables no placeholder assigns (a gap `%2$s` leaves included), both a `ValueError` in PHP; with no variable the
 * call returns an array and nothing is missing or unused.
 *
 * Style `rust` (rust) reads a `format_args!`-family macro call (`format!`, `print!`, `println!`, `eprint!`,
 * `eprintln!`, `write!`, `writeln!`, `panic!`, `assert!`, `assert_eq!`, `unreachable!`, `todo!`, ... by their bare or
 * `std::`/`core::`/`alloc::` paths) as rustc 1.99.0 binds it: `string` captures the template, a string literal token
 * of the macro's token tree (parsed by `rustFormat`); `args` captures the arguments after it (`$$$A`), split at the
 * token tree's top-level commas. `unused` is matched on the first token of each argument no field, width or precision
 * reads (rustc's "argument never used"); `missing` on the fields that read an index past the last argument (rustc's
 * "invalid reference to positional argument"). A field naming no argument captures a variable, which `resolves`
 * reads. A positional argument after a named one, and a template rustc refuses, are unknown.
 */
export interface CodeRuleDslFormatArgs {
  style: "brace" | "percent" | "printf" | "scanf" | "rust";
  /** The capture holding the template: a string literal, or for PHP any format expression (A-Z, 0-9, _, without `$`). */
  string: string;
  /** The capture holding the arguments. */
  args: string;
  unused?: CodeRuleDslAstRule;
  missing?: CodeRuleDslAstRule;
}

/** The sub-grammars `parse` applies (the engine's `literal::Grammar`). */
export type CodeRuleDslParseGrammar = "percentFormat" | "braceFormat" | "annotation" | "eslintDirective" | "printfFormat" | "scanfFormat" | "phpDoc" | "rustFormat" | "identifierWords" | "pythonFString";

/**
 * A string literal parsed by a sub-grammar, and a rule of that grammar's kinds matched on the parsed tree's root: the
 * literal's decoded value (escapes decoded, implicit concatenations joined) as its own document, with its own
 * environment. It holds on a Python `string` or `concatenated_string` the grammar parses; on any other node it does
 * not. A literal whose value is not known (an f-string with a field, bytes, a value past 64 KiB) is unknown: no violation,
 * negated or not. Each parsed node maps to the source bytes it came from (an escape to the whole escape).
 *
 * - `percentFormat`: printf-style formats as Ruff's `cformat` parses them: `format` holds `literal` and `placeholder`;
 *   a placeholder holds `mapping_key`, `flags`, `width`, `precision` (each with a `star` for `*`), `length` and
 *   `conversion`. A placeholder's value is its key, or the index of the positional value it consumes after its stars.
 * - `braceFormat`: `str.format` templates as Ruff's `format` parses them: `format` holds `literal` and `field`; a field
 *   holds `field_name` (valued by the argument it reads: automatic number, index, or keyword), `conversion` and
 *   `format_spec`, whose parts are nested fields or the mini-language (`fill`, `align`, `sign`, `alternate`, `zero`,
 *   `width`, `grouping`, `precision`, `type`, or a `spec_error`).
 *
 * - `annotation`: a string type definition as Ruff reads it (`parse_type_annotation`): only on a string Ruff reads as a
 *   type definition (in an annotation, a type alias's value or bound, a typing generic's slice, the type arguments of
 *   `cast`, `TypeVar`, `NewType`, `NamedTuple`, `TypedDict` and the like; not inside `Literal[...]`), whose names the
 *   scope table reads there as forward references. The root is an `expression_statement` holding the expression, with
 *   Python's kinds; a simple string (one part, its source its value) maps each node to its exact source, any other
 *   string maps every node to the whole string. A string Ruff's parser refuses (F722) is one `ERROR` node. In a string
 *   it parses, each semantic syntax error its checker reports there (F722: `await` or `yield` outside a function or
 *   an async one, a walrus rebinding a comprehension variable, a duplicate keyword argument, a name used before the
 *   scope's `global`, ...; which depends on the scopes around the string) is an `ERROR` child of the root after the
 *   expression, over the offending node, valued by Ruff's message. Its rules may also use
 *   `pattern`, `matches`, `resolves` (not `binding`), and `parse` with this grammar for a nested string. It reads the
 *   file's scope table: not evaluated in files with syntax errors (`source-incomplete`).
 * - `eslintDirective` (TypeScript): a comment ESLint 10.11.0 reads as a directive (`getInlineConfigNodes`: `eslint`,
 *   `eslint-env`, `eslint-enable`, `eslint-disable`, `eslint-disable-line`, `eslint-disable-next-line`, `exported`,
 *   `global` or `globals`, and in a `//` comment only the two line directives); on any other comment or node it does
 *   not hold. The tree maps each node to its exact source in the comment. A `directive` root over the whole comment,
 *   valued by its label, holds an `item` per name of its list (a rule id of the disable directives, a name of
 *   `exported`, `global` or `globals`; valued by the name and over it, a name listed twice once, at its first place, as
 *   ESLint's `parseListConfig` and `parseStringConfig` read them), a `global` item's `setting` child (valued by the
 *   setting, over the text after its `:`), and a `description` for the text after ` -- `. `eslint` and `eslint-env`
 *   have no items. It reads the comment alone: sound in files with syntax errors. Captures are exported as the other
 *   grammars' are, so a violation can be anchored inside the comment (no-redeclare's report on a `/* global *\/` name).
 * - `printfFormat` (php): a PHP printf-family format as PHP 8.5.11's `php_formatted_print` reads it with every value
 *   passed, on a `string`, `encapsed_string` (without interpolation), `heredoc` or `nowdoc` decoded as PHP decodes it:
 *   `format` holds `literal` and `placeholder`; a placeholder holds `argnum` (`1$`), `flags` (`-`, `+`, space, `0`, `'`
 *   and its padding byte), `width`, `precision` (each with a `star` for `*` or `*N$`), `length` (`l`) and `conversion`.
 *   A placeholder's value is the index (from 0, after the format) of the value it consumes, a star's of the value it
 *   reads, an `argnum`'s of the value it names; `%%` is literal, but `%5%` consumes a value as PHP's does. Values are
 *   PHP's (strings are bytes). A quoted string maps each node to its source (an escape to the whole escape); a heredoc
 *   or nowdoc maps every node to the whole literal.
 * - `scanfFormat` (php): a `sscanf`/`fscanf` format as PHP 8.5.11's `ValidateFormat` reads it with no variable
 *   passed: `format` holds `literal` and `placeholder`; a placeholder holds `suppress` (`*`), `argnum`, `width`,
 *   `length` and `conversion` (with a `set` child for `[...]`), valued by the index of the variable it assigns (none
 *   when suppressed). What follows a NUL byte is one `literal` PHP never reads.
 * - `phpDoc` (php): a doc comment (a `comment` PHP lexes as `T_DOC_COMMENT`: `/**` and white space) as PHPStan 2.2.16
 *   reads it through phpstan/phpdoc-parser 2.3.5; on any other comment or node it does not hold. Kinds are the
 *   parser's AST classes in snake case without `Node`: a `php_doc` root holds `php_doc_text` and `php_doc_tag`
 *   (valued by its name, `@param`); a tag holds its value (`param_tag_value`, `return_tag_value`, `var_tag_value`,
 *   `template_tag_value`, `method_tag_value`, `generic_tag_value`, the `@phpstan-*`/`@psalm-*` variants' values,
 *   Doctrine annotations, ...), which holds types (`identifier_type` valued by its name, `union_type`,
 *   `intersection_type`, `nullable_type`, `generic_type`, `array_type`, `array_shape`, `object_shape`,
 *   `callable_type`, `conditional_type`, `const_type`, `offset_access_type`, ...) and constant expressions, plus
 *   `name`, `variable` and `description` leaves where the text spells them. A value the parser refuses is an
 *   `invalid_tag_value` (a type alias's type an `invalid_type`) valued by the parser's message. Each node maps to its
 *   exact source in the comment; nesting past 256 levels is unknown. It reads the comment alone: sound in files with
 *   syntax errors.
 * - `rustFormat` (rust): a `format_args!` template as rustc 1.99.0's `rustc_parse_format` reads it, on a
 *   `string_literal` or `raw_string_literal` decoded as rustc decodes it (not a byte or C string): `format` holds
 *   `literal` and `field`; a field (valued by the argument it names: an explicit index or a name; none for `{}`) holds
 *   `auto` (zero width), `index` or `name`, then its `format_spec` (valued by its text) with the `type` of the trait it
 *   names (`?`, `x?`, `e`, `x`, ...). Each node maps to its source (an escape to the whole escape).
 *
 * A literal a text grammar refuses is a `format` root holding one `error` node whose value is Ruff's message (PHP's
 * `ValueError` message for `printfFormat`). Inside,
 * a rule may use `kind`, `regex` (over the decoded text), `value` (the grammar's values), `compare` (source, unquoted,
 * value or node), `capture`, `all`, `any`, `not`, `count` and the relations (`inside` stops at the parsed root); a
 * text grammar's rule also `host`, and an `eslintDirective` rule `resolves` with `global` alone: the node's text as a
 * declared global of the query's `languageOptions` (its ecmaVersion's builtins and configured globals, `off` aside;
 * `/* global *\/` comments are not read), listed where `global` lists names.
 */
export interface CodeRuleDslParse extends CodeRuleDslAstRule {
  grammar: CodeRuleDslParseGrammar;
  /** The opaque slot text in a pythonFString projection. Defaults to "{}". */
  interpolation?: string;
}

/**
 * A call's arguments bound to named slots, as the language binds them: a Python `call`, a TypeScript `call_expression`
 * or `new_expression`; on any other node it does not hold. Positional arguments fill the positional slots in order;
 * Python keywords fill the slot of their name (`**{'k': v}` with string keys counts as keywords). A starred argument
 * (`*args`, `...args`) may fill every positional slot from its position on, and `**kw` every keyword-passable slot not
 * passed otherwise: those slots are unknown, and a verdict that depends on one is unknown (no violation, negated or not).
 * A slot passed twice (a TypeError) is unknown too. Arguments the signature has no slot for are ignored.
 */
export interface CodeRuleDslArguments {
  /**
   * Slot names (identifiers) in order. Python only: `/` after the positional-only slots, `*` before the keyword-only
   * ones (as in a `def`), each at most once. A default needs no mark: an omitted slot is absent. At most 256 entries.
   */
  signature: string[];
  /** Capture name (without `$`) to slot: the argument passed there becomes that single-node capture; a slot not passed fails the rule. */
  bind?: Record<string, string>;
  /** Slots that must not be passed. */
  absent?: string[];
  /**
   * `language` (default): as the call binds, above. `syntactic` (Python, PHP): as Ruff's `find_argument` and
   * `find_keyword` read the argument list, so nothing is unknown: a slot is the keyword (PHP: named argument) of its
   * name (the first), else the positional argument at its position among those before the first starred (PHP:
   * unpacked) one; `**kw` and `**{...}` pass nothing. Positional-only slots take no keyword, keyword-only slots no
   * position. PHP reads nikic/PHP-Parser's `args`, parentheses unwrapped; an argument list PHP cannot parse is unknown.
   * TypeScript refuses it. `estree` (TypeScript): as ESLint's rules index `node.arguments[i]`, every argument a
   * position, a spread element included (`f(...a, b)` passes `b` second), so nothing is unknown; a tagged template,
   * which has no `arguments` in ESTree, is no call. Python and PHP refuse it.
   */
  mode?: "language" | "syntactic" | "estree";
}

/** Python's `type(v).__name__` for the values `value` knows. */
export type CodeRuleDslPythonValueType = "int" | "float" | "complex" | "str" | "bytes" | "bool" | "NoneType" | "ellipsis" | "tuple" | "list" | "set" | "dict";
/**
 * JavaScript's `typeof`, with `null` and `array` told apart from `object`. A regex literal's RegExp is an `object`, and
 * `regexp` also names it.
 */
export type CodeRuleDslTypeScriptValueType = "string" | "number" | "bigint" | "boolean" | "null" | "undefined" | "array" | "object" | "regexp";

/**
 * Predicates over what the node evaluates to, judged from its own source: literals, and operators, conditionals and
 * builtin constructors folded over them, as CPython (Python) or eslint-utils' `getStaticValue` (TypeScript) computes
 * them. Every predicate listed must hold. Only an expression has a value; on any other node every predicate fails. An
 * expression whose value is not known (a name, a call, a string past 64 KiB) satisfies no predicate and fails none, so
 * `not` over it does not hold either; `constant` alone asks whether the value is known. Builtin names (`undefined`,
 * `list()`) are values only where no file-local binding shadows them and the file has no syntax errors. A JSX attribute
 * string reads as JSX reads it: raw, without escapes, with `&#NNN;`, `&#xHH;` and the XHTML entity names decoded. It
 * binds and reads no metavariables.
 */
export interface CodeRuleDslValueTest {
  /** Equal in the language's sense: Python `==` (`1 == 1.0 == True`), JavaScript `Object.is`. A JSON integer is a Python `int`. */
  eq?: string | number | boolean | null;
  /** The value's type is one of these names. */
  type?: CodeRuleDslPythonValueType | CodeRuleDslTypeScriptValueType | Array<CodeRuleDslPythonValueType | CodeRuleDslTypeScriptValueType>;
  /** `true`: the whole value is known (a list with an unknown element is not). `false`: it is not. */
  constant?: boolean;
  /** A string (`str`; not `bytes`) that starts with one of these. */
  startsWith?: string | string[];
  /**
   * A string whose decoded value this regex (Rust syntax) matches. Backreferences (`(.).*\\1`) are accepted too, and
   * run on a backtracking engine: past 1,000,000 backtracking steps or on a value past 64 KiB the verdict is unknown.
   */
  regex?: string;
  /** An integer sharing a bit with this mask (`mode & mask != 0`). */
  bitsAny?: number;
  /** A number (Python `bool` included) above, at least, below, or at most this. */
  gt?: number;
  ge?: number;
  lt?: number;
  le?: number;
  /** Truthiness: Python's `bool(v)`, JavaScript's ToBoolean. */
  truthy?: boolean;
  /**
   * `true`: a name's value is its binding's (see `resolves.binding.value`), through at most 16 names per decode; past
   * them, unknown. A followed Python list, set or dict keeps only its type (it may be mutated anywhere); a followed
   * TypeScript array or object whose properties a reference mutates is unknown (eslint-utils' `getStaticValue`). Off
   * by default. Rules that follow are not evaluated in files with syntax errors (`source-incomplete`).
   */
  follow?: boolean;
}

/**
 * Whether a reference resolves to a global name or an imported qualified name, judged within its file. It binds and reads no metavariables, and it
 * runs after the rule object's other terms. Under `not` it holds for every other node, binding sites included.
 *
 * Python follows Ruff: module and class bodies see only bindings made earlier; function bodies see the whole module; a
 * class scope is invisible to nested scopes; `global`/`nonlocal` redirect bindings; star imports never shadow.
 * TypeScript follows eslint-scope and typescript-eslint: `var` hoists, `let`/`const`/class/function declarations are
 * block-scoped, values (`identifier`) and types (`type_identifier`) are separate namespaces, `declare global` shadows
 * nothing. Rules using it are not evaluated in files with syntax errors (`source-incomplete`), except a positive
 * `imported`-only predicate, which reads only its import statement and runs around error regions (`source-partial`).
 */
export interface CodeRuleDslResolves {
  /**
   * Identifiers: the node is a reference whose text is one of them and that no file-local binding visible there
   * shadows. At most 1024 names. `true`: any such reference that is a declared global (TypeScript: the
   * `languageOptions` ecmaVersion builtins, CommonJS and `globals`, ESLint's defaults without them) or a builtin of the
   * Python version (Ruff's default target, 3.10, without `languageOptions`).
   */
  global?: string[] | true;
  /**
   * Dotted names (Python only): the node is a name or attribute chain whose head is bound by an import or a definition,
   * and its qualified name joined with the rest of the chain is one of them. `import a.b` binds `a` to `a`,
   * `import m as x` binds `x` to `m`, `from m import x as y` binds `y` to `m.x`. A `def` or `class` is qualified by the
   * file's module path (the snapshot file's `module`, else the file's stem, as Ruff does outside a package); a relative
   * import resolves only against a `module`. Star imports never resolve. With `global` too, either may match. At most
   * 1024 names. `true`: any name or chain with a qualified name (Ruff's `resolve_qualified_name(..).is_some()`): its
   * head is an import, a definition the module path qualifies, or an unshadowed builtin of the Python version. Under
   * `languageOptions.typingModules` a listed `typing.<member>` also matches a typing module's member of that name.
   */
  import?: string[] | true;
  /**
   * Dotted names (Python only): the node is a `dotted_name` an import statement imports, and the qualified name it
   * imports is one of them. `import a.b` and `import a.b as c` import `a.b`; in `from m import x as y` the module
   * name is `m` and `x` is `m.x`; `from __future__ import x` imports `__future__.x`. Relative modules never match.
   * Unlike `import` it reads no bindings. With other keys, either may match. At most 1024 names. A statement with a
   * syntax error before the name imports nothing; alone and not negated it is as sound as a pattern in a file with syntax
   * errors (`source-partial`).
   */
  imported?: string[];
  /** `true`: an `import` or `imported` name also matches every name under it (`pickle` matches `pickle.loads`). */
  prefix?: true;
  /**
   * How `imported` names a relative import (Python). `never` (the default): it names nothing. `text`: by its module text,
   * ignoring the dots, as Ruff's T100, F404 and F407 read an import's `module` without its `level`: `from .pdb import x`
   * imports `pdb.x` and names the module `pdb`; `from . import pdb` and `from .. import pdb` import `pdb`.
   */
  relative?: "never" | "text";
  /**
   * `eslint` (TypeScript only): `/* global a, b: off *\/` block comments in the file declare or remove global names.
   * Without it such comments are ignored. Under `languageOptions` they only decide whether a listed name is global.
   */
  directives?: "eslint";
  /**
   * `global` (TypeScript only, needs `languageOptions`): the node is a `this` whose value is the global object (ESLint's
   * no-eval), at the top level of a `script` or `commonjs` file outside functions (arrow functions aside), field
   * initializers, static blocks and namespaces. no-eval reads a CommonJS file's top level as a script's, though Node
   * binds it to `module.exports`. With `global` or `import` too, either may match.
   *
   * `globalScope` (ESLint's no-alert): a `this` that eslint-scope's global scope holds in a `script`: no function
   * (arrows included), class, static block, namespace, enum, catch clause or with statement encloses it, and from
   * ES2015 no block, switch statement, or loop declaring `let`/`const` either (before ES2015 those are no scopes).
   * A node belongs to its nearest scope-owning ancestor, whichever part of it the node is in. Never in a module or
   * CommonJS file.
   */
  this?: "global" | "globalScope";
  /**
   * `true` (with `this: "global"`): also a `this` whose owner is an ordinary function (a function declaration or
   * expression, generators included; not an arrow, method, accessor, static block or field initializer) that is not
   * strict, since a plain call binds it to the global object (ESLint's no-eval). Strictness follows eslint-scope: from
   * ES5, a module, `parserOptions.ecmaFeatures.impliedStrict`, an enclosing class, or a 'use strict' directive prologue
   * of the function, an enclosing function or the program; ES3 has none. Calls that bind another `this` (methods by
   * property, `bind`/`call`/`apply`, callbacks with a thisArg) are the port's to exclude. Off by default.
   */
  sloppyFunctions?: true;
  /**
   * Under `languageOptions` (default `true`): a `global` name must also be a declared global (TypeScript: the
   * configured ecmaVersion builtins, CommonJS and `globals`) or a builtin of `pythonVersion` (Python). `false` asks only
   * that no file-local binding shadows it, whether or not it is declared (ESLint's no-alert). Needs `global` and
   * `languageOptions`; not with `directives`, which only decide what is declared.
   */
  declared?: boolean;
  /**
   * `true` (TypeScript only, needs `global` and `languageOptions`): in a `script`, a listed name bound at the top level
   * (`var`, `function`, `let`, `const`, `class`) is the global variable itself, as eslint-scope puts a script's
   * top-level declarations in the global scope, so its references match whether or not the name is declared (rules
   * that follow a global variable's references: no-extend-native, no-eval's `window.eval`). Parameters and block
   * bindings still shadow; in a module or CommonJS file nothing changes. Off by default: a declaration shadows.
   */
  scriptDeclarations?: true;
  /**
   * The node is a reference to a file-local binding (not a global, builtin or Ruff's `global` placeholder), and each
   * listed sub-rule matches that binding's node. Python reads the binding the scope table picks (Ruff's: the latest one
   * the read sees; function bodies see the whole module); TypeScript the declaration the name resolves to lexically.
   * Flow is not modelled. The sub-rules share the caller's metavariables, as `has` does, so they may bind captures.
   * Aliasing chains are a recursive utility (`{ any: [<the target>, { resolves: { binding: { value: { matches: <it>
   * } } } }] }`), within the utility depth and step budget; a chain that comes back to a node it is matching fails.
   * Alone in its `resolves` object. Not evaluated in files with syntax errors (`source-incomplete`).
   */
  binding?: CodeRuleDslResolvesBinding;
  /**
   * The node is a reference with no file-local binding (or one an unconditional Python `del` unbound) that is no builtin
   * or declared global: Ruff's F821, ESLint's no-undef. Python counts the builtins of the query's `pythonVersion`
   * (Ruff's default, 3.10, without one) and the names Python binds implicitly (`__module__` and `__qualname__` in a class
   * body, `__class__` in a method or a lambda inside a class, `__path__` in a package's `__init__`); TypeScript the
   * declared globals of `languageOptions` (ESLint's defaults without them), and with `directives` the file's
   * `/* global *\/` comments. `true`: no visible star import may bind the name. `"star"` (Python only): one may (Ruff's
   * F405). Alone in its `resolves` object, `directives` aside. Not evaluated in files with syntax errors.
   */
  unresolved?: true | "star";
  /**
   * TypeScript: `"last"` holds on the node where ESLint's no-unused-vars reports its variable: the last write reference
   * made from the variable's own function scope (eslint-scope's `variableScope`; a declarator's initializer, a `for ...
   * in/of` declaration and each default write at the definition), else the first definition's name. The node is a
   * reference or a binding's name node. `binding` beside it filters that variable; nothing else may be. Not evaluated in
   * files with syntax errors (`source-incomplete`).
   */
  write?: "last";
  /**
   * `true` (Python only): the node is a load in a function (not a lambda or comprehension inside it) that reads the
   * binding its function's own binding of the name, made later, shadows, and is the function's first such read (Ruff's
   * F823, `undefined_local`: the function's latest binding of the name is also its first, and it shadows a binding of an
   * enclosing function or the module; a `global`, `nonlocal` or `del` binding never does). Alone in its `resolves`
   * object. Not evaluated in files with syntax errors.
   */
  beforeLocal?: true;
  /**
   * `true` (Python only, with `imported` top-level module names): the node, of any kind, is one Ruff's traversal visits
   * after an absolute import of one of the modules (Ruff's `seen_module`, which SIM112 and the ASYNC rules read): an
   * `import m`, `import m.x`, `from m import x` or `from m.x import y`, in the module body before the node, or in a
   * function body Ruff visits first (the module body, then function bodies level by level, each level in document
   * order; a lambda's body, a type parameter and a `type` alias's value after every function body). A node's parameters,
   * decorators and defaults belong to the body around its `def`. Alone in its `resolves` object beside `imported`.
   * Not evaluated in files with syntax errors.
   */
  seenBefore?: true;
}

/**
 * How a name is bound: the construct its binding site is. `field`, `method` and `accessor` are TypeScript's private
 * names (`#a`), bindings of their class body that `binds` matches at each declaration's name. `relation` (a SQL
 * `FROM`/`JOIN` relation: its alias, else its table name's last part), `cte` and `target` (an `UPDATE`, `DELETE` or
 * `INSERT` table) are SQL's.
 */
export type CodeRuleDslBindingForm =
  | "assignment"
  | "annotated"
  | "annotation"
  | "augmented"
  | "named"
  | "for"
  | "with"
  | "except"
  | "import"
  | "definition"
  | "parameter"
  | "global"
  | "match"
  | "typeParameter"
  | "typeAlias"
  | "implicit"
  | "deletion"
  | "field"
  | "method"
  | "accessor"
  | "relation"
  | "cte"
  | "target";
/**
 * The kind of scope a binding is held in. Python: `module`, `class`, `function`, `lambda`, `comprehension`,
 * `typeParameters`. TypeScript, as eslint-scope names them: `global` (a script's top level), `module` (a module's or
 * CommonJS file's top level), `function` (arrows and static blocks too), `class` (a class expression's own name),
 * `block` (blocks, loops, `switch`, `catch`, enum bodies), `namespace`, `typeParameters` (type declarations and
 * signatures) and `functionName` (a named function expression's own name). SQL: `query` (a query block, or the block
 * around a `WITH` statement that binds its CTE names).
 */
export type CodeRuleDslScopeKind =
  | "module"
  | "class"
  | "function"
  | "lambda"
  | "comprehension"
  | "typeParameters"
  | "global"
  | "block"
  | "namespace"
  | "functionName"
  | "query";

/**
 * Filters over a binding; each listed one must hold. `used`, `referenced`, `nestedUse` and `exported` are three-valued:
 * where a read may be unseen an unused binding's `used` is unknown, so neither `used: false` nor its negation matches it.
 * Python: an assignment, walrus, `with` name, match capture, `type` alias or parameter of a function that itself calls
 * `locals()`, as Ruff reads it; a module binding under an `__all__` whose value may list names no string entry spells,
 * such as a call or a conditional, though not `None`, a number, a boolean or `...`. TypeScript: a direct `eval(...)` or a
 * `with` statement in the binding's scope or one nested in it.
 *
 * TypeScript reads a variable (one scope's bindings of one name) as ESLint 10.11.0's no-unused-vars does
 * (`isUsedVariable`), with typescript-eslint 8.70.1's additions: a read counts unless it only updates the variable (`a++`
 * or `a += 1` as a statement, `a = a + 1`, outside a function the right side stores), is made from the variable's own
 * function, namespace, enum, interface or type alias, or (for a non-import) sits in a `typeof` type query or a type
 * predicate. `for (x in o) return;` reads `x`. Enum members, mapped type keys, signature and setter parameters,
 * parameter properties, a class expression's own name, `arguments` and declarations of a `declare module` body count as
 * used, as typescript-eslint marks them.
 */
export interface CodeRuleDslBindingFilter {
  /**
   * The binding's form, or one of a list: `assignment` (`x = v`, `x += v` is `augmented`, `x: T = v` is `annotated`),
   * `annotation` (a bare `x: T`, Ruff's `Annotation`: made only where its scope binds the name nowhere earlier, and not
   * assigned, so outside a `.pyi` stub a load passes over it and `resolves.binding` never reads it), `named` (walrus), `for` (loop or comprehension target), `with`, `except`, `import`, `definition` (`def`, `class`),
   * `parameter`, `global` (the statement), `match` (a capture), `typeParameter`, `typeAlias`, `implicit` (Ruff's
   * `global` placeholder), `deletion` (an unconditional `del`).
   */
  form?: CodeRuleDslBindingForm | CodeRuleDslBindingForm[];
  /** The kind of the scope that holds the binding, or one of a list. A `nonlocal` name is held by the enclosing function. */
  scope?: CodeRuleDslScopeKind | CodeRuleDslScopeKind[];
  /**
   * Some read resolves to the binding (a load, a `del`, a `nonlocal` statement, a static module `__all__` entry, or a
   * read of an alias a submodule import shares, as Ruff's `resolve_submodule`), or to the same-scope binding it shadows,
   * whose reads it inherits (Ruff copies them), unless that one is a deletion or an unbound `except` name.
   */
  used?: boolean;
  /**
   * Some reference resolves to the binding, writes included, nothing inherited. TypeScript counts eslint-scope's
   * references: reads, writes, and the writes a declarator's initializer, a `for ... in/of` declaration and each default
   * (`a = 1` in a pattern or parameter) make at the definition. ESLint's `args: "after-used"` reads it.
   */
  referenced?: boolean;
  /** Some such read comes from another scope. */
  nestedUse?: boolean;
  /**
   * Python: an explicit re-export (`import a as a`, `from m import a as a`), or a module binding a static `__all__` names.
   * TypeScript: a definition of the variable stands in an `export` statement (typescript-eslint's `isExported`, over every
   * definition), or, with `directives`, a global variable an ESLint `exported` directive comment lists.
   */
  exported?: boolean;
  /**
   * TypeScript: the variable is a configured global's (ESLint's `eslintImplicitGlobalSetting`, `readonly` or `writable`):
   * it sits in a script's top level (eslint-scope's global scope), and the query's `languageOptions` declare its name;
   * with `directives`, or an ESLint `global` directive comment names it (typescript-eslint's no-redeclare then reports
   * the definitions after the comment). ESLint's no-redeclare reports every definition of such a variable under
   * `builtinGlobals`.
   */
  builtin?: boolean;
  /** TypeScript: `"eslint"` also counts ESLint's `exported` directive block comments (`exported a, b` between comment markers), which name a script's global variables. */
  directives?: "eslint";
  /** The live binding of its name in its scope, the last one made (Ruff's `scope.bindings()`). */
  latest?: boolean;
  /** Made under a `global` or `nonlocal` declaration (the statement, or a later binding of such a name). */
  outer?: boolean;
  /**
   * How many bindings of the name the binding's scope holds, deletions included (Ruff's `scope.get_all`): those whose
   * name node lies inside the node captured as `within`, or all of them. Bindings of other scopes (a comprehension's
   * target, a nested function's local) do not count; a walrus in a comprehension binds in the enclosing scope and
   * does. Ruff's SIM113: `{ "within": "LOOP", "max": 1 }`.
   */
  bindings?: CodeRuleDslBindingCount;
}

/** `min` (default 0) and/or `max`, with `min >= 1` or a `max`; `within` is a capture's name without `$`, as `compare` takes it. */
export interface CodeRuleDslBindingCount {
  within?: string;
  min?: number;
  max?: number;
}

/**
 * A binding's name node: Python's identifier, an unaliased import item's `dotted_name` (`os.path` in `import os.path`,
 * `x` in `from m import x`), an aliased import's alias; TypeScript's declared identifier, type name, enum member or
 * shorthand pattern (`{ a }`), where ESLint reports a definition. The binding passes the filters and each listed sub-rule matches its
 * node, as `resolves.binding`'s do. Not evaluated in files with syntax errors (`source-incomplete`).
 */
export interface CodeRuleDslBinds extends CodeRuleDslBindingFilter, CodeRuleDslBindingRules {
  /**
   * TypeScript: some occurrence of the binding's variable, a definition's name or a reference (reads and writes), matches
   * this rule, as ESLint's no-unused-vars reads `def.name.parent` and `ref.identifier.parent` for
   * `destructuredArrayIgnorePattern` and `ignoreRestSiblings`. The first one that matches binds its metavariables.
   */
  occurrence?: CodeRuleDslAstRule;
  /**
   * Python: the binding shadows one that passes these filters and sub-rules, on the same fork: the binding before it in
   * its scope, or, for the first binding of its name in its scope, the latest one of the nearest enclosing function or
   * module scope that binds the name when it is made (Ruff's `shadowed_bindings`; a function body is read after the
   * scopes around it). Bindings in different `if`/`elif`/`else` bodies, `try` clauses (the body and its `else` are one),
   * `with` bodies or `match` cases never count. The shadowed binding's sub-rules share the metavariables.
   *
   * TypeScript: the variable (one scope's bindings of one name) has an earlier definition, in document order, that
   * passes (ESLint's no-redeclare reports each definition after the first); with `every`, each other definition passes.
   * A named function expression's own name is a variable of its own, and `arguments` is no definition.
   */
  redefines?: CodeRuleDslRedefines;
}

/** What `binds.redefines` asks of the shadowed binding. */
export interface CodeRuleDslRedefines extends CodeRuleDslBindingFilter, CodeRuleDslBindingRules {
  /** Whether the shadowed binding is of the same scope (`false`: an enclosing function's or module's). */
  sameScope?: boolean;
  /** Python: whether both are imports of the same qualified name (`import a.b` imports `a.b`, `from m import x` imports `m.x`). */
  sameImport?: boolean;
  /**
   * TypeScript: every other definition of the variable passes (vacuously when there is none), rather than some earlier
   * one. The sub-rules of each share the metavariables, as `all` does.
   */
  every?: boolean;
}

/**
 * Sub-rules over the nodes of a name's binding (`resolves.binding`); each listed one must match. The filters of `binds`
 * apply to that binding too (`latest` and `outer` are Python's).
 */
export interface CodeRuleDslResolvesBinding extends CodeRuleDslBindingRules, CodeRuleDslBindingFilter {
  /**
   * Python: in a scope that declares the name `global`, read that scope's `global` binding (its site the `global`
   * statement, no value) instead of the module's, as Ruff's `lookup_symbol` does (`find_assigned_value`,
   * `is_bound_to_tuple`). Off by default (Ruff's `resolve_name`). TypeScript refuses it.
   */
  stopAtGlobal?: boolean;
  /**
   * The binding shadows no earlier binding of its name in its scope, whatever form each takes (assignment, walrus,
   * `for`, `with`, `except`, match capture, import, `def`, `class`, parameter, `global`, unconditional `del`), as
   * Ruff's `only_binding`: a read before a rebinding still sees the first. Bindings a nested function makes through
   * `nonlocal` count after the scope's own. TypeScript: the name has this one declaration in its scope. Alone it asks
   * just that.
   */
  only?: boolean;
  /**
   * TypeScript: `false` gives a name destructured from an object or array literal no `value` (by default it carries
   * the element it takes), as ESLint's `ReferenceTracker` and `getStaticValue` follow no destructuring. Python refuses
   * it.
   */
  destructuredValue?: boolean;
  /** TypeScript: some definition name or reference of the variable matches, as `binds.occurrence`. */
  occurrence?: CodeRuleDslAstRule;
}

/** Sub-rules over the nodes of a binding; each listed one must match. */
export interface CodeRuleDslBindingRules {
  /**
   * The construct that binds the name: Python's assignment (the inner one of `a = b = v` for `b`), augmented
   * assignment, walrus, `for` statement or comprehension clause, `as` pattern of `with` or `except`, import item
   * (`dotted_name` or `aliased_import`), `def` or `class`, parameter (`identifier`, `default_parameter`,
   * `typed_parameter`, `typed_default_parameter`, `list_splat_pattern`, `dictionary_splat_pattern`), `global`
   * statement, match pattern, type parameter or `type` statement. TypeScript's variable declarator, parameter,
   * `for ... in/of` statement, catch clause, import clause, specifier or namespace import, or declaration.
   */
  site?: CodeRuleDslAstRule;
  /**
   * The value the binding carries. Python: the right-hand side of a plain name assignment (`x = v`, `(x) = v`,
   * `a = b = v`), annotated assignment with a value, or walrus; an unpacked name's element of a tuple or list display
   * (`a, b = x, y`, nested too) where the arities agree or the name comes before a starred target (Ruff's
   * `find_binding_value`, without sets or unequal arities). TypeScript: the initializer of a
   * declarator of a plain name, or the element a name destructured from an object or array literal takes
   * (`const {a, b: [c]} = {a: 1, b: [2]}`; not past a default, a rest, a spread or computed key that could supply it,
   * or an accessor), where the name has that single definition and no reference writes it (eslint-utils'
   * `isEffectivelyConst`). `value.follow` reads no destructured name, as `getStaticValue` does not. Without one the
   * sub-rule does not match.
   */
  value?: CodeRuleDslAstRule;
  /** The annotation, without its `:`: a Python annotated assignment or typed parameter; a TypeScript declarator, parameter or catch parameter. */
  annotation?: CodeRuleDslAstRule;
}

/**
 * Exactly one relation plus bounds: `min` (default 0) and/or `max`, with `min >= 1` or a `max`. The relation walks as
 * usual, except that `has` keeps descending below a match, so every matching descendant counts. The counted rule reads
 * the caller's metavariables; nothing it binds is kept.
 */
export interface CodeRuleDslAstCount {
  inside?: CodeRuleDslAstRelation;
  has?: CodeRuleDslAstRelation;
  precedes?: CodeRuleDslAstRelation;
  follows?: CodeRuleDslAstRelation;
  min?: number;
  max?: number;
  /**
   * Metavariable name (without `$`) that a count which holds binds to `offset` plus the exact count, as decimal text.
   * Messages and source-text comparisons may read it; under `not` it binds nothing outside that negation. The walk runs to its end, and
   * a count that is not known exactly (a node or stop verdict unknown) leaves the match unknown.
   */
  as?: string;
  /** An integer added to the count `as` binds (default 0); only with `as`. */
  offset?: number;
}

export type CodeRuleCheckpoint = "edit" | "function" | "turn" | "final";
export type CodeRuleSourceLanguage =
  | CodeRuleDialectLabel
  | "json"
  | "yaml"
  | "toml"
  | "markdown"
  | "text";

export interface CodeRuleEdit {
  path: string;
  language: CodeRuleSourceLanguage;
  /** Source observed before the editing tool began. */
  previousText?: string;
  /** Why the pre-edit source could not be captured; distinct from a new file. */
  previousReadError?: string;
  /** Source observed after the editing tool completed. */
  generatedText?: string;
  /** The source existed before the tool ran but is now absent. */
  deleted?: boolean;
  /** The post-edit source could not be read for a reason other than deletion. */
  readError?: string;
}

export interface CodeRuleEditBatch {
  revision: number;
  checkpoints: CodeRuleCheckpoint[];
  edits: CodeRuleEdit[];
}

export interface CodeRuleSourceSpan {
  startByte: number;
  endByte: number;
  startLine: number;
  startColumn: number;
  endLine: number;
  endColumn: number;
}

/** A suggested replacement: `text` replaces `span` (the witness). Suggestion only; Rerail never applies it. */
export interface CodeRuleViolationFix {
  span: CodeRuleSourceSpan;
  text: string;
}

export interface CodeRuleViolation {
  id: string;
  ruleId: string;
  clauseId: string;
  code: string;
  /** The mode of the rule that produced the violation. */
  mode: RuleMode;
  message: string;
  path?: string | undefined; // NOSONAR - exactOptionalPropertyTypes: zod-parsed native violations carry `path: undefined`
  span?: CodeRuleSourceSpan | undefined; // NOSONAR - as `path`
  captures: Record<string, CodeRuleSourceSpan[]>;
  variables: Record<string, boolean | number | string>;
  fix?: CodeRuleViolationFix | undefined; // NOSONAR - exactOptionalPropertyTypes: zod-parsed native violations carry `fix: undefined`
}

export type { AuditFailure } from "../audit/audit.js";

export interface CodeRuleResponse {
  revision: number;
  disposition: "continue" | "steer";
  violations: CodeRuleViolation[];
  failures: AuditFailure[];
}

export interface SteerMetadata {
  revision: number;
  /** Source inputs whose newer revisions invalidate this result. */
  scope: string[];
  /** Rule ids rendered into the delivered content; absent for deliveries that carry no code-rule violations. */
  ruleIds?: readonly string[];
  background?: boolean;
  /** Marks a steer produced by the generated edit settling at a native tool boundary. */
  generatedEdit?: boolean;
}

export interface RuleSteerDelivery {
  requestSteer(
    sessionId: string,
    delivery: "steer",
    content: string,
    metadata?: SteerMetadata,
  ): Promise<void | "queued" | "discarded">;
  /**
   * Sources an evaluation at `revision` read again without an edit of their own (a `package.json`, project configuration
   * or server-graph change gave them new facts, or findings): a steer queued for one of them at an earlier revision is
   * stale and is dropped whole. Returns the other sources the dropped steers spoke for that no later revision revised,
   * whose findings the auditor steers again.
   */
  revised?(sessionId: string, revision: number, paths: readonly string[]): readonly string[];
}
export interface AuditSettings {
  scope?: AuditScope;
  mode?: AuditMode;
}

/** A repository audit records one per checkout: each checkout's audit is claimed first and reported when it ends. */
export interface RepositoryAuditHooks {
  /**
   * False when another live session is already auditing `checkoutRoot`; that checkout is then skipped. `catalogRevision`
   * is the compiled catalog the audit would run against.
   */
  claim(checkoutRoot: string, catalogRevision: string): boolean;
  finish(checkoutRoot: string, auditId: string | undefined): void;
}

export interface CodeRuleAuditor {
  openWorkspace(sessionId: string): Promise<void>;
  /** Refresh live rules before filtering paths for a tool's preimages; `acceptsPath` reflects them on return. */
  refreshCodeRules?(): void;
  /** Activates defaults at the next event boundary; work already queued keeps its snapshot. */
  updateSettings?(settings: Required<AuditSettings>): void;
  /** `path` is relative to the session folder and may lead out of it (`../`) into another checkout. */
  acceptsPath(path: string): boolean;
  /**
   * The folder whose real path must contain an accepted `path`'s real path (its owning checkout); the session folder
   * when undefined or absent.
   */
  sourceRoot?(path: string): string | undefined;
  auditGeneratedEdits(sessionId: string, batch: CodeRuleEditBatch): Promise<CodeRuleResponse>;
  /** Wait until every accepted generated edit has completed, without waiting for unrelated audits. */
  settleGeneratedEdits?(): Promise<void>;
  audit?(sessionId: string, input: {
    scope: "repository" | "touched-files";
    mode: AuditMode;
    paths?: string[];
    audit?: RepositoryAuditHooks;
  }): Promise<readonly string[]>;
  /**
   * Stop outstanding evaluation promptly when its session closes. Every audit the session still runs ends interrupted
   * unless `interruptAudits` is false: an evaluator whose session id was reopened meanwhile must leave the new
   * session's audits alone, and ends only the audits it started itself.
   */
  interrupt?(options?: AuditorCloseOptions): void;
  /** Wait until every accepted generated edit and audit has completed. */
  settle?(): Promise<void>;
  close(options?: AuditorCloseOptions): Promise<void>;
}

export interface AuditorCloseOptions {
  /** False ends only the audits this evaluator started, leaving the rest of the session's running audits as they are; true by default. */
  readonly interruptAudits?: boolean;
}

function violationLocation(violation: CodeRuleViolation): string {
  if (violation.path === undefined) return "";
  if (violation.span === undefined) return ` (${violation.path})`;
  const { startColumn, startLine } = violation.span;
  return ` (${violation.path}:${startLine + 1}:${startColumn + 1})`;
}

/** Longest fix text rendered inline in a steer; a longer fix is cut with a note. */
export const STEER_FIX_CHARS = 240;

function suggestedFix(fix: CodeRuleViolationFix): string {
  const { startLine, startColumn, endLine, endColumn } = fix.span;
  const range = `${startLine + 1}:${startColumn + 1}-${endLine + 1}:${endColumn + 1}`;
  if (fix.text === "") return `  Suggested fix: delete ${range}`;
  const text = fix.text.length > STEER_FIX_CHARS ? `${fix.text.slice(0, STEER_FIX_CHARS)}… (truncated)` : fix.text;
  if (!text.includes("\n")) return `  Suggested fix: replace ${range} with \`${text}\``;
  const indented = text.split("\n").map(line => `    ${line}`).join("\n");
  return `  Suggested fix: replace ${range} with:\n${indented}`;
}

/** Where each rule a steer names was added from; undefined when the rule's provenance is not recorded. */
export type SteerSourceOf = (ruleId: string) => SteerRuleSource | undefined;

/**
 * One block per rule (`steerBlock`), its violations in first-seen order, each with its location and any fix. The block
 * names the rule's pack (or library) and its mode, from the first violation of that rule.
 */
export function renderSteer(violations: readonly CodeRuleViolation[], sourceOf: SteerSourceOf = () => undefined): string {
  const byRule = new Map<string, { mode: RuleMode; lines: string[] }>();
  for (const violation of violations) {
    const line = `${violation.message}${violationLocation(violation)}`;
    const entry = byRule.get(violation.ruleId) ?? { mode: violation.mode, lines: [] };
    entry.lines.push(violation.fix === undefined ? line : `${line}\n${suggestedFix(violation.fix)}`);
    byRule.set(violation.ruleId, entry);
  }
  return [...byRule]
    .map(([ruleId, { mode, lines }]) => steerBlock(ruleId, lines.join("\n"), { mode, origin: steerOrigin(ruleId, sourceOf(ruleId)) }))
    .join("\n\n");
}

/** Rule ids credited with a steer: the distinct `ruleId`s of the rendered violations, in first-seen order. */
export function steerRuleIds(violations: readonly CodeRuleViolation[]): string[] {
  return [...new Set(violations.map((violation) => violation.ruleId))];
}

/** The rendered steer text with its contributing rule ids; the text is byte-identical to `renderSteer`. */
export function buildSteer(
  violations: readonly CodeRuleViolation[],
  sourceOf?: SteerSourceOf,
): { content: string; ruleIds: string[] } {
  return {
    content: renderSteer(violations, sourceOf),
    ruleIds: steerRuleIds(violations),
  };
}
