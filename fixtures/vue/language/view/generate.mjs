// Records what Vue's own compiler reads in a single-file component: the oracle for the Vue pack's embedded view
// (template tree plus the TS/JS of `<script>`, `<script setup>` and template expressions). Run offline by a
// maintainer, never in CI, with the tools of ../../tools.mjs installed in `<checkout>/.rerail/tools/vue`:
//
//   node generate.mjs fixtures                 # fixtures/*.vue → view.expected.json (T2, committed)
//   node generate.mjs corpus [<name>...]       # corpora of ../../corpora.json → <checkout>/.rerail/t3/vue/view-<name>.json
//
// Pins (refused when different): @vue/compiler-sfc, @vue/compiler-dom and @vue/shared 3.5.43.
//
// Per file, from @vue/compiler-sfc's `parse` (descriptor, and the template AST compiler-dom parses in "sfc" mode),
// `compileScript` (non-inline, so the template is compiled separately) and `compileTemplate` with the script's
// binding metadata and `prefixIdentifiers` (how Vue resolves every name a template expression reads):
//   blocks       every top-level block: `{ type, lang, setup?, src?, start, end }`, its content range.
//   bindings     compileScript's binding metadata: each name the template can see from the script and its kind
//                (`setup-const`, `setup-ref`, `setup-maybe-ref`, `setup-let`, `setup-reactive-const`, `literal-const`,
//                `props`, `props-aliased`, `data`, `options`).
//   regions      every embedded script region of an HTML template, in source order: `{ kind, start, end, parse }`.
//                `kind` is `interpolation`, `for-source`, `for-alias`, `slot-params`, `dynamic-arg`, `on` or the
//                directive's name (`bind`, `if`, `else-if`, `show`, `model`, `html`, `text`, `memo`, a custom
//                directive's name). `parse` is how Vue reads it: `expression`; `params` (a v-for alias or slot
//                parameter list, which declares); `handler-member` or `handler-function` (a v-on value Vue calls with
//                the event) or `statements` (any other v-on value, run as inline statements with `$event` in scope).
//   references   every identifier a template expression reads, `{ name, start, end, resolution }`: `setup:<kind>`
//                (`$setup.x`), `props`, `data`, `options` (`$props.x`, `$data.x`, `$options.x`), `instance` (`_ctx.x`:
//                nothing the script declares; a global component property, mixin or undeclared name), `global` (an
//                unprefixed name Vue allows as a global: `Math`, `JSON`, ...) or `local` (a v-for, slot or handler
//                scope variable). A v-on handler's `$event` is `local`.
//   declarations every identifier a v-for alias or slot parameter list declares: `{ name, start, end, kind }`.
//   errors       compiler errors, when Vue does not compile the file (the other fields are then partial).
// Offsets are UTF-8 byte offsets into the file, as Rerail's spans count them. A template in another language (pug)
// has its block only.

import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, statSync, writeFileSync } from "node:fs";
import { basename, dirname, join, relative, resolve } from "node:path";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { corporaDirectory, load, toolList, toolsDirectory } from "../../tools.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const TOOLS_DIR = toolsDirectory(HERE);
const sfc = await load(TOOLS_DIR, "@vue/compiler-sfc");
const dom = await load(TOOLS_DIR, "@vue/compiler-dom");
const { isGloballyAllowed } = await load(TOOLS_DIR, "@vue/shared");
// As @vitejs/plugin-vue does: compileScript resolves imported prop and emit types through TypeScript and the disk.
const typescriptModule = (await load(TOOLS_DIR, "typescript")).default;
sfc.registerTS(() => typescriptModule);
const FS = { fileExists: existsSync, readFile: file => (existsSync(file) ? readFileSync(file, "utf8") : undefined), realpath: realpathSync };

const NodeTypes = { ELEMENT: 1, INTERPOLATION: 5, SIMPLE_EXPRESSION: 4, DIRECTIVE: 7 };
const IDENTIFIER = /^[\p{ID_Start}$_][\p{ID_Continue}$\u200c\u200d]*$/u;

/** A UTF-16 index → UTF-8 byte offset converter for `source`. */
function bytesOf(source) {
  if (Buffer.byteLength(source) === source.length) return index => index;
  const table = new Int32Array(source.length + 1);
  let bytes = 0;
  for (let i = 0; i < source.length; i++) {
    table[i] = bytes;
    const code = source.charCodeAt(i);
    bytes += code < 0x80 ? 1 : code < 0x800 ? 2 : code >= 0xd800 && code < 0xdc00 ? 4 : code >= 0xdc00 && code < 0xe000 ? 0 : 3;
  }
  table[source.length] = bytes;
  return index => table[index];
}

/** The template's embedded regions, from compiler-dom's parse of the template (before any transform). */
function templateRegions(ast, at) {
  const regions = [];
  // A dynamic argument's location includes its brackets; its expression is inside them.
  const range = (loc, bracketed = 0) => ({ start: at(loc.start.offset + bracketed), end: at(loc.end.offset - bracketed) });
  const add = (kind, node, parse, scope) => {
    if (node && node.loc && node.loc.source.trim() !== "") {
      const bracketed = kind === "dynamic-arg" && /^\[[\s\S]*\]$/.test(node.loc.source) ? 1 : 0;
      regions.push({ kind, ...range(node.loc, bracketed), parse, ...(scope ? { scope: range(scope.loc) } : {}) });
    }
  };
  const walk = node => {
    if (node.type === NodeTypes.INTERPOLATION) add("interpolation", node.content, "expression");
    if (node.type === NodeTypes.ELEMENT) {
      for (const prop of node.props) {
        if (prop.type !== NodeTypes.DIRECTIVE) continue;
        if (prop.arg && !prop.arg.isStatic) add("dynamic-arg", prop.arg, "expression");
        if (prop.name === "for" && prop.forParseResult) {
          const { source, value, key, index } = prop.forParseResult;
          for (const alias of [value, key, index]) add("for-alias", alias, "params", node);
          add("for-source", source, "expression");
        } else if (prop.name === "slot") {
          add("slot-params", prop.exp, "params", node);
        } else if (prop.name === "on" && prop.exp && !prop.arg) {
          add("on", prop.exp, "expression"); // v-on="{ event: handler }": an object of handlers
        } else if (prop.name === "on" && prop.exp) {
          const context = { prefixIdentifiers: true, expressionPlugins: ["typescript"] };
          const parse = dom.isMemberExpression(prop.exp, context) ? "handler-member"
            : dom.isFnExpression(prop.exp, context) ? "handler-function" : "statements";
          add("on", prop.exp, parse);
        } else {
          add(prop.name, prop.exp, "expression");
        }
      }
    }
    for (const child of node.children ?? []) walk(child);
    for (const branch of node.branches ?? []) walk(branch);
  };
  walk(ast);
  return regions.sort((a, b) => a.start - b.start || a.end - b.end);
}

const PREFIXES = [["$setup.", "setup"], ["$props.", "props"], ["$data.", "data"], ["$options.", "options"], ["_ctx.", "instance"]];

/**
 * References and declarations, from compileTemplate's transformed AST: compiler-core rewrites each identifier a
 * template expression reads into a simple expression whose content carries Vue's resolution (`$setup.x`, `_ctx.x`,
 * or `x` for a scope variable or allowed global), located at the identifier.
 */
function resolutions(source, ast, bindings, regions, at) {
  const declaring = regions.filter(region => region.parse === "params");
  const nodes = new Map();
  const seen = new Set();
  const visit = node => {
    if (node === null || typeof node !== "object" || seen.has(node)) return;
    seen.add(node);
    if (node.type === NodeTypes.SIMPLE_EXPRESSION && !node.isStatic && node.loc) {
      const bracketed = /^\[[\s\S]*\]$/.test(node.loc.source) ? 1 : 0;
      const name = node.loc.source.slice(bracketed, node.loc.source.length - bracketed);
      const start = node.loc.start.offset + bracketed;
      const end = node.loc.end.offset - bracketed;
      // compiler-core also locates a member's property (`a` of `x.a`); only what follows no `.` is read by name.
      const before = source.slice(0, start).trimEnd();
      if (IDENTIFIER.test(name) && !(before.endsWith(".") && !before.endsWith("...")) && !nodes.has(`${start}:${end}`)) {
        // A dynamic argument's rewrite is `<prefix>name || ""`.
        nodes.set(`${start}:${end}`, { name, start, end,
          content: node.content.replace(/^[\p{ID_Continue}$]+:\s*/u, "").replace(/ \|\| ""$/, "") });
      }
    }
    for (const [key, value] of Object.entries(node)) {
      if (key !== "parent" && key !== "ast") visit(value);
    }
  };
  visit(ast);
  const declarations = [];
  const reads = [];
  for (const node of nodes.values()) {
    const byte = { start: at(node.start), end: at(node.end) };
    const region = declaring.find(r => byte.start >= r.start && byte.end <= r.end);
    // Inside a parameter list an unprefixed name declares, unless it is a destructuring default's value
    // (`{ a = b }` reads `b`; compiler-core prefixes `b` unless it is a scope variable or an allowed global).
    const defaultValue = /(?<![=!<>])=\s*$/.test(source.slice(0, node.start));
    if (region !== undefined && node.content === node.name && !defaultValue) {
      declarations.push({ name: node.name, ...byte, kind: region.kind === "for-alias" ? "v-for" : "v-slot", scope: region.scope });
    } else {
      reads.push({ ...node, ...byte });
    }
  }
  const inScope = (name, offset) => declarations.some(d => d.name === name && offset >= d.scope.start && offset < d.scope.end);
  const references = reads.map(({ name, start, end, content }) => {
    const prefix = PREFIXES.find(([text]) => content.startsWith(text) && content.slice(text.length) === name);
    let resolution;
    if (prefix !== undefined) resolution = prefix[1] === "setup" ? `setup:${bindings[name] ?? "unknown"}` : prefix[1];
    else if (content !== name) resolution = `unrecognized: ${content}`;
    else resolution = isGloballyAllowed(name) && !inScope(name, start) ? "global" : "local";
    return { name, start, end, resolution };
  });
  const order = (a, b) => a.start - b.start || a.end - b.end;
  return { references: references.sort(order), declarations: declarations.sort(order) };
}

/** The view record of one file; `absolute` is its path on disk, for the type imports compileScript resolves. */
function view(source, path, absolute) {
  const at = bytesOf(source);
  const { descriptor, errors: parseErrors } = sfc.parse(source, { filename: absolute, sourceMap: false });
  const blocks = [descriptor.template, descriptor.script, descriptor.scriptSetup, ...descriptor.styles, ...descriptor.customBlocks]
    .filter(Boolean)
    .map(block => ({
      type: block === descriptor.scriptSetup ? "script" : block.type,
      lang: block.lang ?? null,
      ...(block === descriptor.scriptSetup ? { setup: true } : {}),
      ...(block.src ? { src: block.src } : {}),
      start: at(block.loc.start.offset),
      end: at(block.loc.end.offset),
    }))
    .sort((a, b) => a.start - b.start);
  const out = { path, blocks, bindings: {}, regions: [], references: [], declarations: [] };
  const errors = parseErrors.map(error => String(error.message ?? error).split("\n")[0]);
  let bindingsKnown = true;
  if (descriptor.script || descriptor.scriptSetup) {
    try {
      const script = sfc.compileScript(descriptor, { id: "rerail", inlineTemplate: false, sourceMap: false, fs: FS });
      out.bindings = Object.fromEntries(Object.entries(script.bindings ?? {}).filter(([key]) => key !== "__isScriptSetup").sort());
    } catch (error) {
      bindingsKnown = false;
      errors.push(`compileScript: ${String(error.message).split("\n")[0]}`);
    }
  }
  const template = descriptor.template;
  if (template && template.ast && (template.lang ?? "html") === "html") {
    out.regions = templateRegions(template.ast, at);
    const lang = descriptor.scriptSetup?.lang ?? descriptor.script?.lang;
    const compiled = sfc.compileTemplate({
      source: template.content, ast: template.ast, filename: path, id: "rerail",
      compilerOptions: { bindingMetadata: out.bindings, prefixIdentifiers: true, hoistStatic: false, cacheHandlers: false,
        expressionPlugins: lang === "ts" || lang === "tsx" ? ["typescript"] : [] },
    });
    errors.push(...compiled.errors.map(error => `compileTemplate: ${String(error.message ?? error).split("\n")[0]}`));
    if (compiled.ast) {
      const { references, declarations } = resolutions(source, compiled.ast, out.bindings, out.regions, at);
      out.declarations = declarations;
      // Without the script's bindings every name would read as `_ctx.x`; such a file has no references.
      if (bindingsKnown) out.references = references;
      else out.referencesOmitted = "compileScript failed: the script's bindings are unknown";
    }
  }
  if (errors.length > 0) out.errors = errors;
  return out;
}

// --- modes -------------------------------------------------------------------------------------------------------

function header(extra) {
  return { tools: toolList("@vue/compiler-sfc", "@vue/compiler-dom", "@vue/shared"), ...extra };
}

function fixtures() {
  const dir = join(HERE, "fixtures");
  const files = readdirSync(dir).filter(name => name.endsWith(".vue")).sort();
  const records = files.map(name => view(readFileSync(join(dir, name), "utf8"), name, join(dir, name)));
  writeFileSync(join(HERE, "view.expected.json"), `${JSON.stringify(header({ source: "fixtures", files: records }), null, 1)}\n`);
  console.log(`wrote view.expected.json: ${records.length} fixtures`);
}

function vueFiles(root, paths) {
  const out = [];
  const walk = dir => {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      if (entry.name === "node_modules" || entry.name === "dist" || entry.name === ".git") continue;
      const path = join(dir, entry.name);
      if (entry.isDirectory()) walk(path);
      else if (entry.isFile() && entry.name.endsWith(".vue")) out.push(relative(root, path));
    }
  };
  for (const path of paths) {
    if (statSync(join(root, path), { throwIfNoEntry: false })?.isDirectory()) walk(join(root, path));
  }
  return out.sort();
}

function corpus(names) {
  const corpora = JSON.parse(readFileSync(join(HERE, "../../corpora.json"), "utf8"));
  const base = corporaDirectory(HERE);
  const outDir = resolve(TOOLS_DIR, "../../t3/vue");
  mkdirSync(outDir, { recursive: true });
  for (const entry of corpora) {
    if (names.length > 0 && !names.includes(entry.name)) continue;
    const root = join(base, entry.name);
    const head = execFileSync("git", ["-C", root, "rev-parse", "HEAD"], { encoding: "utf8" }).trim();
    if (head !== entry.commit) throw new Error(`${root} is at ${head}; corpora.json pins ${entry.commit}`);
    const hashes = [];
    const records = vueFiles(root, entry.paths).map(path => {
      const bytes = readFileSync(join(root, path));
      hashes.push(`${path}\0${createHash("sha256").update(bytes).digest("hex")}`);
      return view(bytes.toString("utf8"), path, join(root, path));
    });
    const file = join(outDir, `view-${entry.name}.json`);
    writeFileSync(file, `${JSON.stringify(header({
      source: { kind: "corpus", name: entry.name, repository: entry.repository, commit: entry.commit,
        manifestSha256: createHash("sha256").update(hashes.join("\n")).digest("hex") },
      files: records,
    }))}\n`);
    const count = key => records.reduce((sum, record) => sum + record[key].length, 0);
    const resolutionsByKind = {};
    for (const record of records) {
      for (const reference of record.references) resolutionsByKind[reference.resolution] = (resolutionsByKind[reference.resolution] ?? 0) + 1;
    }
    console.log(`wrote ${file}: ${records.length} files, ${count("regions")} regions, ${count("references")} references, `
      + `${count("declarations")} declarations, ${records.filter(r => r.errors).length} files with errors`);
    console.log(`  resolutions ${JSON.stringify(resolutionsByKind)}`);
  }
}

const [mode, ...rest] = process.argv.slice(2).filter((arg, index, all) => !["--tools", "--corpora"].includes(arg)
  && !["--tools", "--corpora"].includes(all[index - 1]));
if (mode === "fixtures") fixtures();
else if (mode === "corpus") corpus(rest);
else throw new Error(`usage: node ${basename(fileURLToPath(import.meta.url))} fixtures | corpus [<name>...]`);
