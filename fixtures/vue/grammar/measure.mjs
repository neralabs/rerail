// T1 candidate-grammar measurement for the Vue pack. Run offline by a maintainer, never in CI:
//
//   node rerail-fixtures/vue/grammar/measure.mjs [--tools <dir>] [--corpora <dir>] [--out <report.json>] [--date D]
//
// Pins (refused when different): @vue/compiler-sfc 3.5.43 and @vue/compiler-dom 3.5.43 (the reference: descriptor
// blocks and the template AST, `parseMode: "sfc"`), vue-eslint-parser 10.4.1 with @typescript-eslint/parser 8.70.1 and
// typescript 6.0.3 (the ESLint-compatible template AST eslint-plugin-vue reads), the corpora in ../corpora.json and the
// grammars in grammars.json, each checked out by commit under `<tools>/grammars/<checkout>`. Each grammar is compiled
// to a shared library and parsed through `dump/` with the tree-sitter 0.27.0 runtime Rerail's engine links.
//
// For every `.vue` file of every corpus the generator records whether each reference accepts it, then compares each
// grammar's tree with each reference by the normalized items of `sfc.mjs` (blocks, elements, attributes, directives,
// directive expressions, interpolations). It reports, per grammar and corpus: the error-file rate on files
// @vue/compiler-sfc accepts, item agreement with both references (on every accepted file and on the files the grammar
// parses without ERROR), parse throughput, and every error file classified by the construct at its first error.
// Per-file detail goes to `<checkout>/.rerail/reports/vue-t1/`. The report goes to `--out`, by default
// `<checkout>/.rerail/reports/vue-t1-grammar-<date>.json` (reports are not committed).

import { execFileSync, spawn } from "node:child_process";
import { createHash } from "node:crypto";
import { createWriteStream, existsSync, mkdirSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, matchesGlob, relative, resolve } from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

import { corporaDirectory, load, toolList, toolsDirectory } from "../tools.mjs";
import { agreement, CATEGORIES, compilerItems, eslintItems, firstError, grammarItems } from "./sfc.mjs";
import { classify, mismatchClasses } from "./classify.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOLS = toolsDirectory(HERE);
const CORPORA = corporaDirectory(HERE);
const CHECKOUT = resolve(TOOLS, "../../..");
const flag = name => {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
};
const DATE = flag("--date") ?? new Date().toISOString().slice(0, 10);
const REPORTS = join(CHECKOUT, ".rerail/reports");
const OUT = resolve(flag("--out") ?? join(REPORTS, `vue-t1-grammar-${DATE}.json`));
const DETAIL = join(REPORTS, "vue-t1");
const ONLY = flag("--corpus");

const corpora = JSON.parse(readFileSync(join(HERE, "../corpora.json"), "utf8"));
const grammars = JSON.parse(readFileSync(join(HERE, "grammars.json"), "utf8"));
const sfc = await load(TOOLS, "@vue/compiler-sfc");
await load(TOOLS, "@vue/compiler-dom");
const eslintParser = await load(TOOLS, "vue-eslint-parser");
const tsParser = await load(TOOLS, "@typescript-eslint/parser");
await load(TOOLS, "typescript");

const git = (dir, ...args) => execFileSync("git", ["-C", dir, ...args], { encoding: "utf8" }).trim();

// --- build -------------------------------------------------------------------------------------------------------

function buildGrammar(grammar) {
  const dir = join(TOOLS, "grammars", grammar.checkout);
  const head = git(dir, "rev-parse", "HEAD");
  if (head !== grammar.commit) throw new Error(`${grammar.name} is at ${head}; grammars.json pins ${grammar.commit}`);
  const lib = join(TOOLS, "lib", `${grammar.checkout}.dylib`);
  mkdirSync(dirname(lib), { recursive: true });
  const objects = join(TOOLS, "lib", `${grammar.checkout}.o`);
  mkdirSync(objects, { recursive: true });
  const src = join(dir, "src");
  execFileSync("clang", ["-O2", "-fPIC", "-c", "-I", src, join(src, "parser.c"), "-o", join(objects, "parser.o")]);
  const cxx = existsSync(join(src, "scanner.cc"));
  execFileSync(cxx ? "clang++" : "clang", ["-O2", "-fPIC", ...(cxx ? ["-std=c++14"] : []), "-c", "-I", src,
    join(src, cxx ? "scanner.cc" : "scanner.c"), "-o", join(objects, "scanner.o")]);
  execFileSync(cxx ? "clang++" : "clang", ["-shared", "-o", lib, join(objects, "parser.o"), join(objects, "scanner.o")]);
  return lib;
}

function buildDumper() {
  const target = join(TOOLS, "target");
  execFileSync("cargo", ["build", "--release", "--offline", "--quiet"], {
    cwd: join(HERE, "dump"), env: { ...process.env, CARGO_TARGET_DIR: target }, stdio: "inherit",
  });
  return join(target, "release/rerail-vue-grammar-dump");
}

// --- corpus ------------------------------------------------------------------------------------------------------

function enumerate(corpus) {
  const root = join(CORPORA, corpus.name);
  const head = git(root, "rev-parse", "HEAD");
  if (head !== corpus.commit) throw new Error(`${corpus.name} is at ${head}; corpora.json pins ${corpus.commit}`);
  const files = [];
  const excluded = [];
  const walk = dir => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name);
      const rel = relative(root, path);
      if (entry.isDirectory()) {
        if (entry.name === ".git") continue;
        if (corpus.exclude.some(glob => matchesGlob(`${rel}/x`, glob))) continue;
        walk(path);
      } else if (entry.isFile() && entry.name.endsWith(".vue")) {
        if (corpus.exclude.some(glob => matchesGlob(rel, glob))) excluded.push(rel);
        else files.push(rel);
      }
    }
  };
  for (const path of corpus.paths) {
    if (!existsSync(join(root, path))) throw new Error(`${corpus.name}: pinned path ${path} does not exist`);
    walk(join(root, path));
  }
  files.sort();
  return { root, files, excluded };
}

function fastest(runs, fn) {
  let best = Infinity;
  let value;
  for (let k = 0; k < runs; k++) {
    const started = process.hrtime.bigint();
    value = fn();
    best = Math.min(best, Number(process.hrtime.bigint() - started));
  }
  return { ns: best, value };
}

function reference(source, path) {
  const compiler = fastest(3, () => {
    sfc.parseCache.clear();
    return sfc.parse(source, { filename: path });
  });
  const { descriptor, errors } = compiler.value;
  const compilerView = compilerItems(source, descriptor);
  let eslint;
  const scriptLang = descriptor.scriptSetup?.lang ?? descriptor.script?.lang;
  const eslintRun = fastest(1, () => {
    try {
      const result = eslintParser.parseForESLint(source, {
        parser: tsParser, sourceType: "module", ecmaVersion: "latest", filePath: path,
        ecmaFeatures: { jsx: scriptLang === "tsx" || scriptLang === "jsx" },
      });
      eslint = { ok: true, templateErrors: (result.ast.templateBody?.errors ?? []).map(error => error.message),
        ...eslintItems(source, result.ast.templateBody) };
    } catch (error) {
      eslint = { ok: false, error: String(error.message).split("\n")[0], items: [], template: false };
    }
  });
  return {
    accepted: errors.length === 0,
    errors: errors.map(error => String(error.message ?? error).split("\n")[0]).slice(0, 3),
    compiler: compilerView,
    templateStart: descriptor.template?.loc.start.offset,
    templateLang: descriptor.template?.lang ?? null,
    eslint,
    compilerNs: compiler.ns,
    eslintNs: eslintRun.ns,
  };
}

function addAgreement(total, { result }) {
  for (const category of CATEGORIES) {
    for (const key of ["reference", "candidate", "matched"]) total[category][key] += result[category][key];
  }
}

const emptyAgreement = () => Object.fromEntries(CATEGORIES.map(c => [c, { reference: 0, candidate: 0, matched: 0 }]));

function summarize(total) {
  const out = {};
  for (const [category, { reference, candidate, matched }] of Object.entries(total)) {
    out[category] = { reference, candidate, matched,
      recall: reference ? round(matched / reference) : null, precision: candidate ? round(matched / candidate) : null };
  }
  return out;
}

const round = x => Math.round(x * 10000) / 10000;

async function runGrammar(dumper, lib, root, files, references, detailPath) {
  const child = spawn(dumper, [lib, "--repeat", "3"], { stdio: ["pipe", "pipe", "inherit"] });
  child.stdin.end(files.map(file => join(root, file)).join("\n") + "\n");
  const detail = createWriteStream(detailPath);
  const totals = {
    files: 0, bytes: 0, parseNs: 0, errorFiles: 0, acceptedFiles: 0, acceptedErrorFiles: 0,
    rejectedFiles: 0, rejectedErrorFiles: 0,
    vsCompiler: emptyAgreement(), vsCompilerClean: emptyAgreement(), vsEslint: emptyAgreement(),
    vsEslintClean: emptyAgreement(), exactFiles: 0, exactCleanFiles: 0, causes: {}, errorFileList: [],
    mismatches: {},
  };
  let index = 0;
  for await (const line of createInterface({ input: child.stdout })) {
    const record = JSON.parse(line);
    const file = files[index];
    const ref = references[index++];
    const source = readFileSync(join(root, file), "utf8");
    totals.files++;
    totals.bytes += record.bytes;
    totals.parseNs += record.parseNs;
    if (record.error) totals.errorFiles++;
    if (!ref.accepted) {
      totals.rejectedFiles++;
      if (record.error) totals.rejectedErrorFiles++;
      detail.write(JSON.stringify({ file, accepted: false, grammarError: record.error, errors: ref.errors }) + "\n");
      continue;
    }
    totals.acceptedFiles++;
    const grammar = grammarItems(source, record.nodes, ref.templateStart);
    // A template the grammar does not find loses all its items; a template the reference does not read as HTML
    // (pug) is compared by its block only.
    const vsCompiler = agreement(ref.compiler.items, ref.compiler.template ? grammar.items
      : grammar.items.filter(item => item.startsWith("block|")));
    addAgreement(totals.vsCompiler, vsCompiler);
    const grammarTemplate = grammar.items.filter(item => !item.startsWith("block|"));
    const vsEslint = ref.eslint.ok && ref.eslint.template ? agreement(ref.eslint.items, grammarTemplate) : undefined;
    if (vsEslint) addAgreement(totals.vsEslint, vsEslint);
    const exact = vsCompiler.missed.length === 0 && vsCompiler.unmatched.length === 0;
    if (exact) totals.exactFiles++;
    let error;
    if (record.error) {
      totals.acceptedErrorFiles++;
      error = firstError(source, record.nodes);
      const cause = classify(source, error, ref, record.nodes);
      totals.causes[cause] = (totals.causes[cause] ?? 0) + 1;
      const { line: l, column } = position(source, error.start);
      totals.errorFileList.push({ path: file, line: l, column, cause });
    } else {
      addAgreement(totals.vsCompilerClean, vsCompiler);
      if (vsEslint) addAgreement(totals.vsEslintClean, vsEslint);
      if (exact) totals.exactCleanFiles++;
      for (const [cls, item, side] of mismatchClasses(source, vsCompiler.missed, vsCompiler.unmatched)) {
        totals.mismatches[cls] ??= { items: 0, files: new Set(), examples: [] };
        totals.mismatches[cls].items++;
        totals.mismatches[cls].files.add(file);
        if (totals.mismatches[cls].examples.length < 4) totals.mismatches[cls].examples.push({ path: file, side, item });
      }
    }
    detail.write(JSON.stringify({
      file, accepted: true, grammarError: record.error, error,
      missed: vsCompiler.missed, unmatched: vsCompiler.unmatched,
      eslintMissed: vsEslint?.missed, eslintUnmatched: vsEslint?.unmatched,
    }) + "\n");
  }
  detail.end();
  const code = await new Promise(resolveExit => child.on("close", resolveExit));
  if (code !== 0) throw new Error(`dumper exited ${code}`);
  if (index !== files.length) throw new Error(`dumper printed ${index} of ${files.length} files`);
  return totals;
}

function position(source, offset) {
  let line = 1;
  let lineStart = 0;
  for (let i = 0; i < offset; i++) {
    if (source.charCodeAt(i) === 10) {
      line++;
      lineStart = i + 1;
    }
  }
  return { line, column: [...source.slice(lineStart, offset)].length + 1 };
}

// --- main --------------------------------------------------------------------------------------------------------

mkdirSync(DETAIL, { recursive: true });
const dumper = buildDumper();
const libs = Object.fromEntries(grammars.map(grammar => [grammar.name, buildGrammar(grammar)]));
const report = {
  format: "rerail.language-conformance/v1",
  pack: "vue",
  tier: "t1-grammar",
  date: DATE,
  kernel: { path: "none: candidate grammars, before a Vue dialect exists", version: "tree-sitter 0.27.0 runtime (=0.27.0, as Rerail's native/Cargo.toml)" },
  corpora: corpora.filter(c => !ONLY || c.name === ONLY).map(({ name, repository, commit }) => ({ name, repository, commit })),
  tools: [
    ...toolList("@vue/compiler-sfc", "@vue/compiler-dom", "vue-eslint-parser", "@typescript-eslint/parser", "typescript"),
    { name: "tree-sitter (runtime)", version: "0.27.0" },
    ...grammars.map(grammar => ({ name: grammar.name, version: grammar.commit })),
  ],
  benchmark: "candidate tree-sitter-vue grammars vs @vue/compiler-sfc and vue-eslint-parser (rerail-fixtures/vue/grammar/measure.mjs)",
  method: "Per file: @vue/compiler-sfc parse (descriptor + template AST) and vue-eslint-parser parseForESLint (TS parser); "
    + "per grammar: tree-sitter 0.27.0 parse, fastest of 3. Items (sfc.mjs): blocks by type and content range; template "
    + "elements by tag, start, end of end tag (or start tag) and parent; static attributes by name and value range; "
    + "directives by Vue name, argument, static-ness, modifiers and range; directive expressions and interpolations by "
    + "range. Offsets in UTF-16. Agreement is a multiset match per category; 'clean' restricts to files the grammar "
    + "parses without ERROR. Template items are compared only for HTML templates. Each syntax-error file is classified "
    + "by the construct at its first innermost ERROR or MISSING node (classify.mjs; 'unclassified' fails), and each item "
    + "a grammar and @vue/compiler-sfc disagree on in a clean file by the construct it misreads (cleanMismatchClasses).",
  grammars: grammars.map(({ checkout, ...rest }) => rest),
  results: {},
};

for (const corpus of corpora) {
  if (ONLY && corpus.name !== ONLY) continue;
  const { root, files, excluded } = enumerate(corpus);
  const hashes = [];
  let sourceBytes = 0;
  const references = [];
  let compilerNs = 0;
  let eslintNs = 0;
  const eslintFailures = [];
  const eslintTemplateErrors = {};
  const rejected = [];
  const nonHtmlTemplates = {};
  const crossReference = emptyAgreement();
  for (const file of files) {
    const bytes = readFileSync(join(root, file));
    sourceBytes += bytes.length;
    hashes.push(`${file}\0${createHash("sha256").update(bytes).digest("hex")}`);
    const ref = reference(bytes.toString("utf8"), file);
    compilerNs += ref.compilerNs;
    eslintNs += ref.eslintNs;
    if (!ref.eslint.ok) eslintFailures.push({ path: file, error: ref.eslint.error });
    else for (const message of new Set(ref.eslint.templateErrors)) {
      eslintTemplateErrors[message] = (eslintTemplateErrors[message] ?? 0) + 1;
    }
    if (!ref.accepted) rejected.push({ path: file, errors: ref.errors });
    if (ref.templateLang && ref.templateLang !== "html") {
      nonHtmlTemplates[ref.templateLang] = (nonHtmlTemplates[ref.templateLang] ?? 0) + 1;
    }
    if (ref.accepted && ref.eslint.ok && ref.eslint.template && ref.compiler.template) {
      addAgreement(crossReference, agreement(ref.compiler.items.filter(item => !item.startsWith("block|")),
        ref.eslint.items));
    }
    references.push(ref);
  }
  const manifestSha256 = createHash("sha256").update(hashes.join("\n")).digest("hex");
  const mib = sourceBytes / 1048576;
  const perCorpus = {
    coverage: { enumeratedFiles: files.length + excluded.length, eligibleFiles: files.length, excluded: excluded.length,
      sourceBytes, manifestSha256 },
    reference: {
      compilerSfc: { acceptedFiles: files.length - rejected.length, rejectedFiles: rejected.length, rejected,
        nonHtmlTemplates, parseMs: round(compilerNs / 1e6), mibPerSecond: round(mib / (compilerNs / 1e9)) },
      vueEslintParser: { failures: eslintFailures, filesWithTemplateErrorsByCode: eslintTemplateErrors,
        parseMs: round(eslintNs / 1e6), mibPerSecond: round(mib / (eslintNs / 1e9)),
        note: "parseForESLint parses the scripts with @typescript-eslint/parser too; its time is not template-only. "
          + "Template errors are HTML parse errors it recovers from (files per code); it still builds the element." },
      compilerVsEslint: summarize(crossReference),
    },
    grammars: {},
  };
  for (const grammar of grammars) {
    const detail = join(DETAIL, `${corpus.name}--${grammar.checkout}.jsonl`);
    const t = await runGrammar(dumper, libs[grammar.name], root, files, references, detail);
    perCorpus.grammars[grammar.name] = {
      filesWithSyntaxErrors: t.errorFiles,
      acceptedFiles: t.acceptedFiles,
      acceptedErrorFiles: t.acceptedErrorFiles,
      errorFileRate: round(t.acceptedErrorFiles / t.acceptedFiles),
      rejectedErrorFiles: `${t.rejectedErrorFiles} of ${t.rejectedFiles}`,
      exactFiles: t.exactFiles,
      exactCleanFiles: t.exactCleanFiles,
      vsCompilerSfc: summarize(t.vsCompiler),
      vsCompilerSfcClean: summarize(t.vsCompilerClean),
      vsVueEslintParser: summarize(t.vsEslint),
      vsVueEslintParserClean: summarize(t.vsEslintClean),
      cleanMismatchClasses: Object.fromEntries(Object.entries(t.mismatches).sort((a, b) => b[1].items - a[1].items)
        .map(([cls, { items, files, examples }]) => [cls, { items, files: files.size, examples }])),
      syntaxErrorCauses: Object.fromEntries(Object.entries(t.causes).sort((a, b) => b[1] - a[1])),
      syntaxErrorFiles: t.errorFileList,
      parseMs: round(t.parseNs / 1e6),
      mibPerSecond: round(mib / (t.parseNs / 1e9)),
      detail: relative(CHECKOUT, detail),
    };
    const g = perCorpus.grammars[grammar.name];
    console.log(`${corpus.name} ${grammar.name}: error files ${t.acceptedErrorFiles}/${t.acceptedFiles}, exact ${t.exactFiles}, `
      + `${g.mibPerSecond} MiB/s`);
  }
  report.results[corpus.name] = perCorpus;
}

mkdirSync(dirname(OUT), { recursive: true });
writeFileSync(OUT, JSON.stringify(report, null, 2) + "\n");
console.log(`wrote ${OUT}`);
