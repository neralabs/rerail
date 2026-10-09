// Writes ui/tokens.generated.css: Nera's design tokens (a nera-landing checkout's web/src/index.css and fonts.css)
// projected into a standalone stylesheet for ui/. Colours are flattened to hex so the page needs no runtime
// colour-mix support; bare var() indirections are preserved so aliases stay aliases. Run offline, not in CI:
//   node scripts/generate/ui/tokens.mjs <nera-landing checkout>
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const landing = process.argv[2];
if (landing === undefined) throw new Error("usage: node scripts/generate/ui/tokens.mjs <nera-landing checkout>");

// The page ships no font files, so the SF Pro entry at the head of every stack is dropped
// and the system stack is emitted as a literal (decision Q6a).
const SYSTEM_STACK_TOKENS = new Set(["--font-sans", "--font-family-sans", "--default-font-family"]);

const stripComments = css => css.replace(/\/\*[\s\S]*?(?:\*\/|$)/gu, "");

// Blocks are matched flat: a nested `{` (e.g. a media query or a nested rule inside :root)
// would otherwise be truncated at the first `}` and silently drop the declarations after it.
function block(css, head) {
  const body = css.match(new RegExp(`^[^\\S\\r\\n]*${head}\\s*\\{([^}]*)\\}`, "mu"))?.[1];
  if (body === undefined) throw new Error(`Expected a top-level \`${head} { … }\` block`);
  if (body.includes("{")) throw new Error(`\`${head} { … }\` contains a nested block; flatten it before regenerating tokens`);
  return body;
}

// Custom-property declarations only, in source order. Later duplicates overwrite the value
// but keep the first position, mirroring the cascade.
function declarations(body, into = new Map()) {
  for (const raw of body.split(";")) {
    const match = raw.match(/^\s*(--[\w-]+)\s*:([\s\S]*)$/u);
    if (!match) continue;
    const value = match[2].trim().replace(/\s+/gu, " ");
    if (value !== "") into.set(match[1], value);
  }
  return into;
}

// Split `a, b(c, d), e` at top-level commas.
function splitArgs(text) {
  const parts = [];
  let depth = 0;
  let start = 0;
  for (let index = 0; index < text.length; index += 1) {
    const char = text[index];
    if (char === "(") depth += 1;
    else if (char === ")") depth -= 1;
    else if (char === "," && depth === 0) {
      parts.push(text.slice(start, index).trim());
      start = index + 1;
    }
  }
  parts.push(text.slice(start).trim());
  return parts;
}

// Index of the `)` matching the `(` at `open`.
function closingParen(text, open) {
  let depth = 0;
  for (let index = open; index < text.length; index += 1) {
    if (text[index] === "(") depth += 1;
    else if (text[index] === ")" && (depth -= 1) === 0) return index;
  }
  throw new Error(`Unbalanced parentheses in \`${text}\``);
}

const clamp = value => Math.min(255, Math.max(0, Math.round(value)));

function hex([r, g, b, a]) {
  const channel = value => clamp(value).toString(16).padStart(2, "0");
  const alpha = clamp(a * 255);
  return `#${channel(r)}${channel(g)}${channel(b)}${alpha === 255 ? "" : channel(alpha)}`;
}

function parseHex(text) {
  const digits = text.slice(1);
  const wide = digits.length >= 6 ? digits : [...digits].map(d => d + d).join("");
  const parts = [0, 2, 4].map(offset => Number.parseInt(wide.slice(offset, offset + 2), 16));
  const alpha = wide.length === 8 ? Number.parseInt(wide.slice(6, 8), 16) / 255 : 1;
  return [...parts, alpha];
}

function parseHsl(args) {
  // `h s% l% / a` (modern) or `h, s%, l%[, a]` (legacy); hue may carry `deg`.
  const tokens = args.replace(/\//gu, " ").replace(/,/gu, " ").trim().split(/\s+/u);
  const [h, s, l, a = "1"] = tokens;
  const hue = ((Number.parseFloat(h) % 360) + 360) % 360;
  const sat = Number.parseFloat(s) / 100;
  const light = Number.parseFloat(l) / 100;
  const alpha = a.endsWith("%") ? Number.parseFloat(a) / 100 : Number.parseFloat(a);
  const chroma = (1 - Math.abs(2 * light - 1)) * sat;
  const x = chroma * (1 - Math.abs(((hue / 60) % 2) - 1));
  const m = light - chroma / 2;
  const sector = Math.floor(hue / 60);
  const [r, g, b] = [[chroma, x, 0], [x, chroma, 0], [0, chroma, x], [0, x, chroma], [x, 0, chroma], [chroma, 0, x]][sector];
  return [(r + m) * 255, (g + m) * 255, (b + m) * 255, alpha];
}

const NAMED = { transparent: [0, 0, 0, 0], black: [0, 0, 0, 1], white: [255, 255, 255, 1] };

// Parse a colour expression to [r, g, b, a]. `lookup` resolves var() references.
function parseColor(text, lookup) {
  const value = text.trim();
  if (value.startsWith("#")) return parseHex(value);
  if (value in NAMED) return NAMED[value];
  const fn = value.match(/^([a-z-]+)\(/u)?.[1];
  const args = fn ? value.slice(fn.length + 1, closingParen(value, fn.length)) : "";
  if (fn === "var") return parseColor(lookup(args.trim()), lookup);
  if (fn === "hsl" || fn === "hsla") return parseHsl(args);
  if (fn === "color-mix") return mix(args, lookup);
  throw new Error(`Unsupported colour \`${value}\``);
}

// color-mix(in srgb, A p%, B q%) with premultiplied alpha, as the CSS spec mixes.
function mix(args, lookup) {
  const [space, first, second] = splitArgs(args);
  if (space !== "in srgb") throw new Error(`Only \`in srgb\` mixing is supported, got \`${space}\``);
  const weighted = arg => {
    const match = arg.match(/^([\s\S]*\S)\s+(\d+(?:\.\d+)?)%$/u);
    return match ? [parseColor(match[1], lookup), Number.parseFloat(match[2]) / 100] : [parseColor(arg, lookup), undefined];
  };
  let [[a, pa], [b, pb]] = [weighted(first), weighted(second)];
  if (pa === undefined && pb === undefined) [pa, pb] = [0.5, 0.5];
  else if (pa === undefined) pa = 1 - pb;
  else if (pb === undefined) pb = 1 - pa;
  const total = pa + pb;
  const [wa, wb] = [pa / total, pb / total];
  const alpha = a[3] * wa + b[3] * wb;
  const channel = index => (alpha === 0 ? 0 : (a[index] * a[3] * wa + b[index] * b[3] * wb) / alpha);
  return [channel(0), channel(1), channel(2), alpha * Math.min(1, total)];
}

// Replace every color-mix()/hsl()/hsla() occurrence inside a value with its hex form.
function flattenColors(value, lookup) {
  let output = value;
  for (;;) {
    const match = output.match(/\b(color-mix|hsla?)\(/u);
    if (!match) return output;
    const open = match.index + match[1].length;
    const close = closingParen(output, open);
    output = output.slice(0, match.index) + hex(parseColor(output.slice(match.index, close + 1), lookup)) + output.slice(close + 1);
  }
}

// calc(var(--x) ± Npx) where --x is a px literal.
function flattenCalc(value, lookup) {
  return value.replace(/calc\(\s*var\((--[\w-]+)\)\s*([+-])\s*(\d+(?:\.\d+)?)px\s*\)/gu, (_, name, sign, delta) => {
    const base = lookup(name).match(/^(\d+(?:\.\d+)?)px$/u)?.[1];
    if (base === undefined) throw new Error(`${name} must be a px literal to resolve calc()`);
    return `${Number.parseFloat(base) + (sign === "-" ? -1 : 1) * Number.parseFloat(delta)}px`;
  });
}

function resolveVars(value, lookup) {
  const match = value.match(/^var\((--[\w-]+)\)$/u);
  return match ? resolveVars(lookup(match[1]), lookup) : value;
}

const stripSfPro = value => splitArgs(value).filter(entry => !/^["']SF Pro["']$/u.test(entry)).join(", ");

function project(tokens, fallback) {
  const lookup = name => {
    const value = tokens.get(name) ?? fallback?.get(name);
    if (value === undefined) throw new Error(`Unknown token ${name}`);
    return value;
  };
  const output = new Map();
  for (const [name, raw] of tokens) {
    let value = SYSTEM_STACK_TOKENS.has(name) ? resolveVars(raw, lookup) : raw;
    value = stripSfPro(flattenCalc(flattenColors(value, lookup), lookup));
    output.set(name, value);
  }
  return output;
}

const body = (tokens, indent) => [...tokens].map(([name, value]) => `${indent}${name}: ${value};`).join("\n");

const indexSource = readFileSync(join(landing, "web/src/index.css"), "utf8");
const fontsSource = readFileSync(join(landing, "web/src/fonts.css"), "utf8");
const index = stripComments(indexSource);
const fonts = stripComments(fontsSource);

// Source order by import position: fonts.css, then @theme, :root and @theme inline.
const light = declarations(block(fonts, ":root"));
declarations(block(index, "@theme"), light);
declarations(block(index, ":root"), light);
for (const [name, value] of declarations(block(index, "@theme inline"))) {
  if (name.startsWith("--radius-")) light.set(name, value);
}
const lightTokens = project(light);
const darkTokens = project(declarations(block(index, "\\.dark")), light);

const dark = body(darkTokens, "    ");
const output = [
  "/* Generated by scripts/generate/ui/tokens.mjs; do not edit. Source: nera-landing web/src/index.css and fonts.css. */",
  ":root {",
  body(lightTokens, "  "),
  "}",
  "@media (prefers-color-scheme: dark) {",
  '  :root:not([data-theme="light"]) {',
  dark,
  "  }",
  "}",
  ':root[data-theme="dark"] {',
  body(darkTokens, "  "),
  "}",
  ".dark {",
  body(darkTokens, "  "),
  "}",
  "",
].join("\n");

writeFileSync(join(import.meta.dirname, "../../../ui/tokens.generated.css"), output);
