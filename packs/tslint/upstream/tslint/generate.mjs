// Records TSLint's own rule tests for the TSLint pack's ports (rerail/packs/tslint/conformance.json), in the format
// rerail/conformance/languages/pack.ts reads:
//   fixtures/<rule>/<variant>/<file>   each test file's source with TSLint's error markup removed
//   fixtures.expected.json             the failures each test's markup expects, in Rerail's coordinates
//   protected-identifiers.json         names the identifier-rename gate leaves alone, per rule and variant
//   ../../licenses/tslint/UPSTREAM.md  its rows: every copied file
// Only the tests a port is scored on are recorded: the variants a port's `fixtures` globs match.
// Run offline by a maintainer, never in CI:
//   node generate.mjs --tslint <checkout>
// <checkout> is github.com/palantir/tslint at 285fc1db18d1fd24680d6a2282c6445abf1566ee (6.1.3; the repository is
// archived), with at least test/rules/ and src/rules/ checked out:
//   git clone --filter=blob:none --sparse https://github.com/palantir/tslint <checkout>
//   git -C <checkout> sparse-checkout set test/rules src/rules && git -C <checkout> checkout 285fc1db18d1fd24680d6a2282c6445abf1566ee
//
// A TSLint rule test is a directory holding a tslint.json (a variant: the rule with its options) and `.lint` files:
// source code annotated with `~~~ [message]` marks under each failure TSLint must report. TSLint's test runner fails
// unless the rule reports exactly the marked failures, so the markup is TSLint's answer for that configuration. A
// variant lints every `.lint` file beneath its directory. This recorder mirrors TSLint's src/verify/{lines,parse}.ts and
// src/test.ts: the first-line `[typescript]: <range>` gate and `#if typescript` blocks are evaluated against TypeScript
// 4.0.2, TSLint's last pinned version, and a file the gate excludes, whose markup does not parse, or whose rule the
// variant's tslint.json does not enable for its file type (`jsRules` for JavaScript) is left out. Each kept file is one
// analysis unit at `<rule>/<variant>/<file without .lint>` (`<rule>/<file>` for a variant at the rule's root). Every
// failure is recorded under the variant's rule: no recorded variant enables a second rule. The `encoding` rule's files
// are decoded as TSLint reads them (UTF-16 byte order marks) and stored as UTF-8.
//
// Positions: TSLint's markup gives 0-based lines split at "\n" and UTF-16 columns. Each failure's start is converted
// to the runner's coordinates: the line ESLint's terminators end (CR, LF, CRLF, U+2028, U+2029), 1-based, and the
// column in code points from the line's start, 1-based. Messages are not recorded: the runner compares positions.
//
// protected-identifiers.json: for each rule, the identifier-like string literals in its implementation
// (src/rules/<camelCase>Rule.ts, outside `metadata`); for each variant, the words and regex patterns in its options.
// The rename gate (test/conformance/identifier-rename.test.ts) keeps those names, which a port may legitimately match.
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, dirname, join, matchesGlob, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const HERE = dirname(fileURLToPath(import.meta.url));
const PACK = JSON.parse(readFileSync(join(HERE, "..", "..", "conformance.json"), "utf8"));
const REPOSITORY = "https://github.com/palantir/tslint";
const COMMIT = "285fc1db18d1fd24680d6a2282c6445abf1566ee";
const VERSION = "6.1.3";
const TYPESCRIPT_VERSION = "4.0.2";

function parseArguments(argv) {
  const options = {};
  for (let index = 0; index < argv.length; index += 1) {
    if (argv[index] === "--tslint") options.tslint = resolve(argv[++index]);
    else throw new Error(`unknown argument ${argv[index]}`);
  }
  if (options.tslint === undefined) throw new Error("--tslint <checkout> is required");
  return options;
}

const options = parseArguments(process.argv.slice(2));
const head = execFileSync("git", ["-C", options.tslint, "rev-parse", "HEAD"], { encoding: "utf8" }).trim(); // NOSONAR - a maintainer's recorder runs the developer's git from PATH
if (head !== COMMIT) throw new Error(`${options.tslint} is at ${head}; this recorder reads TSLint at ${COMMIT}`);
if (PACK.tools.find(tool => tool.name === "tslint")?.version !== VERSION) throw new Error(`conformance.json does not pin tslint ${VERSION}`);
const PORTS = PACK.rules.filter(rule => rule.upstream.tool === "tslint");
const RULES = [...new Set(PORTS.map(rule => rule.upstream.rule))].sort();
/** A file is recorded when a port is scored on it: its rule's port without `fixtures`, or one whose globs match it. */
const scored = (rule, path) => PORTS.some(port => port.upstream.rule === rule && (port.fixtures ?? ["**"]).some(pattern => matchesGlob(path, pattern)));
const TESTS = join(options.tslint, "test", "rules");

// ---- .lint markup: src/verify/lines.ts and parse.ts --------------------------------------------------------------

const multilineErrorRegex = /^\s*(~+|~nil)$/;
const endErrorRegex = /^\s*(~+|~nil)\s*\[(.+)\]\s*$/;
const messageSubstitutionRegex = /^\[([-\w]+?)]: \s*(.+?)\s*$/;

function parseLine(text) {
  if (multilineErrorRegex.test(text)) return { kind: "multiline", startCol: text.indexOf("~") };
  const endError = endErrorRegex.exec(text);
  if (endError !== null) return { kind: "end", startCol: text.indexOf("~") };
  const substitution = messageSubstitutionRegex.exec(text);
  if (substitution !== null) return { kind: "substitution", key: substitution[1], message: substitution[2] };
  return { kind: "code", contents: text };
}

function parseVersion(text) {
  const match = /^v?(\d+)(?:\.(\d+|x|\*))?(?:\.(\d+|x|\*))?/.exec(text.trim());
  if (match === null) return undefined;
  const part = value => (value === undefined || value === "x" || value === "*" ? 0 : Number(value));
  return [part(match[1]), part(match[2]), part(match[3])];
}

/** The comparator forms TSLint's fixtures use: `>= 2.4.0 < 3.5.0`, `<=3.4.5`, `0.0.0`, `a || b`. */
function satisfiesRange(version, range) {
  const current = parseVersion(version);
  return range.split("||").some(alternative => {
    const comparators = alternative.trim().match(/(<=|>=|<|>|=)?\s*[^\s<>=]+/g) ?? [];
    return comparators.every(comparator => {
      const [, operator = "=", rest = ""] = /^(<=|>=|<|>|=)?\s*(.+)$/.exec(comparator) ?? [];
      const target = parseVersion(rest);
      if (target === undefined) return false;
      const order = current[0] - target[0] || current[1] - target[1] || current[2] - target[2];
      return { "<": order < 0, "<=": order <= 0, ">": order > 0, ">=": order >= 0 }[operator] ?? order === 0;
    });
  });
}

function preprocessDirectives(text) {
  if (!/^#(?:if|else|endif)\b/m.test(text)) return text;
  let state = "initial";
  let collecting = true;
  const result = [];
  for (const line of text.split("\n")) {
    if (line.startsWith("#if typescript")) {
      if (state !== "initial") throw new Error("#if directives cannot be nested");
      state = "if";
      collecting = satisfiesRange(TYPESCRIPT_VERSION, line.slice("#if typescript".length).trim());
    } else if (/^#else\s*$/.test(line)) {
      if (state !== "if") throw new Error("unexpected #else");
      state = "else";
      collecting = !collecting;
    } else if (/^#endif\s*$/.test(line)) {
      if (state === "initial") throw new Error("unexpected #endif");
      state = "initial";
      collecting = true;
    } else if (collecting) result.push(line);
  }
  if (state !== "initial") throw new Error("expected #endif");
  return result.join("\n");
}

/** Each failure's start: 0-based line among the code lines, 0-based UTF-16 column. */
function failureStarts(lines) {
  if (lines.length > 0 && lines[0].kind !== "code") throw new Error("text cannot start with an error mark line");
  const errorLines = [];
  for (const line of lines) {
    if (line.kind === "code") errorLines.push([]);
    else if (line.kind !== "substitution") errorLines.at(-1).push(line);
  }
  const starts = [];
  errorLines.forEach((marks, lineNo) => {
    while (marks.length > 0) {
      const mark = marks.shift();
      if (mark.kind === "end") {
        starts.push({ line: lineNo, column: mark.startCol });
        continue;
      }
      for (let next = lineNo + 1; ; next += 1) {
        const continuation = errorLines[next];
        if (continuation === undefined || continuation.length === 0 || continuation[0].startCol !== 0) {
          throw new Error(`error mark starting at ${lineNo}:${mark.startCol} does not end correctly`);
        }
        if (continuation.shift().kind === "end") {
          starts.push({ line: lineNo, column: mark.startCol });
          break;
        }
      }
    }
  });
  return starts;
}

/** A `.lint` file as TSLint's runTest reads it: version gate, `#if` directives, then markup; undefined when gated out. */
function parseLintFile(text) {
  let body = text;
  const first = parseLine(body.split(/\r?\n/)[0] ?? "");
  if (first.kind === "substitution" && first.key === "typescript") {
    if (!satisfiesRange(TYPESCRIPT_VERSION, first.message)) return undefined;
    const lineBreak = body.search(/\n/);
    body = lineBreak === -1 ? "" : body.slice(lineBreak + 1);
  }
  const lines = preprocessDirectives(body).split("\n").map(parseLine);
  const source = lines.flatMap(line => (line.kind === "code" ? [line.contents] : [])).join("\n");
  return { source, starts: failureStarts(lines) };
}

/** TSLint's readBufferWithDetectedEncoding, including its 2-byte skip for a UTF-8 byte order mark. */
function decodeEncodingFixture(buffer) {
  if (buffer.length >= 3 && buffer[0] === 0xef && buffer[1] === 0xbb && buffer[2] === 0xbf) return buffer.toString("utf8", 2);
  if (buffer.length >= 2 && buffer[0] === 0xff && buffer[1] === 0xfe) return buffer.toString("utf16le", 2);
  if (buffer.length >= 2 && buffer[0] === 0xfe && buffer[1] === 0xff) {
    const swapped = Buffer.from(buffer);
    swapped.swap16();
    return swapped.toString("utf16le", 2);
  }
  return buffer.toString("utf8");
}

// ---- variants: src/test.ts -------------------------------------------------------------------------------------

function stripJsonComments(text) {
  let result = "";
  let inString = false;
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
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
    } else result += char;
  }
  return result;
}

function ruleConfig(value) {
  if (value === undefined || value === false || value === null) return { enabled: false, options: [] };
  if (value === true) return { enabled: true, options: [] };
  if (Array.isArray(value)) {
    const [enabled, ...rest] = value;
    return { enabled: enabled !== false, options: rest };
  }
  if (typeof value === "object") {
    const list = value.options === undefined ? [] : Array.isArray(value.options) ? value.options : [value.options];
    return { enabled: value.severity !== "off" && value.severity !== "none", options: list };
  }
  return { enabled: true, options: [] };
}

const posix = path => path.split(sep).join("/");

function walk(directory, visit) {
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
    visit(join(directory, entry.name), entry);
    if (entry.isDirectory()) walk(join(directory, entry.name), visit);
  }
}

/** Every variant of `rule`: a directory holding a tslint.json, linting every `.lint` file beneath it. */
function variants(rule) {
  const ruleDirectory = join(TESTS, rule);
  if (!existsSync(ruleDirectory)) throw new Error(`no tests for ${rule} at ${ruleDirectory}`);
  const directories = existsSync(join(ruleDirectory, "tslint.json")) ? [ruleDirectory] : [];
  walk(ruleDirectory, (path, entry) => {
    if (entry.isDirectory() && existsSync(join(path, "tslint.json"))) directories.push(path);
  });
  return directories.map(directory => {
    const config = JSON.parse(stripJsonComments(readFileSync(join(directory, "tslint.json"), "utf8")));
    const rules = config.rules ?? {};
    const jsRules = config.jsRules === true ? rules : typeof config.jsRules === "object" ? config.jsRules : {};
    for (const [name, value] of Object.entries(rules)) {
      if (name !== rule && ruleConfig(value).enabled) throw new Error(`${posix(relative(TESTS, directory))} also enables ${name}`);
    }
    const lintFiles = [];
    walk(directory, (path, entry) => {
      if (entry.isFile() && entry.name.endsWith(".lint")) lintFiles.push(path);
    });
    return { id: posix(relative(ruleDirectory, directory)) || ".", directory, typescript: ruleConfig(rules[rule]), javascript: ruleConfig(jsRules[rule]), lintFiles };
  });
}

// ---- positions ---------------------------------------------------------------------------------------------------

/** The runner's point for a UTF-16 offset: ESLint's lines (CR, LF, CRLF, U+2028, U+2029), code-point columns. */
function pointAt(source, offset) {
  let line = 1;
  let lineStart = 0;
  for (let index = 0; index < offset; index += 1) {
    const char = source[index];
    if (char === "\r" && source[index + 1] === "\n") continue;
    if (char === "\n" || char === "\r" || char === "\u2028" || char === "\u2029") {
      line += 1;
      lineStart = index + 1;
    }
  }
  return { line, column: [...source.slice(lineStart, offset)].length + 1 };
}

function failureOffset(source, start) {
  let lineStart = 0;
  for (let line = 0; line < start.line; line += 1) {
    const end = source.indexOf("\n", lineStart);
    if (end === -1) throw new Error(`failure on line ${start.line}, past the end of the source`);
    lineStart = end + 1;
  }
  return lineStart + start.column;
}

// ---- protected identifiers -------------------------------------------------------------------------------------

const IDENTIFIER_WORD = /^[A-Za-z_$][\w$]*$/;

/** String literals in the rule's implementation, outside `metadata`, that are identifiers or quote one. */
function ruleSourceNames(rule) {
  const camel = rule.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());
  const path = join(options.tslint, "src", "rules", `${camel}Rule.ts`);
  const names = new Set();
  if (!existsSync(path)) return names;
  const visit = node => {
    if (ts.isPropertyDeclaration(node) && ts.isIdentifier(node.name) && node.name.text === "metadata") return;
    if (ts.isStringLiteralLike(node)) {
      if (IDENTIFIER_WORD.test(node.text)) names.add(node.text);
      for (const quoted of node.text.matchAll(/['"`]([^'"`]+)['"`]/g)) for (const word of quoted[1].match(/[A-Za-z_$][\w$]*/g) ?? []) names.add(word);
    }
    ts.forEachChild(node, visit);
  };
  visit(ts.createSourceFile(path, readFileSync(path, "utf8"), ts.ScriptTarget.ESNext, true));
  return names;
}

function optionStrings(values) {
  const strings = [];
  const visit = value => {
    if (typeof value === "string") strings.push(value);
    else if (Array.isArray(value)) value.forEach(visit);
    else if (typeof value === "object" && value !== null) {
      for (const [key, entry] of Object.entries(value)) {
        visit(key);
        visit(entry);
      }
    }
  };
  values.forEach(visit);
  return strings;
}

/** Option words (ban lists, whitelists) and option strings that compile as regexes (`^start`, `Fo*`). */
function optionProtection(values) {
  const strings = optionStrings(values);
  const patterns = strings.filter(value => {
    try {
      new RegExp(value);
      return true;
    } catch {
      return false;
    }
  });
  return { names: new Set(strings.flatMap(value => value.match(/[A-Za-z_$][\w$]*/g) ?? [])), patterns: new Set(patterns) };
}

// ---- record ------------------------------------------------------------------------------------------------------

const sorted = values => [...values].sort((a, b) => (a < b ? -1 : a > b ? 1 : 0));
const fixturesDirectory = join(HERE, "fixtures");
rmSync(fixturesDirectory, { recursive: true, force: true });
const units = new Map();
const findings = [];
const protectedIdentifiers = {};
const protect = (key, names, patterns) => {
  if (names.size === 0 && patterns.size === 0) return;
  protectedIdentifiers[key] = { names: sorted(names), patterns: sorted(patterns) };
};

for (const rule of RULES) {
  let ruleScored = false;
  for (const variant of variants(rule)) {
    const prefix = variant.id === "." ? rule : `${rule}/${variant.id}`;
    let variantScored = false;
    for (const lintFile of variant.lintFiles) {
      const name = posix(relative(variant.directory, lintFile)).replace(/\.lint$/, "");
      const path = `${prefix}/${name}`;
      const javascript = /\.(?:js|jsx|mjs|cjs)$/.test(basename(name));
      const config = javascript ? variant.javascript : variant.typescript;
      if (!config.enabled || !scored(rule, path)) continue;
      const buffer = readFileSync(lintFile);
      let parsed;
      try {
        parsed = parseLintFile(rule === "encoding" ? decodeEncodingFixture(buffer) : buffer.toString("utf8"));
      } catch {
        continue;
      }
      if (parsed === undefined) continue;
      ruleScored = variantScored = true;
      if (javascript && JSON.stringify(config.options) !== JSON.stringify(variant.typescript.options)) {
        const fileProtection = optionProtection(config.options);
        protect(path, fileProtection.names, fileProtection.patterns);
      }
      const points = parsed.starts.map(start => ({ rule, path, ...pointAt(parsed.source, failureOffset(parsed.source, start)) }));
      const previous = units.get(path);
      if (previous !== undefined) {
        // A nested variant's files are linted by the enclosing variant too; TSLint's markup is the same for both.
        if (previous.source !== parsed.source || JSON.stringify(previous.points) !== JSON.stringify(points)) throw new Error(`${path}: linted twice with different results`);
        continue;
      }
      units.set(path, { source: parsed.source, points });
      findings.push(...points);
      mkdirSync(dirname(join(fixturesDirectory, path)), { recursive: true });
      writeFileSync(join(fixturesDirectory, path), parsed.source);
    }
    if (variantScored) {
      const variantProtection = optionProtection(variant.typescript.options);
      protect(prefix, variantProtection.names, variantProtection.patterns);
    }
  }
  if (ruleScored) protect(rule, ruleSourceNames(rule), new Set());
}

findings.sort((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : a.line - b.line || a.column - b.column || (a.rule < b.rule ? -1 : a.rule > b.rule ? 1 : 0)));
const recording = {
  tool: "tslint",
  version: VERSION,
  source: { kind: "fixtures", origin: `${REPOSITORY}@${COMMIT}`, rules: RULES, files: sorted(units.keys()) },
  findings,
};
writeFileSync(join(HERE, "fixtures.expected.json"), `${JSON.stringify(recording, null, 2)}\n`);
writeFileSync(join(HERE, "protected-identifiers.json"), `${JSON.stringify(Object.fromEntries(Object.entries(protectedIdentifiers).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0))), null, 2)}\n`);

// The licence record lists every copied file (docs/pack-format.md, check 6): its rows after the table header are rewritten.
const upstreamRecord = join(HERE, "..", "..", "licenses", "tslint", "UPSTREAM.md");
const header = "|---|---|---|\n";
const record = readFileSync(upstreamRecord, "utf8");
if (!record.includes(header)) throw new Error(`${upstreamRecord} has no table`);
const rows = [
  "| `rules/` | `src/rules/*Rule.ts` (rule names, descriptions, default options) | adapted |",
  "| `candidates/` | `src/rules/*Rule.ts` (rule names, descriptions, options of `test/rules/**/tslint.json`) | adapted |",
  "| `upstream/tslint/protected-identifiers.json` | `src/rules/*Rule.ts` (string literals), `test/rules/**/tslint.json` (options) | excerpted |",
  ...sorted(units.keys()).map(path => `| \`upstream/tslint/fixtures/${path}\` | \`test/rules/${path}.lint\` | derived |`),
];
writeFileSync(upstreamRecord, `${record.slice(0, record.indexOf(header) + header.length)}${rows.join("\n")}\n`);
console.log(`tslint ${VERSION}: ${RULES.length} rules, ${units.size} files, ${findings.length} findings`);
