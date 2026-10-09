// Writes the React probe pack's policies, packs/react/candidates/probes/rules/<id>.json, from the specs below. The
// probe pack's conformance.json and its hand labels (labels/fixtures.expected.json) are maintained by hand; this
// script checks conformance.json lists exactly these probes. Score: node --import tsx conformance/languages/cli.ts
// --pack-dir packs/react/candidates/probes --fixtures
// Usage: node packs/react/candidates/probes/build.mjs
// Run offline by a maintainer, never in CI.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const DIR = dirname(fileURLToPath(import.meta.url));
const EXT = ["ts", "mts", "cts", "tsx", "js", "jsx", "mjs", "cjs"].map(e => `**/*.${e}`);
const HOOK_NAME = "^use(?:[A-Z0-9][\\s\\S]*)?$";
const JSX = { any: [{ kind: "jsx_element" }, { kind: "jsx_self_closing_element" }] };
const FN_VALUE = [{ kind: "arrow_function" }, { kind: "function_expression" }];
const fnBoundary = { any: [{ kind: "function_declaration" }, { kind: "function_expression" }, { kind: "arrow_function" }, { kind: "method_definition" }] };
const PROBES = [
  {
    id: "react-component-function",
    description: "Fact: a function component: a PascalCase function declaration, or a PascalCase declarator whose value is a function, that returns JSX (a return of JSX in its own body, or an arrow whose body is JSX). Reported at the name.",
    capabilities: ["syntax"],
    rule: {
      kind: "identifier",
      regex: "^[A-Z]",
      any: [
        { inside: { field: "name", kind: "function_declaration", matches: "returns-jsx" } },
        { inside: { field: "name", kind: "variable_declarator", has: { field: "value", any: FN_VALUE, matches: "returns-jsx" } } },
      ],
    },
    utils: {
      "jsx-or-parens": { any: [JSX, { kind: "parenthesized_expression", has: { matches: "jsx-or-parens" } }] },
      "returns-jsx": {
        any: [
          { has: { field: "body", matches: "jsx-or-parens" } },
          { has: { field: "body", has: { stopBy: fnBoundary, kind: "return_statement", has: { matches: "jsx-or-parens" } } } },
        ],
      },
    },
  },
  {
    id: "react-component-class",
    description: "Fact: a class component: a class whose superclass is Component / PureComponent / React.Component / React.PureComponent (eslint-plugin-react's isES6Component). Reported at the class name.",
    capabilities: ["syntax"],
    rule: {
      any: [{ kind: "identifier" }, { kind: "type_identifier" }],
      inside: {
        field: "name",
        kind: "class_declaration",
        has: {
          kind: "class_heritage",
          any: [{ has: { matches: "component-superclass" } }, { has: { kind: "extends_clause", has: { field: "value", matches: "component-superclass" } } }],
        },
      },
    },
    utils: {
      "component-superclass": {
        any: [
          { kind: "identifier", regex: "^(?:Pure)?Component$" },
          { kind: "member_expression", all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", regex: "^(?:Pure)?Component$" } }] },
        ],
      },
    },
  },
  {
    id: "react-component-wrapped",
    description: "Fact: a component made by a wrapper: a declarator whose value is memo(...) / forwardRef(...) (or React.memo / React.forwardRef). Reported at the declarator name.",
    capabilities: ["syntax"],
    rule: { kind: "identifier", inside: { field: "name", kind: "variable_declarator", has: { field: "value", kind: "call_expression", has: { field: "function", matches: "wrapper" } } } },
    utils: {
      wrapper: {
        any: [
          { kind: "identifier", regex: "^(?:memo|forwardRef)$" },
          { kind: "member_expression", all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", regex: "^(?:memo|forwardRef)$" } }] },
        ],
      },
    },
  },
  {
    id: "react-hook-call",
    description: "Fact: a hook call: a call whose callee is `use`, `useX` (X an upper-case letter or digit) or `Namespace.useX` with a PascalCase namespace (rules-of-hooks' isHook). Reported at the call.",
    capabilities: ["syntax"],
    rule: {
      kind: "call_expression",
      has: {
        field: "function",
        any: [
          { kind: "identifier", regex: HOOK_NAME },
          { kind: "member_expression", all: [{ has: { field: "object", kind: "identifier", regex: "^[A-Z]" } }, { has: { field: "property", regex: HOOK_NAME } }] },
        ],
      },
    },
  },
  {
    id: "react-custom-hook",
    description: "Fact: a custom hook definition: a function declaration, or a declarator whose value is a function, named `useX`. Reported at the name.",
    capabilities: ["syntax"],
    rule: {
      kind: "identifier",
      regex: HOOK_NAME,
      any: [{ inside: { field: "name", kind: "function_declaration" } }, { inside: { field: "name", kind: "variable_declarator", has: { field: "value", any: FN_VALUE } } }],
    },
  },
  {
    id: "react-runtime-classic-unused-react",
    description: "Fact: the classic JSX runtime (languageOptions.jsxPragma \"React\", the default): JSX reads the `React` binding, so an imported React is unused only in a file without JSX. Reported at the import's name.",
    capabilities: ["scope"],
    languageOptions: { jsxPragma: "React" },
    rule: { kind: "identifier", regex: "^React$", binds: { form: "import", used: false } },
  },
  {
    id: "react-runtime-automatic-unused-react",
    description: "Fact: the automatic JSX runtime (languageOptions.jsxPragma null): JSX reads no binding, so an imported React that nothing else reads is unused. Reported at the import's name. The runtime is a per-query option today; a project's tsconfig `jsx` setting (react-jsx) cannot select it.",
    capabilities: ["scope"],
    languageOptions: { jsxPragma: null },
    rule: { kind: "identifier", regex: "^React$", binds: { form: "import", used: false } },
  },
  {
    id: "react-jsx-pragma-comment",
    description: "Fact: a file-level JSX pragma directive (`@jsx X` in a comment). Detectable; its name cannot be bound to a rule's pragma (jsxPragma is fixed per query). Reported at the comment.",
    capabilities: ["syntax"],
    rule: { kind: "comment", regex: "@jsx\\s+\\S" },
  },
];
mkdirSync(join(DIR, "rules"), { recursive: true });
for (const probe of PROBES) {
  const document = {
    format: "rerail.policy/v1",
    id: probe.id,
    revision: 1,
    mode: "log",
    description: probe.description,
    scope: { include: EXT, exclude: [] },
    clauses: [
      {
        id: "probe",
        forbid: { ast: { language: "typescript", ...(probe.languageOptions ? { languageOptions: probe.languageOptions } : {}), rule: probe.rule, ...(probe.utils ? { utils: probe.utils } : {}), ...(probe.file ? { file: probe.file } : {}) } },
        violation: { anchor: "witness.start", code: `react-probe/${probe.id.replace(/^react-/, "")}`, message: probe.description.split(":")[0] },
      },
    ],
  };
  writeFileSync(join(DIR, "rules", `${probe.id}.json`), `${JSON.stringify(document, null, 2)}\n`);
}
const listed = JSON.stringify(JSON.parse(readFileSync(join(DIR, "conformance.json"), "utf8")).rules);
const expected = JSON.stringify(
  PROBES.map(p => ({ id: p.id, upstream: { tool: "labels", rule: `react-probe/${p.id.replace(/^react-/, "")}` }, policy: `rules/${p.id}.json`, capabilities: p.capabilities, match: "line+column" })),
);
if (listed !== expected) throw new Error("probes/conformance.json rules differ from the probe specs");
console.log(`wrote ${PROBES.length} probes`);
