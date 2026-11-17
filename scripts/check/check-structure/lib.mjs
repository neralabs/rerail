// Shared predicates for the structure gate's metric modules (scripts/check/check-structure/metrics/).
// Everything here is a pure function of a repo (see repo.mjs): no clock, no network, no git beyond the
// file list.

import { posix } from "node:path";
import { compare } from "./repo.mjs";

export { compare };

export const CODE_EXTENSIONS = [".ts", ".tsx", ".mts", ".cts", ".js", ".mjs", ".cjs", ".rs", ".py", ".php", ".sh"];
export const SCRIPT_EXTENSIONS = [".ts", ".tsx", ".mts", ".cts", ".js", ".mjs", ".cjs"];
export const BINARY_EXTENSIONS = [
  ".bin", ".safetensors", ".onnx", ".pt", ".wasm", ".node", ".so", ".dylib", ".dll", ".exe",
  ".png", ".jpg", ".jpeg", ".gif", ".webp", ".ico", ".pdf", ".zip", ".gz", ".tgz", ".zst", ".7z", ".sqlite", ".db",
];

export function extension(path) {
  const base = basename(path);
  const dot = base.lastIndexOf(".");
  return dot <= 0 ? "" : base.slice(dot);
}

export function basename(path) {
  return path.slice(path.lastIndexOf("/") + 1);
}

export function dirname(path) {
  const slash = path.lastIndexOf("/");
  return slash < 0 ? "" : path.slice(0, slash);
}

export function isCode(path) {
  return CODE_EXTENSIONS.includes(extension(path));
}

export function isScript(path) {
  return SCRIPT_EXTENSIONS.includes(extension(path)) && !path.endsWith(".d.ts");
}

/**
 * A binary file: its first bytes hold a NUL (git's own test), or its extension is a known binary format
 * (BINARY_EXTENSIONS). The extension decides in a checkout without LFS content, where the file is a small
 * text pointer.
 */
export function isBinaryFile(repo, path) {
  return BINARY_EXTENSIONS.includes(extension(path)) || repo.isBinary(path);
}

/**
 * Data roots hold recorded or copied material, not the repository's own code: `fixtures/**` and
 * `packs/<id>/upstream/**` (the fixtures contract, fixtures/README.md).
 */
export function isDataPath(path) {
  return path.startsWith("fixtures/") || /^packs\/[^/]+\/upstream\//.test(path);
}

/** Third-party code vendored byte-identical (REUSE.toml): never restyled or re-laid-out. */
export function isVendored(path) {
  return path.startsWith("native/grammars/") || path.split("/").includes("node_modules");
}

/** A test file by name: `*.test.*` / `*.spec.*` scripts, pytest `test_*.py` / `*_test.py`, PHPUnit `*Test.php`. */
export function isTestFile(path) {
  const base = basename(path);
  return (
    /\.(test|spec)\.[cm]?[jt]sx?$/.test(base) ||
    /^test_.+\.py$/.test(base) ||
    /.+_test\.py$/.test(base) ||
    /.+Test\.php$/.test(base)
  );
}

/** This gate's own sources and tests, which spell out the paths and markers they look for. */
export function isStructureGate(path) {
  return path === "scripts/check/check-structure.mjs" || path.startsWith("scripts/check/check-structure/") || path.startsWith("test/structure/");
}

export function lineAt(text, index) {
  let line = 1;
  for (let i = 0; i < index && i < text.length; i += 1) if (text.charCodeAt(i) === 10) line += 1;
  return line;
}

export function finding(path, message, line) {
  return line === undefined ? { path, message } : { path, line, message };
}

export function sortFindings(findings) {
  return [...findings].sort(
    (a, b) => compare(a.path, b.path) || (a.line ?? 0) - (b.line ?? 0) || compare(a.message, b.message),
  );
}

const jsonCache = new WeakMap();

/** The parsed JSON at `path`, or undefined when it is missing or does not parse. */
export function readJson(repo, path) {
  let cache = jsonCache.get(repo);
  if (!cache) jsonCache.set(repo, (cache = new Map()));
  if (!cache.has(path)) {
    const text = repo.text(path);
    let value;
    if (text !== null) {
      try {
        value = JSON.parse(text);
      } catch {
        value = undefined;
      }
    }
    cache.set(path, value);
  }
  return cache.get(path);
}

export const POLICY_FORMAT = "rerail.policy/v1";
export const RULE_PACK_FORMAT = "rerail.rule-pack/v1";

/**
 * A conversation rule document: a JSON object with no `format` holding a conversation rule's required fields (`id`,
 * `mode`, `match`, `sample`, `action`; src/engine/catalog/rule-schema.ts). A pack may ship one in `rules/`.
 */
function isConversationRuleShape(value) {
  return (
    isObject(value) &&
    !("format" in value) &&
    typeof value.id === "string" &&
    typeof value.mode === "string" &&
    isObject(value.match) &&
    isObject(value.sample) &&
    isObject(value.action)
  );
}

/** A rule document: a JSON object whose `format` is rerail.policy/v1, or a conversation rule. */
export function isRuleDoc(repo, path) {
  if (!path.endsWith(".json")) return false;
  const text = repo.text(path);
  if (text === null || (!text.includes(POLICY_FORMAT) && !text.includes('"sample"'))) return false;
  const value = readJson(repo, path);
  return (isObject(value) && value.format === POLICY_FORMAT) || isConversationRuleShape(value);
}

export function isObject(value) {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** Pack ids: the directories directly under packs/ that hold tracked files. */
export function packIds(repo) {
  const ids = new Set();
  for (const path of repo.files) {
    const match = /^packs\/([^/]+)\//.exec(path);
    if (match) ids.add(match[1]);
  }
  return [...ids].sort(compare);
}

/** Top-level directories holding tracked files. */
export function topLevelDirectories(repo) {
  const dirs = new Set();
  for (const path of repo.files) {
    const slash = path.indexOf("/");
    if (slash > 0) dirs.add(path.slice(0, slash));
  }
  return [...dirs].sort(compare);
}

/**
 * A glob as REUSE.toml and .gitattributes write it, as a RegExp over repo-relative paths: `*` and `?`
 * stay within a path segment, `**` crosses segments, `\` escapes.
 */
export function globToRegExp(glob) {
  let source = "";
  for (let i = 0; i < glob.length; i += 1) {
    const char = glob[i];
    if (char === "\\" && i + 1 < glob.length) {
      i += 1;
      source += escapeRegExp(glob[i]);
    } else if (char === "*") {
      if (glob[i + 1] === "*") {
        i += 1;
        if (glob[i + 1] === "/") {
          i += 1;
          source += "(?:.*/)?";
        } else {
          source += ".*";
        }
      } else {
        source += "[^/]*";
      }
    } else if (char === "?") {
      source += "[^/]";
    } else {
      source += escapeRegExp(char);
    }
  }
  return new RegExp(`^${source}$`);
}

function escapeRegExp(text) {
  return text.replace(/[.*+?^${}()|[\]\\/]/g, "\\$&");
}

const IMPORT_PATTERNS = [
  /\b(?:import|export)\s+(?:type\s+)?(?:[\w*{}\s,$]+?\s+from\s+)?["']([^"'\n]+)["']/g,
  /\bimport\s*\(\s*["']([^"'\n]+)["']\s*\)/g,
  /\brequire\s*\(\s*["']([^"'\n]+)["']\s*\)/g,
];

/** The module specifiers a JS/TS file imports, with their 1-based lines, sorted. */
export function extractImports(text) {
  const imports = [];
  const seen = new Set();
  for (const pattern of IMPORT_PATTERNS) {
    pattern.lastIndex = 0;
    for (let match = pattern.exec(text); match; match = pattern.exec(text)) {
      const index = match.index + match[0].lastIndexOf(match[1]);
      const key = `${index}:${match[1]}`;
      if (seen.has(key)) continue;
      seen.add(key);
      imports.push({ specifier: match[1], line: lineAt(text, index) });
    }
  }
  return imports.sort((a, b) => a.line - b.line || compare(a.specifier, b.specifier));
}

/** A relative specifier resolved to a repo-relative path (extension as written), or null for packages. */
export function resolveRelative(fromFile, specifier) {
  if (!specifier.startsWith("./") && !specifier.startsWith("../")) return null;
  const resolved = posix.normalize(posix.join(dirname(fromFile), specifier));
  return resolved.startsWith("../") ? null : resolved;
}

/**
 * The tracked file a relative import lands on: the written path, its `.ts`/`.tsx` source for a `.js`
 * specifier (NodeNext), or a directory's `index`. Falls back to the written path.
 */
export function resolveImport(repo, fromFile, specifier) {
  const resolved = resolveRelative(fromFile, specifier);
  if (resolved === null) return null;
  const stem = resolved.replace(/\.(m|c)?js$/, "");
  const candidates = [
    resolved,
    `${stem}.ts`,
    `${stem}.tsx`,
    `${stem}.mts`,
    `${stem}.cts`,
    `${resolved}.ts`,
    `${resolved}/index.ts`,
    `${resolved}/index.js`,
    `${resolved}/index.mjs`,
  ];
  return candidates.find((path) => repo.has(path)) ?? resolved;
}

const COMMAND_ENDS = new Set(["\n", ";", "&", "|"]);
const REDIRECTS = new Set([">", "<"]);
/** The characters a backslash escapes inside double quotes. Before any other character the backslash stays. */
const DOUBLE_QUOTE_ESCAPES = new Set(["$", "`", '"', "\\", "\n"]);

/** A character outside any quote: what it does to the tokens and commands so far, and the index of the last character it takes. */
function scanShellChar(script, i, parser) {
  const char = script[i];
  if (char === "\\") return escapeEnd(script, i, parser);
  if (char === "#" && !parser.inToken) return commentEnd(script, i);
  if (COMMAND_ENDS.has(char)) {
    parser.endCommand();
    return commandEnd(script, i);
  }
  if (REDIRECTS.has(char)) {
    parser.startRedirect();
    return redirectOperatorEnd(script, i);
  }
  parser.wordChar(char);
  return i;
}

/**
 * The last character of the backslash escape at `i`, outside quotes. A backslash and the newline after it are both
 * removed (a line continuation); before any other character the backslash is removed and that character is taken
 * literally.
 */
function escapeEnd(script, i, parser) {
  const next = script[i + 1];
  if (next !== undefined && next !== "\n") parser.literalChar(next);
  return i + 1;
}

/** The last character of the separator at `i`: `&&` and `||` are one. */
function commandEnd(script, i) {
  const char = script[i];
  return (char === "&" || char === "|") && script[i + 1] === char ? i + 1 : i;
}

/** The last character of the comment that starts at `i`: the newline after it ends the command. */
function commentEnd(script, i) {
  let end = i;
  while (end + 1 < script.length && script[end + 1] !== "\n") end += 1;
  return end;
}

/** The last character of the redirection operator that starts at `i` (`>`, `>>`, `>&`, `>|`, `<`, `<<`, `<<<`, `<&`, `<>`). */
function redirectOperatorEnd(script, i) {
  let end = i;
  while (/[<>&|]/.test(script[end + 1] ?? "")) end += 1;
  return end;
}

/** A character inside a `quote`: returns the index of the last character it takes. */
function scanQuotedChar(script, i, parser) {
  const char = script[i];
  if (char === parser.quote) parser.quote = null;
  else if (char === "\\" && parser.quote === '"' && DOUBLE_QUOTE_ESCAPES.has(script[i + 1])) {
    if (script[i + 1] !== "\n") parser.token += script[i + 1];
    return i + 1;
  } else parser.token += char;
  return i;
}

/** The tokens and commands of a shell script as they are read. */
function shellParser() {
  const parser = {
    commands: [],
    tokens: [],
    token: "",
    quote: null,
    inToken: false,
    /** Whether a quote or a backslash made part of the current token. */
    quoted: false,
    /** Whether the next token is a redirection's target, which is not part of the command. */
    redirectTarget: false,
    endToken() {
      if (parser.inToken) {
        if (!parser.redirectTarget) parser.tokens.push(parser.token);
        parser.redirectTarget = false;
      }
      parser.token = "";
      parser.inToken = false;
      parser.quoted = false;
    },
    endCommand() {
      parser.endToken();
      parser.redirectTarget = false;
      if (parser.tokens.length > 0) parser.commands.push(parser.tokens);
      parser.tokens = [];
    },
    startRedirect() {
      // An unquoted number right before the operator is the file descriptor it redirects (`2>&1`), not a word.
      if (parser.inToken && !parser.quoted && /^\d+$/.test(parser.token)) parser.inToken = false;
      parser.endToken();
      parser.redirectTarget = true;
    },
    literalChar(char) {
      parser.token += char;
      parser.inToken = true;
      parser.quoted = true;
    },
    wordChar(char) {
      if (char === '"' || char === "'") {
        parser.quote = char;
        parser.quoted = true;
      } else if (char === " " || char === "\t") {
        parser.endToken();
        return;
      } else parser.token += char;
      parser.inToken = true;
    },
  };
  return parser;
}

/** Leading environment assignments and wrappers do not change what runs. */
function withoutAssignments(command) {
  let start = 0;
  while (start < command.length && /^[A-Za-z_][A-Za-z0-9_]*=/.test(command[start])) start += 1;
  return command.slice(start);
}

/**
 * Shell command lines split into simple commands (on `&&`, `||`, `;`, `|`, `&`, newlines), tokenised as a shell reads
 * them, redirections left out.
 */
export function shellCommands(script) {
  const parser = shellParser();
  for (let i = 0; i < script.length; i += 1) i = parser.quote ? scanQuotedChar(script, i, parser) : scanShellChar(script, i, parser);
  parser.endCommand();
  return parser.commands.map(withoutAssignments);
}

/**
 * The `run:` scripts of a GitHub Actions workflow, in file order (block scalars joined), with the line of
 * their `run:` key. `${{ … }}` expressions become the opaque word `GHA_EXPR`: only the runner knows them.
 */
export function workflowRunScripts(yaml) {
  const lines = yaml.replace(/\$\{\{[^}]*\}\}/g, "GHA_EXPR").split("\n");
  const scripts = [];
  for (let i = 0; i < lines.length; i += 1) {
    const match = /^(\s*)(?:-\s+)?run:\s*(.*)$/.exec(lines[i]);
    if (!match) continue;
    const line = i + 1;
    const value = match[2].trim();
    if (!/^[|>][-+]?$/.test(value)) {
      scripts.push({ line, script: value.replace(/^["']|["']$/g, "") });
      continue;
    }
    const body = [];
    let indent = null;
    for (let j = i + 1; j < lines.length; j += 1) {
      const text = lines[j];
      if (text.trim() === "") {
        body.push("");
        continue;
      }
      const textIndent = text.length - text.trimStart().length;
      if (textIndent <= match[1].length) break;
      indent ??= textIndent;
      body.push(text.slice(indent));
      i = j;
    }
    scripts.push({ line, script: body.join("\n") });
  }
  return scripts;
}
