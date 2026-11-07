import { readdir, readFile } from "node:fs/promises";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

// Drift guard 2 (decision Q19a): every `var(--name)` the page references must be declared
// somewhere under rerail/ui — by the generated token sheet or by the page's own CSS. A
// renamed upstream token regenerates cleanly and passes `git diff`; only this catches the
// dangling reference. A `var(--x, fallback)` is not an excuse: it is checked the same way.
// The page's CSS is one sheet per view under ui/css/, so it also checks that index.html links
// every sheet there and links nothing that is missing: an unlinked sheet's tokens would count
// as declared while the page never loads them.

const UI_DIR = fileURLToPath(new URL("../../ui/", import.meta.url));
const SCANNED = new Set([".css", ".js", ".html"]);

async function* walk(dir) {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) yield* walk(path);
    else if (SCANNED.has(path.slice(path.lastIndexOf(".")))) yield path;
  }
}

const declared = new Set();
const referenced = []; // { name, file, line }
const sheets = new Set(); // every .css under ui/, as index.html links it ("/css/base.css")
for await (const path of walk(UI_DIR)) {
  const source = await readFile(path, "utf8");
  if (path.endsWith(".css")) sheets.add(`/${relative(UI_DIR, path).split(sep).join("/")}`);
  for (const [, name] of source.matchAll(/(?<![\w-])(--[\w-]+)\s*:/gu)) declared.add(name);
  for (const [, name] of source.matchAll(/setProperty\(\s*["'](--[\w-]+)["']/gu)) declared.add(name);
  const file = relative(process.cwd(), path);
  source.split("\n").forEach((text, index) => {
    for (const [, name] of text.matchAll(/var\(\s*(--[\w-]+)/gu)) referenced.push({ name, file, line: index + 1 });
  });
}

const page = await readFile(join(UI_DIR, "index.html"), "utf8");
const linked = new Set([...page.matchAll(/<link\s+rel="stylesheet"\s+href="([^"]+)"/gu)].map(([, href]) => href));
const unlinked = [...sheets].filter(sheet => !linked.has(sheet));
const dangling = [...linked].filter(href => !sheets.has(href));

const missing = referenced.filter(({ name }) => !declared.has(name));
if (missing.length === 0 && unlinked.length === 0 && dangling.length === 0) {
  console.log(`check:ui-tokens: ${referenced.length} var() references, ${declared.size} declared tokens, none missing; ${sheets.size} stylesheets, all linked`);
  process.exit(0);
}
if (missing.length > 0) {
  console.error(`check:ui-tokens: ${missing.length} reference(s) to undeclared custom properties under rerail/ui/:`);
  for (const { name, file, line } of missing) console.error(`  ${file}:${line} ${name}`);
  console.error("Declare the token in rerail/ui/css/base.css (page-local tokens) or regenerate with `npm run generate:ui-tokens`.");
}
for (const sheet of unlinked) console.error(`check:ui-tokens: ui${sheet} is not linked from ui/index.html`);
for (const href of dangling) console.error(`check:ui-tokens: ui/index.html links ${href}, which is not under ui/`);
process.exit(1);
