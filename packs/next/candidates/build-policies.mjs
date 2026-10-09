// Builds the Next pack's policies: packs/next/rules/next-<rule>.json (ported rules) and the `rules` lists of
// packs/next/conformance.json and packs/next/pack.json; packs/next/candidates/rules/next-<rule>.json (rules that cannot
// pass their fixtures today, or that fail the V1 rule validation gates) and packs/next/candidates/conformance.json, a
// runnable pack scoring the
// candidates against the main pack's recordings and corpora. The JSON files are the committed artifacts; this script is
// their source: it spells the shared JSX/import/path pieces once (notably the plugin's `split('pages')` filename tests as
// regexes) and keeps the documents consistent. Run offline by a maintainer, never in CI; the output is deterministic.
//
// Usage (from the repository root): node packs/next/candidates/build-policies.mjs
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const PACK = join(HERE, "..");
const PACK_RULES = join(PACK, "rules");
const CANDIDATE_PACK = HERE;
const CANDIDATES = join(CANDIDATE_PACK, "rules");
const SCOPE = { include: ["**/*.ts", "**/*.mts", "**/*.cts", "**/*.tsx", "**/*.js", "**/*.jsx", "**/*.mjs", "**/*.cjs"], exclude: [] };
const DOCS = "https://nextjs.org/docs/messages";

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

const WRITTEN = [];
function policy(rule, description, checks, { candidate = false } = {}) {
  const code = `@next/next/${rule}`;
  const document = {
    format: "rerail.policy/v1",
    id: `next-${rule}`,
    revision: 1,
    mode: "steer",
    description: `@next/eslint-plugin-next 16.3.8 ${code}: ${description}`,
    scope: SCOPE,
    clauses: checks.map(({ id, ast, message, anchor = "witness.span", each }) => ({
      id,
      forbid: { ast: { language: "typescript", ...ast } },
      violation: { anchor, code, message, ...(each === undefined ? {} : { each }), suppress: { eslint: { rules: [code] } } },
    })),
  };
  const directory = candidate ? CANDIDATES : PACK_RULES;
  mkdirSync(directory, { recursive: true });
  rmSync(join(candidate ? PACK_RULES : CANDIDATES, `next-${rule}.json`), { force: true });
  const text = `${JSON.stringify(document, null, 2)}\n`;
  writeFileSync(join(directory, `next-${rule}.json`), text);
  const capabilities = ["syntax", ...(text.includes('"resolves"') ? ["scope"] : []), ...(text.includes('"value"') ? ["value"] : []), "suppression"];
  WRITTEN.push({ rule, path: join(directory, `next-${rule}.json`), candidate, capabilities });
}

// ---- rules ---------------------------------------------------------------------------------------------------------

policy("no-img-element", "an <img> opening element with attributes, not a direct child of <picture>, unless the file is an app-dir metadata image route (the path relative to the ESLint cwd matches ^(src/)?app/ and /opengraph-image|twitter-image|icon.ext).", [
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
    message: `Using \`<img>\` could result in slower LCP and higher bandwidth. Consider using \`<Image />\` from \`next/image\` or a custom image loader to automatically optimize images. This may incur additional usage or cost from your provider. See: ${DOCS}/no-img-element`,
  },
]);

policy("no-head-element", "a <head> opening element, unless the file's path contains `app/`.", [
  {
    id: "upstream",
    ast: { file: { path: { notRegex: "app/" } }, rule: { all: [OPEN, named("head")] } },
    message: `Do not use \`<head>\` element. Use \`<Head />\` from \`next/head\` instead. See: ${DOCS}/no-head-element`,
  },
// A candidate although it passes its fixtures here: the V1 gates (nera-rule-validation#9, ADR 0006) evaluate the
// plugin's invalid[1] at its file name, pages/index.ts, where the engine reads its JSX as TypeScript and reports nothing.
], { candidate: true });

policy("no-sync-scripts", "a <script> opening element with a src attribute and neither async nor defer.", [
  {
    id: "upstream",
    ast: { rule: { all: [OPEN, named("script"), hasAttribute("src"), { not: hasAttribute("async") }, { not: hasAttribute("defer") }] } },
    message: `Synchronous scripts should not be used. See: ${DOCS}/no-sync-scripts`,
  },
]);

policy("no-css-tags", "a <link> with rel=\"stylesheet\" and a string href that does not start with http(s).", [
  {
    id: "upstream",
    ast: { rule: { all: [OPEN, named("link"), hasAttribute("rel", plainString({ eq: "stylesheet" })), hasAttribute("href", { has: { kind: "string", not: { value: { startsWith: "http" } } } })] } },
    message: `Do not include stylesheets manually. See: ${DOCS}/no-css-tags`,
  },
]);

/** The query is the text between the first and second `?` (`href.split('?', 2)[1]`); a display key is a `&` piece `display` or `display=…`. */
const HAS_DISPLAY = "^[^?]*\\?(?:[^&?]*&)*display(?:=|&|\\?|$)";
/** Approximation: some display piece (not only the first, which URLSearchParams.get reads) is auto, block or fallback. */
const BAD_DISPLAY = "^[^?]*\\?(?:[^&?]*&)*display=(?:auto|block|fallback)(?:&|\\?|$)";
policy("google-font-display", "a <link> whose literal href is a Google Fonts CSS URL whose query has no display parameter, or display=auto|block|fallback.", [
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
    message: `A font-display parameter is missing or not recommended. See: ${DOCS}/google-font-display`,
  },
], { candidate: true });

policy("google-font-preconnect", "a <link> whose literal href starts with https://fonts.gstatic.com and whose rel is not the literal \"preconnect\".", [
  {
    id: "upstream",
    ast: {
      rule: {
        all: [OPEN, named("link"), hasAttribute("href", literalString({ startsWith: "https://fonts.gstatic.com" })), { not: hasAttribute("rel", literalString({ eq: "preconnect" })) }],
      },
    },
    message: `\`rel="preconnect"\` is missing from Google Font. See: ${DOCS}/google-font-preconnect`,
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
policy("next-script-for-ga", "a <script> with a literal src containing the Google Analytics or gtag URL, or whose dangerouslySetInnerHTML object's first property is a template literal whose first quasi contains the GA or GTM script URL.", [
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
    message: `Prefer \`@next/third-parties/google\` when using the inline script for Google Analytics and Tag Manager. See: ${DOCS}/next-script-for-ga`,
  },
]);

const POLYFILLED = ["Array.prototype.@@iterator", "Array.prototype.at", "Array.prototype.copyWithin", "Array.prototype.fill", "Array.prototype.find", "Array.prototype.findIndex", "Array.prototype.flatMap", "Array.prototype.flat", "Array.from", "Array.prototype.includes", "Array.of", "Function.prototype.name", "fetch", "Map", "Number.EPSILON", "Number.Epsilon", "Number.isFinite", "Number.isNaN", "Number.isInteger", "Number.isSafeInteger", "Number.MAX_SAFE_INTEGER", "Number.MIN_SAFE_INTEGER", "Number.parseFloat", "Number.parseInt", "Object.assign", "Object.entries", "Object.fromEntries", "Object.getOwnPropertyDescriptor", "Object.getOwnPropertyDescriptors", "Object.hasOwn", "Object.is", "Object.keys", "Object.values", "Reflect", "Set", "Symbol", "Symbol.asyncIterator", "String.prototype.codePointAt", "String.prototype.endsWith", "String.fromCodePoint", "String.prototype.includes", "String.prototype.@@iterator", "String.prototype.padEnd", "String.prototype.padStart", "String.prototype.repeat", "String.raw", "String.prototype.startsWith", "String.prototype.trimEnd", "String.prototype.trimStart", "URL", "URL.prototype.toJSON", "URLSearchParams", "WeakMap", "WeakSet", "Promise", "Promise.prototype.finally", "es2015", "es2016", "es2017", "es2018", "es2019", "es5", "es6", "es7"];
const featureAlternation = POLYFILLED.map(feature => feature.replaceAll(".", "\\.").replaceAll("@", "(?:@|%40)")).join("|");
const SEPARATOR = "(?:,|%2[Cc])";
const POLYFILL_HOSTS = ["https://cdn.polyfill.io/v2/", "https://polyfill.io/v3/", "https://polyfill-fastly.net/", "https://polyfill-fastly.io/", "https://cdnjs.cloudflare.com/polyfill/"];
policy("no-unwanted-polyfillio", "a <script> (or the first local name an import from next/script binds) with a plain string src on a polyfill.io host whose first features parameter lists a feature Next.js already polyfills.", [
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
    message: `No duplicate polyfills from Polyfill.io are allowed. See: ${DOCS}/no-unwanted-polyfillio`,
  },
]);

policy("no-assign-module-variable", "a variable declaration with a declarator whose identifier is `module`.", [
  {
    id: "upstream",
    ast: {
      rule: { any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }], has: { kind: "variable_declarator", has: { field: "name", kind: "identifier", regex: "^module$" } } },
    },
    message: `Do not assign to the variable \`module\`. See: ${DOCS}/no-assign-module-variable`,
  },
]);

policy("no-document-import-in-page", "an import from next/document, unless the part of the path after its last `pages` is empty or starts with `/_document`.", [
  {
    id: "upstream",
    ast: { file: { path: { notRegex: LAST_PIECE_DOCUMENT } }, rule: importFrom("next/document") },
    message: `\`<Document />\` from \`next/document\` should not be imported outside of \`pages/_document.js\`. See: ${DOCS}/no-document-import-in-page`,
  },
]);

policy("no-head-import-in-document", "an import from next/head in a file whose path, between its first and second `pages`, names a document (name starts with `_document`, or `/_document/index`).", [
  {
    id: "upstream",
    ast: { file: { path: { regex: SPLIT2_DOCUMENT } }, rule: importFrom("next/head") },
    message: `\`next/head\` should not be imported in \`pages/_document.js\`. Use \`<Head />\` from \`next/document\` instead. See: ${DOCS}/no-head-import-in-document`,
  },
]);

policy("no-styled-jsx-in-document", "a <style> opening element with a `jsx` attribute in a file whose path, between its first and second `pages`, names a document.", [
  {
    id: "upstream",
    ast: { file: { path: { regex: SPLIT2_DOCUMENT } }, rule: { all: [OPEN, named("style"), hasAttribute("jsx")] } },
    message: `\`styled-jsx\` should not be used in \`pages/_document.js\`. See: ${DOCS}/no-styled-jsx-in-document`,
  },
]);

const ELEMENT_NAMED = name => ({ any: [{ kind: "jsx_self_closing_element", ...named(name) }, { kind: "jsx_element", has: { field: "open_tag", ...named(name) } }] });
policy("no-title-in-document-head", "the first <title> element child of a <Head> element, in a file with an import from next/document that binds the local name Head.", [
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
    message: `Do not use \`<title>\` element with \`<Head />\` component from \`next/document\`. Titles should defined at the page-level using \`<Head />\` from \`next/head\` instead. See: ${DOCS}/no-title-in-document-head`,
  },
]);

policy("no-script-component-in-head", "a <Head> element with a direct <Script> element child, in a file that imports next/head.", [
  {
    id: "upstream",
    ast: {
      rule: {
        kind: "jsx_element",
        all: [{ has: { field: "open_tag", ...named("Head") } }, { has: ELEMENT_NAMED("Script") }, inProgram(importFrom("next/head"))],
      },
    },
    message: `\`next/script\` should not be used in \`next/head\` component. Move \`<Script />\` outside of \`<Head>\` instead. See: ${DOCS}/no-script-component-in-head`,
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
policy("inline-script-id", "an element named by the first local an import from next/script binds, with children or dangerouslySetInnerHTML and no id (attributes and keys of object-literal spreads; any other spread makes it uncheckable).", [
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
    message: `\`next/script\` components with inline content must specify an \`id\` attribute. See: ${DOCS}/inline-script-id`,
  },
]);

const USE_CLIENT = { kind: "expression_statement", has: { kind: "string", value: { eq: "use client" } } };
const DEFAULT_EXPORT_OF_N_AFTER_USE_CLIENT = inProgram({ kind: "export_statement", all: [{ has: { kind: "identifier", pattern: "$N" } }, { follows: { stopBy: "end", ...USE_CLIENT } }] });
const DECLARES_N = { any: [{ kind: "function_declaration", has: { field: "name", pattern: "$N" } }, { any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }], has: { kind: "variable_declarator", has: { field: "name", kind: "identifier", pattern: "$N" } } }] };
const ASYNC_CLIENT_MESSAGE = `Prevent Client Components from being async functions. See: ${DOCS}/no-async-client-component`;
policy("no-async-client-component", "after a top-level \"use client\" statement: an `export default async function` with a capitalized name, or the first top-level declaration of the capitalized name `export default` exports when it is an async function or a variable initialized with an async arrow.", [
  {
    id: "export-default-function",
    ast: {
      rule: {
        kind: "export_statement",
        all: [{ inside: { kind: "program" } }, { regex: "^export\\s+default\\s" }, { has: { kind: "function_declaration", regex: "^async\\s", has: { field: "name", regex: "^[A-Z]" } } }, { follows: { stopBy: "end", ...USE_CLIENT } }],
      },
    },
    message: ASYNC_CLIENT_MESSAGE,
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
    message: ASYNC_CLIENT_MESSAGE,
  },
]);
// ---- third batch ---------------------------------------------------------------------------------------------------
policy("no-before-interactive-script-outside-document", "an opening element named by the default import of next/script with a plain strategy=\"beforeInteractive\", unless the path contains `/app/` or its segment after the first `pages` has a name starting with `_document`.", [
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
    message: `\`next/script\`'s \`beforeInteractive\` strategy should not be used outside of \`pages/_document.js\`. See: ${DOCS}/no-before-interactive-script-outside-document`,
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
policy("no-duplicate-head", "every <Head> child of a returned element after the first, in a return inside a class extending the default import of next/document.", [
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
    message: `Do not include multiple instances of \`<Head/>\`. See: ${DOCS}/no-duplicate-head`,
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
policy("no-typos", "an exported function or variable whose name is one edit away from getStaticProps, getStaticPaths or getServerSideProps, in a file whose segment after the first `pages` is non-empty and not under /api.", [
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
    { kind: "function_declaration", inside: { kind: "export_statement", regex: "^export\\s+default\\s" } },
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
const CUSTOM_FONT_MESSAGE = `Custom fonts not added in \`pages/_document.js\` will only load for a single page. This is discouraged. See: ${DOCS}/no-page-custom-font`;
policy("no-page-custom-font", "a <link> whose literal href is a Google Fonts CSS URL, unless the file is pages/_document (last `pages` piece starts with /_document) and the link is inside the default-exported component.", [
  { id: "page", ast: { file: { path: { notRegex: LAST_PIECE_DOCUMENT } }, rule: GOOGLE_FONT_LINK }, message: CUSTOM_FONT_MESSAGE },
  {
    id: "document",
    ast: { file: { path: { regex: `pages/_document${AV}$` } }, rule: { all: [GOOGLE_FONT_LINK, { not: { inside: { stopBy: "end", ...DEFAULT_COMPONENT } } }] } },
    message: `Using \`<link />\` outside of \`<Head>\` will disable automatic font optimization. This is discouraged. See: ${DOCS}/no-page-custom-font`,
  },
], { candidate: true });

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
const LOCATION_MESSAGE = `Do not use \`location\` to navigate to internal Next.js pages. Use \`redirect()\` in the render phase, or \`useRouter().push()\` in Client Components' event handlers instead. See: ${DOCS}/no-location-assign-relative-destination`;
policy(
  "no-location-assign-relative-destination",
  "an assignment to location.href (or window/globalThis/document/self.location.href) or a call of its assign(), whose value's static string prefix is a relative URL.",
  [
    {
      id: "href",
      ast: {
        languageOptions: LOCATION_GLOBALS,
        utils: LOCATION_UTILS,
        rule: { kind: "assignment_expression", all: [{ has: { field: "left", all: [namedMember("href"), { has: { field: "object", matches: "location-root" } }] } }, { has: { field: "right", matches: "relative-prefix" } }] },
      },
      message: LOCATION_MESSAGE,
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
      message: LOCATION_MESSAGE,
    },
  ],
  // A candidate although it passes its fixtures here: the V1 gates (nera-rule-validation#9, ADR 0006) drop the browser
  // globals the plugin's RuleTester declares, so upstream reports none of the 29 findings this port reproduces.
  { candidate: true },
);

const ported = WRITTEN.filter(entry => !entry.candidate);
// A pack declares exactly the capabilities its rules exercise: check 4 fails a declared capability no rule covers.
const declared = entries => ["syntax", "scope", "value", "suppression"].filter(capability => entries.some(entry => entry.capabilities.includes(capability)));
const manifestPath = join(PACK, "conformance.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
manifest.capabilities = declared(ported);
manifest.rules = ported.map(({ rule, capabilities }) => ({ id: `next-${rule}`, upstream: { tool: "eslint-plugin-next", rule: `@next/next/${rule}` }, policy: `rules/next-${rule}.json`, capabilities, match: "line+column" }));
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
const candidates = WRITTEN.filter(entry => entry.candidate);
writeFileSync(
  join(CANDIDATE_PACK, "conformance.json"),
  `${JSON.stringify(
    {
      ...manifest,
      pack: "next-candidates",
      capabilities: declared(candidates),
      oracles: "../upstream",
      rules: candidates.map(({ rule, capabilities }) => ({ id: `next-${rule}`, upstream: { tool: "eslint-plugin-next", rule: `@next/next/${rule}` }, policy: `rules/next-${rule}.json`, capabilities, match: "line+column" })),
    },
    null,
    2,
  )}\n`,
);
console.log(`written: ${ported.length} ported, ${candidates.length} candidates`);
