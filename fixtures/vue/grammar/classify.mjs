// Classifies a candidate grammar's syntax-error file (on a file @vue/compiler-sfc accepts) by the construct at its
// first innermost ERROR or MISSING node (`sfc.mjs` `firstError`). Each cause is a grammar gap: Vue reads the file,
// the grammar does not. A cause that matches no class stays `unclassified: …` with its context, and fails T1.

import { byteIndex } from "./sfc.mjs";

/** The top-level block holding `offset`, by the reference's descriptor items (content range plus its tags). */
function blockAt(ref, source, offset) {
  for (const item of ref.compiler.items) {
    if (!item.startsWith("block|")) continue;
    const [, type, start, end] = item.split("|");
    const open = source.lastIndexOf("<", Number(start));
    const close = source.indexOf(">", Number(end));
    if (offset >= open && offset <= close) return type;
  }
  return undefined;
}

/** Tag name of the nearest element (start tag, self-closing tag or element) enclosing node `index`. */
function enclosingTag(source, nodes, index) {
  const at = byteIndex(source);
  for (let i = nodes[index][3]; i >= 0; i = nodes[i][3]) {
    if (!/(^|_)(start_tag|self_closing_tag)$/.test(nodes[i][0])) continue;
    const match = /^<\s*([^\s/>]+)/.exec(source.slice(at(nodes[i][1]), at(nodes[i][2])));
    if (match) return match[1];
  }
  return undefined;
}

export function classify(source, error, ref, nodes) {
  const at = error.start;
  const block = blockAt(ref, source, at);
  const text = source.slice(at, Math.max(error.end, at + 1));
  const tag = enclosingTag(source, nodes, error.index);
  const before = source.slice(Math.max(0, at - 600), at);
  if (block === undefined) {
    if (text.trim() !== "" && !text.trimStart().startsWith("<")) return "top-level text outside SFC blocks (Vue ignores it)";
  }
  if (block !== undefined && !["template", "script", "scriptSetup", "style"].includes(block)) {
    return `custom block <${block}> content read as HTML`;
  }
  // xiaoxin-sky's grammar reserves `<component>` for Vue's dynamic component and refuses its usual attributes; the
  // error lands inside the tag or right after it closes.
  if (tag === "component" || /<component\b[^<]*<?$/.test(before) || /^<?component\b/.test(text)) {
    return "<component> (dynamic component) element";
  }
  return `unclassified: ${block ?? "outside blocks"} <${tag ?? "?"}>: ${JSON.stringify(text.slice(0, 60))} after ${JSON.stringify(before.slice(-60))}`;
}

// HTML void elements as tree-sitter-html's scanner lists them (case-insensitively); Vue's compiler-dom compares a tag
// with its void list case-sensitively, so `<Col>` or `<Input>` is a component with children there.
const VOID = new Set(["area", "base", "basefont", "bgsound", "br", "col", "command", "embed", "frame", "hr", "image",
  "img", "input", "isindex", "keygen", "link", "menuitem", "meta", "nextid", "param", "source", "track", "wbr"]);

function itemClass(source, item) {
  const parts = item.split("|");
  const kind = parts[0];
  if (kind === "el" || kind === "attr") {
    const name = parts[1];
    if (name.includes(".")) return "dotted component tag (<A.B>) split at the dot";
    if (kind === "el") {
      if (VOID.has(name.toLowerCase()) && name !== name.toLowerCase()) return "PascalCase component named like an HTML void element read as void";
      const parent = /^<([^\s/>]+)/.exec(source.slice(Number(parts[4])))?.[1];
      if (parent && VOID.has(parent.toLowerCase()) && parent !== parent.toLowerCase()) {
        return "PascalCase component named like an HTML void element read as void";
      }
      // xiaoxin-sky's grammar keeps no element for Vue's <Suspense>; its children hang off the outer element.
      if (name === "Suspense" || parent === "Suspense") return "Vue built-in <Suspense> kept as no element";
    }
  }
  if (kind === "dir") {
    const text = source.slice(Number(parts[5]), Number(parts[6]));
    if (/^(?:v-[\w-]+:|[:@#.])[^\s="'/>[]*:/.test(text)) return "directive argument holding ':' split at the colon";
    if (/^(?:v-bind:|[:.])[\w-]+(?:\s|\/?>|$)/.test(text)) return "valueless v-bind shorthand (:name) absorbs the next attribute";
  }
  return undefined;
}

/**
 * Classes of the items a grammar and the reference disagree on in one file the grammar parses cleanly, as
 * `[class, item, side]`. An item with no class of its own takes the class of an item on either side at the same
 * kind, name and start (for a directive: the same end), the construct the grammar misreads; an item no rule explains
 * is `unexplained <kind>`.
 */
export function mismatchClasses(source, missed, unmatched) {
  const keyOf = item => {
    const parts = item.split("|");
    if (parts[0] === "dir") return `dir|${parts[6]}`;
    return parts[0] === "el" ? `el|${parts[2]}` : `${parts[0]}|${parts[1]}|${parts[2]}`;
  };
  const items = [...missed.map(item => [item, "reference"]), ...unmatched.map(item => [item, "grammar"])];
  const own = items.map(([item]) => itemClass(source, item));
  const byKey = new Map();
  items.forEach(([item], index) => {
    if (own[index] !== undefined && !byKey.has(keyOf(item))) byKey.set(keyOf(item), own[index]);
  });
  return items.map(([item, side], index) =>
    [own[index] ?? byKey.get(keyOf(item)) ?? `unexplained ${item.split("|")[0]}`, item, side]);
}
