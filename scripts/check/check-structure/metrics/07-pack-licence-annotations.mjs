// #7 One location pattern for licences.
//
// Predicate, for every rule document (`rerail.policy/v1`) of a pack, resolved by REUSE's rules and
// compared as sets of `AND` terms against the pack's `packs/<id>/package.json` `license`:
//   - under `packs/<id>/rules/` (shipped): the resolved licence equals the pack's, since the package's
//     `license` describes exactly what it ships;
//   - under `packs/<id>/candidates/` (scored, never shipped): the resolved licence names only terms the
//     pack declares; Rerail's own probe rules legitimately resolve to Apache-2.0 alone.
// And for every file a `packs/<id>/licenses/<project>/UPSTREAM.md` table lists (copied from that upstream), the
// resolved licence names the one that UPSTREAM.md records (`License: <SPDX id>`; `MIT OR Apache-2.0` names MIT), and the resolved copyright
// names a holder besides Rerail: a copied file left to the catch-all annotation reads as Rerail's own work.
// Resolution follows REUSE 3.x for REUSE.toml: of the `[[annotations]]` whose `path` glob
// matches, the last one in the file applies; with `precedence = "override"` it wins outright, otherwise an
// in-file `SPDX-License-Identifier` (or a `<file>.license` sidecar) wins when present, and `aggregate`
// combines both; `SPDX-FileCopyrightText` resolves the same way. A pack without a `license` fails every one of
// its files.
// REUSE.toml is read with a TOML subset parser: `[[annotations]]` tables of string or string-array keys.

import { finding, globToRegExp, isRuleDoc, readJson, sortFindings } from "../lib.mjs";

export const id = 7;
export const title = "Pack rules and copied files carry their licences (REUSE)";

/** Rerail's copyright holder, the catch-all annotation's. */
const RERAIL_HOLDER = "Nera Software Inc.";
/** The licence an UPSTREAM.md records, and its table rows' files and upstream paths; as conformance/pack-validation.ts reads them. */
const UPSTREAM_LICENSE = /\bLicense: ([A-Za-z0-9][A-Za-z0-9.+-]*)/u;
const UPSTREAM_ROW = /^\|\s*`([^`]+)`\s*\|\s*([^|]*?)\s*\|/u;
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

/** The `[[annotations]]` tables of a REUSE.toml, in file order: `{ paths, precedence, license, copyright }`. */
export function parseReuseToml(text) {
  const annotations = [];
  let current = null;
  const lines = text.split("\n");
  for (let i = 0; i < lines.length; i += 1) {
    const line = stripComment(lines[i]).trim();
    if (line === "") continue;
    if (line.startsWith("[")) {
      current = line === "[[annotations]]" ? { paths: [], precedence: "closest", license: null, copyright: null } : null;
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
    else if (key === "SPDX-FileCopyrightText") current.copyright = Array.isArray(value) ? value : [value];
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

/** The SPDX ids an expression names, whatever joins them (`AND`, `OR`, `WITH`): `MIT OR Apache-2.0` names MIT. */
function licenceIds(expression) {
  return String(expression)
    .split(/\s+(?:AND|OR|WITH)\s+|[()]/)
    .map((part) => part.trim())
    .filter(Boolean);
}

// Assembled so that REUSE does not read this source's own patterns as licence and copyright tags.
const IN_FILE_TAG = new RegExp(`${["SPDX", "License", "Identifier"].join("-")}:\\s*([^\\n*]+?)\\s*(?:\\*\\/|-->)?\\s*$`, "m");
const IN_FILE_COPYRIGHT = new RegExp(`${["SPDX", "FileCopyrightText"].join("-")}:\\s*([^\\n*]+?)\\s*(?:\\*\\/|-->)?\\s*$`, "gm");

function inFileLicence(repo, path) {
  for (const source of [`${path}.license`, path]) {
    const text = repo.has(source) && !repo.isBinary(source) ? repo.text(source) : null;
    const match = text && IN_FILE_TAG.exec(text);
    if (match) return match[1];
  }
  return null;
}

function inFileCopyright(repo, path) {
  for (const source of [`${path}.license`, path]) {
    const text = repo.has(source) && !repo.isBinary(source) ? repo.text(source) : null;
    const holders = text ? [...text.matchAll(IN_FILE_COPYRIGHT)].map((match) => match[1]) : [];
    if (holders.length > 0) return holders;
  }
  return null;
}

function lastAnnotation(annotations, path) {
  let last = null;
  for (const annotation of annotations) {
    if (annotation.patterns.some((pattern) => pattern.test(path))) last = annotation;
  }
  return last;
}

/** The licence expression REUSE resolves for `path`, or null when nothing covers it. */
export function effectiveLicence(repo, annotations, path) {
  const last = lastAnnotation(annotations, path);
  const inFile = inFileLicence(repo, path);
  if (!last || last.license === null) return inFile;
  if (last.precedence === "override") return last.license;
  if (last.precedence === "aggregate" && inFile) return `${last.license} AND ${inFile}`;
  return inFile ?? last.license;
}

/** The copyright holders REUSE resolves for `path`, as written; empty when nothing covers it. */
export function effectiveCopyright(repo, annotations, path) {
  const last = lastAnnotation(annotations, path);
  const inFile = inFileCopyright(repo, path);
  if (!last || last.copyright === null) return inFile ?? [];
  if (last.precedence === "override") return last.copyright;
  if (last.precedence === "aggregate" && inFile) return [...last.copyright, ...inFile];
  return inFile ?? last.copyright;
}

/**
 * Each file a `packs/<id>/licenses/<project>/UPSTREAM.md` lists as copied, repository-relative, with the licence it
 * records. A row whose upstream path is `none` is a Rerail file listed among the copies and says so; it is skipped.
 */
function copiedFiles(repo) {
  const copied = [];
  for (const record of repo.files) {
    const match = /^packs\/([^/]+)\/licenses\/[^/]+\/UPSTREAM\.md$/.exec(record);
    if (!match) continue;
    const text = repo.text(record);
    const licence = UPSTREAM_LICENSE.exec(text)?.[1] ?? null;
    for (const line of text.split("\n")) {
      const row = UPSTREAM_ROW.exec(line);
      if (row && row[2] !== "none") copied.push({ path: `packs/${match[1]}/${row[1]}`, record, licence });
    }
  }
  return copied;
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
  for (const { path, record, licence } of copiedFiles(repo)) {
    // A listed file that does not exist is pack validation's finding (check 6), not this metric's.
    if (!repo.has(path)) continue;
    total += 1;
    if (licence === null) {
      findings.push(finding(path, `${record} records no "License: <SPDX id>"`));
      continue;
    }
    const resolved = effectiveLicence(repo, annotations, path);
    const holders = effectiveCopyright(repo, annotations, path);
    if (resolved === null || !licenceIds(resolved).includes(licence)) {
      findings.push(finding(path, `REUSE resolves ${resolved ?? "no licence"}; ${record} lists it as copied under ${licence}`));
    } else if (!holders.some((holder) => !holder.includes(RERAIL_HOLDER))) {
      const named = holders.length === 0 ? "no copyright holder" : `only ${RERAIL_HOLDER} as its copyright holder`;
      findings.push(finding(path, `REUSE names ${named}; ${record} lists it as copied`));
    }
  }
  const covered = total - findings.length;
  const percent = total === 0 ? 100 : Math.floor((covered * 100) / total);
  return { actual: `${percent}% (${covered}/${total})`, findings: sortFindings(findings) };
}
