/**
 * How ESLint reads its configurations and RuleTester cases, as the gates need it: shared by eslint-driver.mjs (which
 * imports this file directly, so it stays free of imports and of TypeScript syntax Node cannot strip) and the
 * adapter.
 */

/** A rule setting as a flat config writes it: a severity, or a severity followed by the rule's options. */
export type RuleSetting = string | number | [string | number, ...unknown[]];

/** One named configuration a package exports, as the flat config array a user spreads into theirs. */
export interface ApprovedConfiguration {
  name: string;
  configs: { rules?: Record<string, RuleSetting> }[];
}

/**
 * The configuration a package's `configs` export holds under a name, read as a user writes it: `flat/recommended` is
 * one key, `flat.recommended` the `recommended` member of `configs.flat` (eslint-plugin-react, eslint-plugin-react-hooks).
 */
export function configNamed(configs: Record<string, unknown>, name: string): unknown {
  return name.split(".").reduce<unknown>((value, key) => (value !== null && typeof value === "object" ? (value as Record<string, unknown>)[key] : undefined), configs);
}

export function isOff(setting: RuleSetting): boolean {
  const severity = Array.isArray(setting) ? setting[0] : setting;
  return severity === "off" || severity === 0;
}

/**
 * The first approved configuration, in order, that turns the rule on, with the options it sets; null when none does.
 * A configuration's later entries override its earlier ones, as ESLint merges a flat config array.
 */
export function firstApproved(approved: ApprovedConfiguration[], ruleId: string): { config: string; options: unknown[] } | null {
  for (const { name, configs } of approved) {
    const rules: Record<string, RuleSetting> = Object.assign({}, ...configs.map((entry) => entry.rules ?? {}));
    const setting = rules[ruleId];
    if (setting !== undefined && !isOff(setting)) return { config: name, options: Array.isArray(setting) ? setting.slice(1) : [] };
  }
  return null;
}

/**
 * A flat configuration (one object, or an array of them) with one rule's setting replaced wherever it sets that rule:
 * a case tester's configuration sets the rule under test with the case's own options, and matches-upstream-tests lints
 * it with the approved setting instead.
 */
export function withSetting<T>(config: T, ruleKey: string, setting: RuleSetting): T {
  if (Array.isArray(config)) return config.map((entry: unknown) => withSetting(entry, ruleKey, setting)) as T;
  const object = config as { rules?: Record<string, RuleSetting> } | null;
  if (object === null || typeof object !== "object" || object.rules === undefined || !Object.hasOwn(object.rules, ruleKey)) return config;
  return { ...object, rules: { ...object.rules, [ruleKey]: setting } } as T;
}

interface DeprecatedInfo { message?: string; deprecatedSince?: string; replacedBy?: { rule?: { name?: string } }[] }

/** Upstream's own deprecation notice for a rule (`meta.deprecated`, a flag or ESLint's DeprecatedInfo), or null. */
export function deprecationText(meta: { deprecated?: boolean | DeprecatedInfo } | undefined): string | null {
  const deprecated = meta?.deprecated;
  if (deprecated === undefined || deprecated === false) return null;
  if (deprecated === true) return "deprecated";
  const replaced = (deprecated.replacedBy ?? []).map((entry) => entry.rule?.name).filter((name): name is string => typeof name === "string");
  const details = [deprecated.deprecatedSince === undefined ? "" : `since ${deprecated.deprecatedSince}`, replaced.length === 0 ? "" : `replaced by ${replaced.join(", ")}`].filter(Boolean);
  return `${(deprecated.message ?? "deprecated").replace(/\.$/u, "")}${details.length === 0 ? "" : ` (${details.join("; ")})`}`;
}

/** The languages a parser's trees are of, as the engine has file types for them. */
export type ParserLanguage = "javascript" | "typescript" | "vue";

/**
 * The language a case's parser reads: by the name the parser declares (`meta.name`: espree, typescript-eslint's
 * parser, vue-eslint-parser), or else by the npm package it was loaded from, as the manifest's `parsers` maps it (older
 * parsers declare no name). Undefined for any other parser.
 */
export function parserLanguage(name: string | undefined, packageName: string | undefined, parsers: Record<string, ParserLanguage>): ParserLanguage | undefined {
  if (name === "typescript-eslint/parser") return "typescript";
  if (name === "espree") return "javascript";
  if (name === "vue-eslint-parser") return "vue";
  return packageName === undefined ? undefined : parsers[packageName];
}

/** The file type a case's code is in: a Vue single-file component, TypeScript (TSX with JSX), or JSX, CommonJS or JavaScript. */
export function fileType(language: { language: ParserLanguage; jsx: boolean; sourceType: string | undefined }): string {
  if (language.language === "vue") return ".vue";
  if (language.language === "typescript") return language.jsx ? ".tsx" : ".ts";
  return language.jsx ? ".jsx" : language.sourceType === "commonjs" ? ".cjs" : ".js";
}

/**
 * Whether vue-eslint-parser reads a case as a single-file component (its isVueFile): a `.vue` file, or code that
 * starts with `<` after white space whatever its name (`<input>`, Linter's name for a case that names none, included).
 * Any other code it reads as one script.
 */
export function vueReadsAsComponent(filename: string | undefined, code: string): boolean {
  return /\.vue$/u.test(filename ?? "") || /^\s*</u.test(code);
}

/**
 * The `<script lang>` vue-eslint-parser picks a `parserOptions.parser` entry by when it reads a case as one script: its
 * file's extension, `tsx` falling back to `ts` and `jsx` to `js`.
 */
export function vueScriptLangs(filename: string | undefined): string[] {
  const extension = (/\.([^./\\]*)$/u.exec(filename ?? "unknown.js")?.[1] ?? "").toLowerCase();
  return /^[jt]sx$/u.test(extension) ? [extension, extension.slice(0, -1)] : [extension];
}

/**
 * The script parser vue-eslint-parser's `parserOptions.parser` picks for one of `langs` (its getScriptParser): a parser
 * object, a module name, or undefined for espree. A map without an entry for any lang falls back to its `js`.
 */
export function vueScriptParser(parser: unknown, langs: string[]): unknown {
  const isParser = (value: unknown): boolean => typeof value === "object" && value !== null
    && (("parse" in value && typeof value.parse === "function") || ("parseForESLint" in value && typeof value.parseForESLint === "function"));
  if (isParser(parser)) return parser;
  if (typeof parser === "object" && parser !== null) {
    const entries = new Map(Object.entries(parser));
    const picked = langs.map((lang) => entries.get(lang)).find((value) => typeof value === "string" || isParser(value));
    return picked ?? entries.get("js");
  }
  return typeof parser === "string" ? parser : undefined;
}

/** The npm package a module file belongs to: the path segment(s) after its last `node_modules` (`@scope/name` or `name`). */
export function modulePackage(file: string): string | undefined {
  const parts = file.replaceAll("\\", "/").split("/");
  const at = parts.lastIndexOf("node_modules");
  if (at === -1 || at + 1 >= parts.length) return undefined;
  return parts[at + 1]!.startsWith("@") ? parts.slice(at + 1, at + 3).join("/") : parts[at + 1];
}

/**
 * The file type of the code a test fixture parser froze a tree for, by the checkout directory it was loaded from (the
 * manifest's `fixture_parsers`); undefined for any other fixture parser, which returns a fixed tree whatever the code.
 */
export function frozenFileType(fixture: string, fixtureParsers: Record<string, string>): string | undefined {
  const directory = Object.keys(fixtureParsers).find((prefix) => fixture.startsWith(prefix));
  return directory === undefined ? undefined : fixtureParsers[directory];
}

/**
 * A configuration without what a case or its RuleTester sets beside its language: the globals it declares
 * (`languageOptions.globals`) and its shared settings (`settings`, such as eslint-plugin-react's React version and
 * pragma). matches-upstream-tests lints each case in the environment upstream's approved configuration sets, which
 * declares neither, as it sets the rule's options, and keeps only the language the case's code is in (parser, parser
 * options, ecmaVersion, sourceType). Globals the code declares in comments stay.
 */
export function withoutEnvironment(config: unknown): unknown {
  if (Array.isArray(config)) return config.map(withoutEnvironment);
  if (config === null || typeof config !== "object") return config;
  let result = config as { settings?: unknown; languageOptions?: { globals?: unknown } };
  if (Object.hasOwn(result, "settings")) {
    const { settings: _settings, ...rest } = result;
    result = rest;
  }
  if (result.languageOptions?.globals !== undefined) {
    const { globals: _globals, ...languageOptions } = result.languageOptions;
    result = { ...result, languageOptions };
  }
  return result;
}

/** Whether a configuration declares globals or non-empty settings: an environment the approved configuration does not set. */
export function declaresEnvironment(config: unknown): boolean {
  if (Array.isArray(config)) return config.some(declaresEnvironment);
  if (config === null || typeof config !== "object") return false;
  const entry = config as { settings?: object; languageOptions?: { globals?: unknown } };
  return entry.languageOptions?.globals !== undefined || Object.keys(entry.settings ?? {}).length > 0;
}

/** A finding position as ESLint reports it: 1-based line and column, 1-based exclusive end, ends when reported. */
export type MessagePosition = [number, number, number | null, number | null];

/**
 * Upstream's written expectation for a case, where it is comparable: only when the case's own options are the
 * approved ones, and for an invalid case only when every error pins all four positions. Valid cases expect none.
 */
export function writtenExpectation(kind: "valid" | "invalid", errors: unknown, ownOptions: unknown[] | undefined, approvedOptions: unknown[], equal: (a: unknown, b: unknown) => boolean): MessagePosition[] | null {
  if (!equal(ownOptions ?? [], approvedOptions)) return null;
  if (kind === "valid") return [];
  if (!Array.isArray(errors)) return null;
  const positioned = errors.every((error) => error !== null && typeof error === "object" && ["line", "column", "endLine", "endColumn"].every((key) => Object.hasOwn(error, key)));
  return positioned ? errors.map((error: { line: number; column: number; endLine: number; endColumn: number }) => [error.line, error.column, error.endLine, error.endColumn]) : null;
}

/** A case's key among upstream's test files: the file, the RuleTester run in it, and the case's place in the run. */
export function caseFixture(testFile: string, run: number, kind: "valid" | "invalid", index: number): string {
  return `${testFile}#${run}/${kind}[${index}]`;
}

/**
 * The path the engine evaluates a case at. When upstream names the file it lints the case as, that file, directories
 * kept, because rules read them (Next.js's read `pages/`, `app/` and `_document`, some anchored at the path's start):
 * relative to the upstream checkout when it lies there (typescript-eslint's fixtures), a Windows name
 * (`C:\Users\...\app\layout.tsx`) as the same directories, any other absolute name relative to the filesystem root.
 * A name elsewhere in the gates cache that holds the checkout and its runtime was built from the directory the
 * tests run in (eslint-plugin-unicorn's filename-case names a file above it), so its directories are this machine's,
 * not upstream's: it keeps only its base name, in the case's own directory. A case upstream names no file for gets a
 * directory of its own: `<rule>/<test file>.<run>.<kind>-<index>/input`, with the case's file type.
 */
export function casePath(rule: string, testFile: string, run: number, kind: "valid" | "invalid", index: number, filename: string | undefined, checkout: string, cache: string, extension: string): string {
  const named = filename ?? "";
  const posix = /^[A-Za-z]:[\\/]/u.test(named) ? named.slice(2).replaceAll("\\", "/") : named;
  const inCheckout = posix.startsWith(`${checkout}/`) ? posix.slice(checkout.length + 1) : posix;
  const segments = inCheckout.split("/").filter((segment) => segment !== "" && segment !== ".");
  const stem = (testFile.split("/").pop() ?? testFile).replace(/(?:\.test)?\.[cm]?[jt]sx?$/u, "");
  const own = `${rule}/${stem}.${run}.${kind}-${index}`;
  if (inCheckout.startsWith(`${cache}/`)) return `${own}/${segments.at(-1)}`;
  if (segments.length > 0) return segments.join("/");
  return `${own}/input${extension}`;
}

/**
 * The file type the engine reads a case as, when it is not its path's. RuleTester parses a case with the case's parser
 * whatever its file name, and hands the name to the rule only as `context.filename`. A TypeScript name (`.ts`, `.mts`,
 * `.cts`, `.tsx`) for a case a JavaScript parser read, such as @next/eslint-plugin-next's JSX at `pages/index.ts`,
 * would make the engine read TypeScript upstream never parsed, so the case is read in the parser's file type
 * (`extension`) at its own path; undefined when the path's file type is the language upstream read. Where upstream
 * lints `.vue` files as single-file components (`components`), a `.vue` name for a case a script parser read
 * (eslint-plugin-vue's script cases at `test.vue`, which its rules take for a component's script by that name) is
 * read the same way: the engine reads a `.vue` path as a component, and the case's code is a script.
 */
export function parsedAs(path: string, extension: string, components = false): string | undefined {
  if (components && extension !== ".vue" && /\.vue$/u.test(path)) return extension;
  return !/^\.tsx?$/u.test(extension) && /\.[cm]?tsx?$/u.test(path) ? extension : undefined;
}

/** Every string a value holds, depth first: the values of a message's `data`, whatever their nesting. */
export function strings(value: unknown): string[] {
  if (typeof value === "string") return [value];
  if (Array.isArray(value)) return value.flatMap(strings);
  if (value !== null && typeof value === "object") return Object.values(value).flatMap(strings);
  return [];
}

interface TestCaseFields {
  code?: unknown;
  output?: unknown;
  filename?: unknown;
  options?: unknown;
  settings?: unknown;
  errors?: unknown;
  languageOptions?: { globals?: Record<string, unknown>; parserOptions?: { ecmaFeatures?: { jsx?: unknown } } };
}
interface ReportedFields { message?: unknown; desc?: unknown; data?: unknown; output?: unknown; suggestions?: unknown }

/**
 * Everything a RuleTester case holds for not-overfit's test words: the code it lints and the code it expects after fixes
 * (`output`, one string or one per fix pass, and each suggestion's output), read as code; and its test configuration
 * (options, settings, the names of the globals it declares) with what it expects reported (each error's message, the
 * values of its `data`, its suggestions' descriptions and data), read as words.
 */
export function caseWords(raw: unknown): { code: string; outputs: string[]; jsx: boolean; configuration: string } {
  const item: TestCaseFields = typeof raw === "string" ? { code: raw } : ((raw ?? {}) as TestCaseFields);
  const errors = (Array.isArray(item.errors) ? item.errors : []) as (string | ReportedFields | null)[];
  const objects = (entries: unknown[]) => entries.filter((entry): entry is ReportedFields => entry !== null && typeof entry === "object");
  const suggestions = objects(errors).flatMap((error) => objects(Array.isArray(error.suggestions) ? error.suggestions : []));
  const outputs = [item.output, ...suggestions.map((suggestion) => suggestion.output)].flatMap((output) => (Array.isArray(output) ? output : [output]));
  const reported = [...errors, ...suggestions].flatMap((entry) => (typeof entry === "string" ? [entry] : entry === null ? [] : [entry.message, entry.desc, ...strings(entry.data)]));
  const filename = typeof item.filename === "string" ? item.filename : "";
  return {
    code: typeof item.code === "string" ? item.code : "",
    outputs: [...new Set(outputs.filter((output): output is string => typeof output === "string"))],
    jsx: item.languageOptions?.parserOptions?.ecmaFeatures?.jsx === true || /\.[jt]sx$/u.test(filename),
    configuration: JSON.stringify([item.options ?? null, item.settings ?? null, Object.keys(item.languageOptions?.globals ?? {}), reported.filter((text) => typeof text === "string")]),
  };
}
