// #7 One location pattern for licences.
//
// Predicate, for every rule document (`rerail.policy/v1`) of a pack, resolved by REUSE's rules and
// compared as sets of `AND` terms against the pack's `packs/<id>/package.json` `license`:
//   - under `packs/<id>/rules/` (shipped): the resolved licence equals the pack's, since the package's
//     `license` describes exactly what it ships;
//   - under `packs/<id>/candidates/` (scored, never shipped): the resolved licence names only terms the
//     pack declares; Rerail's own probe rules legitimately resolve to Apache-2.0 alone.
// Resolution follows REUSE 3.x for REUSE.toml: of the `[[annotations]]` whose `path` glob
// matches, the last one in the file applies; with `precedence = "override"` it wins outright, otherwise an
// in-file `SPDX-License-Identifier` (or a `<file>.license` sidecar) wins when present, and `aggregate`
// combines both. A pack without a `license` fails every one of its files.
// REUSE.toml is read with a TOML subset parser: `[[annotations]]` tables of string or string-array keys.

import { finding, globToRegExp, isRuleDoc, readJson, sortFindings } from "../lib.mjs";

export const id = 7;
export const title = "Pack rules carry the pack's licence (REUSE)";
export const target = "100%";

function parseValue(raw) {
  const text = raw.trim();
  if (text.startsWith("[")) {
    const items = [];
    const pattern = /"((?:[^"\\]|\\.)*)"|'([^']*)'/g;
    for (let match = pattern.exec(text); match; match = pattern.exec(text)) {
      items.push(match[1] !== undefined ? JSON.parse(`"${match[1]}"`) : match[2]);
    }
    return items;
  }
  if (text.startsWith('"')) return JSON.parse(text.slice(0, text.lastIndexOf('"') + 1));
  if (text.startsWith("'")) return text.slice(1, text.indexOf("'", 1));
  return text;
}

function stripComment(line) {
  let quote = null;
  for (let i = 0; i < line.length; i += 1) {
    const char = line[i];
    if (quote) {
      if (char === "\\" && quote === '"') i += 1;
      else if (char === quote) quote = null;
    } else if (char === '"' || char === "'") quote = char;
    else if (char === "#") return line.slice(0, i);
  }
  return line;
}

/** The `[[annotations]]` tables of a REUSE.toml, in file order: `{ paths, precedence, license }`. */
export function parseReuseToml(text) {
  const annotations = [];
  let current = null;
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i += 1) {
    const line = stripComment(lines[i]).trim();
    if (line === "") continue;
    if (line.startsWith("[")) {
      current = line === "[[annotations]]" ? { paths: [], precedence: "closest", license: null } : null;
      if (current) annotations.push(current);
      continue;
    }
    const eq = line.indexOf("=");
    if (eq < 0 || !current) continue;
    const key = line.slice(0, eq).trim().replace(/^"|"$/g, "");
    let raw = line.slice(eq + 1);
    if (raw.trim().startsWith("[")) {
      while (!stripComment(raw).trimEnd().endsWith("]") && i + 1 < lines.length) {
        i += 1;
        raw += `\n${stripComment(lines[i])}`;
      }
    }
    const value = parseValue(stripComment(raw));
    if (key === "path") current.paths = Array.isArray(value) ? value : [value];
    else if (key === "precedence") current.precedence = value;
    else if (key === "SPDX-License-Identifier") current.license = Array.isArray(value) ? value.join(" AND ") : value;
  }
  return annotations.map((annotation) => ({ ...annotation, patterns: annotation.paths.map(globToRegExp) }));
}

export function licenceTerms(expression) {
  return [
    ...new Set(
      String(expression)
        .replace(/[()]/g, " ")
        .split(/\s+AND\s+/)
        .map((term) => term.trim())
        .filter(Boolean),
    ),
  ].sort();
}

// Assembled so that REUSE does not read this source's own pattern as a licence tag.
const IN_FILE_TAG = new RegExp(`${["SPDX", "License", "Identifier"].join("-")}:\\s*([^\\n*]+?)\\s*(?:\\*\\/|-->)?\\s*$`, "m");

function inFileLicence(repo, path) {
  for (const source of [`${path}.license`, path]) {
    const text = repo.has(source) && !repo.isBinary(source) ? repo.text(source) : null;
    const match = text && IN_FILE_TAG.exec(text);
    if (match) return match[1];
  }
  return null;
}

/** The licence expression REUSE resolves for `path`, or null when nothing covers it. */
export function effectiveLicence(repo, annotations, path) {
  let last = null;
  for (const annotation of annotations) {
    if (annotation.patterns.some((pattern) => pattern.test(path))) last = annotation;
  }
  const inFile = inFileLicence(repo, path);
  if (!last || last.license === null) return inFile;
  if (last.precedence === "override") return last.license;
  if (last.precedence === "aggregate" && inFile) return `${last.license} AND ${inFile}`;
  return inFile ?? last.license;
}

export function check(repo) {
  const annotations = repo.has("REUSE.toml") ? parseReuseToml(repo.text("REUSE.toml")) : [];
  const findings = [];
  let total = 0;
  for (const path of repo.files) {
    const match = /^packs\/([^/]+)\/(rules|candidates)\//.exec(path);
    if (!match || !isRuleDoc(repo, path)) continue;
    total += 1;
    const declared = readJson(repo, `packs/${match[1]}/package.json`)?.license;
    if (typeof declared !== "string") {
      findings.push(finding(path, `packs/${match[1]}/package.json declares no license`));
      continue;
    }
    const resolved = effectiveLicence(repo, annotations, path);
    const terms = resolved === null ? [] : licenceTerms(resolved);
    const packTerms = licenceTerms(declared);
    const ok =
      match[2] === "rules"
        ? terms.join(" AND ") === packTerms.join(" AND ")
        : terms.length > 0 && terms.every((term) => packTerms.includes(term));
    if (!ok) {
      const relation = match[2] === "rules" ? "the pack declares" : "outside the pack's";
      findings.push(finding(path, `REUSE resolves ${resolved ?? "no licence"}; ${relation} ${declared}`));
    }
  }
  const covered = total - findings.length;
  const percent = total === 0 ? 100 : Math.floor((covered * 100) / total);
  return { actual: `${percent}% (${covered}/${total})`, findings: sortFindings(findings) };
}
