// #14 Production code that reads `fixtures/`, and one fixtures-root resolver per language.
//
// A file resolves a fixtures root when it writes
//   - on a path-building line (one with `join(`, `resolve(`, `URL(`, `Path::new(`, `PathBuf::from(`, `||`,
//     `??` or a pathlib ` / `), a relative path literal whose first component is `fixtures`
//     (`"fixtures/ecma/…"`, `"./fixtures/"`, `"../../fixtures"`), resolved against the file or the working
//     directory; or
//   - the bare segment `"fixtures"` as a direct argument of a `join(`/`resolve(` call (`path.join`,
//     `os.path.join`, … included) whose earlier arguments are one root, which may itself be any balanced
//     expression (`repoRoot()`, `process.cwd()`), followed only by `"."`/`".."` segments; the call may
//     span lines (`join(root, "fixtures")`, `join(here, "..", "..", "fixtures")`); or
//   - the bare segment anywhere on a path-building line when the file also reads the
//     `RERAIL_FIXTURES_DIR` override (`root.join("fixtures")`).
// A bare `"fixtures"` deeper in a path (`join(root, "labels", "fixtures")`, an upstream tool's own
// `fixtures/`) is not the fixtures root. A literal that is only compared (`startsWith(`, `endsWith(`,
// `includes(`, `===`, `!==`) classifies a path rather than reading one, and name lists are not path
// building.
// Predicate:
//   - no tracked file under `src/` resolves the fixtures root (production code never reads fixtures);
//   - per language (Rust, TypeScript/JavaScript, Python, PHP, shell), at most one tracked file resolves it.
// Exemption `data-roots`: generators under `fixtures/` and `packs/<id>/upstream/` write their own
// recordings with paths relative to themselves.
// Exemption `structure-gate`: this gate (`scripts/check-structure*`) and its tests in `test/structure/`
// name layout paths to classify them; they never read fixture data through them.

import { extension, finding, isCode, isDataPath, isStructureGate, isVendored, lineAt, sortFindings } from "../lib.mjs";

export const id = 14;
export const title = "Production reads `fixtures/`; one root resolver per language";
export const target = "0; ≤1 per language";

const LANGUAGES = {
  ".rs": "Rust",
  ".ts": "TypeScript",
  ".tsx": "TypeScript",
  ".mts": "TypeScript",
  ".cts": "TypeScript",
  ".js": "TypeScript",
  ".mjs": "TypeScript",
  ".cjs": "TypeScript",
  ".py": "Python",
  ".php": "PHP",
  ".sh": "shell",
};

const LITERAL = /(["'`])((?:\.\.?\/)*fixtures(?:\/[^"'`\n]*)?)\1/g;
const PATH_BUILDING = /\bjoin\s*\(|\bresolve\s*\(|\bURL\s*\(|Path::new\s*\(|PathBuf::from\s*\(|\|\||\?\?|\s\/\s/;
const COMPARISON = /(?:\.(?:startsWith|endsWith|includes)\s*\(|[!=]==?)\s*$/;
const CALL = /\b(?:join|resolve)\s*\(/g;
const DOT_SEGMENT = /^\s*(["'`])\.\.?\1\s*$/;
const CALL_WINDOW = 2000;

/** The top-level arguments of `text` (the inside of a call up to some point), or null if a bracket it closes was not opened there. */
function argumentsOf(text) {
  const args = [""];
  let depth = 0;
  for (let i = 0; i < text.length; i += 1) {
    const c = text[i];
    if (c === '"' || c === "'" || c === "`") {
      let end = i + 1;
      while (end < text.length && text[end] !== c) end += text[end] === "\\" ? 2 : 1;
      args[args.length - 1] += text.slice(i, end + 1);
      i = end;
      continue;
    }
    if ("([{".includes(c)) depth += 1;
    else if (")]}".includes(c) && (depth -= 1) < 0) return null;
    if (c === "," && depth === 0) args.push("");
    else args[args.length - 1] += c;
  }
  return depth === 0 ? args : null;
}

/** Whether the bare `"fixtures"` literal at `index` is the first segment after a root in a join/resolve call. */
function isRootSegment(text, index) {
  const from = Math.max(0, index - CALL_WINDOW);
  const calls = [...text.slice(from, index).matchAll(CALL)].reverse();
  for (const call of calls) {
    const args = argumentsOf(text.slice(from + call.index + call[0].length, index));
    if (args === null) continue;
    return args.length >= 2 && args.at(-1).trim() === "" && args.slice(1, -1).every((arg) => DOT_SEGMENT.test(arg));
  }
  return false;
}

/** The first line at which a file's text resolves a fixtures root, or null. */
export function resolverLine(text) {
  const override = text.includes("RERAIL_FIXTURES_DIR");
  LITERAL.lastIndex = 0;
  for (let match = LITERAL.exec(text); match; match = LITERAL.exec(text)) {
    const start = text.lastIndexOf("\n", match.index) + 1;
    const end = text.indexOf("\n", match.index);
    const before = text.slice(start, match.index);
    if (COMPARISON.test(before)) continue;
    if (match[2] === "fixtures" && isRootSegment(text, match.index)) return lineAt(text, match.index);
    if (!PATH_BUILDING.test(text.slice(start, end < 0 ? text.length : end))) continue;
    if (match[2] !== "fixtures" || override) return lineAt(text, match.index);
  }
  return null;
}

export function check(repo) {
  const findings = [];
  const byLanguage = new Map();
  let production = 0;
  for (const path of repo.files) {
    if (!isCode(path) || isDataPath(path) || isVendored(path) || isStructureGate(path)) continue;
    const line = resolverLine(repo.text(path));
    if (line === null) continue;
    if (path.startsWith("src/")) {
      production += 1;
      findings.push(finding(path, "production code resolves fixtures/", line));
    }
    const language = LANGUAGES[extension(path)];
    if (!byLanguage.has(language)) byLanguage.set(language, []);
    byLanguage.get(language).push({ path, line });
  }
  const counts = [];
  for (const [language, resolvers] of [...byLanguage].sort(([a], [b]) => (a < b ? -1 : 1))) {
    counts.push(`${language} ${resolvers.length}`);
    if (resolvers.length <= 1) continue;
    for (const { path, line } of resolvers) {
      findings.push(finding(path, `one of ${resolvers.length} ${language} fixtures-root resolvers; keep one`, line));
    }
  }
  return { actual: `${production}; ${counts.join(", ") || "no resolvers"}`, findings: sortFindings(findings) };
}
