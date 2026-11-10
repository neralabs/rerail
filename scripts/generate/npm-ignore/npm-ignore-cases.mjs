// Writes native/src/code_rule_dsl/npm_ignore/cases.json: the oracle npm_ignore/mod.rs's tests check the port of
// eslint-plugin-n's getNpmignore (and the ignore and globrex packages under it) against. Each case holds one answer per
// path of `paths`, in order, as a string of `1` (true), `0` (false) and `-` (throws). `ignore` holds
// `[patterns, answers]`: node-ignore's `ignore().add(patterns).ignores(path)` for every pattern set below;
// `publication` holds `[package, answers]`: eslint-plugin-n's `getNpmignore(...).match(path)` for a package folder
// holding `package.files` and `package.main` in its package.json and `package.npmignore`/`package.gitignore` as its
// .npmignore/.gitignore, run with the package folder as the working directory (where upstream's never-ignored names
// apply). Run offline, not in CI:
//   node scripts/generate/npm-ignore/npm-ignore-cases.mjs <tools>/node_modules
// where <tools> is the Node pack's pinned toolchain (eslint-plugin-n 18.4.1, ignore 5.3.2, globrex 0.1.2). A different
// version is refused; bump PINS, move the port in npm_ignore/mod.rs and regenerate together.
import { mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

const PINS = { ignore: "5.3.2", globrex: "0.1.2", "eslint-plugin-n": "18.4.1" };
if (process.argv[2] === undefined) {
  throw new Error("usage: node scripts/generate/npm-ignore/npm-ignore-cases.mjs <path to node_modules>");
}
const modules = resolve(process.argv[2]);
const require = createRequire(join(modules, "noop.js"));
for (const [name, pin] of Object.entries(PINS)) {
  const { version } = require(join(modules, name, "package.json"));
  if (version !== pin) throw new Error(`${name} ${version} found; the cases are pinned to ${name} ${pin}`);
}
const ignore = require(join(modules, "ignore"));
const { getNpmignore } = await import(pathToFileURL(join(modules, "eslint-plugin-n/lib/util/get-npmignore.js")).href);

const PATHS = [
  "a.js", "index.js", "a.min.js", "x.JS", "b.ts", ".hidden", ".hidden/a.js", "..foo.js", "a b.js", "a#b.js", "file[1].js",
  "file1.js", "file2.js", "file4.js", "!important.js", "#a.js", "*.js", "a\\b", "a.js ", "lib", "lib/a.js", "lib/b.js",
  "lib/x.js", "lib/a.test.js", "lib/sub/a.js", "lib/sub/index.js", "lib/test/a.js", "Lib/A.js", "LIB/a.js", "libx/a.js",
  "src/lib/a.js", "bin/cli.js", "bin/cli", "dist/a.js", "dist/a.js.map", "dist/sub/b.js.map", "test/a.js",
  "test/lib/a.js", "foo/bar.js", "foo/bar/baz.js", "foo/sub/x.js", "x/sub/y.js", "a/b", "a/x/b", "a/x/y/b", "aXb", "ab",
  "keep/a.js", "src/keep/a.js", "node_modules/x/a.js", "README.md", "readme", "Readme.txt", "docs/README.md",
  "LICENSE", "license.txt", "LICENCE.md", "CHANGELOG.md", "changes", "HISTORY", "history.md.txt", "package.json",
  "lib/package.json", "a.log", "logs/a.log", "1.js", "01.js", "02.js", "a.js.1.js", "c.js", "x.1.js", "x.3.js",
  "[.js", "a].js", "&.js", "~.js", "lib/&.js", "bin/[.js",
];

const PATTERNS = [
  "*.js", "/*.js", "lib", "lib/", "/lib", "/lib/", "lib/**", "/lib/**", "**/lib", "**/lib/**", "lib/**/a.js", "a/**/b",
  "*", "/*", "**", "**/*", "foo/*", "foo/**/*.js", "*.min.*", "?.js", "a?.js", "[abc].js", "[a-z]*.js", "[z-a].js",
  "[!a].js", "file[0-9].js", "file[1].js", "\\!important.js", "\\#a.js", "#a.js", "a.js\\ ", "a.js  ", "\\*.js",
  "a**b", "**.js", "lib/*.js", "lib/sub", "sub", "*/sub/*", "test", ".hidden", ".*", "*.JS", "node_modules/", "a b.js",
  "a\\\\b", "[]", "[", "]", "{a,b}.js", "a+b.js", "(x).js", "$x", "x|y", "^x", "a.b", "foo/**/", "**/", "/", "!", "foo//bar.js",
  "*/", "lib/sub/", "dist/**/*.map", "*.map", "\\", "a\\", "bin/cli", "lib/a.js/", "**/keep/*.js", "**/keep", "README*",
  "\uFEFFlib", "  ", "lib # not a comment", "lib/[ab].js", "lib/[a\\]].js", "[[]a].js", "[[].js", "[a&&b].js", "[&~].js",
  "[[:alnum:]].js",
];

const PATTERN_SETS = [
  ...PATTERNS.map(pattern => [pattern]),
  ["lib", "!lib/a.js"],
  ["lib/*", "!lib/a.js"],
  ["lib/", "!lib/sub/"],
  ["lib/**", "!lib/sub/**"],
  ["*.js", "!index.js"],
  ["*.js", "!index.js", "index.js"],
  ["/*", "!/lib", "!/package.json"],
  ["**/*.js", "!**/keep/*.js"],
  ["*", "!*/", "!*.js"],
  ["!lib/a.js", "lib"],
  ["test\r", "*.log"],
  ["# comment", "", "dist", "!dist/a.js"],
];

const answer = value => (value === null ? "-" : value ? "1" : "0");
const cases = { paths: PATHS, ignore: [], publication: [] };
for (const set of PATTERN_SETS) {
  const patterns = set.join("\n");
  let matcher;
  try {
    matcher = ignore().add(patterns);
  } catch {
    matcher = undefined;
  }
  const answers = PATHS.map(path => {
    if (matcher === undefined) return null;
    try {
      return matcher.ignores(path);
    } catch {
      return null;
    }
  });
  cases.ignore.push([patterns, answers.map(answer).join("")]);
}

const FILES = [
  ["lib"], ["lib/"], ["/lib"], ["./lib"], ["lib/*.js"], ["lib/**/*.js"], ["*.js"], ["index.js"], ["lib", "!lib/test"],
  ["lib", "!*.test.js"], ["lib/{a,b}.js"], ["lib/*.{js,ts}"], ["@(lib|bin)"], ["+(a|b).js"], ["!(test)/**"],
  ["file{1..3}.js"], ["lib", "!{test,sub}"], ["dist", "!dist/**/*.map"], [], ["bin/"], ["../x"], ["lib/../bin"], ["LIB"],
  ["lib//a.js"], ["", 1, null, "bin"], ["?(x)lib"], ["lib/[ab].js"], ["x.{1..2}.js"], ["{01..02}.js"], ["{a..c}.js"],
  ["file{1..5..2}.js"], ["!lib"], ["**"], ["lib", "!lib/x.js", "lib/x.js"], ["*(lib)"], ["lib/[[:alnum:]].js"],
  ["!(lib)"], ["lib", "!lib/{a,b}.js"], ["!*.test.js", "lib"], ["{z..x}.js"], ["lib/[[a].js"], ["{lib,bin}/[[&~].js"],
  ["{lib,bin}/[a&&b].js"],
];
const PACKAGES = [
  ...FILES.map(files => ({ files })),
  { files: "lib" },
  { files: ["lib"], main: "index.js" },
  { files: ["lib"], main: "./a.js" },
  { files: ["lib"], main: "/b.ts" },
  { files: ["lib"], main: "bin/../c.js" },
  { files: ["lib"], main: "lib/" },
  { files: [], main: "a.js" },
  { files: ["lib"], npmignore: "lib/test\n" },
  { files: ["lib"], gitignore: "lib/x.js\n" },
  { npmignore: "test\n*.log\n" },
  { npmignore: "*\n!lib/\n" },
  { npmignore: "/*\n!/lib\n" },
  { gitignore: "dist\n" },
  { npmignore: "", gitignore: "lib" },
  {},
  { main: "a.js" },
  { npmignore: "*\n", main: "./lib/../index.js" },
  { npmignore: "*\n" },
  { gitignore: "*.js\n!bin/*\n", main: "lib/a.js" },
];

for (const pkg of PACKAGES) {
  // Its real path, so the working directory (`process.cwd()`, resolved) is the folder upstream's package.json path names.
  const folder = realpathSync(mkdtempSync(join(tmpdir(), "npm-ignore-cases-")));
  try {
    const manifest = { name: "fixture", ...(pkg.files === undefined ? {} : { files: pkg.files }), ...(pkg.main === undefined ? {} : { main: pkg.main }) };
    writeFileSync(join(folder, "package.json"), JSON.stringify(manifest));
    if (pkg.npmignore !== undefined) writeFileSync(join(folder, ".npmignore"), pkg.npmignore);
    if (pkg.gitignore !== undefined) writeFileSync(join(folder, ".gitignore"), pkg.gitignore);
    const cwd = process.cwd();
    process.chdir(folder);
    try {
      const matcher = getNpmignore(join(folder, "index.js"));
      cases.publication.push([pkg, PATHS.map(path => answer(matcher.match(path))).join("")]);
    } finally {
      process.chdir(cwd);
    }
  } finally {
    rmSync(folder, { recursive: true, force: true });
  }
}

const out = resolve(import.meta.dirname, "../../../native/src/code_rule_dsl/npm_ignore/cases.json");
const line = value => JSON.stringify(value);
writeFileSync(
  out,
  `{\n  "paths": ${line(cases.paths)},\n  "ignore": [\n${cases.ignore.map(c => `    ${line(c)}`).join(",\n")}\n  ],\n` +
    `  "publication": [\n${cases.publication.map(c => `    ${line(c)}`).join(",\n")}\n  ]\n}\n`,
);
process.stdout.write(`${out}: ${cases.ignore.length} pattern sets, ${cases.publication.length} packages, ${PATHS.length} paths each\n`);
