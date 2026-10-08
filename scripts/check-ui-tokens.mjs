import { readdir, readFile } from "node:fs/promises";
import { join, relative } from "node:path";
import { fileURLToPath } from "node:url";

// Drift guard 2 (decision Q19a): every `var(--name)` the page references must be declared
// somewhere under rerail/ui — by the generated token sheet or by the page's own CSS. A
// renamed upstream token regenerates cleanly and passes `git diff`; only this catches the
// dangling reference. A `var(--x, fallback)` is not an excuse: it is checked the same way.

const UI_DIR = fileURLToPath(new URL("../ui/", import.meta.url));
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
for await (const path of walk(UI_DIR)) {
  const source = await readFile(path, "utf8");
  for (const [, name] of source.matchAll(/(?<![\w-])(--[\w-]+)\s*:/gu)) declared.add(name);
  for (const [, name] of source.matchAll(/setProperty\(\s*["'](--[\w-]+)["']/gu)) declared.add(name);
  const file = relative(process.cwd(), path);
  source.split("\n").forEach((text, index) => {
    for (const [, name] of text.matchAll(/var\(\s*(--[\w-]+)/gu)) referenced.push({ name, file, line: index + 1 });
  });
}

const missing = referenced.filter(({ name }) => !declared.has(name));
if (missing.length === 0) {
  console.log(`check:ui-tokens: ${referenced.length} var() references, ${declared.size} declared tokens, none missing`);
  process.exit(0);
}
console.error(`check:ui-tokens: ${missing.length} reference(s) to undeclared custom properties under rerail/ui/:`);
for (const { name, file, line } of missing) console.error(`  ${file}:${line} ${name}`);
console.error("Declare the token in rerail/ui/app.css or regenerate with `npm run generate:ui-tokens`.");
process.exit(1);
