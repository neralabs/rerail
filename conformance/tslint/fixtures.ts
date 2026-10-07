import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, statSync } from "node:fs";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import type { PolicySourceLanguage } from "../../src/executable-policy.js";
import { parseLintFile, type ParsedLintFile } from "./lint-format.js";
import { compareDeterministicStrings } from "../../src/stable-json.js";

/** palantir/tslint master at the time the harness was built (the repository is archived; master no longer moves). */
export const TSLINT_REPOSITORY = "https://github.com/palantir/tslint";
export const TSLINT_SHA = "285fc1db18d1fd24680d6a2282c6445abf1566ee";

export const HARNESS_DIRECTORY = dirname(fileURLToPath(import.meta.url));
export const RERAIL_ROOT = resolve(HARNESS_DIRECTORY, "..", "..");
export const REPOSITORY_ROOT = resolve(RERAIL_ROOT, "..");
export const FIXTURE_CHECKOUT = join(REPOSITORY_ROOT, "tmp", "tslint-fixtures");

/** `path` resolved absolute; throws unless it lies inside the repository checkout. */
export function repositoryPath(path: string): string {
  const absolute = resolve(path);
  if (absolute !== REPOSITORY_ROOT && !absolute.startsWith(`${REPOSITORY_ROOT}${sep}`)) {
    throw new Error(`${path} is outside the repository (${REPOSITORY_ROOT})`);
  }
  return absolute;
}
export const FIXTURE_RULES = join(FIXTURE_CHECKOUT, "test", "rules");
/** Fixtures, TSLint's own test-format parser, and the rule implementations that define exact semantics. */
const SPARSE_PATHS = ["/test/rules/", "/src/verify/", "/src/test.ts", "/src/rules/"];

export interface RuleInfo {
  name: string;
  requiresTypeInfo: boolean;
  hasFixer: boolean;
}

/** The 163 TSLint core rules, with `requiresTypeInfo` and fixer flags from each rule's metadata. */
export function loadRuleList(): RuleInfo[] {
  return readFileSync(join(HARNESS_DIRECTORY, "tslint-rules.tsv"), "utf8")
    .split("\n")
    .filter(line => line.trim() !== "")
    .map(line => {
      const [name = "", typeInfo, fixer] = line.split("\t");
      return { name, requiresTypeInfo: typeInfo === "T", hasFixer: fixer === "F" };
    });
}

/** Rules taken out of the equivalence set (rule to reason). They are listed in reports but never evaluated. */
export function loadExcludedRules(): Map<string, string> {
  const parsed: unknown = JSON.parse(readFileSync(join(HARNESS_DIRECTORY, "excluded-rules.json"), "utf8"));
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) throw new Error("excluded-rules.json must map rule names to reasons");
  const excluded = new Map<string, string>();
  for (const [rule, reason] of Object.entries(parsed)) {
    if (typeof reason !== "string") throw new Error(`excluded-rules.json: reason for ${rule} must be a string`);
    excluded.set(rule, reason);
  }
  return excluded;
}

/** The core rules minus the excluded ones: what the harness evaluates. */
export function loadScoredRules(): RuleInfo[] {
  const excluded = loadExcludedRules();
  return loadRuleList().filter(rule => !excluded.has(rule.name));
}

function git(args: string[], cwd: string): string {
  return execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim(); // NOSONAR - dev-only harness resolves the developer's git from PATH
}

/** Sparse, shallow checkout of TSLint's rule fixtures at the pinned SHA into the gitignored `tmp/`. Idempotent. */
export function fetchFixtures(log: (line: string) => void = console.log): void {
  if (existsSync(join(FIXTURE_CHECKOUT, ".git"))) {
    let head = "";
    try {
      head = git(["rev-parse", "HEAD"], FIXTURE_CHECKOUT);
    } catch {
      head = "";
    }
    if (head === TSLINT_SHA && existsSync(FIXTURE_RULES)) {
      git(["sparse-checkout", "set", "--no-cone", ...SPARSE_PATHS], FIXTURE_CHECKOUT);
      log(`fixtures present at ${relative(REPOSITORY_ROOT, FIXTURE_CHECKOUT)} (${TSLINT_SHA})`);
      return;
    }
  } else {
    mkdirSync(FIXTURE_CHECKOUT, { recursive: true });
    git(["init", "-q"], FIXTURE_CHECKOUT);
    git(["remote", "add", "origin", TSLINT_REPOSITORY], FIXTURE_CHECKOUT);
  }
  log(`fetching ${TSLINT_REPOSITORY} @ ${TSLINT_SHA} (sparse: ${SPARSE_PATHS.join(" ")})`);
  git(["sparse-checkout", "set", "--no-cone", ...SPARSE_PATHS], FIXTURE_CHECKOUT);
  git(["fetch", "-q", "--depth", "1", "origin", TSLINT_SHA], FIXTURE_CHECKOUT);
  git(["checkout", "-q", "--force", "FETCH_HEAD"], FIXTURE_CHECKOUT);
  log(`fixtures ready at ${relative(REPOSITORY_ROOT, FIXTURE_CHECKOUT)}`);
}

export function ensureFixtures(log?: (line: string) => void): void {
  if (!existsSync(FIXTURE_RULES)) fetchFixtures(log);
}

/** One TSLint rule configuration as given by a variant's `tslint.json`. */
export interface RuleConfig {
  enabled: boolean;
  options: unknown[];
}

export interface FixtureFile {
  /** Path relative to the variant directory, e.g. `test.ts.lint` or `edge-case/test.ts.lint`. */
  lintPath: string;
  /** Snapshot path handed to the kernel: `<rule>/<variant>/<file without .lint>`. */
  path: string;
  language: Extract<PolicySourceLanguage, "typescript" | "tsx" | "javascript">;
  /** TSLint enables `jsRules` for JavaScript fixtures. */
  javascript: boolean;
  config: RuleConfig;
  parsed: ParsedLintFile | { skipped: true; requirement: string; parseError: string };
}

export interface Variant {
  /** Directory of the variant's `tslint.json` relative to the rule directory; `.` for the rule directory itself. */
  id: string;
  config: RuleConfig;
  /** Other rules the variant's tslint.json enables; their failures may also appear in the markup. */
  otherRules: string[];
  /** Variant has a tsconfig.json, so TSLint ran it with a type-checked program. */
  typeChecked: boolean;
  files: FixtureFile[];
}

function stripJsonComments(text: string): string {
  let result = "";
  let inString = false;
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index]!;
    const next = text[index + 1];
    if (inString) {
      result += char;
      if (char === "\\") result += text[++index] ?? "";
      else if (char === '"') inString = false;
    } else if (char === '"') {
      inString = true;
      result += char;
    } else if (char === "/" && next === "/") {
      while (index < text.length && text[index] !== "\n") index += 1;
      result += "\n";
    } else if (char === "/" && next === "*") {
      index += 2;
      while (index < text.length && !(text[index] === "*" && text[index + 1] === "/")) index += 1;
      index += 1;
    } else {
      result += char;
    }
  }
  return result;
}

function ruleConfig(value: unknown): RuleConfig {
  if (value === undefined || value === false || value === null) return { enabled: false, options: [] };
  if (value === true) return { enabled: true, options: [] };
  if (Array.isArray(value)) {
    const [enabled, ...options] = value as unknown[];
    return { enabled: enabled !== false, options };
  }
  if (typeof value === "object") {
    const { options, severity } = value as { options?: unknown; severity?: unknown };
    const list = options === undefined ? [] : Array.isArray(options) ? (options as unknown[]) : [options];
    return { enabled: severity !== "off" && severity !== "none", options: list };
  }
  return { enabled: true, options: [] };
}

function decodeFixture(buffer: Buffer, encodingRule: boolean): string {
  if (!encodingRule) return buffer.toString("utf8");
  // Mirrors TSLint's readBufferWithDetectedEncoding, including its 2-byte skip for a UTF-8 BOM.
  if (buffer.length >= 3 && buffer[0] === 0xef && buffer[1] === 0xbb && buffer[2] === 0xbf) return buffer.toString("utf8", 2);
  if (buffer.length >= 2 && buffer[0] === 0xff && buffer[1] === 0xfe) return buffer.toString("utf16le", 2);
  if (buffer.length >= 2 && buffer[0] === 0xfe && buffer[1] === 0xff) {
    const swapped = Buffer.from(buffer);
    swapped.swap16();
    return swapped.toString("utf16le", 2);
  }
  return buffer.toString("utf8");
}

function lintFilesUnder(directory: string): string[] {
  const found: string[] = [];
  const walk = (current: string): void => {
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      const path = join(current, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.name.endsWith(".lint")) found.push(path);
    }
  };
  walk(directory);
  return found.sort(compareDeterministicStrings);
}

function variantDirectories(ruleDirectory: string): string[] {
  const found: string[] = [];
  const walk = (current: string): void => {
    if (existsSync(join(current, "tslint.json"))) found.push(current);
    for (const entry of readdirSync(current, { withFileTypes: true })) {
      if (entry.isDirectory()) walk(join(current, entry.name));
    }
  };
  walk(ruleDirectory);
  return found.sort(compareDeterministicStrings);
}

function languageFor(fileName: string): { language: FixtureFile["language"]; javascript: boolean } {
  const javascript = /\.(?:js|jsx|mjs|cjs)$/.test(fileName);
  if (javascript) return { language: "javascript", javascript: true };
  return { language: fileName.endsWith(".tsx") ? "tsx" : "typescript", javascript: false };
}

/**
 * Every test variant of a rule, following TSLint's runner: a variant is any directory holding a `tslint.json`, and it
 * lints every `**\/*.lint` beneath it (so a nested variant's files are also linted by an enclosing variant).
 */
export function loadVariants(rule: string): Variant[] {
  const ruleDirectory = join(FIXTURE_RULES, rule);
  if (!existsSync(ruleDirectory) || !statSync(ruleDirectory).isDirectory()) return [];
  return variantDirectories(ruleDirectory).map(directory => {
    const tslintJson = JSON.parse(stripJsonComments(readFileSync(join(directory, "tslint.json"), "utf8"))) as {
      rules?: Record<string, unknown>;
      jsRules?: Record<string, unknown> | boolean;
    };
    const rules = tslintJson.rules ?? {};
    const jsRules = tslintJson.jsRules === true ? rules : tslintJson.jsRules === false || tslintJson.jsRules === undefined ? {} : tslintJson.jsRules;
    const id = relative(ruleDirectory, directory).split(sep).join("/") || ".";
    const files = lintFilesUnder(directory).map((lintFile): FixtureFile => {
      const lintPath = relative(directory, lintFile).split(sep).join("/");
      const compileName = lintPath.replace(/\.lint$/, "");
      const { language, javascript } = languageFor(basename(compileName));
      const text = decodeFixture(readFileSync(lintFile), rule === "encoding");
      let parsed: FixtureFile["parsed"];
      try {
        parsed = parseLintFile(text);
      } catch (error) {
        parsed = { skipped: true, requirement: "", parseError: error instanceof Error ? error.message : String(error) };
      }
      return {
        lintPath,
        path: [rule, id === "." ? "" : id, compileName].filter(part => part !== "").join("/"),
        language,
        javascript,
        config: ruleConfig(javascript ? jsRules[rule] : rules[rule]),
        parsed,
      };
    });
    return {
      id,
      config: ruleConfig(rules[rule]),
      otherRules: Object.keys(rules).filter(name => name !== rule && ruleConfig(rules[name]).enabled),
      typeChecked: existsSync(join(directory, "tsconfig.json")),
      files,
    };
  });
}
