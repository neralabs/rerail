#!/usr/bin/env node
//
// Builds the documentation site GitHub Pages serves (.github/workflows/docs-site.yml) from `docs/`, into
// `dist-docs-site/` (gitignored), so every doc reads in a browser, the DSL reference above all, which GitHub's file
// view shows as source. Each `*.md` becomes a page of the same name (`README.md` an `index.html`): GitHub Flavored
// Markdown through marked, heading ids slugged as GitHub slugs them so `doc.md#anchor` links keep working, in one
// template. Its colours, fonts, radii and shadows are Nera's design tokens, the sheet the dashboard uses
// (ui/tokens.generated.css, copied as `tokens.css` and linked from every page), plus a page-local block derived from
// them as ui/css/base.css derives the dashboard's; the build fails when the site's CSS uses a `var(--x)` neither
// declares. Fenced code is highlighted at build time by highlight.js (the info string's first word picks the language;
// one it does not know stays plain) and gets a copy button; a ```rule block, the readable form of a rule, renders as a
// rule card instead. The DSL reference is copied with the tokens linked and its own palette mapped onto them; every
// other file (`images/`) is copied byte for byte. Links are rewritten: a doc's link to a `.md` in `docs/` to its page,
// a link that leaves `docs/` to the file on GitHub (`tree` for a directory); absolute URLs and in-page anchors are left
// alone. A relative link to a file that does not exist fails the build, listing each, and nothing is written.
// ```mermaid blocks render in the browser: a page that has one loads mermaid from jsDelivr, pinned to an exact version,
// and themes it from the tokens. No network at build time, and the output depends on the checkout alone.
// The engine's design notes, measurement journals and rule-syntax workbench (`UNPUBLISHED`) stay out of the site: no
// page, copy, Markdown twin, search entry or llms line, and a published doc's link to one goes to the file on GitHub, as
// a link that leaves `docs/` does.
//
// Every page, the DSL reference too, has the same top bar with a search box over `search-index.json` (each page's h2
// and h3 sections, the reference's cards and grammar productions); a page with three or more sections has an outline
// beside it on wide screens. The bar also has the one theme toggle: every page follows the colour scheme until a reader
// chooses, and that choice holds on every page.
//
// For agents (llmstxt.org), every page also gets a Markdown twin at the same path with `.md` (a doc's own Markdown with
// its links pointed at twins; the DSL reference converted from its HTML), named in its <head> and in a line at the top
// of the page; `llms.txt` lists the twins in docs/README.md's sections and `llms-full.txt` holds them all. The code
// rule's JSON Schema (`rerail.policy.v1.schema.json`) is generated from the zod schema in src/engine/code-rule/, with
// an `$option` reference allowed wherever the engine resolves one.
//
//   node --import tsx scripts/build/docs-site.mjs   (npm run build:docs-site)

import { cpSync, existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join, posix, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import css from "highlight.js/lib/languages/css";
import diff from "highlight.js/lib/languages/diff";
import ini from "highlight.js/lib/languages/ini";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import php from "highlight.js/lib/languages/php";
import phpTemplate from "highlight.js/lib/languages/php-template";
import python from "highlight.js/lib/languages/python";
import rust from "highlight.js/lib/languages/rust";
import sql from "highlight.js/lib/languages/sql";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";
import { Marked } from "marked";
import { NodeHtmlMarkdown } from "node-html-markdown";
import { z } from "zod";
import { codeRuleSchema } from "../../src/engine/index.ts";

const ROOT = dirname(dirname(dirname(fileURLToPath(import.meta.url))));
const DOCS = join(ROOT, "docs");
const OUT = join(ROOT, "dist-docs-site");
const TOKENS = join(ROOT, "ui", "tokens.generated.css");
const REPO = "https://github.com/neralabs/rerail";
const BRANCH = "main";
const MERMAID = "https://cdn.jsdelivr.net/npm/mermaid@12.2.0/dist/mermaid.esm.min.mjs";
const REFERENCE = "code-rule-dsl-reference.html";
/** Where the site is served; llms.txt and the JSON Schema's $id use absolute URLs. */
const SITE = "https://neralabs.github.io/rerail";
/** The JSON Schema of a code-rule document, generated from the zod schema the engine's host validates with. */
const SCHEMA = "rerail.policy.v1.schema.json";

/**
 * The paths under `docs/` the site leaves out, a trailing `/` naming a directory: maintainers' engine design notes,
 * spikes and measurement journals, and the rule-syntax workbench (the engine compiles docs/rule-syntax/lib/core.rr, so
 * it stays where it is). Each must exist, so the list cannot outlive what it names.
 */
const UNPUBLISHED = [
  "final-shape/",
  "parser-generators/",
  "rule-engine-direction/",
  "rule-syntax/",
  "batched-sql-performance.md",
  "code-rule-profiling.md",
  "data-layer-batch-five.md",
  "data-layer-batch-four-root-causes.md",
  "data-layer-batch-four.md",
  "data-layer-batch-six.md",
  "data-layer-batch-three.md",
  "data-layer-batch-two.md",
  "data-layer-engine-gaps.md",
  "data-layer-performance-journal.md",
  "data-layer-port-execution-plan.md",
  "data-layer-rule-port.md",
  "gpu-producer-execution-contract.md",
  "native-relations-implementation.md",
  "native-relations-spikes.md",
  "reachability-spikes.md",
  "relational-execution-design.md",
  "rule-syntax-options.md",
  "shared-producer-contracts.md",
  "shared-producer-workplan.md",
  "shared-sql-performance.md",
  "shared-syntax-implementation.md",
  "shared-syntax-requirement.md",
  "sql-composition-implementation.md",
  "sql-composition-opportunities.md",
  "sql-execution-chunk-c.md",
  "sql-execution-opportunities.md",
  "sql-execution-owner-spike.md",
  "sql-execution-stages.md",
  "sql-executor-recovery-plan.md",
  "sql-executor.md",
  "sql-materialization-shape.md",
  "sql-performance-recovery-workplan.md",
  "sql-performance-spikes.md",
  "sql-post-c-spikes.md",
  "sql-preparation-architecture.md",
  "sql-preparation-spikes.md",
  "sql-shape-spikes.md",
  "sql-unification-and-updates-plan.md",
  "sql-workload-profiling.md",
];

/** Whether docs path `path` (relative to `docs/`, `/`-separated) is one the site leaves out. */
const unpublished = (path) => UNPUBLISHED.some((entry) => (entry.endsWith("/") ? `${path}/`.startsWith(entry) : path === entry));

/** The header's links after the docs index, each shown when the site has its page. */
const NAV = [
  ["architecture-rule-examples.html", "Architecture examples"],
  [REFERENCE, "DSL reference"],
];

/** A fenced block's language (its info string's first word) to the highlight.js grammar that colours it. */
const LANGUAGES = {
  ts: "typescript", tsx: "typescript", typescript: "typescript",
  js: "javascript", jsx: "javascript", mjs: "javascript", javascript: "javascript",
  py: "python", python: "python",
  rs: "rust", rust: "rust",
  php: "php", blade: "php-template",
  vue: "xml", html: "xml", xml: "xml",
  css: "css",
  sh: "bash", shell: "bash", bash: "bash", console: "bash",
  json: "json", jsonc: "json",
  sql: "sql",
  diff: "diff", patch: "diff",
  yaml: "yaml", yml: "yaml",
  toml: "ini", ini: "ini",
};
// xml colours JSX and Vue's markup, and hands a <script> to javascript and a <style> to css; php-template hands its
// markup to xml and its `<?php` blocks to php.
const GRAMMARS = { bash, css, diff, ini, javascript, json, php, "php-template": phpTemplate, python, rust, sql, typescript, xml, yaml };
for (const [name, grammar] of Object.entries(GRAMMARS)) hljs.registerLanguage(name, grammar);

/** The modes a ```rule block's `rule` line may end with. */
const RULE_MODES = new Set(["log", "steer", "block"]);

/** Every file under `dir` (dotfiles skipped), as sorted `/`-separated paths relative to it. */
function walk(dir, base = dir) {
  const files = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.name.startsWith(".")) continue;
    const path = join(dir, entry.name);
    if (entry.isDirectory()) files.push(...walk(path, base));
    else files.push(relative(base, path).split(sep).join("/"));
  }
  return files.sort();
}

const escapeHtml = (text) =>
  text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;").replace(/'/g, "&#39;");

const NAMED = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'" };
const decodeHtml = (html) =>
  html.replace(/&(?:#(\d+)|#x([0-9a-f]+)|([a-z]+));/gi, (entity, dec, hex, name) =>
    dec !== undefined ? String.fromCodePoint(Number(dec)) : hex !== undefined ? String.fromCodePoint(parseInt(hex, 16)) : (NAMED[name] ?? entity),
  );

/** A doc's `.md` path to its page's path: `README.md` is its directory's `index.html`. */
const pagePath = (path) => (posix.basename(path) === "README.md" ? posix.join(posix.dirname(path), "index.html") : path.replace(/\.md$/, ".html"));

/** GitHub's heading anchors (github-slugger): lowercase, punctuation and symbols dropped but `-` and `_`, spaces to `-`. */
function slugger() {
  const seen = new Map();
  return (text) => {
    const base = text.toLowerCase().replace(/[^\p{L}\p{M}\p{N}\p{Pc} -]/gu, "").replace(/ /g, "-");
    let slug = base;
    while (seen.has(slug)) {
      const count = seen.get(base) + 1;
      seen.set(base, count);
      slug = `${base}-${count}`;
    }
    seen.set(slug, 0);
    return slug;
  };
}

/**
 * `url`, written in docs file `from`, as the site links it, or `{ missing }` when it names no file. Paths are
 * relative to `docs/`.
 */
function rewriteUrl(url, from) {
  if (url === "" || url.startsWith("#") || /^[a-z][a-z0-9+.-]*:/i.test(url) || url.startsWith("//")) return { url };
  const split = url.search(/[?#]/);
  const path = split === -1 ? url : url.slice(0, split);
  const suffix = split === -1 ? "" : url.slice(split);
  const target = path.startsWith("/") ? posix.normalize(`../${path.slice(1)}`) : posix.normalize(posix.join(posix.dirname(from), decodeURI(path)));
  const repoPath = posix.join("docs", target).replace(/\/$/, "");
  const absolute = join(ROOT, ...repoPath.split("/"));
  if (repoPath === ".." || repoPath.startsWith("../") || !existsSync(absolute)) return { missing: true };
  const directory = statSync(absolute).isDirectory();
  if (!repoPath.startsWith("docs/") || directory || unpublished(target)) {
    const tail = repoPath === "." ? "" : `/${encodeURI(repoPath)}`;
    return { url: `${REPO}/${directory ? "tree" : "blob"}/${BRANCH}${tail}${suffix}` };
  }
  if (!target.endsWith(".md")) return { url };
  const page = posix.relative(posix.dirname(from), pagePath(target));
  return { url: `${encodeURI(page)}${suffix}` };
}

/**
 * A fenced block of `language`, highlighted when highlight.js has its grammar, with a copy button. The block is labelled
 * with its grammar, or with the fence's own word (`text`) when it has none, so a block written without one stands out.
 */
function codeBlock(text, language) {
  const grammar = LANGUAGES[language];
  let html = grammar === undefined ? escapeHtml(text) : hljs.highlight(text, { language: grammar, ignoreIllegals: true }).value;
  // highlight.js marks any word that names a shell builtin (`test`, `log`, `wait`, `dir`) even inside an argument or flag;
  // in these command lines that colour misleads more than it helps.
  if (grammar === "bash") html = html.replace(/<span class="hljs-built_in">([^<]*)<\/span>/gu, "$1");
  const name = grammar ?? (/^[\w-]+$/u.test(language) ? language : "");
  const label = name === "" ? "" : ` language-${name}`;
  // The bar over the block names its language (or, set by render, its file) and carries the copy button, so the button
  // never sits on the code.
  const caption = /^[\w-]+$/u.test(language) ? language : "text";
  return `<div class="code-block"><div class="code-bar"><span class="code-label">${escapeHtml(caption)}</span><button type="button" class="copy-button" aria-live="polite">Copy</button></div><pre><code class="hljs${label}">${html}</code></pre></div>\n`;
}

/** A rule-card value: its "quoted" strings, `<Tag>`s and `$VARIABLES` marked, the rest breakable escaped text. */
const ruleValue = (text) =>
  text
    .split(/("[^"]*"|<\/?[A-Za-z][\w.:-]*(?:\s[^<>]*?)?\/?>|\$[A-Z][A-Z0-9_]*)/)
    .map((part, index) => {
      if (index % 2 === 0) return breakAt(escapeHtml(part));
      const kind = part.startsWith('"') ? "str" : part.startsWith("$") ? "var" : "tag";
      return `<span class="rule-${kind}">${breakAt(escapeHtml(part))}</span>`;
    })
    .join("");

/**
 * A ```rule block as a rule card: each line is a keyword (`rule`, `files`, `forbid`, `say`, `allow`, `misses`) and its
 * value, an indented line continues the one above (a soft wrap, joined with a space), and the `rule` line's trailing
 * mode becomes a badge.
 */
function ruleCard(text) {
  const rows = [];
  for (const line of text.split("\n")) {
    if (line.trim() === "") continue;
    const [, key = "", value = ""] = /^(\S+)\s*(.*)$/.exec(line.trim()) ?? [];
    if (/^\s/.test(line) && rows.length > 0) rows.at(-1).values.push(line.trim());
    else rows.push({ key, values: [value] });
  }
  const html = rows.map(({ key, values }) => {
    let value = values.map(ruleValue).join(" ");
    if (key === "rule") {
      const [, id = values[0], mode] = /^(.*?)\s+(\S+)$/.exec(values[0]) ?? [];
      value = RULE_MODES.has(mode) ? `<code class="rule-id">${escapeHtml(id)}</code><span class="rule-mode rule-mode-${mode}">${mode}</span>` : `<code class="rule-id">${escapeHtml(values[0])}</code>`;
      value += values.slice(1).map((more) => ` ${ruleValue(more)}`).join("");
    }
    return `<div class="rule-row${key === "rule" ? " rule-head" : ""}"><span class="rule-key">${escapeHtml(key)}</span><span class="rule-value">${value}</span></div>`;
  });
  return `<div class="rule-card">\n${html.join("\n")}\n</div>\n`;
}

/** Docs file `path`'s markdown as `{ title, body, mermaid, copyable, missing }`. */
function render(markdown, path) {
  const slug = slugger();
  const missing = [];
  const sections = [];
  let title = null;
  let mermaid = false;
  let copyable = false;
  const marked = new Marked({ gfm: true });
  const rewrite = (url) => {
    const result = rewriteUrl(url, path);
    if (result.missing) {
      missing.push(url);
      return url;
    }
    return result.url;
  };
  marked.use({
    walkTokens(token) {
      if (token.type === "link" || token.type === "image") token.href = rewrite(token.href);
      else if (token.type === "html") {
        token.text = token.text.replace(/\b(href|src)=(["'])(.*?)\2/g, (_, name, quote, url) => `${name}=${quote}${escapeHtml(rewrite(decodeHtml(url)))}${quote}`);
      }
    },
    renderer: {
      heading({ tokens, depth }) {
        const html = this.parser.parseInline(tokens);
        const text = decodeHtml(html.replace(/<[^>]*>/g, ""));
        if (depth === 1 && title === null) title = text.trim();
        const id = slug(text);
        if (depth === 2) sections.push({ id, text: text.trim() });
        return `<h${depth} id="${escapeHtml(id)}"><a class="anchor" href="#${escapeHtml(id)}" aria-hidden="true">#</a>${html}</h${depth}>\n`;
      },
      code({ text, lang }) {
        const language = (lang ?? "").split(/\s/)[0];
        if (language === "mermaid") {
          mermaid = true;
          return `<pre class="mermaid">${escapeHtml(text)}</pre>\n`;
        }
        if (language === "rule") return ruleCard(text);
        copyable = true;
        return codeBlock(text, language);
      },
    },
  });
  // A paragraph that is only a file's path, right before a code block, becomes that block's caption; a GitHub alert
  // (`> [!WARNING]`) becomes a titled box; a fold's <summary> gets its inline Markdown (`code`, emphasis), which raw
  // HTML blocks otherwise keep as written.
  const body = breakCode(
    labelTableCells(
      marked
        .parse(markdown)
        .replace(/<p><code>([^<\s]+\.\w+)<\/code><\/p>\n<div class="code-block"><div class="code-bar"><span class="code-label">[^<]*<\/span>/g, (_, file) => `<div class="code-block"><div class="code-bar"><span class="code-label code-file">${breakAt(file)}</span>`)
        .replace(/<blockquote>\n<p>\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\]\s*(?:<\/p>\s*<p>)?/gu, (_, kind) => {
          const title = kind[0] + kind.slice(1).toLowerCase();
          return `<blockquote class="alert alert-${kind.toLowerCase()}">\n<p class="alert-title">${title}</p>\n<p>`;
        })
        .replace(/<summary>([\s\S]*?)<\/summary>/gu, (_, summary) => `<summary>${marked.parseInline(summary)}</summary>`)
        .replace(/<table>[\s\S]*?<\/table>/gu, (table) => `<div class="table-scroll">${table}</div>`),
      5,
    ),
  );
  return { title, body, mermaid, copyable, missing, sections };
}

/**
 * Each body cell of a table of up to `maxColumns` columns labelled with its column's heading (`data-label`): a phone
 * stacks such a table, each cell under its label. A wider table keeps its grid and scrolls.
 */
function labelTableCells(html, maxColumns) {
  return html.replace(/<table\b[^>]*>([\s\S]*?)<\/table>/gu, (table, inner) => {
    const heads = [...(/<thead>([\s\S]*?)<\/thead>/u.exec(inner)?.[1] ?? "").matchAll(/<th[^>]*>([\s\S]*?)<\/th>/gu)].map(([, cell]) =>
      escapeHtml(decodeHtml(cell.replace(/<[^>]+>/gu, "")).trim()),
    );
    const labelled =
      heads.length === 0 || heads.length > maxColumns
        ? table
        : table.replace(/<tr\b([^>]*)>([\s\S]*?)<\/tr>/gu, (row, attributes, cells) => {
            let column = 0;
            return `<tr${attributes}>${cells.replace(/<td(?=[\s>])/gu, () => `<td data-label="${heads[column++] ?? ""}"`)}</tr>`;
          });
    // A short cell of a few words ("10 GiB", "537 (318)", "kebab-case") stays on one line.
    return labelled.replace(/<(t[dh])([^>]*)>([^<]{1,12})<\/t[dh]>/gu, (cell, tag, attributes, text) =>
      /[ -]/u.test(text) && !attributes.includes("class=") ? `<${tag}${attributes} class="short">${text}</${tag}>` : cell,
    );
  });
}

/**
 * Escaped text that may break after `/`, `\`, `.`, `=`, `,`, `|` or `::` (`<wbr>`) where a name continues on both
 * sides: not after a leading `.` or `/`, not inside `1.0`, and not before a short extension (`README.md`). `:` joins
 * one name (`fixtures:pull`), and so do `_`, `-` and case changes in a short one. A name of 21 characters or more (a
 * test name, an operation, a long constant) may break at its underscores, and one of 24 or more where its case changes
 * (`recoverRepeated|Unterminated…`) or at its hyphens, so even a phone's line never splits a word inside it. A call may
 * break after its `(`, a query string before `?` and `&`, markup between two tags, a format between `][`. A long path
 * then wraps at its separators before mid-name, and a table column holding one narrows there without any cell breaking
 * a word.
 */
const breakAt = (escaped) =>
  escaped
    .replace(/(?<=[\p{L}\p{N})\]}*;])([/\\=,|(])(?=[\p{L}*&{([])/gu, "$1<wbr>")
    .replace(/(?<=[\p{L}\p{N})\]}*;])\.(?=\p{L}[\p{L}\p{N}_-]{4})/gu, ".<wbr>")
    .replace(/(?<=[\p{L}\p{N}])::(?=\p{L})/gu, "::<wbr>")
    .replace(/(?<=[\p{L}\p{N}…])(?=\?|&amp;)/gu, "<wbr>")
    .replace(/&gt;(?=&lt;|@)/gu, "&gt;<wbr>")
    .replace(/\](?=\[)/gu, "]<wbr>")
    .replace(/[\p{L}\p{N}_]{21,}/gu, (name) => {
      const snake = name.replace(/_(?=\p{L})/gu, "_<wbr>");
      return name.length >= 24 ? snake.replace(/(?<=\p{Ll})(?=\p{Lu})/gu, "<wbr>") : snake;
    })
    .replace(/[\p{L}\p{N}-]{24,}/gu, (name) => name.replace(/-(?=\p{L})/gu, "-<wbr>"));

/** Inline code up to this long is one unbroken piece: a flag, a name or a short command never splits. */
const WHOLE_CODE = 14;
/** Up to this long it is one piece where the column is wider than a phone's; on a phone it moves to its own line whole. */
const WIDE_CODE = 24;
/** A piece of longer code up to this long never splits; a longer one (a very long name) splits only if wider than the line. */
const PIECE = 30;

/**
 * Every inline `<code>` of plain text, in pieces the line may break between but never inside: a browser would otherwise
 * also break after every `-` (`--steer-|strategy`) or `>`. Short code is one piece (`whole`, `wide`; on a phone a `wide`
 * one moves to the next line together with the punctuation around it, `.code-unit`). Longer code breaks at its spaces
 * and at `breakAt`'s separators, each piece a nowrap span (`.seg`, `.seg.long` for one wider than a phone's line). Runs
 * of spaces keep their width as non-breaking spaces before the last, and spaced single letters (`R E R A I L`) stay
 * together. Highlighted blocks carry a class and are left alone.
 */
const breakCode = (html) =>
  html.replace(/(\(?)<code>([^<]*)<\/code>([.,;:!?)]*)/gu, (_, open, code, close) => {
    const length = decodeHtml(code).length;
    if (length <= WHOLE_CODE) return `${open}<code class="whole">${code}</code>${close}`;
    if (length <= WIDE_CODE) return `<span class="code-unit">${open}<code class="whole wide">${code}</code>${close}</span>`;
    const spaced = code
      .replace(/ {2,}/gu, (run) => `${"&nbsp;".repeat(run.length - 1)} `)
      .replace(/(?<=(?:^| )\S) (?=\S(?: |$))/gu, "&nbsp;");
    const words = spaced.split(/( )/u).map((word) =>
      word === " "
        ? word
        : breakAt(word)
            .split("<wbr>")
            .map((piece) => `<span class="seg${decodeHtml(piece).length > PIECE ? " long" : ""}">${piece}</span>`)
            .join("<wbr>"),
    );
    return `${open}<code>${words.join("")}</code>${close}`;
  });

/** The inline-code pieces of `breakCode`, for the docs pages and the DSL reference alike. */
const CODE_PIECES_CSS = `code { white-space: normal; box-decoration-break: clone; -webkit-box-decoration-break: clone; }
code.whole, code .seg { white-space: pre; }
@media (max-width: 640px) {
  code .seg.long { display: inline-block; max-width: 100%; margin: 0; white-space: pre-wrap; overflow-wrap: break-word; }
  .code-unit { display: inline-block; max-width: 100%; margin: 0; }
  .code-unit code.whole.wide { white-space: pre-wrap; overflow-wrap: break-word; }
}
`;

/**
 * The site's page-local tokens: what Nera's sheet cannot express, each a token or a mix of them, as ui/css/base.css
 * derives the dashboard's (--header, --canvas, the steer chip, the card shadow follow it). The only colour literals in
 * the site's CSS are the dark shadows here, as in base.css. --code-surface: code blocks and table heads, between page
 * and card. --syn-*: syntax colours, from the semantic foregrounds so they read in both themes.
 */
const LIGHT_TOKENS = `
  --header: color-mix(in srgb, var(--sidebar-primary) 75%, var(--brand));
  --canvas: radial-gradient(125% 115% at 100% 100%, color-mix(in srgb, var(--accent) 45%, var(--background)) 0%, color-mix(in srgb, var(--accent) 26%, var(--background)) 45%, var(--background) 100%);
  --code-surface: color-mix(in srgb, var(--border) 40%, var(--card));
  --log-fg: var(--muted-foreground);
  --log-bg: var(--subtle);
  --steer-fg: color-mix(in srgb, var(--warning-foreground) 82%, var(--foreground));
  --steer-bg: color-mix(in srgb, var(--warning) 10%, transparent);
  --block-fg: var(--danger-foreground);
  --block-bg: var(--danger-subtle);
  --brand-border: color-mix(in srgb, var(--brand) 45%, transparent);
  --card-shadow: 0 6px 16px -4px color-mix(in srgb, var(--brand) 30%, transparent);
  --bar-shadow: 0 8px 20px -12px color-mix(in srgb, var(--foreground) 22%, transparent);
  --syn-keyword: var(--brand-foreground);
  --syn-string: color-mix(in srgb, var(--success-foreground) 72%, var(--foreground));
  --syn-number: color-mix(in srgb, var(--warning-foreground) 72%, var(--foreground));
  --syn-literal: var(--danger-foreground);
  --syn-attr: color-mix(in srgb, var(--info-foreground) 72%, var(--foreground));
  --syn-title: var(--header);
  --syn-comment: var(--muted-foreground);
  --syn-punctuation: var(--muted-foreground);
`;
const DARK_TOKENS = `
    --header: var(--foreground);
    --canvas: radial-gradient(125% 115% at 100% 100%, color-mix(in srgb, var(--brand) 16%, var(--background)) 0%, color-mix(in srgb, var(--brand) 8%, var(--background)) 45%, var(--background) 100%);
    --code-surface: color-mix(in srgb, var(--background) 45%, var(--card));
    --steer-bg: color-mix(in srgb, var(--warning) 14%, transparent);
    --card-shadow: 0 6px 16px -4px rgb(0 0 0 / .55);
    --bar-shadow: 0 8px 20px -12px rgb(0 0 0 / .6);
`;
const SITE_TOKENS = `:root {${LIGHT_TOKENS}}
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) {${DARK_TOKENS}  }
}
:root[data-theme="dark"] {${DARK_TOKENS}}
`;

const MONO = "font-family: var(--font-mono);";
/** The site's top bar, shared by the docs pages and the DSL reference. */
const HEADER_CSS = `
.site-header {
  position: sticky; top: 0; z-index: 10; border-bottom: 1px solid var(--border);
  background: color-mix(in srgb, var(--card) 85%, transparent); backdrop-filter: saturate(1.4) blur(10px); box-shadow: var(--bar-shadow);
}
.site-header nav { max-width: 64rem; margin: 0 auto; padding: 0.7rem 1.25rem; display: flex; align-items: center; gap: 0.25rem 1.4rem; flex-wrap: wrap; font-family: var(--font-sans); font-size: 1rem; line-height: 1.6; }
.site-header .brand { display: inline-flex; align-items: center; gap: 0.55rem; margin-right: auto; color: var(--header); font-weight: 700; letter-spacing: -0.01em; }
.site-header .brand::before { content: ""; width: 0.8rem; height: 0.8rem; border-radius: var(--radius-sm); background: var(--brand); box-shadow: 0.25rem 0.25rem 0 var(--brand-subtle-hover); }
.site-header nav a:not(.brand) { color: var(--muted-foreground); font-size: 0.92rem; font-weight: 500; }
.site-header nav a:not(.brand):hover, .site-header nav a[aria-current="page"] { color: var(--brand-foreground); text-decoration: none; }
.site-header nav .site-search li a { color: var(--foreground); }
.site-search { position: relative; }
.site-search input {
  width: 13rem; padding: 0.3rem 0.7rem; border: 1px solid var(--border); border-radius: var(--radius-lg); background: var(--card);
  color: var(--foreground); font: inherit; font-size: 0.88rem;
}
.site-search input:focus { outline: 2px solid var(--brand-border); outline-offset: 1px; }
.site-search input::placeholder { color: var(--muted-foreground); opacity: 1; }
.site-search ol {
  position: absolute; right: 0; top: calc(100% + 0.4rem); z-index: 20; width: min(28rem, 92vw); max-height: 70vh; overflow-y: auto;
  list-style: none; margin: 0; padding: 0.35rem; background: var(--card); border: 1px solid var(--border); border-radius: var(--radius-xl); box-shadow: var(--card-shadow);
}
.site-search li a { display: block; padding: 0.45rem 0.65rem; border-radius: var(--radius-lg); color: var(--foreground); font-size: 0.9rem; line-height: 1.4; }
.site-search li a:hover, .site-search li[aria-selected] a { background: var(--brand-subtle); text-decoration: none; }
/* While the pointer is over the list, only the result under it is highlighted. */
.site-search ol:hover li[aria-selected] a:not(:hover) { background: none; }
.site-search li small { display: block; color: var(--muted-foreground); font-size: 0.78rem; }
.site-search li.none { padding: 0.45rem 0.65rem; color: var(--muted-foreground); font-size: 0.9rem; }
.site-theme { display: inline-flex; padding: 0.35rem; border: 1px solid var(--border); border-radius: var(--radius-lg); background: none; color: var(--muted-foreground); cursor: pointer; }
.site-theme svg { display: block; }
.site-theme .ico-sun { display: none; }
@media (prefers-color-scheme: dark) {
  :root:not([data-theme="light"]) .site-theme .ico-sun { display: block; }
  :root:not([data-theme="light"]) .site-theme .ico-moon { display: none; }
}
:root[data-theme="dark"] .site-theme .ico-sun { display: block; }
:root[data-theme="dark"] .site-theme .ico-moon { display: none; }
.site-theme:hover { color: var(--brand-foreground); border-color: var(--brand-border); }
/* Where the bar wraps, search takes its own full row and its results open under it, and the bar scrolls away with the
   page instead of covering a third of a phone screen and every heading a link jumps to. */
@media (max-width: 900px) {
  .site-header { position: relative; }
  .site-search { flex-basis: 100%; }
  .site-search input { width: 100%; }
  .site-search ol { left: 0; right: auto; width: 100%; }
}
`;

/** Where a reader's theme is kept: the key the DSL reference's own script reads, so that page agrees with the rest. */
const THEME_KEY = "rerail-dsl-ref-theme";
/** Where the theme toggle keeps a diagram page's place and open folds across the reload it makes. */
const RESTORE_KEY = "rerail-docs-restore";

/** Applies the reader's theme in <head>, before the page paints. Without a choice, the colour scheme decides. */
const THEME_SCRIPT = `<script>
(() => {
  let stored = null;
  try { stored = localStorage.getItem("${THEME_KEY}"); } catch { /* private mode */ }
  if (stored === "light" || stored === "dark") {
    document.documentElement.dataset.theme = stored;
    document.documentElement.style.colorScheme = stored;
  }
})();
</script>
`;

/**
 * The toggle's glyphs, Phosphor bold on a 256 grid as the dashboard's are (ui/dom.js). It shows the theme a click switches
 * to: the sun in dark, the moon in light. CSS picks one from the same selectors as the tokens, so it is right before any
 * script runs.
 */
const THEME_ICONS = [
  ["ico-sun", "M116,36V20a12,12,0,0,1,24,0V36a12,12,0,0,1-24,0Zm80,92a68,68,0,1,1-68-68A68.07,68.07,0,0,1,196,128Zm-24,0a44,44,0,1,0-44,44A44.05,44.05,0,0,0,172,128ZM51.51,68.49a12,12,0,1,0,17-17l-12-12a12,12,0,0,0-17,17Zm0,119-12,12a12,12,0,0,0,17,17l12-12a12,12,0,1,0-17-17ZM196,72a12,12,0,0,0,8.49-3.51l12-12a12,12,0,0,0-17-17l-12,12A12,12,0,0,0,196,72Zm8.49,115.51a12,12,0,0,0-17,17l12,12a12,12,0,0,0,17-17ZM48,128a12,12,0,0,0-12-12H20a12,12,0,0,0,0,24H36A12,12,0,0,0,48,128Zm80,80a12,12,0,0,0-12,12v16a12,12,0,0,0,24,0V220A12,12,0,0,0,128,208Zm108-92H220a12,12,0,0,0,0,24h16a12,12,0,0,0,0-24Z"],
  ["ico-moon", "M236.37,139.4a12,12,0,0,0-12-3A84.07,84.07,0,0,1,119.6,31.59a12,12,0,0,0-15-15A108.86,108.86,0,0,0,49.69,55.07,108,108,0,0,0,136,228a107.09,107.09,0,0,0,64.93-21.69,108.86,108.86,0,0,0,38.44-54.94A12,12,0,0,0,236.37,139.4Zm-49.88,47.74A84,84,0,0,1,68.86,69.51,84.93,84.93,0,0,1,92.27,48.29Q92,52.13,92,56A108.12,108.12,0,0,0,200,164q3.87,0,7.71-.27A84.79,84.79,0,0,1,186.49,187.14Z"],
].map(([name, path]) => `<svg class="${name}" viewBox="0 0 256 256" width="16" height="16" fill="currentColor" aria-hidden="true"><path d="${path}"/></svg>`).join("");

/**
 * The top bar's toggle. A page with a diagram reloads, since mermaid takes its colours from the tokens once; the reload
 * keeps the reader's place and open folds (`RESTORE_KEY`, read back by `MERMAID_SCRIPT`).
 */
const THEME_TOGGLE_SCRIPT = `<script>
(() => {
  const root = document.documentElement;
  document.querySelector(".site-theme")?.addEventListener("click", () => {
    const dark = root.dataset.theme ? root.dataset.theme === "dark" : matchMedia("(prefers-color-scheme: dark)").matches;
    const theme = dark ? "light" : "dark";
    root.dataset.theme = theme;
    root.style.colorScheme = theme;
    try {
      localStorage.setItem("${THEME_KEY}", theme);
      if (document.querySelector("pre.mermaid")) {
        const open = [...document.querySelectorAll("details")].map((fold) => fold.open);
        sessionStorage.setItem("${RESTORE_KEY}", JSON.stringify({ path: location.pathname, open, y: scrollY }));
        location.reload();
      }
    } catch { /* private mode: this page only */ }
  });
})();
</script>
`;

/** The outline beside a long page marks the section being read: the last one whose heading is above the fold's top. */
const OUTLINE_SCRIPT = `<script>
(() => {
  const links = [...document.querySelectorAll(".page-outline a")];
  const headings = links.map((a) => document.getElementById(a.getAttribute("href").slice(1)));
  let frame = 0;
  const mark = () => {
    frame = 0;
    let index = -1;
    headings.forEach((h, i) => { if (h && h.getBoundingClientRect().top < 140) index = i; });
    // At the end of the page the last sections cannot scroll up to the line; the last one is the one being read, unless
    // a link just brought another heading to the top.
    if (innerHeight + scrollY >= document.documentElement.scrollHeight - 2) index = headings.length - 1;
    const target = headings.indexOf(document.getElementById(decodeURIComponent(location.hash.slice(1))));
    if (target !== -1 && Math.abs(headings[target].getBoundingClientRect().top - 80) < 24) index = target;
    links.forEach((a, i) => (i === index ? a.setAttribute("aria-current", "true") : a.removeAttribute("aria-current")));
  };
  addEventListener("scroll", () => { frame ||= requestAnimationFrame(mark); }, { passive: true });
  mark();
})();
</script>
`;

const STYLE = `
* { box-sizing: border-box; }
html { -webkit-text-size-adjust: 100%; background-color: var(--background); }
body {
  margin: 0; color: var(--foreground); font-family: var(--font-sans); font-size: 16px; line-height: 1.7;
  -webkit-font-smoothing: antialiased;
}
body::before { content: ""; position: fixed; inset: 0; z-index: -1; pointer-events: none; background-image: var(--canvas); background-repeat: no-repeat; background-size: 100% 100%; }
a { color: var(--brand-foreground); text-decoration: none; }
a:hover { text-decoration: underline; text-underline-offset: 2px; }
a:focus-visible, button:focus-visible, summary:focus-visible { outline: 2px solid var(--brand); outline-offset: 2px; border-radius: var(--radius-md); }

${HEADER_CSS}

/* One column for prose, code, cards and tables, so every block shares both edges; one gap between blocks (--flow),
 * a larger one above a section (--section, --subsection) and a smaller one under a heading; one inner padding for
 * every boxed block (--pad-y, --pad-x). */
:root { --flow: 1.25rem; --section: 4rem; --subsection: 3rem; --under-heading: 0.75rem; --pad-y: 1rem; --pad-x: 1.25rem; }
main {
  display: grid; grid-template-columns: 1fr min(48rem, 100% - 2.5rem) 1fr;
  padding: 3rem 0 1rem;
}
main > * { grid-column: 2; min-width: 0; }

h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin: 0; scroll-margin-top: 5rem; letter-spacing: -0.012em; color: var(--header); }
/* A link to a row, term or item (not a heading) lands below the bar too, and marks what it pointed at. */
:is(dt, dd, li, tr, p, span, div):target { scroll-margin-top: 5rem; background: var(--brand-subtle); border-radius: var(--radius-md); }
h1 { font-size: 2.3rem; font-weight: 750; letter-spacing: -0.025em; }
h1 + p { font-size: 1.12rem; color: var(--muted-foreground); }
h2 { font-size: 1.55rem; font-weight: 700; }
h3 { font-size: 1.2rem; font-weight: 650; }
h4, h5, h6 { font-size: 1rem; font-weight: 650; }
h1 .anchor, h2 .anchor, h3 .anchor, h4 .anchor, h5 .anchor, h6 .anchor { float: left; margin-left: -1.1em; width: 1.1em; color: var(--subtle-foreground); font-weight: 400; visibility: hidden; }
h1:hover .anchor, h2:hover .anchor, h3:hover .anchor, h4:hover .anchor, h5:hover .anchor, h6:hover .anchor { visibility: visible; text-decoration: none; }
/* Below this width the column's margin is narrower than the "#" that hangs in it. */
@media (max-width: 900px) { h1 .anchor, h2 .anchor, h3 .anchor, h4 .anchor, h5 .anchor, h6 .anchor { display: none; } }
:where(p, ul, ol, table, blockquote, .code-block, .rule-card, details, pre, figure) { margin: 0; }
main > * + *, blockquote > * + *, li > * + * { margin-top: var(--flow); }
main > h2 { margin-top: var(--section); }
main > h3, main > h4 { margin-top: var(--subsection); }
main > :is(h1, h2, h3, h4, h5, h6) + * { margin-top: var(--under-heading); }
main > h1 + p { margin-top: var(--flow); }
main > h2 + h3 { margin-top: var(--flow); }
ul, ol { padding-left: 1.4em; }
li + li, li > ul, li > ol { margin-top: 0.35rem; }
/* A GitHub task list: the checkbox stands in for the bullet. */
li:has(> input[type="checkbox"]:first-child) { list-style: none; }
li > input[type="checkbox"]:first-child { margin: 0 0.45em 0 -1.35em; vertical-align: -0.1em; }
strong { font-weight: 650; }
/* Prose is justified and hyphenated where the column is wide enough for even spacing; narrower, it is set ragged. A
   paragraph or item holding code is set ragged too: code never breaks inside a name, so justifying around it leaves
   wide gaps. A section's first paragraph is its one-sentence summary, set as a lede. */
@media (min-width: 1024px) {
  :is(main > p, main > ul > li, main > ol > li, main blockquote p, main > details > p, main > details > ul > li, main > details > ol > li):not(:has(code)) { text-align: justify; hyphens: auto; -webkit-hyphens: auto; hyphenate-limit-chars: 7 3 3; }
}
main > h2 + p { font-size: 1.06rem; color: var(--muted-foreground); }

code, pre, .rule-key, .rule-id, .rule-tag, .rule-var { ${MONO} }
/* Inline code is never hyphenated and keeps its own spacing. It breaks only between the pieces breakCode marks: a
   short one never, a long one after a space or a separator, and inside a piece only when it alone is wider than the line. */
code { font-size: 0.86em; background: var(--subtle); color: var(--foreground); padding: 0.12em 0.4em; border-radius: var(--radius-md); overflow-wrap: break-word; hyphens: manual; -webkit-hyphens: manual; }
${CODE_PIECES_CSS}
a code { color: inherit; }
/* A code block keeps its lines as written (indentation, aligned comments, diagrams) and scrolls inside itself when a
   line is wider than the column; the page never scrolls sideways. A shade at an edge with more code past it shows
   that the block scrolls (two covers that travel with the text hide the shades at the ends). */
pre {
  font-size: 0.86em; line-height: 1.6; color: var(--foreground); padding: var(--pad-y) var(--pad-x); border: 1px solid var(--border);
  border-radius: var(--radius-xl); white-space: pre; overflow-x: auto; tab-size: 4; text-align: left; hyphens: manual; -webkit-hyphens: manual;
  background:
    linear-gradient(to right, var(--code-surface) 40%, transparent) left / 2.5rem 100% no-repeat local,
    linear-gradient(to left, var(--code-surface) 40%, transparent) right / 2.5rem 100% no-repeat local,
    linear-gradient(to right, color-mix(in srgb, var(--foreground) 16%, transparent), transparent) left / 1rem 100% no-repeat scroll,
    linear-gradient(to left, color-mix(in srgb, var(--foreground) 16%, transparent), transparent) right / 1rem 100% no-repeat scroll,
    var(--code-surface);
}
pre code { font-size: 1em; background: none; padding: 0; border-radius: 0; white-space: inherit; overflow-wrap: normal; }
pre.mermaid {
  text-align: center; white-space: normal;
  background:
    linear-gradient(to right, var(--card) 40%, transparent) left / 2.5rem 100% no-repeat local,
    linear-gradient(to left, var(--card) 40%, transparent) right / 2.5rem 100% no-repeat local,
    linear-gradient(to right, color-mix(in srgb, var(--foreground) 16%, transparent), transparent) left / 1rem 100% no-repeat scroll,
    linear-gradient(to left, color-mix(in srgb, var(--foreground) 16%, transparent), transparent) right / 1rem 100% no-repeat scroll,
    var(--card);
}
/* Where a block still overflows (long lines in other docs, wide tables), its scrollbar is thin and in the theme. */
pre, table { scrollbar-width: thin; scrollbar-color: var(--border-strong) transparent; }

.code-block { position: relative; }
.code-block pre { margin: 0; border-radius: 0 0 var(--radius-xl) var(--radius-xl); }
.code-bar {
  display: flex; align-items: center; justify-content: space-between; gap: 1rem; padding: 0.3rem 0.4rem 0.3rem var(--pad-x);
  background: var(--card); border: 1px solid var(--border); border-bottom: 0; border-radius: var(--radius-xl) var(--radius-xl) 0 0;
}
.code-label { ${MONO} font-size: 0.78rem; line-height: 1.4; color: var(--muted-foreground); overflow-wrap: break-word; min-width: 0; }
.code-file { color: var(--foreground); }
.copy-button {
  flex: none; padding: 0.15rem 0.6rem; border: 1px solid var(--border); border-radius: var(--radius);
  background: var(--card); color: var(--muted-foreground); font-family: inherit; font-size: 0.76rem; font-weight: 500; line-height: 1.4; cursor: pointer;
  transition: color 0.15s, border-color 0.15s;
}
.copy-button:hover { color: var(--foreground); border-color: var(--brand-border); }
.copy-button.done { color: var(--success-foreground); border-color: var(--success-foreground); }

.hljs-keyword, .hljs-meta .hljs-keyword, .hljs-selector-tag, .hljs-doctag { color: var(--syn-keyword); }
.hljs-string, .hljs-regexp, .hljs-template-tag, .hljs-addition { color: var(--syn-string); }
.hljs-number { color: var(--syn-number); }
.hljs-literal, .hljs-symbol, .hljs-variable.language_, .hljs-deletion { color: var(--syn-literal); }
.hljs-attr, .hljs-attribute, .hljs-property, .hljs-type, .hljs-built_in, .hljs-selector-attr, .hljs-selector-pseudo { color: var(--syn-attr); }
.hljs-title, .hljs-section, .hljs-selector-id, .hljs-selector-class { color: var(--syn-title); font-weight: 500; }
.hljs-variable, .hljs-template-variable { color: var(--syn-attr); }
.hljs-comment, .hljs-quote, .hljs-meta { color: var(--syn-comment); }
.hljs-comment, .hljs-quote { font-style: italic; }
.hljs-punctuation, .hljs-tag { color: var(--syn-punctuation); }
.hljs-tag .hljs-name { color: var(--syn-keyword); }
.hljs-tag .hljs-attr { color: var(--syn-attr); }
.hljs-tag .hljs-string { color: var(--syn-string); }
.hljs-addition, .hljs-deletion { display: inline-block; width: 100%; }
.hljs-addition { background: var(--success-subtle); }
.hljs-deletion { background: var(--danger-subtle); }

.rule-card { border: 1px solid var(--border); border-left: 3px solid var(--brand); border-radius: var(--radius-xl); background: var(--card); box-shadow: var(--card-shadow); padding: calc(var(--pad-y) - 0.35rem) 0; font-size: 0.95rem; line-height: 1.55; }
.rule-row { display: grid; grid-template-columns: 4.8rem minmax(0, 1fr); column-gap: 1rem; padding: 0.35rem var(--pad-x); }
.rule-head { border-bottom: 1px solid var(--border); padding-bottom: 0.75rem; margin-bottom: 0.4rem; }
.rule-key { color: var(--brand-foreground); font-weight: 700; font-size: 0.86em; padding-top: 0.12em; }
.rule-value { overflow-wrap: break-word; }
.rule-head .rule-value { display: flex; align-items: center; justify-content: space-between; gap: 0.5rem 1rem; flex-wrap: wrap; }
.rule-id { font-size: 0.9em; font-weight: 600; color: var(--header); background: none; padding: 0; overflow-wrap: anywhere; }
.rule-mode { font-size: 0.72rem; font-weight: 600; letter-spacing: 0.04em; text-transform: uppercase; padding: 0.12rem 0.6rem; border-radius: 999px; background: var(--log-bg); color: var(--log-fg); }
.rule-mode-steer { background: var(--steer-bg); color: var(--steer-fg); }
.rule-mode-block { background: var(--block-bg); color: var(--block-fg); }
.rule-str { color: var(--syn-string); }
.rule-tag, .rule-var { font-size: 0.88em; padding: 0.05em 0.3em; border-radius: var(--radius-md); background: var(--subtle); color: var(--syn-keyword); }
.rule-var { color: var(--syn-attr); }

details { border: 1px solid var(--border); border-radius: var(--radius-xl); background: var(--card); box-shadow: var(--card-shadow); }
/* A summary is one run of text (code in it wraps with the words); its chevron sits in the left padding. */
details > summary {
  display: block; position: relative; padding: 0.8rem var(--pad-x) 0.8rem calc(var(--pad-x) + 1.3rem); cursor: pointer; list-style: none;
  font-weight: 600; color: var(--header); border-radius: var(--radius-xl); user-select: none;
}
details > summary::-webkit-details-marker { display: none; }
details > summary::before {
  content: ""; position: absolute; left: calc(var(--pad-x) + 0.15rem); top: 1.28rem; width: 0.45rem; height: 0.45rem;
  border-right: 2px solid var(--muted-foreground); border-bottom: 2px solid var(--muted-foreground); transform: rotate(-45deg); transition: transform 0.15s;
}
details[open] > summary { border-bottom: 1px solid var(--border); border-radius: var(--radius-xl) var(--radius-xl) 0 0; }
details[open] > summary::before { transform: rotate(45deg); top: 1.12rem; }
details > summary:hover { background: var(--hover); }
details > :not(summary) { margin: var(--pad-y) var(--pad-x); }

/* Tables fit the column: cells wrap between words; on a phone each row stacks, every cell labelled by its column. A
   table whose unbreakable cells (names, routes) are wider than the column together scrolls inside its frame. */
.table-scroll { overflow-x: auto; scrollbar-width: thin; scrollbar-color: var(--border-strong) transparent; }
table { width: 100%; border-collapse: separate; border-spacing: 0; font-size: 0.93em; line-height: 1.55; background: var(--card); border: 1px solid var(--border); border-radius: var(--radius-xl); overflow: hidden; }
/* Cells wrap between words (break-word keeps each word whole when sizing columns; anywhere would let a column shrink to
   one letter wide). Code breaks at the <wbr> after its separators. */
th, td { padding: 0.55em 0.9em; text-align: left; vertical-align: top; border-bottom: 1px solid var(--border); overflow-wrap: break-word; }
@media (max-width: 900px) { th, td { padding: 0.5em 0.6em; } }
td code, th code { overflow-wrap: break-word; }
:is(td, th).short { white-space: nowrap; }
tr:last-child td { border-bottom: 0; }
th { background: var(--code-surface); color: var(--header); font-weight: 650; }
td + td, th + th { border-left: 1px solid var(--border); }
tbody tr:hover td { background: var(--hover); }
@media (max-width: 640px) {
  table:has(td[data-label]), table:has(td[data-label]) :is(thead, tbody, tr, th, td) { display: block; }
  table:has(td[data-label]) thead { display: none; }
  table:has(td[data-label]) tr { border-bottom: 1px solid var(--border); padding: 0.4em 0; }
  table:has(td[data-label]) tr:last-child { border-bottom: 0; }
  td[data-label], td[data-label] + td { border: 0; padding: 0.2em 0.9em; }
  td[data-label]::before { content: attr(data-label); display: block; font-size: 0.78rem; font-weight: 650; color: var(--muted-foreground); }
  td[data-label]:empty { display: none; }
}
blockquote { padding: var(--pad-y) var(--pad-x); border-left: 3px solid var(--brand); border-radius: 0 var(--radius-xl) var(--radius-xl) 0; background: var(--brand-subtle); }
/* GitHub's alerts (> [!NOTE], > [!WARNING]…): a titled box in the alert's colour. */
blockquote.alert-note, blockquote.alert-important { border-left-color: var(--info); background: var(--info-subtle); }
blockquote.alert-tip { border-left-color: var(--success); background: var(--success-subtle); }
blockquote.alert-warning { border-left-color: var(--warning); background: var(--warning-subtle); }
blockquote.alert-caution { border-left-color: var(--danger); background: var(--danger-subtle); }
blockquote .alert-title { font-weight: 650; color: var(--header); }
img { max-width: 100%; height: auto; }
hr { margin: 0; border: 0; border-top: 1px solid var(--border); }
main > hr, main > hr + * { margin-top: var(--subsection); }

.site-footer { border-top: 1px solid var(--border); margin-top: 4rem; color: var(--muted-foreground); font-size: 0.9rem; }
.site-footer p { max-width: 64rem; margin: 0 auto; padding: 1.4rem 1.25rem 2.5rem; }
/* A long page's outline: beside the text where the margin has room for it, hidden otherwise. */
.page-outline { display: none; }
@media (min-width: 1300px) {
  .page-outline {
    display: block; position: fixed; top: 6.5rem; right: max(1.5rem, calc((100vw - 48rem) / 2 - 16rem)); width: 13rem;
    max-height: calc(100vh - 8.5rem); overflow-y: auto; font-size: 0.82rem; line-height: 1.45;
  }
  .page-outline p { margin: 0 0 0.5rem; font-weight: 650; color: var(--header); }
  .page-outline ol { list-style: none; margin: 0; padding: 0 0 0 0.75rem; border-left: 1px solid var(--border); }
  .page-outline li + li { margin-top: 0.4rem; }
  .page-outline a { color: var(--muted-foreground); }
  .page-outline a:hover, .page-outline a[aria-current] { color: var(--brand-foreground); text-decoration: none; }
  .page-outline a[aria-current] { font-weight: 600; }
}
.agent-note { font-size: 0.82rem; color: var(--muted-foreground); }
.agent-note a { color: inherit; text-decoration: underline; text-decoration-color: var(--border-strong); text-underline-offset: 2px; }
main > .agent-note + * { margin-top: 0.75rem; }
`;
/**
 * The DSL reference under the site's top bar: its sidebar and every jump stop below the bar. At the page's own narrow
 * breakpoint (960px) the sidebar is a collapsed bar above the text, so it keeps its height and scrolls away: full height
 * it would cover the page, and stuck it would cover the top bar, which wraps there.
 */
const REFERENCE_UNDER_HEADER = `
figure.ex-fig .src .src-path { overflow-wrap: break-word; }
@media (min-width: 961px) { #toc { top: 3.6rem; height: calc(100vh - 3.6rem); } }
@media (max-width: 960px) { #toc { position: static; } }
html { scroll-padding-top: 4.25rem; }
.card, h2, h3, h4 { scroll-margin-top: 0.5rem; }
:is(dt, dd, li, tr, span):target { background: var(--accent-soft); border-radius: 4px; }
/* A figure keeps its labels at least about 10px: below the column width that allows, it scrolls inside its frame. */
@media (max-width: 1100px) { svg.figure:not(.heatmap) { min-width: 820px; } }
code { hyphens: manual; -webkit-hyphens: manual; }
${CODE_PIECES_CSS}
:is(td, th).short { white-space: nowrap; }
/* A table of up to five columns stacks on a phone, each cell under its column's name; a wider matrix keeps its grid and
   scrolls inside its frame. */
@media (max-width: 640px) {
  .table-wrap:has(td[data-label]) { overflow-x: visible; }
  .table-wrap table:has(td[data-label]), .table-wrap table:has(td[data-label]) :is(thead, tbody, tr, th, td) { display: block; }
  .table-wrap table:has(td[data-label]) thead { display: none; }
  .table-wrap table:has(td[data-label]) tr { border-bottom: 1px solid var(--border); padding: 4px 0; }
  .table-wrap table:has(td[data-label]) tr:last-child { border-bottom: 0; }
  .table-wrap td[data-label] { border: 0; padding: 2px 10px; }
  .table-wrap td[data-label]::before { content: attr(data-label); display: block; font-size: 12px; font-weight: 650; color: var(--muted); }
  .table-wrap td[data-label]:empty { display: none; }
}
`;

/**
 * The DSL reference's palette mapped onto the tokens, for the site's copy of it. Its own dark blocks are more specific
 * than `:root`, so each mapping is `!important`; the values follow the tokens, whose own selectors (`data-theme`, which
 * the top bar's toggle sets, and the colour scheme) pick the theme. Its own toggle is hidden: the top bar's replaces it. Its --card and --border are the tokens' own names, and the
 * tokens' sheet comes after its <style>, so those need no mapping.
 */
const REFERENCE_MAP = `:root {
  --bg: var(--background) !important; --fg: var(--foreground) !important;
  --muted: var(--muted-foreground) !important; --faint: var(--muted-foreground) !important;
  --rule: var(--border) !important; --shadow: var(--card-shadow) !important;
  --accent: var(--brand-foreground) !important; --accent-soft: var(--brand-subtle) !important; --accent-ink: var(--header) !important;
  --code-bg: var(--code-surface) !important; --code-fg: var(--foreground) !important;
  --key: var(--syn-keyword) !important; --str: var(--syn-string) !important; --num: var(--syn-number) !important; --lit: var(--syn-literal) !important;
  --ok: var(--success-foreground) !important; --ok-soft: var(--success-subtle) !important;
  --warn: var(--warning-foreground) !important; --warn-soft: var(--warning-subtle) !important;
  --bad: var(--danger-foreground) !important; --bad-soft: var(--danger-subtle) !important;
  --info: var(--info-foreground) !important; --info-soft: var(--info-subtle) !important;
  --fig-line: var(--muted-foreground) !important; --fig-fill: var(--card) !important;
  --fig-soft: color-mix(in srgb, var(--brand) 6%, var(--card)) !important;
  --fig-hi: color-mix(in srgb, var(--brand) 20%, var(--card)) !important;
  --fig-hi2: color-mix(in srgb, var(--warning) 28%, var(--card)) !important;
  --fig-hi3: color-mix(in srgb, var(--success) 22%, var(--card)) !important;
  --fig-hi4: color-mix(in srgb, var(--danger) 18%, var(--card)) !important;
  --hm0: var(--code-surface) !important;
  --hm1: color-mix(in srgb, var(--brand) 12%, var(--card)) !important;
  --hm2: color-mix(in srgb, var(--brand) 25%, var(--card)) !important;
  --hm3: color-mix(in srgb, var(--brand) 40%, var(--card)) !important;
  --hm4: color-mix(in srgb, var(--brand) 58%, var(--card)) !important;
  --hm5: color-mix(in srgb, var(--brand) 78%, var(--card)) !important;
  --hm6: var(--brand) !important;
}
#theme-toggle { display: none; }
body { font-family: var(--font-sans); }
code, pre, .mono { ${MONO} }
.grammar .about, .grammar .d, .grammar .gnote { font-family: var(--font-sans); }
.agent-note { margin: 0 0 6px; font-size: 13px; color: var(--muted-foreground); }
.agent-note a { color: inherit; text-decoration: underline; text-decoration-color: var(--border-strong); text-underline-offset: 2px; }
`;

/** The copy buttons: each copies its block's text exactly, or selects it when the clipboard is unavailable. */
const COPY_SCRIPT = `<script>
for (const button of document.querySelectorAll(".copy-button")) {
  button.addEventListener("click", async () => {
    const code = button.closest(".code-block").querySelector("code");
    let label = "Copied";
    try {
      await navigator.clipboard.writeText(code.textContent);
    } catch {
      getSelection().selectAllChildren(code);
      label = "Selected";
    }
    button.textContent = label;
    button.classList.add("done");
    clearTimeout(button.reset);
    button.reset = setTimeout(() => { button.textContent = "Copy"; button.classList.remove("done"); }, 1600);
  });
}
</script>
`;

/** A link into folded detail opens the folds around its target, so the reader lands on visible text. */
const UNFOLD_SCRIPT = `<script>
(() => {
  const unfold = () => {
    const id = decodeURIComponent(location.hash.slice(1));
    const target = id && document.getElementById(id);
    if (!target) return;
    for (let fold = target.closest("details"); fold; fold = fold.parentElement.closest("details")) fold.open = true;
    target.scrollIntoView();
  };
  addEventListener("hashchange", unfold);
  unfold();
})();
</script>
`;

/** Mermaid's base theme coloured from the tokens as the page resolved them. */
const MERMAID_SCRIPT = `<script type="module">
import mermaid from "${MERMAID}";
const style = getComputedStyle(document.documentElement);
const token = (name) => style.getPropertyValue(name).trim();
mermaid.initialize({
  startOnLoad: false,
  theme: "base",
  themeVariables: {
    darkMode: document.documentElement.dataset.theme ? document.documentElement.dataset.theme === "dark" : matchMedia("(prefers-color-scheme: dark)").matches,
    fontFamily: token("--font-sans"),
    background: token("--card"),
    primaryColor: token("--card"),
    primaryTextColor: token("--foreground"),
    primaryBorderColor: token("--brand"),
    secondaryColor: token("--secondary"),
    tertiaryColor: token("--background"),
    lineColor: token("--muted-foreground"),
    textColor: token("--foreground"),
    clusterBkg: token("--secondary"),
    clusterBorder: token("--border"),
    edgeLabelBackground: token("--card"),
  },
});
await mermaid.run();
// A diagram is drawn to fit the column unless that shrinks its labels below 0.8 of their size; then it keeps that size
// and scrolls inside its frame.
for (const svg of document.querySelectorAll("pre.mermaid svg")) {
  const natural = svg.viewBox.baseVal?.width;
  const frame = getComputedStyle(svg.parentElement);
  const room = svg.parentElement.clientWidth - parseFloat(frame.paddingLeft) - parseFloat(frame.paddingRight);
  if (!natural || room / natural >= 0.8) continue;
  svg.style.maxWidth = "none";
  svg.style.width = natural * 0.8 + "px";
  // Start in the middle, where a top-down diagram's first nodes are, with the shades showing it continues both ways.
  svg.parentElement.scrollLeft = (svg.parentElement.scrollWidth - svg.parentElement.clientWidth) / 2;
}
try {
  const saved = JSON.parse(sessionStorage.getItem("${RESTORE_KEY}") ?? "null");
  sessionStorage.removeItem("${RESTORE_KEY}");
  if (saved?.path === location.pathname) {
    document.querySelectorAll("details").forEach((fold, i) => { fold.open = saved.open[i] ?? fold.open; });
    scrollTo(0, saved.y);
  }
} catch { /* private mode: nothing was kept */ }
</script>
`;

/** The custom properties `css` uses (`var(--x)`, `getPropertyValue("--x")`) that neither it nor `tokens` declares. */
function undeclaredTokens(tokens, css) {
  const declared = new Set([...`${tokens}\n${css}`.matchAll(/(?<![\w-])(--[\w-]+)\s*:/gu)].map(([, name]) => name));
  const used = [...css.matchAll(/var\(\s*(--[\w-]+)|getPropertyValue\(\s*["'](--[\w-]+)["']/gu)].map(([, name, property]) => name ?? property);
  return [...new Set(used.filter((name) => !declared.has(name)))].sort();
}

/**
 * Search over every page's sections, the DSL reference's cards and grammar productions, from `search-index.json`
 * (built with the site, fetched on first use). Every term must appear in an entry's section, page or text; a hit in
 * the section's title counts most. `/` focuses the box; arrows move, Enter opens, Escape closes.
 */
const SEARCH_SCRIPT = `<script>
(() => {
  const box = document.querySelector(".site-search");
  if (!box) return;
  const input = box.querySelector("input");
  const list = box.querySelector("ol");
  const root = box.dataset.root;
  let index = null;
  let hits = [];
  let active = -1;
  const esc = (text) => text.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);
  const load = async () => { index = index || (await (await fetch(root + "/search-index.json")).json()); };
  const draw = () => {
    list.innerHTML = hits.length
      ? hits.map((hit, i) => '<li' + (i === active ? ' aria-selected="true"' : '') + '><a href="' + root + '/' + hit.u + '"><span>' + esc(hit.s) + '</span><small>' + esc(hit.p) + '</small></a></li>').join("")
      : '<li class="none">No results</li>';
    list.hidden = false;
  };
  const search = async () => {
    await load();
    const terms = input.value.toLowerCase().split(/\\s+/).filter(Boolean);
    if (terms.length === 0) { list.hidden = true; return; }
    hits = index.map((entry) => {
      const s = entry.s.toLowerCase(), p = entry.p.toLowerCase(), t = entry.t.toLowerCase();
      let score = 0;
      for (const term of terms) {
        const inS = s.includes(term), inP = p.includes(term), inT = t.includes(term);
        if (!inS && !inP && !inT) return null;
        score += (inS ? 6 : 0) + (s.startsWith(term) ? 3 : 0) + (inP ? 2 : 0) + (inT ? 1 : 0);
      }
      return { ...entry, score };
    }).filter(Boolean).sort((a, b) => b.score - a.score).slice(0, 8);
    active = hits.length ? 0 : -1;
    draw();
  };
  input.addEventListener("focus", load, { once: true });
  input.addEventListener("input", search);
  input.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (!hits.length) return;
      active = (active + (event.key === "ArrowDown" ? 1 : hits.length - 1)) % hits.length;
      draw();
    } else if (event.key === "Enter" && hits[active]) {
      location.href = root + "/" + hits[active].u;
    } else if (event.key === "Escape") {
      list.hidden = true;
      input.blur();
    }
  });
  document.addEventListener("keydown", (event) => {
    if (event.key === "/" && !/^(input|textarea|select)$/i.test(document.activeElement?.tagName ?? "")) {
      event.preventDefault();
      input.focus();
      input.select();
    }
  });
  document.addEventListener("click", (event) => { if (!box.contains(event.target)) list.hidden = true; });
  // A result on this same page changes only the hash, so the list closes itself.
  list.addEventListener("click", (event) => { if (event.target.closest("a")) list.hidden = true; });
})();
</script>
`;

/** The bar across the top of every page, the DSL reference's too: the docs' name, its main pages, GitHub, theme, search. */
function siteHeader(root, current, pages) {
  const link = (target, label) => `<a href="${root}/${target}"${target === current ? ' aria-current="page"' : ""}>${label}</a>`;
  const nav = [link("index.html", "Docs"), ...NAV.filter(([target]) => pages.has(target)).map(([target, label]) => link(target, label)), `<a href="${REPO}">GitHub</a>`, `<button type="button" class="site-theme" aria-label="Switch between light and dark" title="Switch between light and dark">${THEME_ICONS}</button>`];
  const search = `<div class="site-search" data-root="${root}" role="search"><input type="search" placeholder="Search docs" aria-label="Search the docs (press /)" autocomplete="off"><ol hidden></ol></div>`;
  return `<header class="site-header"><nav><a class="brand" href="${root}/index.html">Rerail docs</a>${nav.join("")}${search}</nav></header>\n${SEARCH_SCRIPT}${THEME_TOGGLE_SCRIPT}`;
}

/** Plain text of an HTML fragment, its whitespace collapsed; inline tags (code, links, emphasis) join their text. */
const textOf = (html) =>
  decodeHtml(html.replace(/<\/?(?:a|b|code|em|i|span|strong|wbr)\b[^>]*>/gu, "").replace(/<[^>]+>/gu, " "))
    .replace(/\s+/gu, " ")
    .trim();

/** How much of a section's text the search reads: enough for the terms deep in a long section, not the whole page. */
const SEARCH_TEXT = 2000;

/** A section's searchable text: its prose, tables and code, without diagram source. */
const sectionText = (html) => textOf(html.replace(/<pre class="mermaid">[\s\S]*?<\/pre>/gu, "")).slice(0, SEARCH_TEXT);

/** A docs page's search entries: the page itself, then each h2 and h3 section with its text (`sectionText`). */
function pageSearchEntries(body, page, title) {
  const parts = body.split(/(<h[23] id="[^"]+">[\s\S]*?<\/h[23]>)/u);
  const entries = [{ p: title, s: title, u: page, t: sectionText(parts[0].replace(/<h1[\s\S]*?<\/h1>/u, "")) }];
  for (let i = 1; i < parts.length; i += 2) {
    const id = /id="([^"]+)"/u.exec(parts[i])[1];
    entries.push({ p: title, s: textOf(parts[i].replace(/<a class="anchor"[^>]*>#<\/a>/u, "")), u: `${page}#${id}`, t: sectionText(parts[i + 1] ?? "") });
  }
  return entries;
}

/** The DSL reference's search entries: its section headings, construct cards and grammar productions. */
function referenceSearchEntries(html, title) {
  const entries = [];
  // Each section's own prose, its cards and example figures left to their own entries.
  const main = html.slice(html.indexOf("<main"), html.lastIndexOf("</main>"));
  const parts = main.split(/(<h[23] id="[^"]+"[^>]*>[\s\S]*?<\/h[23]>)/u);
  for (let i = 1; i < parts.length; i += 2) {
    const id = /id="([^"]+)"/u.exec(parts[i])[1];
    const prose = (parts[i + 1] ?? "").replace(/<section class="card"[\s\S]*?<\/section>|<figure[\s\S]*?<\/figure>/gu, "");
    entries.push({ p: title, s: textOf(parts[i].replace(/<a class="anchor"[^>]*>#<\/a>/u, "")), u: `${REFERENCE}#${id}`, t: sectionText(prose) });
  }
  // A card's text, without its example figures (JSON the reader searches by key, through the grammar entries).
  for (const [, id, heading, card] of html.matchAll(/<section class="card" id="([^"]+)"[^>]*>\s*<h3>([\s\S]*?)<\/h3>([\s\S]*?)<\/section>/gu)) {
    entries.push({ p: title, s: textOf(heading.replace(/<a class="anchor"[^>]*>#<\/a>/u, "")), u: `${REFERENCE}#${id}`, t: sectionText(card.replace(/<figure[\s\S]*?<\/figure>/gu, "")) });
  }
  for (const [, id, name, about] of html.matchAll(/<(?:details|div) class="prod[^"]*" id="(g-[^"]+)"[^>]*>[\s\S]*?<span class="nt">([^<]+)<\/span>[\s\S]*?<span class="about">([\s\S]*?)<\/span>/gu)) {
    entries.push({ p: `${title}: grammar`, s: name, u: `${REFERENCE}#${id}`, t: textOf(about) });
  }
  return entries;
}

/** A long page's outline (its h2 sections), shown beside the text on wide screens; none for a page with fewer than 3. */
function outline(sections) {
  if (sections.length < 3) return "";
  const items = sections.map(({ id, text }) => `<li><a href="#${escapeHtml(id)}">${escapeHtml(text)}</a></li>`).join("");
  return `<nav class="page-outline" aria-label="On this page"><p>On this page</p><ol>${items}</ol></nav>\n${OUTLINE_SCRIPT}`;
}

function page({ title, body, mermaid, copyable, sections }, path, pages) {
  const root = posix.relative(posix.dirname(path), ".") || ".";
  const current = pagePath(path);
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="color-scheme" content="light dark">
${THEME_SCRIPT}<title>${escapeHtml(title)} · Rerail docs</title>
<link rel="alternate" type="text/markdown" href="${root}/${twinPath(current)}">
<link rel="stylesheet" href="${root}/tokens.css">
<style>
${SITE_TOKENS}${STYLE}</style>
</head>
<body>
${siteHeader(root, current, pages)}
<main>
${agentNote(root, current)}${body}</main>
${outline(sections)}<footer class="site-footer"><p><a href="${REPO}/blob/${BRANCH}/docs/${encodeURI(path)}">View source on GitHub</a></p></footer>
${copyable ? COPY_SCRIPT : ""}${mermaid ? MERMAID_SCRIPT : ""}${body.includes("<details") ? UNFOLD_SCRIPT : ""}</body>
</html>
`;
}

/**
 * The DSL reference with the tokens linked after its own <style> and its palette mapped onto them, its Markdown twin
 * named in its head, and the agents' line at the top of its main column.
 */
function themedReference(html, pages) {
  const end = html.indexOf("</style>");
  const body = html.indexOf("<body>");
  const main = html.indexOf('<main id="top">');
  if (end === -1 || body === -1 || main === -1) throw new Error(`docs/${REFERENCE}: no </style>, <body> or <main id="top"> to build around; update scripts/build/docs-site.mjs`);
  const at = end + "</style>".length;
  const open = body + "<body>".length;
  const top = main + '<main id="top">'.length;
  const content = breakCode(labelTableCells(html.slice(top), 5)).replace(/<span class="src-path">([^<]*)<\/span>/gu, (_, path) => `<span class="src-path">${breakAt(path)}</span>`);
  return `${html.slice(0, at)}\n${THEME_SCRIPT}<link rel="alternate" type="text/markdown" href="${twinPath(REFERENCE)}">\n<link rel="stylesheet" href="tokens.css">\n<style>\n${SITE_TOKENS}${HEADER_CSS}${REFERENCE_MAP}${REFERENCE_UNDER_HEADER}</style>${html.slice(at, open)}\n${siteHeader(".", REFERENCE, pages)}${html.slice(open, top)}\n${agentNote(".", REFERENCE)}${content}`;
}

/** A page's Markdown twin, which agents read instead of the HTML: `x.html` → `x.md`. */
const twinPath = (page) => page.replace(/\.html$/u, ".md");

/** `url`, written in docs file `from`, as a Markdown twin links it: another doc's twin, or what the site links. */
function twinUrl(url, from) {
  const result = rewriteUrl(url, from);
  if (result.missing || /^[a-z][a-z0-9+.-]*:/iu.test(result.url)) return result.url ?? url;
  return result.url.replace(/^([^#?]*)\.html(?=$|[#?])/u, "$1.md");
}

/** Docs file `path`'s Markdown as its twin: links to other docs point at their twins, links out of docs/ at GitHub. */
function markdownTwin(markdown, path) {
  let fenced = false;
  return markdown
    .split("\n")
    .map((line) => {
      if (/^\s*(```|~~~)/u.test(line)) fenced = !fenced;
      if (fenced) return line;
      return line
        .replace(/(\]\()([^)\s]+)(\))/gu, (_, open, url, close) => `${open}${twinUrl(url, path)}${close}`)
        .replace(/\b(href|src)="([^"]+)"/gu, (_, name, url) => `${name}="${twinUrl(url, path)}"`);
    })
    .join("\n");
}

const classes = (node) => (node.getAttribute("class") ?? "").split(/\s+/u);

/**
 * The DSL reference as Markdown: its main column without the page furniture (table of contents, buttons, figures'
 * drawings, heading anchors), each grammar production as a heading with its keys as a list
 * (`key?`: type — description), and every example as a fenced JSON block.
 */
function referenceMarkdown(html) {
  const main = html.slice(html.indexOf("<main"), html.lastIndexOf("</main>") + "</main>".length);
  const translators = {
    "svg,button,script,style,nav": { ignore: true },
    a: ({ node, base }) => (classes(node).includes("anchor") ? { ignore: true } : base),
    summary: { prefix: "#### ", surroundingNewlines: 2 },
    details: { surroundingNewlines: 2 },
    pre: ({ node }) =>
      classes(node).includes("ex")
        ? { content: `\`\`\`json\n${node.text.trim()}\n\`\`\``, noEscape: true, recurse: false, surroundingNewlines: 2 }
        : { content: `\`\`\`\n${node.text.replace(/\n$/u, "")}\n\`\`\``, noEscape: true, recurse: false, surroundingNewlines: 2 },
    span: ({ node }) => {
      const kind = classes(node);
      if (kind.includes("key")) return { content: `\`${node.text.trim()}\``, recurse: false, noEscape: true };
      if (kind.includes("t")) return { postprocess: ({ content }) => (content.trim() ? `: ${content.trim()}` : "") };
      if (kind.includes("d") || kind.includes("about")) return { prefix: " — " };
      if (kind.includes("gist")) return { ignore: true };
      if (kind.includes("c")) return { prefix: " (", postfix: ")" };
      return {};
    },
    div: ({ node }) => {
      const kind = classes(node);
      if (kind.includes("value")) return { prefix: "- ", surroundingNewlines: 1 };
      if (kind.includes("shape")) return { content: `\`\`\`\n${node.text.trim()}\n\`\`\``, noEscape: true, recurse: false, surroundingNewlines: 2 };
      if (kind.includes("variant")) return { surroundingNewlines: 1 };
      if (kind.includes("grammar-tools") || kind.includes("figure-wrap")) return { ignore: true };
      return { surroundingNewlines: 1 };
    },
    h3: ({ node, base }) => (classes(node).includes("gsec") ? { prefix: "### ", surroundingNewlines: 2 } : base),
    dt: { prefix: "**", postfix: "**", surroundingNewlines: 2 },
    dd: { surroundingNewlines: 2 },
  };
  return NodeHtmlMarkdown.translate(main, { bulletMarker: "-", maxConsecutiveNewlines: 2 }, translators).replace(/\n{3,}/gu, "\n\n");
}

/** An `$option` reference, which a document may hold anywhere but format, id, revision, options and config. */
const OPTION_REF = {
  type: "object",
  description: "Stands for the value of one of the document's options; resolved before the rest of the document is validated.",
  required: ["$option"],
  properties: {
    $option: { type: "string" },
    cases: { type: "object" },
    default: {},
    escape: { const: "regex" },
    map: { type: "string" },
    join: { type: "string" },
    template: { type: "string" },
    spread: { type: "boolean" },
  },
  additionalProperties: false,
};

/**
 * `schema` with an `$option` reference allowed in every value position, as the zod schema allows it: it resolves the
 * references (`resolveCodeRuleOptions`) before it validates, which a JSON Schema cannot do.
 */
function allowOptionRefs(schema) {
  const ref = { $ref: "#/$defs/OptionRef" };
  const wrap = (node) => ({ anyOf: [walk(node), ref] });
  const walk = (node) => {
    if (node === null || typeof node !== "object" || Array.isArray(node)) return node;
    const out = { ...node };
    if (out.properties) out.properties = Object.fromEntries(Object.entries(out.properties).map(([key, value]) => [key, wrap(value)]));
    if (out.items && typeof out.items === "object") out.items = wrap(out.items);
    if (Array.isArray(out.prefixItems)) out.prefixItems = out.prefixItems.map(wrap);
    if (out.additionalProperties && typeof out.additionalProperties === "object") out.additionalProperties = wrap(out.additionalProperties);
    for (const key of ["anyOf", "oneOf", "allOf"]) if (Array.isArray(out[key])) out[key] = out[key].map(walk);
    if (out.not) out.not = walk(out.not);
    return out;
  };
  const defs = Object.fromEntries(Object.entries(schema.$defs ?? {}).map(([name, def]) => [name, walk(def)]));
  const root = walk({ ...schema, $defs: undefined });
  // A document's own identity is never a reference. (A reference inside a define's options or config is refused when
  // the options resolve, which this schema leaves to the engine.)
  for (const key of ["format", "id", "revision"]) if (root.properties?.[key]) root.properties[key] = root.properties[key].anyOf[0];
  return { ...root, $defs: { ...defs, OptionRef: OPTION_REF } };
}

/** The JSON Schema of a code-rule document, from the zod schema (refinements zod cannot express are left out). */
function ruleSchema() {
  const schema = allowOptionRefs(z.toJSONSchema(codeRuleSchema, { io: "input", unrepresentable: "any", cycles: "ref", reused: "ref" }));
  return `${JSON.stringify({
    $id: `${SITE}/${SCHEMA}`,
    title: "Rerail code rule (rerail.policy/v1)",
    description: `A Rerail code-rule document. Generated from src/engine/code-rule/code-rule-schema.ts; the engine also enforces rules this schema cannot express, documented in ${SITE}/${REFERENCE}.`,
    ...schema,
  }, null, 2)}\n`;
}

/**
 * llms.txt (llmstxt.org): what Rerail is, then every doc's Markdown twin with its one-line description, in the
 * sections docs/README.md lists them under; docs README does not list go under "Optional".
 */
function llmsTxt(readme, titles) {
  const listed = new Set();
  const sections = [];
  let section = null;
  for (const block of readme.split(/\n(?=- |## )/u)) {
    const heading = /^## (.+)/u.exec(block);
    if (heading) {
      section = { title: heading[1].trim(), items: [] };
      sections.push(section);
      continue;
    }
    const item = /^- \[[^\]]+\]\(([^)]+)\):\s*([\s\S]*)/u.exec(block);
    if (item === null || section === null || item[1].startsWith("..") || unpublished(item[1])) continue;
    const page = pagePath(item[1]);
    listed.add(page);
    const description = item[2].replace(/\s+/gu, " ").replace(/\[([^\]]+)\]\([^)]+\)/gu, "$1").trim();
    section.items.push(`- [${titles.get(page) ?? item[1]}](${SITE}/${twinPath(page)}): ${description}`);
  }
  const rest = [...titles.keys()].filter((page) => page !== "index.html" && !listed.has(page)).sort();
  return [
    "# Rerail",
    "",
    "> Rerail checks the code a coding agent writes against the repository's rules while the agent works, and steers it to fix what breaks them. Rules are rerail.policy/v1 JSON documents.",
    "",
    `Every page of these docs has a Markdown twin at the same path with .md in place of .html. A code rule's JSON Schema is ${SITE}/${SCHEMA}; the full language is in the [DSL reference](${SITE}/${twinPath(REFERENCE)}).`,
    "",
    ...sections.filter((s) => s.items.length > 0).flatMap((s) => [`## ${s.title}`, "", ...s.items, ""]),
    "## Optional",
    "",
    `- [All docs in one file](${SITE}/llms-full.txt): every page's Markdown twin, concatenated`,
    ...rest.map((page) => `- [${titles.get(page)}](${SITE}/${twinPath(page)})`),
    "",
  ].join("\n");
}

/** The line that tells agents where the page's Markdown twin is, at the top of every page. */
const agentNote = (root, page) =>
  `<p class="agent-note">For AI agents: <a href="${root}/${twinPath(page)}">this page as Markdown</a> · <a href="${root}/llms.txt">llms.txt</a> · <a href="${root}/${SCHEMA}">JSON Schema</a></p>\n`;

const tokens = readFileSync(TOKENS, "utf8");
const undeclared = undeclaredTokens(tokens, `${SITE_TOKENS}${HEADER_CSS}${STYLE}${REFERENCE_MAP}${COPY_SCRIPT}${MERMAID_SCRIPT}${OUTLINE_SCRIPT}${UNFOLD_SCRIPT}`);
if (undeclared.length > 0) {
  process.stderr.write(`docs-site: the site's CSS uses custom properties neither ${relative(ROOT, TOKENS)} nor its own CSS declares: ${undeclared.join(", ")}\n`);
  process.exit(1);
}

const stale = UNPUBLISHED.filter((entry) => !existsSync(join(DOCS, ...entry.split("/"))));
if (stale.length > 0) {
  process.stderr.write(`docs-site: UNPUBLISHED names paths docs/ does not have: ${stale.join(", ")}\n`);
  process.exit(1);
}

const files = walk(DOCS).filter((path) => !unpublished(path));
const pages = new Set(files.map((path) => (path.endsWith(".md") ? pagePath(path) : path)));
const outputs = new Map([["tokens.css", tokens]]);
const copies = [];
const missing = [];
const titles = new Map([[REFERENCE, "Rerail code-rule DSL reference"]]);
const searchIndex = [];
for (const path of files) {
  if (path === REFERENCE) {
    const html = readFileSync(join(DOCS, path), "utf8");
    outputs.set(path, themedReference(html, pages));
    outputs.set(twinPath(path), `${referenceMarkdown(html)}\n`);
    searchIndex.push(...referenceSearchEntries(html, titles.get(REFERENCE)));
  } else if (!path.endsWith(".md")) copies.push(path);
  else {
    const markdown = readFileSync(join(DOCS, path), "utf8");
    const rendered = render(markdown, path);
    for (const url of rendered.missing) missing.push(`docs/${path}: ${url}`);
    const title = rendered.title ?? path;
    titles.set(pagePath(path), title);
    outputs.set(pagePath(path), page({ ...rendered, title }, path, pages));
    outputs.set(twinPath(pagePath(path)), markdownTwin(markdown, path));
    searchIndex.push(...pageSearchEntries(rendered.body, pagePath(path), title));
  }
}
outputs.set("search-index.json", JSON.stringify(searchIndex));
outputs.set(SCHEMA, ruleSchema());
outputs.set("llms.txt", llmsTxt(readFileSync(join(DOCS, "README.md"), "utf8"), titles));
outputs.set(
  "llms-full.txt",
  [...titles.keys()]
    .sort((a, b) => (a === "index.html" ? -1 : b === "index.html" ? 1 : a.localeCompare(b)))
    .map((page) => `<!-- ${SITE}/${twinPath(page)} -->\n\n${outputs.get(twinPath(page)).trim()}\n`)
    .join("\n\n"),
);

if (missing.length > 0) {
  process.stderr.write(`docs-site: relative links to files that do not exist:\n${missing.map((line) => `  ${line}`).join("\n")}\n`);
  process.exit(1);
}

rmSync(OUT, { recursive: true, force: true });
for (const [path, html] of outputs) {
  mkdirSync(dirname(join(OUT, path)), { recursive: true });
  writeFileSync(join(OUT, path), html);
}
for (const path of copies) {
  mkdirSync(dirname(join(OUT, path)), { recursive: true });
  cpSync(join(DOCS, path), join(OUT, path));
}
process.stdout.write(`docs-site: ${outputs.size} pages and stylesheets written and ${copies.length} files copied to ${relative(ROOT, OUT)}\n`);
