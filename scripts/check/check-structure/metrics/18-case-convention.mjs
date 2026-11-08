// #18 One case convention per language.
//
// Predicate, over tracked files outside the exemptions below:
//   - TypeScript/JavaScript (.ts .tsx .mts .cts .js .mjs .cjs): the basename's first dot-separated part and
//     every directory in the path are kebab-case (`[a-z0-9]+(-[a-z0-9]+)*`); later dot parts (`.test`,
//     `.generated`, `.d`, `.config`) are lowercase. Directories starting with `.` (tool config such as
//     `.github/`) are not checked.
//   - Rust (.rs): the basename and every directory below the crate's `src/` or `tests/` are snake_case
//     (`[a-z0-9]+(_[a-z0-9]+)*`), the module-path convention.
//   - Python (.py): the basename is snake_case (PEP 8 module names).
//   For Rust and Python too, the `.generated` infix of the generated-file convention (#20) is not part of
//   the stem: `builtins.generated.rs` is checked as `builtins`.
// Exemption `data-roots`: `fixtures/**` and `packs/<id>/upstream/**` name files after upstream tools and
// engine modules (fixtures/README.md: generators are `generate_<tool>.<ext>` beside their expected findings).
// Exemption `vendored`: `native/grammars/**` is third-party code kept byte-identical.
// Exemption `fixture-trees`: files below any `fixtures/` directory mimic the projects under test (framework
// route files such as `pages/_app.tsx` or `app/(shop)/[id]/`); #3 decides where they may live.

import { extension, finding, isDataPath, isVendored, sortFindings } from "../lib.mjs";

export const id = 18;
export const title = "One case convention per language";
export const target = "100%";

const KEBAB = /^[a-z0-9]+(-[a-z0-9]+)*$/;
const SNAKE = /^[a-z0-9]+(_[a-z0-9]+)*$/;
const SUFFIX = /^[a-z0-9]+(-[a-z0-9]+)*$/;
const SCRIPT = [".ts", ".tsx", ".mts", ".cts", ".js", ".mjs", ".cjs"];

function scriptProblem(path) {
  const segments = path.split("/");
  const base = segments.pop();
  const parts = base.split(".");
  parts.pop();
  if (!KEBAB.test(parts[0] ?? "") || !parts.slice(1).every((part) => SUFFIX.test(part))) {
    return `${base} is not kebab-case`;
  }
  const bad = segments.find((segment) => !segment.startsWith(".") && !KEBAB.test(segment));
  return bad === undefined ? null : `directory ${bad}/ is not kebab-case`;
}

/** The stem of a Rust or Python file name, without its extension and the `.generated` infix. */
function moduleStem(base) {
  return base.slice(0, base.lastIndexOf(".")).replace(/\.generated$/, "");
}

function rustProblem(path) {
  const segments = path.split("/");
  const base = segments.pop();
  if (!SNAKE.test(moduleStem(base))) return `${base} is not snake_case`;
  const start = segments.findIndex((segment) => segment === "src" || segment === "tests");
  const bad = start < 0 ? undefined : segments.slice(start + 1).find((segment) => !SNAKE.test(segment));
  return bad === undefined ? null : `module directory ${bad}/ is not snake_case`;
}

function pythonProblem(path) {
  const base = path.slice(path.lastIndexOf("/") + 1);
  return SNAKE.test(moduleStem(base)) || base === "__init__.py" || base === "__main__.py" ? null : `${base} is not snake_case`;
}

export function check(repo) {
  const findings = [];
  let checked = 0;
  for (const path of repo.files) {
    if (isDataPath(path) || isVendored(path) || path.split("/").slice(0, -1).includes("fixtures")) continue;
    const ext = extension(path);
    let problem;
    if (SCRIPT.includes(ext)) problem = scriptProblem(path);
    else if (ext === ".rs") problem = rustProblem(path);
    else if (ext === ".py") problem = pythonProblem(path);
    else continue;
    checked += 1;
    if (problem !== null) findings.push(finding(path, problem));
  }
  const passing = checked - findings.length;
  const percent = checked === 0 ? 100 : Math.floor((passing * 100) / checked);
  return { actual: `${percent}% (${passing}/${checked})`, findings: sortFindings(findings) };
}
