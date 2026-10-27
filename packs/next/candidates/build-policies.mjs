// Builds the Next pack's policies: packs/next/rules/next-<rule>.json (ported rules) and the `rules` lists of
// packs/next/conformance.json and packs/next/pack.json; packs/next/candidates/rules/next-<rule>.json (rules that cannot
// pass their fixtures today, or that fail the V1 rule validation gates), which conformance.json lists after the ported
// rules, so they are scored and reported but gate nothing. The JSON files are the committed artifacts; this script is
// their source: it spells the shared JSX/import/path pieces once (notably the plugin's `split('pages')` filename tests as
// regexes) and keeps the documents consistent. Run offline by a maintainer, never in CI; the output is deterministic.
//
// Usage (from the repository root): node packs/next/candidates/build-policies.mjs
//
// Each violation names its rule under suppress.eslint and, for the rules oxlint 1.88.0's `nextjs` plugin implements
// (OXLINT_NEXTJS_RULES), under suppress.oxlint. suppress.biome belongs to scripts/generate/packs/biome-suppression.mjs:
// this script carries each violation's suppress.biome over from the committed document, and biome-suppression.mjs, run
// after it, updates it. A document keeps its committed revision; one with no committed version starts at revision 1.
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const PACK = join(HERE, "..");
const PACK_RULES = join(PACK, "rules");
const CANDIDATES = join(HERE, "rules");
const SCOPE = { include: ["**/*.ts", "**/*.mts", "**/*.cts", "**/*.tsx", "**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"], exclude: [] };
// The capabilities a port can exercise, in the order conformance.json lists them.
const CAPABILITIES = ["syntax", "scope", "value", "suppression"];

// ---- path predicates: the plugin's string tests on context.filename, as regexes over the host path ----------------
// Strings with no occurrence of "pages" ("pages" overlaps itself nowhere, so this is the 5-state automaton spelled out).
const avoid = (extra = "") => {
  const P = `p(?:p|ap|agp|agep)*`;
  const L0 = `(?:[^p${extra}]|${P}(?:[^pa${extra}]|a[^gp${extra}]|ag[^ep${extra}]|age[^sp${extra}]))*`;
  return `${L0}(?:${P}(?:a|ag|age)?)?`;
};
const AV = avoid();
const AVNS = avoid("/");
/** One character that does not start "pages" (or the end): the segment after "pages" is non-empty. */
const NOT_PAGES_START = `(?:[^p]|p(?:[^a]|$)|pa(?:[^g]|$)|pag(?:[^e]|$)|page(?:[^s]|$))`;
/** `filename.split('pages', 2)[1]` is a document: path.parse(it).name starts with `_document`, or dir is `/_document` and name `index`. */
const SPLIT2_DOCUMENT = `^${AV}pages(?:(?:${AV}/)?_document${AVNS}/*|/_document/index(?:\\.[^./]*)?/*)(?:pages.*)?$`;
/** `path.parse(filename.split('pages', 2)[1]).name` starts with `_document` (no-before-interactive-script-outside-document). */
const SPLIT2_NAME_DOCUMENT = `^${AV}pages(?:${AV}/)?_document${AVNS}/*(?:pages.*)?$`;
/** The last `split('pages')` piece is empty or starts with `/_document` (no-document-import-in-page, no-page-custom-font). */
const LAST_PIECE_DOCUMENT = `pages(?:/_document${AV})?$`;

// ---- shared AST pieces ---------------------------------------------------------------------------------------------
const OPEN = { any: [{ kind: "jsx_opening_element" }, { kind: "jsx_self_closing_element" }] };
const named = name => ({ has: { field: "name", kind: "identifier", regex: `^${name}$` } });
const attribute = (name, ...rules) => ({ kind: "jsx_attribute", all: [{ has: { kind: "property_identifier", regex: `^${name}$` } }, ...rules] });
// No `field: "attribute"`: a field reads one child (the first), and an element has many attributes.
const hasAttribute = (name, ...rules) => ({ has: attribute(name, ...rules) });
const ANY_ATTRIBUTE = { has: { any: [{ kind: "jsx_attribute" }, { kind: "jsx_expression" }] } };
/** NodeAttributes' value: a string attribute, or `{literal}`; `value` constrains that string. */
const literalString = value => ({ has: { any: [{ kind: "string", value }, { kind: "jsx_expression", has: { kind: "string", value } }] } });
/** A plain string attribute (`attr.value.type === 'Literal'`). */
const plainString = value => ({ has: { kind: "string", value } });
/** As literalString, with a whole rule on the string node. */
const literalStringRule = rule => ({ has: { any: [{ kind: "string", ...rule }, { kind: "jsx_expression", has: { kind: "string", ...rule } }] } });
const importFrom = (module, ...rules) => ({ kind: "import_statement", all: [{ has: { field: "source", kind: "string", value: { eq: module } } }, ...rules] });
const inProgram = rule => ({ inside: { kind: "program", stopBy: "end", has: rule } });
/** An import_clause rule: its first specifier's local name (`node.specifiers[0].local.name`) matches `match`. */
const firstLocal = match => ({
  has: {
    kind: "import_clause",
    any: [
      { has: { kind: "identifier", nthChild: 1, ...match } },
      { has: { kind: "namespace_import", nthChild: 1, has: { kind: "identifier", ...match } } },
      {
        has: {
          kind: "named_imports",
          nthChild: 1,
          has: {
            kind: "import_specifier",
            nthChild: 1,
            any: [{ has: { field: "alias", ...match } }, { all: [{ not: { has: { field: "alias", kind: "identifier" } } }, { has: { field: "name", ...match } }] }],
          },
        },
      },
    ],
  },
});
/** An import_statement rule: some specifier's local name matches `match`. */
const someLocal = match => ({
  has: {
    kind: "import_clause",
    any: [
      { has: { kind: "identifier", ...match } },
      { has: { kind: "namespace_import", has: { kind: "identifier", ...match } } },
      {
        has: {
          kind: "named_imports",
          has: {
            kind: "import_specifier",
            any: [{ has: { field: "alias", ...match } }, { all: [{ not: { has: { field: "alias", kind: "identifier" } } }, { has: { field: "name", ...match } }] }],
          },
        },
      },
    ],
  },
});
/** An import_statement rule: its default specifier's local name matches `match`. */
const defaultLocal = match => ({ has: { kind: "import_clause", has: { kind: "identifier", ...match } } });
const FIRST_LOCAL_IS_N = firstLocal({ pattern: "$N" });

// The rules this script ports that oxlint 1.88.0's `nextjs` plugin implements (`oxlint --rules --format json`, scope
// `nextjs`; oxc tag oxlint_v1.88.0). Oxlint names them without the plugin in its suppression comments:
// `@next/next/no-img-element` is `no-img-element`.
const OXLINT_NEXTJS_RULES = new Set([
  "google-font-display", "google-font-preconnect", "inline-script-id", "next-script-for-ga", "no-assign-module-variable",
  "no-async-client-component", "no-before-interactive-script-outside-document", "no-css-tags", "no-document-import-in-page",
  "no-duplicate-head", "no-head-element", "no-head-import-in-document", "no-html-link-for-pages", "no-img-element",
  "no-page-custom-font", "no-script-component-in-head", "no-styled-jsx-in-document", "no-sync-scripts",
  "no-title-in-document-head", "no-typos", "no-unwanted-polyfillio",
]);

/** The committed document of rule `rule`, wherever it lives now, or undefined. */
function committedDocument(rule) {
  for (const directory of [PACK_RULES, CANDIDATES]) {
    const path = join(directory, `next-${rule}.json`);
    if (existsSync(path)) return JSON.parse(readFileSync(path, "utf8"));
  }
  return undefined;
}

const WRITTEN = [];
function policy(rule, description, checks, { candidate = false } = {}) {
  const code = `@next/next/${rule}`;
  const committed = committedDocument(rule);
  const biome = new Map((committed?.clauses ?? []).filter(clause => clause.violation?.suppress?.biome !== undefined).map(clause => [clause.id, clause.violation.suppress.biome]));
  const suppress = id => ({
    eslint: { rules: [code] },
    ...(OXLINT_NEXTJS_RULES.has(rule) ? { oxlint: { rules: [rule] } } : {}),
    ...(biome.has(id) ? { biome: biome.get(id) } : {}),
  });
  const document = {
    format: "rerail.policy/v1",
    id: `next-${rule}`,
    revision: committed?.revision ?? 1,
    mode: "steer",
    description,
    scope: SCOPE,
    clauses: checks.map(({ id, ast, message, anchor = "witness.span", each }) => ({
      id,
      forbid: { ast: { language: "typescript", ...ast } },
      violation: { anchor, code, message, ...(each === undefined ? {} : { each }), suppress: suppress(id) },
    })),
  };
  const directory = candidate ? CANDIDATES : PACK_RULES;
  mkdirSync(directory, { recursive: true });
  rmSync(join(candidate ? PACK_RULES : CANDIDATES, `next-${rule}.json`), { force: true });
  const text = `${JSON.stringify(document, null, 2)}\n`;
  writeFileSync(join(directory, `next-${rule}.json`), text);
  const exercised = { syntax: true, scope: text.includes('"resolves"'), value: text.includes('"value"'), suppression: true };
  const capabilities = CAPABILITIES.filter(capability => exercised[capability]);
  WRITTEN.push({ rule, path: join(directory, `next-${rule}.json`), candidate, capabilities });
}

// ---- rules ---------------------------------------------------------------------------------------------------------

policy("no-img-element", "Use the next/image Image component instead of img elements (picture children and metadata image routes aside); it optimizes images for faster loads and less bandwidth.", [
  {
    id: "upstream",
    ast: {
      file: { path: { notRegex: "^(?:src/)?app(?:/.*)?/opengraph-image|^(?:src/)?app/.*twitter-image|^(?:src/)?app/.*icon\\.[A-Za-z0-9_]+$" } },
      rule: {
        all: [
          OPEN,
          named("img"),
          ANY_ATTRIBUTE,
          // node.parent.parent is not a <picture> element, said positively (an outward relation that holds stays sound
          // in a file with syntax errors; `not inside` would skip such files).
          {
            any: [
              { kind: "jsx_self_closing_element", inside: { matches: "not-picture" } },
              { kind: "jsx_opening_element", inside: { kind: "jsx_element", inside: { matches: "not-picture" } } },
            ],
          },
        ],
      },
      utils: {
        // Every term is positive: a `not` anywhere under an outward relation makes the check skip files with syntax errors.
        "not-picture": {
          any: [
            {
              kind: "jsx_element",
              has: {
                field: "open_tag",
                kind: "jsx_opening_element",
                any: [
                  { has: { field: "name", kind: "identifier", regex: "^(?:.{0,6}|.{8,}|[^p].{6}|p[^i].{5}|pi[^c].{4}|pic[^t].{3}|pict[^u].{2}|pictu[^r].|pictur[^e])$" } },
                  { has: { field: "name", any: [{ kind: "member_expression" }, { kind: "jsx_namespace_name" }] } },
                  { regex: "^<\\s*>$" },
                ],
              },
            },
            {
              any: [
                "parenthesized_expression", "jsx_expression", "arrow_function", "return_statement", "variable_declarator", "assignment_expression", "augmented_assignment_expression",
                "arguments", "array", "pair", "ternary_expression", "binary_expression", "unary_expression", "spread_element", "jsx_attribute", "expression_statement",
                "sequence_expression", "await_expression", "yield_expression", "export_statement", "template_substitution", "member_expression", "subscript_expression",
                "as_expression", "satisfies_expression", "non_null_expression", "public_field_definition", "assignment_pattern", "object_assignment_pattern", "program",
              ].map(kind => ({ kind })),
            },
          ],
        },
      },
    },
    message: `Replace this img with Image from next/image, giving it width and height or fill unless src is a static import.`,
  },
]);

policy("no-head-element", "Use the Head component from next/head instead of a raw head element outside the app directory; Next.js manages the page head itself.", [
  {
    id: "upstream",
    ast: { file: { path: { notRegex: "app/" } }, rule: { all: [OPEN, named("head")] } },
    message: `Do not use \`<head>\` element. Use \`<Head />\` from \`next/head\` instead. See: https://nextjs.org/docs/messages/no-head-element`,
  },
// The plugin's invalid[1], JSX at pages/index.ts that upstream's own parser rejects, is not scored.
]);

policy("no-sync-scripts", "Add async or defer to script tags that load an external src; synchronous scripts block page rendering.", [
  {
    id: "upstream",
    ast: { rule: { all: [OPEN, named("script"), hasAttribute("src"), { not: hasAttribute("async") }, { not: hasAttribute("defer") }] } },
    message: `Add async or defer to this external script tag so it stops blocking page rendering.`,
  },
]);

policy("no-css-tags", "Import local stylesheets instead of adding them with link tags, so Next.js can bundle and optimize the CSS; links to external stylesheet URLs are allowed.", [
  {
    id: "upstream",
    ast: { rule: { all: [OPEN, named("link"), hasAttribute("rel", plainString({ eq: "stylesheet" })), hasAttribute("href", { has: { kind: "string", not: { value: { startsWith: "http" } } } })] } },
    message: `Import this local stylesheet from the component (import './styles.css') instead of adding it with a link tag.`,
  },
]);

/** The query is the text between the first and second `?` (`href.split('?', 2)[1]`); a display key is a `&` piece `display` or `display=…`. */
const HAS_DISPLAY = "^[^?]*\\?(?:[^&?]*&)*display(?:=|&|\\?|$)";
/** Approximation: some display piece (not only the first, which URLSearchParams.get reads) is auto, block or fallback. */
const BAD_DISPLAY = "^[^?]*\\?(?:[^&?]*&)*display=(?:auto|block|fallback)(?:&|\\?|$)";
policy("google-font-display", "Add a display value of swap or optional to Google Fonts stylesheet links; a missing, auto, block or fallback value can hide or shift text while the font loads.", [
  {
    id: "upstream",
    ast: {
      rule: {
        all: [
          OPEN,
          named("link"),
          hasAttribute(
            "href",
            literalStringRule({ value: { startsWith: "https://fonts.googleapis.com/css" }, any: [{ not: { value: { regex: HAS_DISPLAY } } }, { value: { regex: BAD_DISPLAY } }] }),
          ),
        ],
      },
    },
    message: `Add display=swap or display=optional to this Google Fonts URL so text stays visible while the font loads.`,
  },
]);

policy("google-font-preconnect", "Mark links to Google's font file host (fonts.gstatic.com) as preconnect; it opens the connection early so fonts load sooner.", [
  {
    id: "upstream",
    ast: {
      rule: {
        all: [OPEN, named("link"), hasAttribute("href", literalString({ startsWith: "https://fonts.gstatic.com" })), { not: hasAttribute("rel", literalString({ eq: "preconnect" })) }],
      },
    },
    message: `\`rel="preconnect"\` is missing from Google Font. See: https://nextjs.org/docs/messages/google-font-preconnect`,
  },
]);

const GA_URL = "www\\.google-analytics\\.com/analytics\\.js";
const GTAG_URL = "www\\.googletagmanager\\.com/gtag/js";
const GTM_SRC = "www\\.googletagmanager\\.com/gtm\\.js";
/** The first quasi (raw) of the template literal that is the value of the first property of dangerouslySetInnerHTML={{...}}. */
const firstQuasiContains = url => ({
  kind: "jsx_attribute",
  all: [
    { has: { kind: "property_identifier", regex: "^dangerouslySetInnerHTML$" } },
    { has: { kind: "jsx_expression", has: { kind: "object", has: { kind: "pair", nthChild: 1, has: { field: "value", kind: "template_string", regex: `^\`(?:[^$]|\\$[^{])*?${url}` } } } } },
  ],
});
policy("next-script-for-ga", "Use the Google helpers from @next/third-parties instead of raw script tags for Google Analytics or Tag Manager; they load the tags without slowing the page.", [
  {
    id: "upstream",
    ast: {
      rule: {
        all: [
          OPEN,
          named("script"),
          ANY_ATTRIBUTE,
          {
            any: [
              hasAttribute("src", literalString({ regex: `${GA_URL}|${GTAG_URL}` })),
              { has: firstQuasiContains(`(?:${GA_URL}|${GTM_SRC})`) },
            ],
          },
        ],
      },
    },
    message: `Replace this inline Google Analytics or Tag Manager script with GoogleAnalytics or GoogleTagManager from @next/third-parties/google.`,
  },
]);

const POLYFILLED = ["Array.prototype.@@iterator", "Array.prototype.at", "Array.prototype.copyWithin", "Array.prototype.fill", "Array.prototype.find", "Array.prototype.findIndex", "Array.prototype.flatMap", "Array.prototype.flat", "Array.from", "Array.prototype.includes", "Array.of", "Function.prototype.name", "fetch", "Map", "Number.EPSILON", "Number.Epsilon", "Number.isFinite", "Number.isNaN", "Number.isInteger", "Number.isSafeInteger", "Number.MAX_SAFE_INTEGER", "Number.MIN_SAFE_INTEGER", "Number.parseFloat", "Number.parseInt", "Object.assign", "Object.entries", "Object.fromEntries", "Object.getOwnPropertyDescriptor", "Object.getOwnPropertyDescriptors", "Object.hasOwn", "Object.is", "Object.keys", "Object.values", "Reflect", "Set", "Symbol", "Symbol.asyncIterator", "String.prototype.codePointAt", "String.prototype.endsWith", "String.fromCodePoint", "String.prototype.includes", "String.prototype.@@iterator", "String.prototype.padEnd", "String.prototype.padStart", "String.prototype.repeat", "String.raw", "String.prototype.startsWith", "String.prototype.trimEnd", "String.prototype.trimStart", "URL", "URL.prototype.toJSON", "URLSearchParams", "WeakMap", "WeakSet", "Promise", "Promise.prototype.finally", "es2015", "es2016", "es2017", "es2018", "es2019", "es5", "es6", "es7"];
const featureAlternation = POLYFILLED.map(feature => feature.replaceAll(".", "\\.").replaceAll("@", "(?:@|%40)")).join("|");
const SEPARATOR = "(?:,|%2[Cc])";
const POLYFILL_HOSTS = ["https://cdn.polyfill.io/v2/", "https://polyfill.io/v3/", "https://polyfill-fastly.net/", "https://polyfill-fastly.io/", "https://cdnjs.cloudflare.com/polyfill/"];
policy("no-unwanted-polyfillio", "Don't load polyfills from polyfill.io for features Next.js already polyfills; they duplicate code and add page weight.", [
  {
    id: "upstream",
    ast: {
      rule: {
        all: [
          OPEN,
          { any: [named("script"), { all: [{ has: { field: "name", kind: "identifier", pattern: "$N" } }, inProgram(importFrom("next/script", FIRST_LOCAL_IS_N))] }] },
          ANY_ATTRIBUTE,
          hasAttribute(
            "src",
            plainString({
              regex: `^(?:${POLYFILL_HOSTS.map(host => host.replaceAll(".", "\\.")).join("|")})[^?#]*\\?(?:[^&#]*&)*features=(?:[^&#]*?${SEPARATOR})?(?:${featureAlternation})(?:${SEPARATOR}|&|#|$)`,
            }),
          ),
        ],
      },
    },
    message: `No duplicate polyfills from Polyfill.io are allowed. See: https://nextjs.org/docs/messages/no-unwanted-polyfillio`,
  },
]);

policy("no-assign-module-variable", "Don't declare a variable named module in Next.js code; it shadows the module object Node.js and the bundler rely on.", [
  {
    id: "upstream",
    ast: {
      rule: { any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }], has: { kind: "variable_declarator", has: { field: "name", kind: "identifier", regex: "^module$" } } },
    },
    message: `Do not assign to the variable \`module\`. See: https://nextjs.org/docs/messages/no-assign-module-variable`,
  },
]);

policy("no-document-import-in-page", "Import next/document only in pages/_document; the custom Document API does not work in regular pages or components.", [
  {
    id: "upstream",
    ast: { file: { path: { notRegex: LAST_PIECE_DOCUMENT } }, rule: importFrom("next/document") },
    message: `Move this next/document import into pages/_document; the custom Document API does not work in pages or components.`,
  },
]);

policy("no-head-import-in-document", "Use Head from next/document, not next/head, in pages/_document; next/head is meant for individual pages.", [
  {
    id: "upstream",
    ast: { file: { path: { regex: SPLIT2_DOCUMENT } }, rule: importFrom("next/head") },
    message: `Import Head from next/document instead of next/head in this custom Document.`,
  },
]);

policy("no-styled-jsx-in-document", "Don't use styled-jsx style tags in pages/_document; the custom Document does not support them.", [
  {
    id: "upstream",
    ast: { file: { path: { regex: SPLIT2_DOCUMENT } }, rule: { all: [OPEN, named("style"), hasAttribute("jsx")] } },
    message: `Move these styled-jsx styles out of pages/_document into a global stylesheet imported in pages/_app.`,
  },
]);

const ELEMENT_NAMED = name => ({ any: [{ kind: "jsx_self_closing_element", ...named(name) }, { kind: "jsx_element", has: { field: "open_tag", ...named(name) } }] });
policy("no-title-in-document-head", "Set page titles with Head from next/head on each page, not with a title inside the custom Document's Head from next/document.", [
  {
    id: "upstream",
    ast: {
      rule: {
        all: [
          ELEMENT_NAMED("title"),
          { inside: { kind: "jsx_element", has: { field: "open_tag", ...named("Head") } } },
          { not: { follows: { stopBy: "end", ...ELEMENT_NAMED("title") } } },
          inProgram(importFrom("next/document", someLocal({ regex: "^Head$" }))),
        ],
      },
    },
    message: `Move this title out of the Document's Head into each page's Head from next/head.`,
  },
]);

policy("no-script-component-in-head", "Place next/script Script components outside Head from next/head; scripts inside Head are not loaded as intended.", [
  {
    id: "upstream",
    ast: {
      rule: {
        kind: "jsx_element",
        all: [{ has: { field: "open_tag", ...named("Head") } }, { has: ELEMENT_NAMED("Script") }, inProgram(importFrom("next/head"))],
      },
    },
    message: `Move this Script component outside the Head from next/head; Script places the tag itself.`,
  },
]);

// ---- second batch --------------------------------------------------------------------------------------------------
/** The element (JSXElement: jsx_element or self-closing) whose opening tag satisfies `rules`. */
const elementWhoseTag = (...rules) => ({ any: [{ kind: "jsx_self_closing_element", all: rules }, { kind: "jsx_element", has: { field: "open_tag", kind: "jsx_opening_element", all: rules } }] });
const objectSpreadKey = name => ({
  kind: "jsx_expression",
  has: { kind: "spread_element", has: { kind: "object", has: { any: [{ kind: "pair", has: { field: "key", kind: "property_identifier", regex: `^${name}$` } }, { kind: "shorthand_property_identifier", regex: `^${name}$` }] } } },
});
const hasAttributeOrSpreadKey = name => ({ has: { any: [attribute(name), objectSpreadKey(name)] } });
policy("inline-script-id", "Give every next/script component with inline content an id; Next.js needs it to track and optimize the script.", [
  {
    id: "upstream",
    ast: {
      rule: {
        all: [
          elementWhoseTag(
            { has: { field: "name", kind: "identifier", pattern: "$N" } },
            { not: { has: { kind: "jsx_expression", has: { kind: "spread_element", not: { has: { kind: "object" } } } } } },
            { not: hasAttributeOrSpreadKey("id") },
          ),
          { any: [{ kind: "jsx_element", has: { not: { any: [{ kind: "jsx_opening_element" }, { kind: "jsx_closing_element" }] } } }, elementWhoseTag(hasAttributeOrSpreadKey("dangerouslySetInnerHTML"))] },
          inProgram(importFrom("next/script", FIRST_LOCAL_IS_N)),
        ],
      },
    },
    message: `\`next/script\` components with inline content must specify an \`id\` attribute. See: https://nextjs.org/docs/messages/inline-script-id`,
  },
]);

const USE_CLIENT = { kind: "expression_statement", has: { kind: "string", value: { eq: "use client" } } };
const DEFAULT_EXPORT_OF_N_AFTER_USE_CLIENT = inProgram({ kind: "export_statement", all: [{ has: { kind: "identifier", pattern: "$N" } }, { follows: { stopBy: "end", ...USE_CLIENT } }] });
const DECLARES_N = { any: [{ kind: "function_declaration", has: { field: "name", pattern: "$N" } }, { any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }], has: { kind: "variable_declarator", has: { field: "name", kind: "identifier", pattern: "$N" } } }] };
policy("no-async-client-component", "Don't make the default-exported component of a \"use client\" file an async function; Client Components cannot be async.", [
  {
    id: "export-default-function",
    ast: {
      rule: {
        kind: "export_statement",
        all: [{ inside: { kind: "program" } }, { regex: "^export\\s+default\\s" }, { has: { kind: "function_declaration", regex: "^async\\s", has: { field: "name", regex: "^[A-Z]" } } }, { follows: { stopBy: "end", ...USE_CLIENT } }],
      },
    },
    message: `Remove async from this Client Component and load its data in a parent Server Component or an effect instead.`,
  },
  {
    id: "exported-declaration",
    ast: {
      rule: {
        all: [
          { inside: { kind: "program" } },
          {
            any: [
              { kind: "function_declaration", regex: "^async\\s", has: { field: "name", regex: "^[A-Z]", pattern: "$N" } },
              {
                any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }],
                has: { kind: "variable_declarator", all: [{ has: { field: "name", kind: "identifier", regex: "^[A-Z]", pattern: "$N" } }, { has: { field: "value", kind: "arrow_function", regex: "^async\\b" } }] },
              },
            ],
          },
          { not: { follows: { stopBy: "end", ...DECLARES_N } } },
          DEFAULT_EXPORT_OF_N_AFTER_USE_CLIENT,
        ],
      },
    },
    message: `Remove async from this Client Component and load its data in a parent Server Component or an effect instead.`,
  },
]);
// ---- third batch ---------------------------------------------------------------------------------------------------
policy("no-before-interactive-script-outside-document", "Use the beforeInteractive strategy of next/script only in pages/_document or the app directory; elsewhere it cannot run before hydration.", [
  {
    id: "upstream",
    ast: {
      file: { path: { notRegex: `(?:^|/)app/|${SPLIT2_NAME_DOCUMENT}` } },
      rule: {
        all: [
          OPEN,
          { has: { field: "name", kind: "identifier", pattern: "$N" } },
          hasAttribute("strategy", plainString({ eq: "beforeInteractive" })),
          inProgram(importFrom("next/script", defaultLocal({ pattern: "$N" }))),
        ],
      },
    },
    message: `Move this beforeInteractive Script into pages/_document or the app directory, or switch it to afterInteractive.`,
  },
]);

/** A class declaration (or an anonymous `export default class`) extending the default import of next/document. */
const DOCUMENT_CLASS = {
  all: [
    { any: [{ kind: "class_declaration" }, { kind: "class", inside: { kind: "export_statement" } }] },
    { has: { kind: "class_heritage", any: [{ has: { kind: "identifier", pattern: "$D" } }, { has: { kind: "extends_clause", has: { field: "value", kind: "identifier", pattern: "$D" } } }] } },
    inProgram(importFrom("next/document", defaultLocal({ pattern: "$D" }))),
  ],
};
policy("no-duplicate-head", "Keep a single Head element in a custom Next.js Document's render output; duplicate heads produce unexpected markup.", [
  {
    id: "upstream",
    ast: {
      rule: {
        all: [
          ELEMENT_NAMED("Head"),
          { follows: { stopBy: "end", ...ELEMENT_NAMED("Head") } },
          { inside: { kind: "jsx_element", any: [{ inside: { kind: "return_statement" } }, { inside: { kind: "parenthesized_expression", inside: { kind: "return_statement" } } }] } },
          { inside: { stopBy: "end", ...DOCUMENT_CLASS } },
        ],
      },
    },
    message: `Do not include multiple instances of \`<Head/>\`. See: https://nextjs.org/docs/messages/no-duplicate-head`,
  },
]);

// no-typos: names at Levenshtein distance 1 from a data-fetching export.
const DATA_FUNCTIONS = ["getStaticProps", "getStaticPaths", "getServerSideProps"];
const IDENTIFIER_CHAR = "[A-Za-z0-9_$]";
const escapeName = text => text.replaceAll("$", "\\$");
const distanceOne = word => {
  const variants = new Set();
  for (let index = 0; index < word.length; index += 1) {
    variants.add(`${escapeName(word.slice(0, index))}${IDENTIFIER_CHAR}${escapeName(word.slice(index + 1))}`);
    variants.add(escapeName(word.slice(0, index) + word.slice(index + 1)));
  }
  for (let index = 0; index <= word.length; index += 1) variants.add(`${escapeName(word.slice(0, index))}${IDENTIFIER_CHAR}${escapeName(word.slice(index))}`);
  return [...variants];
};
const TYPO = `^(?:${DATA_FUNCTIONS.flatMap(distanceOne).join("|")})$`;
const TYPO_NAME = { regex: TYPO, not: { regex: `^(?:${DATA_FUNCTIONS.join("|")})$` } };
policy("no-typos", "Spell getStaticProps, getStaticPaths and getServerSideProps exactly in page files; an export one letter off is silently ignored by Next.js.", [
  {
    id: "upstream",
    ast: {
      file: { path: { regex: `^${AV}pages${NOT_PAGES_START}`, notRegex: `^${AV}pages/api${AV}/(?:[^/p]|p(?:[^a]|$)|pa(?:[^g]|$)|pag(?:[^e]|$)|page(?:[^s]|$))` } },
      rule: {
        all: [
          {
            any: [
              { kind: "function_declaration", has: { field: "name", ...TYPO_NAME } },
              { kind: "variable_declarator", has: { field: "name", kind: "identifier", ...TYPO_NAME }, inside: { any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }] } },
            ],
          },
          { inside: { stopBy: "end", kind: "export_statement", pattern: "$E", not: { regex: "^export\\s+default\\s" } } },
        ],
      },
    },
    anchor: "capture.E",
    message: `A data-fetching export may be a typo of getStaticProps, getStaticPaths or getServerSideProps.`,
  },
]);

const GOOGLE_FONT_LINK = { all: [OPEN, named("link"), hasAttribute("href", literalString({ startsWith: "https://fonts.googleapis.com/css" }))] };
/** The ancestor no-page-custom-font accepts as the default-exported component. */
const DEFAULT_COMPONENT = {
  any: [
    // `export default function () {}` is a function_expression in the grammar; the plugin reads it as a
    // FunctionDeclaration whose null id matches the null default-export id.
    { any: [{ kind: "function_declaration" }, { kind: "function_expression" }], inside: { kind: "export_statement", regex: "^export\\s+default\\s" } },
    { all: [DOCUMENT_CLASS, { inside: { kind: "export_statement", regex: "^export\\s+default\\s" } }] },
    {
      all: [
        { any: [{ kind: "function_declaration" }, { kind: "class_declaration" }, { kind: "function_expression" }, { kind: "class" }, { kind: "variable_declarator" }] },
        { has: { field: "name", pattern: "$X" } },
        inProgram({ kind: "export_statement", has: { field: "value", kind: "identifier", pattern: "$X" } }),
      ],
    },
  ],
};
policy("no-page-custom-font", "Add Google Fonts stylesheet links only inside the default-exported custom Document in pages/_document; fonts linked elsewhere load for one page and skip font optimization.", [
  { id: "page", ast: { file: { path: { notRegex: LAST_PIECE_DOCUMENT } }, rule: GOOGLE_FONT_LINK }, message: `Move this Google Fonts link into the Head of the custom Document in pages/_document so the font loads on every page.` },
  {
    id: "document",
    ast: { file: { path: { regex: `pages/_document${AV}$` } }, rule: { all: [GOOGLE_FONT_LINK, { not: { inside: { stopBy: "end", ...DEFAULT_COMPONENT } } }] } },
    message: `Move this Google Fonts link into the Head rendered by this file's default-exported Document component.`,
  },
]);

// no-location-assign-relative-destination
const ABSOLUTE_URL = "^(?:[A-Za-z][0-9+.A-Za-z-]*:|//)";
const LOCATION_GLOBALS = { globals: { location: "readonly", window: "readonly", document: "readonly", self: "readonly" } };
const namedMember = name => ({
  any: [
    { kind: "member_expression", has: { field: "property", kind: "property_identifier", regex: `^${name}$` } },
    { kind: "subscript_expression", has: { field: "index", kind: "string", value: { eq: name } } },
  ],
});
const LOCATION_UTILS = {
  "location-root": {
    any: [
      { kind: "identifier", resolves: { global: ["location"] } },
      { all: [namedMember("location"), { has: { field: "object", kind: "identifier", resolves: { global: ["window", "globalThis", "document", "self"] } } }] },
    ],
  },
  "relative-prefix": {
    any: [
      { value: { type: "string", follow: true }, not: { value: { regex: ABSOLUTE_URL, follow: true } } },
      { kind: "template_string", has: { kind: "template_substitution" }, not: { regex: "^`(?:[A-Za-z][0-9+.A-Za-z-]*:|//)" } },
      { kind: "binary_expression", has: { field: "operator", regex: "^\\+$" }, all: [{ has: { field: "left", matches: "relative-prefix" } }] },
      { kind: "parenthesized_expression", has: { matches: "relative-prefix" } },
      { kind: "identifier", resolves: { binding: { value: { matches: "relative-prefix" } } } },
      { kind: "identifier", resolves: { binding: { occurrence: { kind: "identifier", inside: { kind: "assignment_expression", field: "left", has: { field: "right", matches: "relative-prefix" } } } } } },
    ],
  },
};
policy(
  "no-location-assign-relative-destination",
  "Don't navigate to internal pages by setting location.href or calling location.assign with a relative URL; use redirect() or the router for client-side navigation.",
  [
    {
      id: "href",
      ast: {
        languageOptions: LOCATION_GLOBALS,
        utils: LOCATION_UTILS,
        rule: { kind: "assignment_expression", all: [{ has: { field: "left", all: [namedMember("href"), { has: { field: "object", matches: "location-root" } }] } }, { has: { field: "right", matches: "relative-prefix" } }] },
      },
      message: `Replace this location.href assignment with redirect() during render or useRouter().push() in a client event handler.`,
    },
    {
      id: "assign",
      ast: {
        languageOptions: LOCATION_GLOBALS,
        utils: LOCATION_UTILS,
        rule: {
          kind: "call_expression",
          all: [
            { has: { field: "function", all: [namedMember("assign"), { has: { field: "object", matches: "location-root" } }] } },
            { has: { field: "arguments", has: { nthChild: 1, not: { kind: "spread_element" }, matches: "relative-prefix" } } },
          ],
        },
      },
      message: `Replace this location.assign call with redirect() during render or useRouter().push() in a client event handler.`,
    },
  ],
  // The samples' labels were made in eslint-config-next's environment, whose browser globals let upstream report the 29
  // findings this port reproduces.
);

const ported = WRITTEN.filter(entry => !entry.candidate);
const candidates = WRITTEN.filter(entry => entry.candidate);
// conformance.json lists the ported rules, then the candidates (policy under candidates/rules/), and declares exactly the
// capabilities they exercise: check 4 fails a declared capability no listed rule covers.
const listedPorts = [...ported, ...candidates];
const manifestPath = join(PACK, "conformance.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
manifest.capabilities = CAPABILITIES.filter(capability => listedPorts.some(entry => entry.capabilities.includes(capability)));
manifest.rules = listedPorts.map(({ rule, capabilities, candidate }) => ({
  id: `next-${rule}`,
  upstream: { tool: "eslint-plugin-next", rule: `@next/next/${rule}` },
  policy: `${candidate ? "candidates/" : ""}rules/next-${rule}.json`,
  capabilities,
  match: "line+column",
}));
writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`);
// The shipped pack lists the ported rules in the same order; a rule's hand-written fields (examples, mode, on) are kept.
const rulePackPath = join(PACK, "pack.json");
const rulePack = JSON.parse(readFileSync(rulePackPath, "utf8"));
const listed = new Map(rulePack.rules.map(entry => [entry.id, entry]));
const title = rule => `${rule[0].toUpperCase()}${rule.slice(1).replaceAll("-", " ")}`;
const generated = new Set(["id", "document", "title", "group"]);
rulePack.rules = ported.map(({ rule }) => ({
  id: `next-${rule}`,
  document: `rules/next-${rule}.json`,
  title: title(rule),
  group: "Next.js",
  ...Object.fromEntries(Object.entries(listed.get(`next-${rule}`) ?? {}).filter(([key]) => !generated.has(key))),
}));
writeFileSync(rulePackPath, `${JSON.stringify(rulePack, null, 2)}\n`);
console.log(`written: ${ported.length} ported, ${candidates.length} candidates`);
