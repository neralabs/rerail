// Normalized SFC structure, shared by the Vue pack's offline generators (never CI).
//
// Every reader below turns one source into the same comparable items, so a candidate tree-sitter grammar, Vue's own
// compiler and vue-eslint-parser can be compared item for item. Offsets are UTF-16 code-unit indices into the source
// (JavaScript string indices, as both references report them); grammar byte offsets are converted.
//
// Items (strings, compared as multisets):
//   block  `block|<type>|<contentStart>|<contentEnd>`              each top-level SFC block, `script setup` included
//   el     `el|<tag>|<start>|<end>|<parentStart>`                  an element of the template; `end` is the end of its
//                                                                   end tag, or of its start tag when it has none
//   attr   `attr|<name>|<nameStart>|<nameEnd>|<valueStart>|<valueEnd>`  a static attribute; value with its quotes
//   dir    `dir|<name>|<arg>|<static>|<modifiers>|<start>|<end>`   a directive, by Vue's name (`bind` for `:` and
//                                                                   `v-bind`, `on` for `@`, `slot` for `#`)
//   exp    `exp|<start>|<end>`                                      a directive's expression, inside its quotes
//   interp `interp|<start>|<end>|<contentStart>|<contentEnd>`      an interpolation; content without its whitespace
//
// A directive's expression is present only when it holds more than whitespace, on every side.

export const CATEGORIES = ["block", "el", "attr", "dir", "exp", "interp"];

const SHORTHANDS = { ":": "bind", "@": "on", "#": "slot", ".": "bind" };

function trimmed(source, start, end) {
  while (start < end && /\s/.test(source[start])) start++;
  while (end > start && /\s/.test(source[end - 1])) end--;
  return [start, end];
}

function expression(source, start, end) {
  const [s, e] = trimmed(source, start, end);
  return s < e ? `exp|${start}|${end}` : undefined;
}

function unquoted(source, start, end) {
  const quote = source[start];
  if ((quote === '"' || quote === "'") && end - start >= 2 && source[end - 1] === quote) return [start + 1, end - 1];
  return [start, end];
}

/** A byte offset → UTF-16 index converter for `source`. */
export function byteIndex(source) {
  const bytes = Buffer.byteLength(source, "utf8");
  if (bytes === source.length) return offset => offset;
  const map = new Int32Array(bytes + 1);
  let byte = 0;
  for (let index = 0; index < source.length; index++) {
    const code = source.codePointAt(index);
    const width = code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
    for (let k = 0; k < width; k++) map[byte + k] = index;
    byte += width;
    if (code >= 0x10000) index++;
  }
  map[bytes] = source.length;
  return offset => map[offset];
}

// --- @vue/compiler-sfc --------------------------------------------------------------------------------------------

/** Items from @vue/compiler-sfc's descriptor and the template AST it parses (compiler-dom, `parseMode: "sfc"`). */
export function compilerItems(source, descriptor) {
  const items = [];
  const blocks = [descriptor.template, descriptor.script, descriptor.scriptSetup, ...descriptor.styles,
    ...descriptor.customBlocks].filter(Boolean);
  for (const block of blocks) items.push(`block|${block.type}|${block.loc.start.offset}|${block.loc.end.offset}`);
  const template = descriptor.template;
  if (!template?.ast || (template.lang && template.lang !== "html")) return { items, template: false };
  const walk = (node, parentStart) => {
    if (node.type === 1) {
      items.push(`el|${node.tag}|${node.loc.start.offset}|${node.loc.end.offset}|${parentStart}`);
      for (const prop of node.props) {
        if (prop.type === 6) {
          const value = prop.value ? `${prop.value.loc.start.offset}|${prop.value.loc.end.offset}` : "|";
          items.push(`attr|${prop.name}|${prop.nameLoc.start.offset}|${prop.nameLoc.end.offset}|${value}`);
        } else {
          const arg = prop.arg ? `${prop.arg.content}|${prop.arg.isStatic}` : "|";
          // Vue adds `prop` for the `.name` shorthand, `camel` for nothing else at parse time.
          const modifiers = prop.modifiers.map(m => (typeof m === "string" ? m : m.content)).join(".");
          items.push(`dir|${prop.name}|${arg}|${modifiers}|${prop.loc.start.offset}|${prop.loc.end.offset}`);
          if (prop.exp) {
            const exp = expression(source, prop.exp.loc.start.offset, prop.exp.loc.end.offset);
            if (exp) items.push(exp);
          }
        }
      }
      for (const child of node.children) walk(child, node.loc.start.offset);
    } else if (node.type === 5) {
      const [s, e] = trimmed(source, node.content.loc.start.offset, node.content.loc.end.offset);
      items.push(`interp|${node.loc.start.offset}|${node.loc.end.offset}|${s}|${e}`);
    } else if (node.type === 0) {
      for (const child of node.children) walk(child, -1);
    }
  };
  walk(template.ast, -1);
  return { items, template: true };
}

// --- vue-eslint-parser -----------------------------------------------------------------------------------------

/** Items from vue-eslint-parser's `templateBody` (the HTML template only; it does not split other blocks). */
export function eslintItems(source, templateBody) {
  const items = [];
  if (!templateBody) return { items, template: false };
  const walk = (node, parentStart) => {
    if (node.type === "VElement") {
      const end = node.endTag ? node.endTag.range[1] : node.startTag.range[1];
      if (parentStart !== null) items.push(`el|${node.rawName}|${node.range[0]}|${end}|${parentStart}`);
      for (const attribute of node.startTag.attributes) {
        if (!attribute.directive) {
          const value = attribute.value ? `${attribute.value.range[0]}|${attribute.value.range[1]}` : "|";
          items.push(`attr|${attribute.key.rawName}|${attribute.key.range[0]}|${attribute.key.range[1]}|${value}`);
          continue;
        }
        const key = attribute.key;
        let arg = "|";
        if (key.argument?.type === "VIdentifier") arg = `${key.argument.rawName}|true`;
        else if (key.argument?.type === "VExpressionContainer") {
          arg = `${source.slice(key.argument.range[0] + 1, key.argument.range[1] - 1)}|false`;
        }
        const modifiers = key.modifiers.map(m => m.rawName);
        if (key.name.rawName === ".") modifiers.unshift("prop");
        items.push(`dir|${key.name.name}|${arg}|${modifiers.join(".")}|${attribute.range[0]}|${attribute.range[1]}`);
        if (attribute.value) {
          const [s, e] = unquoted(source, attribute.value.range[0], attribute.value.range[1]);
          const exp = expression(source, s, e);
          if (exp) items.push(exp);
        }
      }
      for (const child of node.children) walk(child, parentStart === null ? -1 : node.range[0]);
    } else if (node.type === "VExpressionContainer") {
      const [s, e] = trimmed(source, node.range[0] + 2, node.range[1] - 2);
      items.push(`interp|${node.range[0]}|${node.range[1]}|${s}|${e}`);
    }
  };
  walk(templateBody, null);
  return { items, template: true };
}

// --- tree-sitter grammars ------------------------------------------------------------------------------------------

const ELEMENT_KINDS = new Set(["element", "template_element", "script_element", "style_element", "vue_component",
  "suspense"]);
const ROOT_KINDS = new Set(["component", "document"]);

/**
 * Items from a tree-sitter-vue tree as `rerail-vue-grammar-dump` prints it (`[kind, start, end, parent, field, flags]`,
 * byte offsets). The three candidate grammars name the same constructs differently (`directive_argument` or
 * `directive_value`; `directive_name` or an anonymous `:`/`@`/`#`/`.`); this reads all of them.
 */
export function grammarItems(source, nodes, templateContentStart) {
  const at = byteIndex(source);
  const children = nodes.map(() => []);
  nodes.forEach((node, index) => {
    if (node[3] >= 0) children[node[3]].push(index);
  });
  const kind = index => nodes[index][0];
  const start = index => at(nodes[index][1]);
  const end = index => at(nodes[index][2]);
  const text = index => source.slice(start(index), end(index));
  const named = index => !(nodes[index][5] & 8);
  const child = (index, ...kinds) => children[index].find(c => kinds.includes(kind(c)));
  const tagOf = index => child(index, "start_tag", "self_closing_tag");
  const tagName = index => {
    const tag = tagOf(index);
    const name = tag === undefined ? undefined : child(tag, "tag_name");
    return name === undefined ? undefined : text(name);
  };
  const items = [];
  const root = nodes.findIndex(node => node[3] === -1);
  let template;
  for (const block of children[root] ?? []) {
    if (!ELEMENT_KINDS.has(kind(block))) continue;
    const name = tagName(block);
    const open = tagOf(block);
    if (name === undefined || open === undefined) continue;
    const close = child(block, "end_tag");
    const contentStart = end(open);
    const contentEnd = close === undefined ? end(block) : start(close);
    // @vue/compiler-sfc drops a block other than <template> whose content is only whitespace and that has no `src`
    // (`ignoreEmpty`, its default): what a block is, not how it parses, so the view does the same.
    const hasSrc = children[open].some(a => kind(a) === "attribute" && child(a, "attribute_name") !== undefined
      && text(child(a, "attribute_name")) === "src");
    if (name !== "template" && !hasSrc && source.slice(contentStart, contentEnd).trim() === "") continue;
    const type = name;
    items.push(`block|${type}|${contentStart}|${contentEnd}`);
    if (name === "template" && contentStart === templateContentStart) template = block;
  }
  if (template === undefined) return { items, template: false };
  const attributes = (tag, out) => {
    for (const attribute of children[tag]) {
      const k = kind(attribute);
      if (k === "attribute") {
        const name = child(attribute, "attribute_name");
        const value = child(attribute, "quoted_attribute_value", "attribute_value");
        const v = value === undefined ? "|" : `${start(value)}|${end(value)}`;
        if (name !== undefined) out.push(`attr|${text(name)}|${start(name)}|${end(name)}|${v}`);
      } else if (k === "script_lang" || k === "style_lang" || k === "props") {
        // xiaoxin-sky's grammar names `lang="…"` (and `props`) as nodes of their own.
        const raw = text(attribute);
        const equals = raw.indexOf("=");
        const nameEnd = start(attribute) + (equals < 0 ? raw.length : equals);
        const v = equals < 0 ? "|" : `${nameEnd + 1}|${end(attribute)}`;
        out.push(`attr|${raw.slice(0, nameEnd - start(attribute))}|${start(attribute)}|${nameEnd}|${v}`);
      } else if (k === "directive_attribute") {
        const nameNode = children[attribute].find(c => kind(c) === "directive_name"
          || (!named(c) && SHORTHANDS[kind(c)] !== undefined && start(c) === start(attribute)));
        const raw = nameNode === undefined ? "" : text(nameNode);
        const name = SHORTHANDS[raw] ?? raw.replace(/^v-/, "");
        const staticArg = child(attribute, "directive_argument", "directive_value");
        const dynamicArg = child(attribute, "directive_dynamic_argument", "dynamic_directive_value");
        let arg = "|";
        if (staticArg !== undefined) arg = `${text(staticArg)}|true`;
        else if (dynamicArg !== undefined) {
          const inner = child(dynamicArg, "directive_dynamic_argument_value", "dynamic_directive_inner_value");
          arg = `${inner === undefined ? "" : text(inner)}|false`;
        }
        const modifierList = child(attribute, "directive_modifiers");
        const modifiers = modifierList === undefined ? []
          : children[modifierList].filter(c => kind(c) === "directive_modifier").map(text);
        if (raw === ".") modifiers.unshift("prop");
        out.push(`dir|${name}|${arg}|${modifiers.join(".")}|${start(attribute)}|${end(attribute)}`);
        const quoted = child(attribute, "quoted_attribute_value");
        if (quoted !== undefined) {
          const [s, e] = unquoted(source, start(quoted), end(quoted));
          const exp = expression(source, s, e);
          if (exp) out.push(exp);
        } else {
          const value = child(attribute, "attribute_value");
          if (value !== undefined) {
            const exp = expression(source, start(value), end(value));
            if (exp) out.push(exp);
          }
        }
      }
    }
  };
  const walk = (index, parentStart) => {
    for (const c of children[index]) {
      const k = kind(c);
      if (ELEMENT_KINDS.has(k)) {
        const tag = tagOf(c);
        const name = tagName(c);
        if (tag === undefined || name === undefined) {
          walk(c, parentStart);
          continue;
        }
        const close = child(c, "end_tag");
        items.push(`el|${name}|${start(c)}|${close === undefined ? end(tag) : end(close)}|${parentStart}`);
        attributes(tag, items);
        walk(c, start(c));
      } else if (k === "interpolation") {
        const raw = child(c, "raw_text");
        const [s, e] = raw === undefined ? [start(c) + 2, start(c) + 2] : trimmed(source, start(raw), end(raw));
        items.push(`interp|${start(c)}|${end(c)}|${s}|${e}`);
      } else if (k === "ERROR" || ROOT_KINDS.has(k)) {
        walk(c, parentStart);
      }
    }
  };
  walk(template, -1);
  return { items, template: true };
}

/**
 * First innermost ERROR or MISSING node of a dumped tree (one with no error node inside it), as
 * `{ kind, start, end, index }` with UTF-16 offsets, or undefined. A recovering grammar often wraps the whole file in
 * an ERROR; the construct it could not read is the innermost one.
 */
export function firstError(source, nodes) {
  const at = byteIndex(source);
  const hasErrorInside = new Uint8Array(nodes.length);
  for (let index = nodes.length - 1; index >= 0; index--) {
    const parent = nodes[index][3];
    if (parent >= 0 && (hasErrorInside[index] || nodes[index][5] & 3)) hasErrorInside[parent] = 1;
  }
  let best;
  nodes.forEach((node, index) => {
    if (!(node[5] & 3) || hasErrorInside[index]) return;
    if (best === undefined || node[1] < nodes[best][1]) best = index;
  });
  if (best === undefined) return undefined;
  const node = nodes[best];
  return { kind: node[5] & 2 ? `MISSING ${node[0]}` : "ERROR", start: at(node[1]), end: at(node[2]), index: best };
}

/** Multiset intersection size per category: `{ category: { reference, candidate, matched } }`. */
export function agreement(reference, candidate) {
  const counts = new Map();
  for (const item of reference) counts.set(item, (counts.get(item) ?? 0) + 1);
  const result = Object.fromEntries(CATEGORIES.map(c => [c, { reference: 0, candidate: 0, matched: 0 }]));
  for (const item of reference) result[item.slice(0, item.indexOf("|"))].reference++;
  const unmatched = [];
  for (const item of candidate) {
    const category = item.slice(0, item.indexOf("|"));
    result[category].candidate++;
    const left = counts.get(item) ?? 0;
    if (left > 0) {
      counts.set(item, left - 1);
      result[category].matched++;
    } else unmatched.push(item);
  }
  const missed = [];
  for (const [item, left] of counts) for (let k = 0; k < left; k++) missed.push(item);
  return { result, missed, unmatched };
}
