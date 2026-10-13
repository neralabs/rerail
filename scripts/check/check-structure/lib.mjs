// Shared predicates for the structure gate's metric modules (scripts/check/check-structure/metrics/).
// Everything here is a pure function of a repo (see repo.mjs): no clock, no network, no git beyond the
// file list.

import { posix } from "node:path";
import { compare } from "./repo.mjs";

export { compare };

export const CODE_EXTENSIONS = [".ts", ".tsx", ".mts", ".cts", ".js", ".mjs", ".cjs", ".rs", ".py", ".php", ".sh"];
export const SCRIPT_EXTENSIONS = [".ts", ".tsx", ".mts", ".cts", ".js", ".mjs", ".cjs"];

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
 * A rule document, as the catalog's `parseRuleDocument` reads one: a JSON object whose `format` is rerail.policy/v1 (a
 * code rule), or one with no `format` that has a string `id` and `match`, `sample` and `action` objects (a
 * conversation rule).
 */
export function isRuleDoc(repo, path) {
  if (!path.endsWith(".json")) return false;
  const text = repo.text(path);
  if (text === null || !(text.includes(POLICY_FORMAT) || text.includes('"sample"'))) return false;
  const value = readJson(repo, path);
  if (!isObject(value)) return false;
  if (value.format === POLICY_FORMAT) return true;
  return !("format" in value) && typeof value.id === "string" && isObject(value.match) && isObject(value.sample) && isObject(value.action);
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

/** Shell command lines split into simple commands (on `&&`, `||`, `;`, `|`, `&`, newlines), tokenised. */
export function shellCommands(script) {
  const commands = [];
  let tokens = [];
  let token = "";
  let quote = null;
  let inToken = false;
  const endToken = () => {
    if (inToken) tokens.push(token);
    token = "";
    inToken = false;
  };
  const endCommand = () => {
    endToken();
    if (tokens.length > 0) commands.push(tokens);
    tokens = [];
  };
  for (let i = 0; i < script.length; i += 1) {
    const char = script[i];
    if (quote) {
      if (char === quote) quote = null;
      else if (char === "\\" && quote === '"' && i + 1 < script.length) token += script[(i += 1)];
      else token += char;
      continue;
    }
    if (char === '"' || char === "'") {
      quote = char;
      inToken = true;
    } else if (char === "\\" && script[i + 1] === "\n") {
      i += 1;
    } else if (char === "#" && !inToken) {
      while (i + 1 < script.length && script[i + 1] !== "\n") i += 1;
    } else if (char === "\n" || char === ";" || char === "&" || char === "|") {
      if ((char === "&" || char === "|") && script[i + 1] === char) i += 1;
      endCommand();
    } else if (char === " " || char === "\t") {
      endToken();
    } else if (char === ">" || char === "<") {
      endToken();
      // Skip the redirect target.
      while (script[i + 1] === ">" || script[i + 1] === "&" || script[i + 1] === " ") i += 1;
      while (i + 1 < script.length && !/[\s;&|]/.test(script[i + 1])) i += 1;
    } else {
      token += char;
      inToken = true;
    }
  }
  endCommand();
  return commands.map((command) => {
    // Leading environment assignments and wrappers do not change what runs.
    let start = 0;
    while (start < command.length && /^[A-Za-z_][A-Za-z0-9_]*=/.test(command[start])) start += 1;
    return command.slice(start);
  });
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
