// The React pack's policies as compact specs, one entry per pack rule (`candidate: true`: a port that does not pass
// its fixtures or the V1 rule validation gates yet, written to packs/react/candidates/). build.mjs turns each into a
// rerail.policy/v1 document (scope, the JSX-runtime gate on every query, violation, eslint suppression and a shared
// `define` are added there) and lists it, with the HAND_WRITTEN ports, in packs/react/conformance.json. Edit here, then
// run build.mjs.
const R = "eslint-plugin-react";
const H = "eslint-plugin-react-hooks";
const VOID = "^(?:area|base|br|col|embed|hr|img|input|keygen|link|menuitem|meta|param|source|track|wbr)$";

const OPENING = [{ kind: "jsx_opening_element" }, { kind: "jsx_self_closing_element" }];
/** jsx-ast-utils elementType: the tag's full text; isDOMComponent: it starts with a lower-case ASCII letter. */
const DOM_TAG = { has: { field: "name", regex: "^[a-z]" } };
const prop = name => ({ kind: "jsx_attribute", has: { kind: "property_identifier", regex: `^(?:${name})$` } });
/** An object literal's property whose key is the identifier `name` (ESTree Property with an Identifier key). */
const objectKey = name => ({
  any: [
    { kind: "pair", has: { field: "key", kind: "property_identifier", regex: `^(?:${name})$` } },
    { kind: "shorthand_property_identifier", regex: `^(?:${name})$` },
    { kind: "method_definition", has: { field: "name", kind: "property_identifier", regex: `^(?:${name})$` } },
  ],
});

/**
 * eslint-plugin-react's isCreateElement with the default pragma: `React.createElement`, or a `createElement` whose
 * declaration is a declarator initialised from `React` / `React.x` / `require('react')` / `require('react').x`, or an
 * import from 'react' (util/isDestructuredFromPragmaImport).
 */
const CREATE_ELEMENT_UTILS = {
  "create-element": {
    any: [
      {
        kind: "member_expression",
        all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", regex: "^createElement$" } }],
      },
      { kind: "identifier", regex: "^createElement$", resolves: { binding: { site: { matches: "react-site" } } } },
    ],
  },
  "react-site": {
    any: [
      { kind: "variable_declarator", has: { field: "value", matches: "react-init" } },
      { inside: { stopBy: "end", kind: "import_statement", has: { field: "source", kind: "string", value: { eq: "react" } } } },
    ],
  },
  "react-init": {
    any: [
      { kind: "identifier", regex: "^React$" },
      { kind: "member_expression", has: { field: "object", any: [{ kind: "identifier", regex: "^React$" }, { matches: "require-react" }] } },
      { matches: "require-react" },
    ],
  },
  "require-react": {
    kind: "call_expression",
    all: [
      { has: { field: "function", kind: "identifier", regex: "^require$" } },
      { has: { field: "arguments", has: { kind: "string", nthChild: 1, value: { eq: "react" } } } },
    ],
  },
};

/** A spread `{...name}` of a variable whose declaration initialises it with an object matching util `object`. */
const spreadOf = object => ({ kind: "identifier", resolves: { binding: { value: { matches: object } } } });

/** no-danger-with-children's findObjectProp: an object literal with a `prop` key, also through `...name` spreads. */
function objectPropUtil(name, propName) {
  return { [name]: { kind: "object", has: { any: [...objectKey(propName).any, { kind: "spread_element", has: spreadOf(name) }] } } };
}

/** no-danger-with-children's findJsxProp on an opening element: the attribute, or a spread of an object having it. */
function jsxPropUtil(name, propName) {
  return {
    [name]: { any: [{ has: prop(propName) }, { has: { kind: "jsx_expression", has: { kind: "spread_element", has: spreadOf(`${name}-obj`) } } }] },
    ...objectPropUtil(`${name}-obj`, propName),
  };
}

/** createElement's props argument: an object literal, or a variable initialised with one, matching every util. */
function propsMatching(utils) {
  const all = utils.map(util => ({ matches: util }));
  return { any: [{ kind: "object", all }, { kind: "identifier", resolves: { binding: { value: { kind: "object", all } } } }] };
}

/** Tree-sitter expression kinds other than literals, plain templates, conditionals and parentheses (ESTree's non-literal expressions). */
const EXPRESSION_KINDS = [
  "identifier", "this", "super", "undefined", "member_expression", "subscript_expression", "call_expression", "new_expression",
  "await_expression", "unary_expression", "binary_expression", "update_expression", "assignment_expression",
  "augmented_assignment_expression", "arrow_function", "function_expression", "generator_function", "class", "object", "array",
  "jsx_element", "jsx_self_closing_element", "sequence_expression", "yield_expression",
  "as_expression", "satisfies_expression", "non_null_expression", "meta_property", "import",
];
/** A node, or the parentheses around it, matching util `target`. */
const unparen = (name, target) => ({ [name]: { any: [{ matches: target }, { kind: "parenthesized_expression", has: { matches: name } }] } });
/** A conditional (parentheses aside) whose `field` branch matches util `target`. */
const ternaryBranch = (name, field, target) => ({
  [name]: { any: [{ kind: "ternary_expression", has: { field, matches: target } }, { kind: "parenthesized_expression", has: { matches: name } }] },
});
const leafUtil = (what, branch) => (branch ? `${branch}-${what}` : `u-${what}`);
const TYPE_VALUES = "^(?:button|submit|reset)$";
const buttonName = { has: { field: "name", kind: "identifier", regex: "^button$" } };
const BUTTON_TAG = [
  { kind: "jsx_opening_element", ...buttonName },
  { kind: "jsx_self_closing_element", ...buttonName },
];
const BUTTON_UTILS = {
  ...CREATE_ELEMENT_UTILS,
  "type-attribute": {
    any: [
      { kind: "jsx_attribute", has: { kind: "property_identifier", regex: "^(?i:type)$" } },
      { kind: "jsx_expression", has: { kind: "spread_element", has: { kind: "object", has: objectKey("(?i:type)") } } },
    ],
  },
  "first-type-attribute": { matches: "type-attribute", not: { follows: { stopBy: "end", matches: "type-attribute" } } },
  "plain-template": { kind: "template_string", not: { has: { kind: "template_substitution" } } },
  "invalid-leaf": {
    any: [
      { kind: "string", not: { value: { regex: TYPE_VALUES } } },
      { matches: "plain-template", not: { regex: "^`(?:button|submit|reset)`$" } },
      { kind: "number" },
      { kind: "true" },
      { kind: "false" },
      { kind: "null" },
      { kind: "regex" },
    ],
  },
  "complex-leaf": {
    any: [
      ...EXPRESSION_KINDS.map(kind => ({ kind })),
      { kind: "template_string", has: { kind: "template_substitution" } },
    ],
  },
  "button-create-element": {
    kind: "call_expression",
    pattern: "$CALL",
    all: [{ has: { field: "arguments", has: { kind: "string", nthChild: 1, value: { eq: "button" } } } }, { has: { field: "function", matches: "create-element" } }],
  },
  "ce-first-type-property": {
    ...objectKey("type"),
    all: [
      { not: { follows: { stopBy: "end", ...objectKey("type") } } },
      { inside: { kind: "object", nthChild: 2, inside: { kind: "arguments", inside: { matches: "button-create-element" } } } },
    ],
  },
  "complex-capture": { matches: "complex-leaf", pattern: "$LEAF" },
  ...unparen("u-invalid", "invalid-leaf"),
  ...unparen("u-complex", "complex-capture"),
  ...ternaryBranch("consequence-invalid", "consequence", "u-invalid"),
  ...ternaryBranch("alternative-invalid", "alternative", "u-invalid"),
  ...ternaryBranch("consequence-complex", "consequence", "u-complex"),
  ...ternaryBranch("alternative-complex", "alternative", "u-complex"),
};

/** jsx-key's positions. Each util holds on a node or on the parentheses around it (ESTree has no parentheses). */
const FUNCTION_KINDS = [{ kind: "arrow_function" }, { kind: "function_expression" }, { kind: "generator_function" }];
const throughParens = (name, rule) => ({ [name]: { any: [rule, { inside: { kind: "parenthesized_expression", matches: name } }] } });
const JSX_KEY_UTILS = {
  "keyless-element": {
    any: [{ kind: "jsx_element", has: { field: "open_tag", has: { field: "name", regex: "[\\s\\S]" } } }, { kind: "jsx_self_closing_element" }],
  },
  "has-key-ci": {
    any: [
      { has: { field: "open_tag", has: { kind: "jsx_attribute", has: { kind: "property_identifier", regex: "^(?i:key)$" } } } },
      { has: { kind: "jsx_attribute", has: { kind: "property_identifier", regex: "^(?i:key)$" } } },
    ],
  },
  ...throughParens("array-element", { inside: { kind: "array" } }),
  "to-array-call": {
    kind: "call_expression",
    has: {
      field: "function",
      kind: "member_expression",
      all: [
        { has: { field: "property", regex: "^toArray$" } },
        {
          has: {
            field: "object",
            any: [
              { kind: "identifier", regex: "^Children$" },
              {
                kind: "member_expression",
                all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", regex: "^Children$" } }],
              },
            ],
          },
        },
      ],
    },
  },
  "in-to-array": { inside: { stopBy: "end", matches: "to-array-call" } },
  "member-call": { kind: "call_expression", has: { field: "function", kind: "member_expression" } },
  ...throughParens("callback-slot", {
    any: [
      { nthChild: 1, inside: { kind: "arguments", inside: { matches: "member-call", has: { field: "function", has: { field: "property", regex: "^map$" } } } } },
      { nthChild: 2, inside: { kind: "arguments", inside: { matches: "member-call", has: { field: "function", has: { field: "property", regex: "^from$" } } } } },
    ],
  }),
  "callback": { any: FUNCTION_KINDS, matches: "callback-slot" },
  ...throughParens("callback-body", { inside: { field: "body", kind: "arrow_function", matches: "callback" } }),
  ...throughParens("callback-branch", {
    any: [
      { inside: { field: "consequence", kind: "ternary_expression", matches: "callback-body" } },
      { inside: { field: "alternative", kind: "ternary_expression", matches: "callback-body" } },
      { inside: { field: "right", kind: "binary_expression", has: { field: "operator", regex: "^(?:&&|\\|\\||\\?\\?)$" }, matches: "callback-body" } },
    ],
  }),
  "callback-block": { kind: "statement_block", inside: { field: "body", any: FUNCTION_KINDS, matches: "callback" } },
  "scanned-statement": {
    any: [{ kind: "return_statement" }, { kind: "if_statement" }],
    all: [
      {
        any: [
          { inside: { matches: "callback-block" } },
          { inside: { kind: "statement_block", matches: "scanned-branch" } },
          { matches: "scanned-branch" },
        ],
      },
    ],
  },
  "scanned-branch": {
    any: [
      { inside: { field: "consequence", kind: "if_statement", matches: "scanned-statement" } },
      { inside: { kind: "else_clause", inside: { kind: "if_statement", matches: "scanned-statement" } } },
    ],
  },
  ...throughParens("returned", { inside: { kind: "return_statement", matches: "scanned-statement" } }),
  "iterator-element": { any: [{ matches: "callback-body" }, { matches: "callback-branch" }, { matches: "returned" }] },
};

/**
 * react-in-jsx-scope: a binding of the pragma an element sees, approximated syntactically (see the policy description).
 * `name` matches a declared name: `React`'s text, or a name equal to the file's `@jsx` pragma (PRAGMA).
 */
const scopeUtils = (prefix, name) => {
  const declared = `declares-${prefix}`;
  return {
    [declared]: {
      any: [
        { kind: "import_statement", has: { stopBy: "end", kind: "identifier", ...name } },
        {
          any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }],
          has: {
            kind: "variable_declarator",
            has: {
              field: "name",
              any: [
                { kind: "identifier", ...name },
                { has: { stopBy: "end", any: [{ kind: "identifier" }, { kind: "shorthand_property_identifier_pattern" }], ...name } },
              ],
            },
          },
        },
        { any: [{ kind: "function_declaration" }, { kind: "class_declaration" }], has: { field: "name", ...name } },
        { kind: "export_statement", has: { field: "declaration", matches: declared } },
      ],
    },
    [`${prefix}-visible`]: {
      any: [
        { inside: { stopBy: "end", kind: "program", has: { matches: declared } } },
        { inside: { stopBy: "end", kind: "statement_block", has: { matches: declared } } },
        { inside: { stopBy: "end", any: [...FUNCTION_KINDS, { kind: "function_declaration" }, { kind: "method_definition" }], matches: `${prefix}-parameter` } },
      ],
    },
    [`${prefix}-parameter`]: {
      any: [
        { has: { field: "parameters", has: { stopBy: "end", kind: "identifier", ...name, binds: { form: "parameter" } } } },
        { has: { field: "parameter", kind: "identifier", ...name } },
      ],
    },
  };
};
/** A comment whose `@jsx` pragma (its first `.`-segment) is an identifier: plugin's JSX_ANNOTATION_REGEX and JS_IDENTIFIER_REGEX. */
const PRAGMA_COMMENT = { kind: "comment", regex: "@jsx\\s+[_$a-zA-Z][_$a-zA-Z0-9]*(?:[.\\s]|\\*/$|$)" };
const REACT_PRAGMA = "@jsx\\s+React(?:[.\\s]|\\*/$|$)";
const REACT_SCOPE_UTILS = {
  ...scopeUtils("react", { regex: "^React$" }),
  "other-pragma-file": { inside: { stopBy: "end", kind: "program", has: { ...PRAGMA_COMMENT, not: { regex: REACT_PRAGMA } } } },
};
/** Only where PRAGMA is bound: the compare runs in the check that captures the pragma comment. */
const PRAGMA_SCOPE_UTILS = scopeUtils("pragma", { all: [{ pattern: "$DN" }, { compare: { left: "DN", op: "eq", right: "PRAGMA" } }] });

/** jsx-no-target-blank's attribute readings. */
const attr = name => ({ kind: "jsx_attribute", has: { kind: "property_identifier", regex: `^${name}$` } });
const lastAttr = name => ({ ...attr(name), not: { precedes: { stopBy: "end", ...attr(name) } } });
const relTernary = (test, branch) => ({
  has: {
    matches: "last-rel",
    has: { kind: "jsx_expression", has: { kind: "ternary_expression", all: [{ has: { field: "condition", kind: "identifier", pattern: test } }, ...(branch ? [{ has: { field: branch, matches: "ok-literal" } }] : [])] } },
  },
});
const targetTernary = (test, branches) => ({
  has: {
    matches: "last-target",
    has: { kind: "jsx_expression", has: { kind: "ternary_expression", all: [{ has: { field: "condition", kind: "identifier", pattern: test } }, ...branches] } },
  },
});
const TARGET_BLANK_UTILS = {
  "last-target": lastAttr("target"),
  "last-href": lastAttr("href"),
  "last-rel": lastAttr("rel"),
  "blank-literal": { any: [{ kind: "string", value: { regex: "(?i)^_blank$" } }, { kind: "parenthesized_expression", has: { matches: "blank-literal" } }] },
  "exact-blank": { any: [{ kind: "string", value: { eq: "_blank" } }, { kind: "parenthesized_expression", has: { matches: "exact-blank" } }] },
  "possibly-blank": {
    any: [
      { has: { kind: "string", value: { regex: "(?i)^_blank$" } } },
      { has: { kind: "jsx_expression", has: { matches: "blank-literal" } } },
      {
        has: {
          kind: "jsx_expression",
          has: { kind: "ternary_expression", any: [{ has: { field: "consequence", matches: "blank-literal" } }, { has: { field: "alternative", matches: "blank-literal" } }] },
        },
      },
    ],
  },
  "dangerous-link": {
    any: [
      { has: { matches: "last-href", all: [{ has: { kind: "string", value: { regex: "^(?:[A-Za-z0-9_]+:|//)" } } }] } },
      { has: { ...attr("href"), all: [{ has: { kind: "jsx_expression" } }] } },
    ],
  },
  "ok-literal": { any: [{ kind: "string", value: { regex: "(?i)(?:^| )noreferrer(?: |$)" } }, { kind: "parenthesized_expression", has: { matches: "ok-literal" } }] },
  "ok-template": { kind: "template_string", regex: "\\A`(?:(?:[^$`]|\\$[^{`])*? )?(?i:noreferrer)(?: |`|\\$\\{)" },
  "same-test": { all: [relTernary("$RT"), targetTernary("$TT", []), { compare: { left: "RT", op: "eq", right: "TT" } }] },
  "secure-rel": {
    any: [
      { has: { matches: "last-rel", has: { any: [{ kind: "string", value: { regex: "(?i)(?:^| )noreferrer(?: |$)" } }, { kind: "jsx_expression", has: { any: [{ matches: "ok-literal" }, { matches: "ok-template" }] } }] } } },
      {
        all: [
          { has: { matches: "last-rel", has: { kind: "jsx_expression", has: { kind: "ternary_expression", all: [{ has: { field: "consequence", matches: "ok-literal" } }, { has: { field: "alternative", matches: "ok-literal" } }] } } } },
          { not: { matches: "same-test" } },
        ],
      },
      { all: [relTernary("$RC", "consequence"), targetTernary("$TC", [{ has: { field: "consequence", matches: "exact-blank" } }]), { compare: { left: "RC", op: "eq", right: "TC" } }] },
      {
        all: [
          relTernary("$RA", "alternative"),
          targetTernary("$TA", [{ not: { has: { field: "consequence", matches: "exact-blank" } } }, { has: { field: "alternative", matches: "exact-blank" } }]),
          { compare: { left: "RA", op: "eq", right: "TA" } },
        ],
      },
    ],
  },
};

/** rules-of-hooks: hooks, the function (code path) a hook runs in, and how that function is named. */
const HOOK_NAME = "^use(?:[A-Z0-9][\\s\\S]*)?$";
const FN_KINDS = ["function_declaration", "generator_function_declaration", "function_expression", "generator_function", "arrow_function", "method_definition"].map(kind => ({ kind }));
const NAMED_FN_KINDS = ["function_declaration", "generator_function_declaration", "function_expression", "generator_function"].map(kind => ({ kind }));
const ANON_FN_KINDS = ["function_expression", "generator_function", "arrow_function"].map(kind => ({ kind }));
/** getFunctionName(F) satisfies `name` (a rule over the name node). */
const functionNamed = name => ({
  any: [
    { any: NAMED_FN_KINDS, has: { field: "name", ...name } },
    {
      any: ANON_FN_KINDS,
      not: { has: { field: "name", regex: "[\\s\\S]" } },
      all: [
        {
          any: [
            { inside: { field: "value", kind: "variable_declarator", has: { field: "name", ...name } } },
            { inside: { field: "right", kind: "assignment_expression", has: { field: "left", ...name } } },
            { inside: { field: "value", kind: "pair", has: { field: "key", not: { kind: "computed_property_name" }, all: [name] } } },
            { inside: { field: "right", kind: "assignment_pattern", has: { field: "left", ...name } } },
            { inside: { field: "right", kind: "object_assignment_pattern", has: { field: "left", ...name } } },
          ],
        },
      ],
    },
    { kind: "method_definition", inside: { kind: "object" }, has: { field: "name", not: { kind: "computed_property_name" }, all: [name] } },
  ],
});
const COMPONENT_NAME = { any: [{ kind: "identifier" }, { kind: "property_identifier" }, { kind: "shorthand_property_identifier_pattern" }], regex: "^[A-Z]" };
const HOOK_NAME_NODE = {
  any: [
    { any: [{ kind: "identifier" }, { kind: "property_identifier" }, { kind: "shorthand_property_identifier_pattern" }], regex: HOOK_NAME },
    { kind: "member_expression", all: [{ has: { field: "object", kind: "identifier", regex: "^[A-Z]" } }, { has: { field: "property", regex: HOOK_NAME } }] },
  ],
};
const insideFn = rule => ({ inside: { stopBy: { matches: "fn" }, matches: "fn", all: [rule] } });
const beforeFn = rule => ({ inside: { stopBy: { matches: "fn" }, not: { matches: "fn" }, all: [rule] } });
const HOOKS_UTILS = {
  "class-field": { kind: "public_field_definition" },
  fn: { any: FN_KINDS },
  hook: {
    kind: "call_expression",
    has: {
      field: "function",
      any: [
        { kind: "identifier", regex: HOOK_NAME },
        { kind: "member_expression", all: [{ has: { field: "object", kind: "identifier", regex: "^[A-Z]" } }, { has: { field: "property", kind: "property_identifier", regex: HOOK_NAME } }] },
      ],
    },
  },
  "use-hook": {
    has: {
      field: "function",
      any: [
        { kind: "identifier", regex: "^use$" },
        { kind: "member_expression", all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", regex: "^use$" } }] },
      ],
    },
  },
  "named-component": functionNamed(COMPONENT_NAME),
  "named-hook": functionNamed(HOOK_NAME_NODE),
  "named": functionNamed({ regex: "[\\s\\S]" }),
  "memo-callback": {
    inside: {
      kind: "arguments",
      inside: {
        kind: "call_expression",
        has: {
          field: "function",
          any: [
            { kind: "identifier", regex: "^(?:memo|forwardRef)$" },
            { kind: "member_expression", all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", regex: "^(?:memo|forwardRef)$" } }] },
          ],
        },
      },
    },
  },
  "component-or-hook": { any: [{ matches: "named-component" }, { matches: "named-hook" }, { not: { matches: "named" }, matches: "memo-callback" }] },
  "class-function": {
    any: [
      { kind: "method_definition", inside: { kind: "class_body" } },
      { any: ANON_FN_KINDS, inside: { field: "value", matches: "class-field" } },
    ],
  },
  "in-do-while": { inside: { stopBy: "end", kind: "do_statement" } },
  cycled: beforeFn({ any: [{ kind: "for_statement" }, { kind: "for_in_statement" }, { kind: "while_statement" }, { kind: "do_statement" }] }),
  conditional: { any: [{ matches: "conditional-position" }, beforeFn({ matches: "conditional-position" })] },
  "conditional-position": {
    any: [
      { inside: { field: "consequence", kind: "if_statement" } },
      { inside: { field: "alternative", kind: "if_statement" } },
      { inside: { field: "consequence", kind: "ternary_expression" } },
      { inside: { field: "alternative", kind: "ternary_expression" } },
      { inside: { field: "right", kind: "binary_expression", has: { field: "operator", regex: "^(?:&&|\\|\\||\\?\\?)$" } } },
      { inside: { field: "right", kind: "augmented_assignment_expression", has: { field: "operator", regex: "^(?:&&=|\\|\\|=|\\?\\?=)$" } } },
      { inside: { field: "body", kind: "switch_case" } },
      { inside: { field: "body", kind: "switch_default" } },
      { kind: "catch_clause" },
      { inside: { field: "body", kind: "try_statement" } },
      { inside: { field: "right", kind: "assignment_pattern" } },
      { inside: { field: "right", kind: "object_assignment_pattern" } },
    ],
  },
  exit: { any: [{ kind: "return_statement" }, { kind: "break_statement", has: { kind: "statement_identifier" } }] },
  "has-return": { not: { matches: "fn" }, has: { stopBy: { matches: "fn" }, matches: "exit" } },
  "early-return": {
    any: [{ follows: { stopBy: "end", matches: "has-return" } }, beforeFn({ follows: { stopBy: "end", matches: "has-return" } })],
  },
  "effect-event-callee": {
    any: [
      { kind: "identifier", regex: "^useEffectEvent$" },
      { kind: "member_expression", all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", regex: "^useEffectEvent$" } }] },
    ],
  },
  "effect-event-value": { has: { field: "value", kind: "call_expression", has: { field: "function", kind: "identifier", regex: "^useEffectEvent$" } } },
  "effect-event-declarator": {
    matches: "effect-event-value",
    inside: {
      any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }],
      inside: {
        kind: "statement_block",
        inside: { field: "body", any: [{ kind: "function_declaration" }, { kind: "arrow_function" }], matches: "inside-or-is-component-or-hook" },
      },
    },
  },
  "effect-call": {
    kind: "call_expression",
    has: { field: "arguments", has: { regex: "[\\s\\S]", not: { regex: "^[(),]$" } } },
    all: [
      {
        has: {
          field: "function",
          any: [
            { kind: "identifier", regex: "^(?:useEffect|useLayoutEffect|useInsertionEffect|useEffectEvent)$" },
            {
              kind: "member_expression",
              all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", regex: "^(?:useEffect|useLayoutEffect|useInsertionEffect|useEffectEvent)$" } }],
            },
          ],
        },
      },
    ],
  },
  "inside-or-is-component-or-hook": { any: [{ matches: "component-or-hook" }, { matches: "inside-component-or-hook" }] },
  "inside-component-or-hook": { inside: { stopBy: "end", matches: "fn", all: [{ matches: "component-or-hook" }] } },
};

export const SPECS = [
  {
    id: "react-no-danger",
    tool: R,
    upstream: "react/no-danger",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "Avoid dangerouslySetInnerHTML on DOM elements; injecting raw HTML opens the door to cross-site scripting.",
    checks: [
      {
        id: "upstream",
        rule: { ...prop("dangerouslySetInnerHTML"), inside: { stopBy: "neighbor", any: OPENING, ...DOM_TAG } },
        message: "Render this content as JSX instead of dangerouslySetInnerHTML; raw HTML from data opens the door to cross-site scripting.",
      },
    ],
  },
  {
    id: "react-no-find-dom-node",
    tool: R,
    upstream: "react/no-find-dom-node",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "Use refs instead of findDOMNode(); it is deprecated, fails in StrictMode and doesn't work with function components.",
    checks: [
      {
        id: "upstream",
        rule: {
          kind: "call_expression",
          has: {
            field: "function",
            any: [
              { kind: "identifier", regex: "^findDOMNode$" },
              { kind: "member_expression", has: { field: "property", regex: "^findDOMNode$" } },
            ],
          },
        },
        message: "Replace findDOMNode() with a ref on the element you need; findDOMNode is deprecated and fails in StrictMode.",
      },
    ],
  },
  {
    id: "react-no-is-mounted",
    tool: R,
    upstream: "react/no-is-mounted",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "Avoid this.isMounted() in components; it is deprecated, so cancel pending work when the component unmounts instead.",
    checks: [
      {
        id: "upstream",
        rule: {
          kind: "call_expression",
          has: {
            field: "function",
            kind: "member_expression",
            all: [{ has: { field: "object", kind: "this" } }, { has: { field: "property", regex: "^isMounted$" } }],
          },
          inside: { stopBy: "end", any: [{ kind: "pair" }, { kind: "method_definition" }, { kind: "pair_pattern" }] },
        },
        message: "Remove this isMounted() check, and cancel the pending work when the component unmounts instead.",
      },
    ],
  },
  {
    id: "react-no-render-return-value",
    tool: R,
    upstream: "react/no-render-return-value",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "Don't use the return value of ReactDOM.render(); put a callback ref on the root element to get the instance instead.",
    checks: [
      {
        id: "upstream",
        rule: {
          kind: "call_expression",
          has: {
            field: "function",
            kind: "member_expression",
            all: [{ has: { field: "object", kind: "identifier", regex: "^ReactDOM$" } }, { has: { field: "property", regex: "^render$" } }],
          },
          inside: {
            stopBy: { not: { kind: "parenthesized_expression" } },
            any: [
              { kind: "variable_declarator" },
              { kind: "pair" },
              { kind: "return_statement" },
              { kind: "arrow_function" },
              { kind: "assignment_expression" },
              { kind: "augmented_assignment_expression" },
            ],
          },
        },
        message: "Stop using the return value of ReactDOM.render(); put a callback ref on the root element to get the instance.",
      },
    ],
  },
  {
    id: "react-jsx-no-duplicate-props",
    tool: R,
    upstream: "react/jsx-no-duplicate-props",
    capabilities: ["syntax", "compare", "suppression"],
    match: "line+column",
    description: "Don't give a JSX element the same prop twice; only the last value takes effect.",
    checks: [
      {
        id: "upstream",
        rule: {
          kind: "jsx_attribute",
          has: { kind: "property_identifier", pattern: "$NAME" },
          follows: {
            stopBy: "end",
            kind: "jsx_attribute",
            has: { kind: "property_identifier", pattern: "$EARLIER" },
            compare: { left: "EARLIER", op: "eq", right: "NAME" },
          },
        },
        message: "Remove one of the two `$NAME` props on this element; only the last one takes effect.",
      },
    ],
  },
  {
    id: "react-no-children-prop",
    tool: R,
    upstream: "react/no-children-prop",
    capabilities: ["syntax", "scope", "value", "arguments", "suppression"],
    match: "line+column",
    description: "Nest children between JSX tags, or pass them as extra createElement arguments, instead of passing a children prop.",
    checks: [
      { id: "attribute", rule: prop("children"), message: "Do not pass children as props. Instead, nest children between the opening and closing tags." },
      {
        id: "create-element",
        rule: {
          kind: "call_expression",
          has: { field: "function", matches: "create-element" },
          arguments: { signature: ["type", "props"], bind: { PROPS: "props" }, mode: "estree" },
        },
        constraints: { PROPS: { kind: "object", has: objectKey("children") } },
        utils: CREATE_ELEMENT_UTILS,
        message: "Do not pass children as props. Instead, pass them as additional arguments to React.createElement.",
      },
    ],
  },
  {
    id: "react-jsx-no-comment-textnodes",
    tool: R,
    upstream: "react/jsx-no-comment-textnodes",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "Wrap comments inside JSX children in braces; otherwise the comment text renders on the page.",
    checks: [
      {
        id: "upstream",
        rule: { kind: "jsx_text", regex: "(?m)^\\s*/[/*]", not: { follows: { stopBy: "neighbor", kind: "html_character_reference" } } },
        message: "Comments inside children section of tag should be placed inside braces",
      },
    ],
  },
  {
    id: "react-void-dom-elements-no-children",
    tool: R,
    upstream: "react/void-dom-elements-no-children",
    capabilities: ["syntax", "scope", "value", "arguments", "suppression"],
    match: "line+column",
    description: "Don't give void DOM elements such as img, br and input children or a dangerouslySetInnerHTML prop; React throws when they have content.",
    checks: [
      {
        id: "jsx-children",
        rule: {
          kind: "jsx_element",
          has: { field: "open_tag", has: { field: "name", kind: "identifier", regex: VOID }, matches: "nonempty-open" },
        },
        utils: {
          "nonempty-open": {
            any: [
              { precedes: { stopBy: "neighbor", not: { kind: "jsx_closing_element" } } },
              { layout: { after: { to: "sibling", breaks: { min: 1 } } } },
            ],
          },
        },
        message: "Void DOM element cannot receive children.",
      },
      {
        id: "jsx-attribute",
        rule: {
          any: [{ kind: "jsx_element", has: { field: "open_tag", matches: "void-with-prop" } }, { kind: "jsx_self_closing_element", matches: "void-with-prop" }],
        },
        utils: { "void-with-prop": { all: [{ has: { field: "name", kind: "identifier", regex: VOID } }, { has: prop("children|dangerouslySetInnerHTML") }] } },
        message: "Remove the children or dangerouslySetInnerHTML prop from this void element; img, br, input and other void elements cannot hold content.",
      },
      {
        id: "create-element-child",
        rule: {
          kind: "call_expression",
          has: { field: "function", any: [{ kind: "member_expression" }, { kind: "identifier" }], matches: "create-element" },
          arguments: { signature: ["type", "props", "child"], bind: { TYPE: "type", PROPS: "props", CHILD: "child" }, mode: "estree" },
        },
        constraints: { TYPE: { kind: "string", value: { regex: VOID } }, PROPS: { kind: "object" } },
        utils: CREATE_ELEMENT_UTILS,
        message: "Drop the child arguments from this createElement call; img, br, input and other void elements cannot hold content.",
      },
      {
        id: "create-element-prop",
        rule: {
          kind: "call_expression",
          has: { field: "function", any: [{ kind: "member_expression" }, { kind: "identifier" }], matches: "create-element" },
          arguments: { signature: ["type", "props"], bind: { TYPE: "type", PROPS: "props" }, mode: "estree" },
        },
        constraints: { TYPE: { kind: "string", value: { regex: VOID } }, PROPS: { kind: "object", has: objectKey("children|dangerouslySetInnerHTML") } },
        utils: CREATE_ELEMENT_UTILS,
        message: "Remove children and dangerouslySetInnerHTML from these props; img, br, input and other void elements cannot hold content.",
      },
    ],
  },
  {
    id: "react-self-closing-comp",
    tool: R,
    upstream: "react/self-closing-comp",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "Write elements without children as self-closing tags instead of an empty opening and closing pair.",
    checks: [
      {
        id: "upstream",
        rule: {
          kind: "jsx_opening_element",
          any: [{ has: { field: "name", any: [{ kind: "identifier" }, { kind: "member_expression" }] } }, { has: { field: "name", kind: "jsx_namespace_name", regex: "^[a-z]" } }],
          inside: {
            stopBy: "neighbor",
            kind: "jsx_element",
            not: { has: { not: { any: [{ kind: "jsx_opening_element" }, { kind: "jsx_closing_element" }, { kind: "jsx_text", regex: "^[\\s&&[^\\xA0]]*\\n[\\s&&[^\\xA0]]*$" }] } } },
          },
        },
        message: "Empty components are self-closing",
      },
    ],
  },
  {
    id: "react-no-string-refs",
    tool: R,
    upstream: "react/no-string-refs",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "Use callback refs or createRef instead of string refs; string refs are deprecated and removed in React 19.",
    checks: [
      {
        id: "upstream",
        rule: { kind: "jsx_attribute", all: [{ has: { kind: "property_identifier", regex: "^ref$" } }, { has: { any: [{ kind: "string" }, { kind: "jsx_expression", has: { matches: "string-in-parens" } }] } }] },
        utils: { "string-in-parens": { any: [{ kind: "string" }, { kind: "parenthesized_expression", has: { matches: "string-in-parens" } }] } },
        message: "Replace this string ref with a callback ref or createRef(); string refs are removed in React 19.",
      },
    ],
  },
  {
    id: "react-no-direct-mutation-state",
    tool: R,
    upstream: "react/no-direct-mutation-state",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "Update class component state with setState() instead of assigning to this.state outside the constructor; direct mutation doesn't trigger a re-render.",
    checks: [
      {
        id: "upstream",
        rule: {
          kind: "member_expression",
          all: [
            { has: { field: "object", kind: "this" } },
            { has: { field: "property", regex: "^state$" } },
            { matches: "mutated-chain" },
            { any: [{ matches: "in-es6-component" }, { matches: "in-es5-component" }] },
            { not: { all: [{ matches: "in-es6-component" }, { matches: "in-constructor" }, { not: { matches: "in-constructor-call" } }] } },
          ],
        },
        utils: {
          "mutated-chain": {
            any: [
              { inside: { field: "left", any: [{ kind: "assignment_expression" }, { kind: "augmented_assignment_expression" }] } },
              { inside: { kind: "update_expression" } },
              { inside: { field: "object", any: [{ kind: "member_expression" }, { kind: "subscript_expression" }], matches: "mutated-chain" } },
            ],
          },
          "class-node": { any: [{ kind: "class_declaration" }, { kind: "class" }, { kind: "abstract_class_declaration" }] },
          "component-superclass": {
            any: [
              { kind: "identifier", regex: "^(?:Pure)?Component$" },
              {
                kind: "member_expression",
                all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", regex: "^(?:Pure)?Component$" } }],
              },
            ],
          },
          "es6-component": {
            matches: "class-node",
            has: {
              kind: "class_heritage",
              any: [{ has: { matches: "component-superclass" } }, { has: { kind: "extends_clause", has: { field: "value", matches: "component-superclass" } } }],
            },
          },
          "in-es6-component": { inside: { stopBy: { matches: "class-node" }, matches: "es6-component" } },
          "in-constructor": {
            inside: { stopBy: { matches: "class-node" }, kind: "method_definition", has: { field: "name", regex: "^constructor$" } },
          },
          "in-constructor-call": { inside: { stopBy: { kind: "method_definition" }, kind: "call_expression" } },
          "create-class-object": {
            kind: "object",
            inside: {
              kind: "arguments",
              inside: {
                kind: "call_expression",
                has: {
                  field: "function",
                  any: [
                    { kind: "identifier", regex: "^createReactClass$" },
                    {
                      kind: "member_expression",
                      all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", regex: "^createReactClass$" } }],
                    },
                  ],
                },
              },
            },
          },
          "in-es5-component": {
            inside: {
              stopBy: "end",
              any: [
                {
                  any: [{ kind: "function_expression" }, { kind: "arrow_function" }, { kind: "generator_function" }],
                  inside: { kind: "pair", inside: { matches: "create-class-object" } },
                },
                { kind: "method_definition", inside: { matches: "create-class-object" } },
              ],
            },
          },
        },
        message: "Do not mutate state directly. Use setState().",
      },
    ],
  },
  {
    id: "react-no-danger-with-children",
    tool: R,
    upstream: "react/no-danger-with-children",
    capabilities: ["syntax", "scope", "arguments", "suppression"],
    match: "line+column",
    description: "Don't give an element both dangerouslySetInnerHTML and children; React throws when both are set.",
    checks: [
      {
        id: "jsx",
        rule: {
          any: [
            {
              kind: "jsx_element",
              has: { field: "open_tag", matches: "dangerous" },
              any: [
                {
                  has: {
                    field: "open_tag",
                    precedes: { stopBy: "neighbor", not: { kind: "jsx_closing_element" } },
                    not: { layout: { after: { to: "sibling", breaks: { min: 1 } } } },
                  },
                },
                { has: { field: "open_tag", matches: "children" } },
              ],
            },
            { kind: "jsx_self_closing_element", all: [{ matches: "dangerous" }, { matches: "children" }] },
          ],
        },
        utils: {
          ...jsxPropUtil("dangerous", "dangerouslySetInnerHTML"),
          ...jsxPropUtil("children", "children"),
        },
        message: "Only set one of `children` or `props.dangerouslySetInnerHTML`",
      },
      {
        id: "create-element-child",
        rule: {
          kind: "call_expression",
          has: { field: "function", kind: "member_expression", has: { field: "property", regex: "^createElement$" } },
          arguments: { signature: ["type", "props", "child"], bind: { PROPS: "props", CHILD: "child" }, mode: "estree" },
        },
        constraints: { PROPS: { matches: "props-dangerous" } },
        utils: { ...objectPropUtil("dangerous-obj", "dangerouslySetInnerHTML"), "props-dangerous": propsMatching(["dangerous-obj"]) },
        message: "Only set one of `children` or `props.dangerouslySetInnerHTML`",
      },
      {
        id: "create-element-prop",
        rule: {
          kind: "call_expression",
          has: { field: "function", kind: "member_expression", has: { field: "property", regex: "^createElement$" } },
          arguments: { signature: ["type", "props", "child"], bind: { PROPS: "props" }, absent: ["child"], mode: "estree" },
        },
        constraints: { PROPS: { matches: "props-both" } },
        utils: {
          ...objectPropUtil("dangerous-obj", "dangerouslySetInnerHTML"),
          ...objectPropUtil("children-obj", "children"),
          "props-both": propsMatching(["dangerous-obj", "children-obj"]),
        },
        message: "Only set one of `children` or `props.dangerouslySetInnerHTML`",
      },
    ],
  },
  {
    id: "react-button-has-type",
    shared: BUTTON_UTILS,
    tool: R,
    upstream: "react/button-has-type",
    capabilities: ["syntax", "scope", "value", "suppression"],
    match: "line+column",
    description: "Give every button an explicit type of button, submit or reset; a button without one submits its form by default.",
    checks: [
      {
        id: "jsx-missing",
        rule: { any: BUTTON_TAG, not: { has: { matches: "type-attribute" } } },
        utils: BUTTON_UTILS,
        message: "Missing an explicit type attribute for button",
      },
      {
        id: "jsx-invalid-attribute",
        rule: {
          any: BUTTON_TAG,
          has: {
            kind: "jsx_attribute",
            matches: "first-type-attribute",
            any: [
              { not: { has: { any: [{ kind: "string" }, { kind: "jsx_expression" }, { kind: "jsx_element" }, { kind: "jsx_self_closing_element" }] } } },
              { has: { kind: "string", not: { value: { regex: TYPE_VALUES } } } },
              { has: { any: [{ kind: "jsx_element" }, { kind: "jsx_self_closing_element" }] } },
            ],
          },
        },
        utils: BUTTON_UTILS,
        message: "Set this button type to \"button\", \"submit\" or \"reset\".",
      },
      ...["", "consequence", "alternative"].flatMap(branch => [
        {
          id: `jsx-invalid${branch ? `-${branch}` : ""}`,
          rule: { any: BUTTON_TAG, has: { kind: "jsx_attribute", matches: "first-type-attribute", has: { kind: "jsx_expression", has: { matches: leafUtil("invalid", branch) } } } },
          utils: BUTTON_UTILS,
          message: branch ? 'Change this ternary branch to "button", "submit" or "reset".' : 'Set this button type to "button", "submit" or "reset".',
        },
        {
          id: `jsx-complex${branch ? `-${branch}` : ""}`,
          rule: { any: BUTTON_TAG, has: { kind: "jsx_attribute", matches: "first-type-attribute", has: { kind: "jsx_expression", has: { matches: leafUtil("complex", branch) } } } },
          anchor: "capture.LEAF",
          utils: BUTTON_UTILS,
          message: branch ? 'The button type attribute must be specified by a static string or a trivial ternary expression' : 'The button type attribute must be specified by a static string or a trivial ternary expression',
        },
      ]),
      {
        id: "create-element-missing",
        rule: {
          matches: "button-create-element",
          not: { has: { field: "arguments", has: { kind: "object", nthChild: 2, has: objectKey("type") } } },
        },
        utils: { ...BUTTON_UTILS, ...CREATE_ELEMENT_UTILS },
        message: "Missing an explicit type attribute for button",
      },
      ...["", "consequence", "alternative"].flatMap(branch => [
        {
          id: `create-element-invalid${branch ? `-${branch}` : ""}`,
          rule: { matches: "button-create-element", has: { field: "arguments", has: { kind: "object", nthChild: 2, has: { kind: "pair", matches: "ce-first-type-property", has: { field: "value", matches: leafUtil("invalid", branch) } } } } },
          utils: BUTTON_UTILS,
          message: branch ? 'Change this ternary branch to "button", "submit" or "reset".' : 'Set this button type to "button", "submit" or "reset".',
        },
        {
          id: `create-element-complex${branch ? `-${branch}` : ""}`,
          rule: { matches: "button-create-element", has: { field: "arguments", has: { kind: "object", nthChild: 2, has: { kind: "pair", matches: "ce-first-type-property", has: { field: "value", matches: leafUtil("complex", branch) } } } } },
          anchor: "capture.LEAF",
          utils: BUTTON_UTILS,
          message: branch ? 'The button type attribute must be specified by a static string or a trivial ternary expression' : 'The button type attribute must be specified by a static string or a trivial ternary expression',
        },
      ]),
      {
        id: "create-element-complex-shorthand",
        rule: { matches: "button-create-element", has: { field: "arguments", has: { kind: "object", nthChild: 2, has: { kind: "shorthand_property_identifier", regex: "^type$", pattern: "$LEAF", matches: "ce-first-type-property" } } } },
        anchor: "capture.LEAF",
        utils: BUTTON_UTILS,
        message: "Replace the shorthand `$LEAF` with type: \"button\", \"submit\" or \"reset\" so the button type is a literal.",
      },
    ],
  },
  {
    id: "react-jsx-key",
    shared: JSX_KEY_UTILS,
    tool: R,
    upstream: "react/jsx-key",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "Add a key prop to JSX elements in array literals and to those returned from map or Array.from callbacks; React needs keys to track list items between renders.",
    checks: [
      {
        id: "array",
        rule: { matches: "keyless-element", all: [{ not: { has: { field: "open_tag", has: { kind: "jsx_attribute", has: { kind: "property_identifier", regex: "^key$" } } } } }, { not: { has: { kind: "jsx_attribute", has: { kind: "property_identifier", regex: "^key$" } } } }, { matches: "array-element" }, { not: { matches: "in-to-array" } }] },
        utils: JSX_KEY_UTILS,
        message: "Missing \"key\" prop for element in array",
      },
      {
        id: "iterator",
        rule: { matches: "keyless-element", all: [{ not: { matches: "has-key-ci" } }, { matches: "iterator-element" }, { not: { matches: "in-to-array" } }] },
        utils: JSX_KEY_UTILS,
        message: "Missing \"key\" prop for element in iterator",
      },
    ],
  },
  {
    id: "react-jsx-no-undef",
    tool: R,
    upstream: "react/jsx-no-undef",
    capabilities: ["syntax", "scope", "suppression"],
    match: "line+column",
    description: "Import or declare every component used in JSX in the same file; an undefined component name crashes at render.",
    checks: [
      {
        id: "upstream",
        rule: {
          kind: "identifier",
          pattern: "$NAME",
          any: [{ inside: { field: "name", any: OPENING }, regex: "^[^a-z]" }, { matches: "member-root" }],
          not: { resolves: { binding: {} } },
        },
        utils: {
          "name-chain": { any: [{ inside: { field: "name", any: OPENING } }, { inside: { field: "object", kind: "member_expression", matches: "name-chain" } }] },
          "member-root": { inside: { field: "object", kind: "member_expression", matches: "name-chain" } },
        },
        message: "'$NAME' is not defined.",
      },
    ],
  },
  // Ships off when promoted: its pack.json entry takes `"on": false`, mirroring plugin:react/jsx-runtime.
  // Under React 17's automatic JSX runtime (tsconfig `"jsx": "react-jsx"`, babel `runtime: "automatic"`, Next.js,
  // Vite) `React` need not be in scope, so the rule flags every JSX file and steers agents to add an unused
  // `import React`. Upstream has no option for this either, and a policy cannot read the project's runtime. A repo should
  // turn it on itself when its ESLint config enables react/react-in-jsx-scope or it uses the classic runtime. Evaluation
  // runs must not attach this candidate directly.
  {
    id: "react-react-in-jsx-scope",
    tool: R,
    upstream: "react/react-in-jsx-scope",
    capabilities: ["syntax", "scope", "suppression"],
    match: "line+column",
    description: "Import React in every file that uses JSX; the classic JSX transform compiles tags to React.createElement calls.",
    checks: [
      {
        id: "upstream",
        rule: {
          any: [{ kind: "jsx_opening_element" }, { kind: "jsx_self_closing_element" }],
          not: { any: [{ matches: "react-visible" }, { matches: "other-pragma-file" }] },
        },
        utils: REACT_SCOPE_UTILS,
        message: "'React' must be in scope when using JSX",
      },
      {
        id: "pragma",
        rule: {
          any: [{ kind: "jsx_opening_element" }, { kind: "jsx_self_closing_element" }],
          inside: { stopBy: "end", kind: "program", has: { ...PRAGMA_COMMENT, not: { regex: REACT_PRAGMA }, pattern: "$CM" } },
          not: { matches: "pragma-visible" },
        },
        utils: PRAGMA_SCOPE_UTILS,
        transform: { PRAGMA: { replace: { source: "$CM", replace: "(?s)^.*?@jsx\\s+([_$a-zA-Z][_$a-zA-Z0-9]*)[\\s\\S]*$", by: "${1}" } } },
        message: "'$PRAGMA' must be in scope when using JSX",
      },
    ],
  },
  {
    id: "react-no-unescaped-entities",
    tool: R,
    upstream: "react/no-unescaped-entities",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "Escape \" and ' in JSX text as HTML entities or string expressions; a stray quote there is often a leftover from a string or attribute.",
    checks: [
      ["double-quote", "\"", "&quot;", `{'"'}`],
      ["single-quote", "'", "&apos;", `{"'"}`],
    ].map(([id, character, entity, expression]) => ({
      id,
      rule: { kind: "jsx_text", regex: `(?<ENTITY>${character})` },
      anchor: "capture.ENTITY",
      each: "ENTITY",
      message: `Write this ${character} in JSX text as ${entity} or wrap it in a ${expression} string expression.`,
    })),
  },
  {
    id: "react-jsx-no-target-blank",
    tool: R,
    upstream: "react/jsx-no-target-blank",
    capabilities: ["syntax", "value", "compare", "suppression"],
    match: "line+column",
    description: "Add a noreferrer rel to links that open external or dynamic URLs with target _blank; otherwise the new page can reach back through window.opener.",
    checks: [
      {
        id: "upstream",
        rule: {
          any: [
            { kind: "jsx_opening_element", has: { field: "name", kind: "identifier", regex: "^a$" } },
            { kind: "jsx_self_closing_element", has: { field: "name", kind: "identifier", regex: "^a$" } },
          ],
          all: [
            { has: { matches: "last-target", all: [{ matches: "possibly-blank" }] } },
            { matches: "dangerous-link" },
            { not: { matches: "secure-rel" } },
          ],
        },
        utils: TARGET_BLANK_UTILS,
        message: "Using target=\"_blank\" without rel=\"noreferrer\" (which implies rel=\"noopener\") is a security risk in older browsers",
      },
    ],
  },
  {
    id: "react-hooks-rules-of-hooks",
    shared: Object.fromEntries(Object.entries(HOOKS_UTILS).filter(([name]) => name !== "class-field" && name !== "class-function")),
    tool: H,
    upstream: "react-hooks/rules-of-hooks",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "Call React hooks only at the top level of components and custom hooks, never in conditions, loops, nested callbacks, async functions or after an early return.",
    checks: [
      ...[["ts", "public_field_definition", "\\.(?:[cm]?ts|tsx)$"], ["js", "field_definition", "\\.(?:[cm]?js|jsx)$"]].map(([dialect, fieldKind, path]) => (
      {
        id: `placement-${dialect}`,
        rule: {
          kind: "call_expression",
          all: [{ matches: "hook" }, { any: [
            {
              all: [
                insideFn({ matches: "component-or-hook" }),
                { not: { matches: "use-hook" } },
                { not: { matches: "cycled" } },
                { not: { matches: "in-do-while" } },
                { any: [{ matches: "conditional" }, { matches: "early-return" }] },
              ],
            },
            insideFn({ not: { matches: "component-or-hook" }, matches: "class-function" }),
            insideFn({ not: { any: [{ matches: "component-or-hook" }, { matches: "class-function" }] }, matches: "named" }),
            { not: { inside: { stopBy: "end", any: [...FN_KINDS, { matches: "class-field" }, { kind: "class_static_block" }] } } },
            {
              all: [
                insideFn({ not: { any: [{ matches: "component-or-hook" }, { matches: "class-function" }, { matches: "named" }] }, matches: "inside-component-or-hook" }),
                { not: { matches: "use-hook" } },
              ],
            },
          ] }],
        },
        utils: { ...HOOKS_UTILS, "class-field": { kind: fieldKind } },
        file: { path: { regex: path } },
        message: "Move this hook call to the top level of the component or custom hook, before any condition or early return.",
      }
      )),
      {
        id: "loop",
        rule: { kind: "call_expression", all: [{ matches: "hook" }, { not: { matches: "use-hook" } }, { any: [{ matches: "cycled" }, { matches: "in-do-while" }] }] },
        utils: HOOKS_UTILS,
        message: "React Hook may be executed more than once. Possibly because it is called in a loop.",
      },
      {
        id: "async",
        rule: { kind: "call_expression", all: [{ matches: "hook" }, insideFn({ matches: "component-or-hook", has: { regex: "^async$" } })] },
        utils: HOOKS_UTILS,
        message: "React Hook cannot be called in an async function.",
      },
      {
        id: "effect-event-reference",
        rule: {
          any: [{ kind: "identifier" }, { kind: "shorthand_property_identifier" }],
          all: [
            {
              inside: {
                stopBy: "end",
                any: [{ kind: "function_declaration" }, { kind: "arrow_function" }],
                has: { field: "body", kind: "statement_block", has: { any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }], has: { kind: "variable_declarator", matches: "effect-event-value" } } },
              },
            },
            { not: { inside: { field: "name", kind: "variable_declarator" } } },
            { resolves: { binding: { site: { kind: "variable_declarator", matches: "effect-event-declarator" } } } },
            { not: { inside: { stopBy: "end", matches: "effect-call" } } },
          ],
        },
        utils: HOOKS_UTILS,
        message: "A function created with React Hook \"useEffectEvent\" can only be called from Effects and Effect Events in the same component.",
      },
      {
        id: "effect-event-passed",
        rule: {
          kind: "call_expression",
          has: { field: "function", matches: "effect-event-callee" },
          not: { inside: { any: [{ kind: "variable_declarator" }, { kind: "expression_statement" }] } },
        },
        utils: HOOKS_UTILS,
        message: "React Hook \"useEffectEvent\" can only be called at the top level of your component. It cannot be passed down.",
      },
      {
        id: "use-in-try",
        rule: { kind: "call_expression", all: [{ matches: "use-hook" }, { inside: { stopBy: "end", any: [{ kind: "try_statement" }, { kind: "catch_clause" }] } }] },
        utils: HOOKS_UTILS,
        message: "Move this use() call out of the try/catch block, and handle its errors with an error boundary.",
      },
    ],
  },
  // exhaustive-deps: every report of ExhaustiveDeps.ts, each its own clause (README "Rule details"). Captures thread
  // the hook's context: CALL (the hook call), C (the function it runs in), CB (its callback), DEPS and DA (its
  // dependencies, and the array they are), CSN (the scope the call runs in). Utilities are prefixed `ed-`.
  (() => {
    const callee = re => ({
      any: [
        { kind: "identifier", regex: re },
        { kind: "member_expression", all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", kind: "property_identifier", regex: re } }] },
      ],
    });
    const calling = re => ({ kind: "call_expression", has: { field: "function", ...callee(re) } });
    const same = (left, right, text = "node") => ({ compare: { left, op: "eq", right, text } });
    /** The same dependency path: by reference, or by name for a shorthand property (which ESTree reads as a name). */
    const samePath = (left, right) => ({ any: [same(left, right, "reference"), same(left, right, "source")] });
    /** `name` holds on the expression inside an `as` cast or parentheses (ESTree has neither). */
    const unwrapped = name => ({ any: [{ kind: "as_expression" }, { kind: "parenthesized_expression" }], has: { nthChild: 1, matches: name } });
    /** `rule` holds on the dependency array, written bare or cast with `as`. */
    const onArray = rule => ({ any: [{ kind: "array", ...rule }, { kind: "as_expression", has: { kind: "array", ...rule } }] });
    /** The `position`th argument, comments aside. */
    const arg = position => ({ position, ofRule: { not: { kind: "comment" } } });
    const FN = ["function_declaration", "generator_function_declaration", "function_expression", "generator_function", "arrow_function", "method_definition"].map(kind => ({ kind }));
    const INLINE_FN = [{ kind: "arrow_function" }, { kind: "function_expression" }, { kind: "generator_function" }];
    const DECLARED_FN = [{ kind: "function_declaration" }, { kind: "generator_function_declaration" }];
    const REF = [{ kind: "identifier" }, { kind: "shorthand_property_identifier" }];
    const ASSIGN = [{ kind: "assignment_expression" }, { kind: "augmented_assignment_expression" }];
    const LITERAL = ["string", "number", "true", "false", "null", "regex"].map(kind => ({ kind }));
    /** Utility `name`: at a callback slot whose callback (inline, or the function the name there declares) holds `target` in its body. */
    const inCallback = (name, target) => ({
      [name]: {
        any: [
          { any: INLINE_FN, has: { stopBy: "end", matches: target } },
          unwrapped(name),
          {
            kind: "identifier",
            resolves: {
              binding: {
                site: {
                  any: [
                    { any: DECLARED_FN, has: { stopBy: "end", matches: target } },
                    { kind: "variable_declarator", has: { field: "value", any: INLINE_FN, has: { stopBy: "end", matches: target } } },
                  ],
                },
              },
            },
          },
        ],
      },
    });
    const U = {
      "ed-fn": { any: FN },
      "ed-scope-node": {
        any: [
          ...FN,
          { kind: "program" },
          { kind: "class_body" },
          { kind: "statement_block", not: { inside: { field: "body", any: FN } } },
          { kind: "for_statement" },
          { kind: "for_in_statement" },
          { kind: "catch_clause" },
          { kind: "switch_statement" },
        ],
      },
      "ed-index0-call": calling("^(?:useEffect|useLayoutEffect|useCallback|useMemo)$"),
      "ed-imperative-call": calling("^useImperativeHandle$"),
      "ed-effect-call": calling("^(?:useEffect|useLayoutEffect)$"),
      "ed-memo-call": calling("^(?:useCallback|useMemo)$"),
      "ed-hook-call": { any: [{ matches: "ed-index0-call" }, { matches: "ed-imperative-call" }] },
      // The argument a hook's callback (index 0, useImperativeHandle's 1) or its dependencies (the next one) stand in.
      "ed-cb-slot": {
        not: { kind: "comment" },
        any: [
          { nthChild: arg(1), inside: { kind: "arguments", inside: { matches: "ed-index0-call" } } },
          { nthChild: arg(2), inside: { kind: "arguments", inside: { matches: "ed-imperative-call" } } },
        ],
      },
      "ed-deps-slot": {
        not: { any: [{ regex: "^undefined$" }, { kind: "comment" }] },
        any: [
          { nthChild: arg(2), inside: { kind: "arguments", inside: { matches: "ed-index0-call" } } },
          { nthChild: arg(3), inside: { kind: "arguments", inside: { matches: "ed-imperative-call" } } },
        ],
      },
      "ed-has-deps": { has: { field: "arguments", has: { matches: "ed-deps-slot" } } },
      "ed-effect-cb": {
        any: [{ matches: "ed-cb-slot", inside: { kind: "arguments", inside: { matches: "ed-effect-call" } } }, { inside: { any: [{ kind: "as_expression" }, { kind: "parenthesized_expression" }], matches: "ed-effect-cb" } }],
      },
      "ed-param-site": {
        any: [{ kind: "required_parameter" }, { kind: "optional_parameter" }, { inside: { kind: "formal_parameters" } }, { inside: { field: "parameter", kind: "arrow_function" } }],
      },
      // The site declares its name in the scope the hook call runs in (CSN).
      "ed-call-scope-site": { inside: { stopBy: { matches: "ed-scope-node" }, matches: "ed-scope-node", pattern: "$SSN", ...same("SSN", "CSN") } },
      // A callback passed by name: a name the dependencies are given and do not list, declared in the hook's scope.
      "ed-named-cb": {
        kind: "identifier",
        pattern: "$CBN",
        all: [
          { inside: { stopBy: "end", kind: "arguments", has: { matches: "ed-deps-slot", not: onArray({ has: { kind: "identifier", pattern: "$CBD", ...same("CBD", "CBN", "name") } }) } } },
          { resolves: { binding: { site: { matches: "ed-call-scope-site" } } } },
        ],
      },
      // The callback at a slot: inline, through casts and parentheses, or the function the name passed declares. Binds CB.
      "ed-cb": {
        any: [
          { any: INLINE_FN, pattern: "$CB" },
          unwrapped("ed-cb"),
          {
            matches: "ed-named-cb",
            resolves: {
              binding: {
                site: { any: [{ any: DECLARED_FN, pattern: "$CB" }, { kind: "variable_declarator", has: { field: "value", any: INLINE_FN, pattern: "$CB" } }] },
              },
            },
          },
        ],
      },
      // At an analysed hook call (an effect, or one given dependencies) inside a function: binds CALL, CSN, C and CB.
      "ed-call-context": {
        kind: "call_expression",
        pattern: "$CALL",
        all: [
          { any: [{ matches: "ed-effect-call" }, { matches: "ed-imperative-call" }, { matches: "ed-memo-call" }] },
          { any: [{ matches: "ed-effect-call" }, { matches: "ed-has-deps" }] },
          { inside: { stopBy: { matches: "ed-scope-node" }, matches: "ed-scope-node", pattern: "$CSN" } },
          { inside: { stopBy: { matches: "ed-fn" }, matches: "ed-fn", pattern: "$C" } },
          { has: { field: "arguments", has: { matches: "ed-cb-slot", all: [{ matches: "ed-cb" }] } } },
        ],
      },
      // ... whose callback assigns no value of C (that is reported first, and alone).
      "ed-call-root": { all: [{ matches: "ed-call-context" }, { not: { has: { field: "arguments", has: { matches: "ed-cb-slot", all: [{ matches: "ed-cb-stale-write" }] } } } }] },
      // At the dependencies of such a call: binds DEPS, and DA when they are an array.
      "ed-deps-root": {
        pattern: "$DEPS",
        all: [
          { matches: "ed-deps-slot" },
          { inside: { kind: "arguments", inside: { matches: "ed-call-root" } } },
          { any: [{ kind: "array", pattern: "$DA" }, { kind: "as_expression", has: { kind: "array", pattern: "$DA" } }, { regex: "^" }] },
        ],
      },
      // A name the callback reads that is declared in C, outside the callback (a value of the render).
      "ed-ref": {
        any: REF,
        not: {
          any: [
            { inside: { stopBy: "end", kind: "type_query" } },
            { inside: { field: "name", any: [...INLINE_FN, ...DECLARED_FN, { kind: "class_declaration" }] } },
          ],
        },
        resolves: {
          binding: {
            site: {
              all: [
                { inside: { stopBy: "end", matches: "ed-fn", pattern: "$RSC", ...same("RSC", "C") } },
                { not: { inside: { stopBy: "end", pattern: "$RSCB", ...same("RSCB", "CB") } } },
                { not: { has: { field: "value", pattern: "$RSV", ...same("RSV", "CALL") } } },
                // A function expression's own name lives in a scope of its own, outside the render's.
                { not: { any: [{ kind: "function_expression" }, { kind: "generator_function" }], pattern: "$RSS", ...same("RSS", "CB") } },
              ],
            },
          },
        },
      },
      "ed-unwrap-literal": { any: [{ kind: "string" }, { kind: "number" }, { kind: "null" }, unwrapped("ed-unwrap-literal")] },
      "ed-unwrap-stable-call": { any: [calling("^(?:useRef|useEffectEvent)$"), unwrapped("ed-unwrap-stable-call")] },
      "ed-unwrap-effect-event": { any: [calling("^useEffectEvent$"), unwrapped("ed-unwrap-effect-event")] },
      "ed-unwrap-tuple-call": { any: [calling("^(?:useState|useReducer|useActionState|useTransition)$"), unwrapped("ed-unwrap-tuple-call")] },
      "ed-unwrap-state-call": { any: [calling("^useState$"), unwrapped("ed-unwrap-state-call")] },
      // A value React keeps stable, or a constant literal: useRef, useEffectEvent, const x = 'a' | 1 | null.
      "ed-stable-site": {
        kind: "variable_declarator",
        any: [
          { all: [{ has: { field: "name", kind: "identifier" } }, { has: { field: "value", matches: "ed-unwrap-stable-call" } }] },
          { all: [{ inside: { kind: "lexical_declaration", has: { regex: "^const$" } } }, { has: { field: "value", matches: "ed-unwrap-literal" } }] },
        ],
      },
      // The second of exactly two names a tuple is destructured into, the name SETR reads.
      "ed-second-of-two": {
        has: {
          field: "name",
          kind: "array_pattern",
          has: {
            kind: "identifier",
            pattern: "$SETN",
            ...same("SETN", "SETR", "source"),
            follows: { stopBy: "neighbor", regex: "^,$", not: { follows: { stopBy: "end", regex: "^,$" } } },
            not: { precedes: { stopBy: "end", not: { regex: "^[,\\]]$" } } },
          },
        },
      },
      "ed-setter-site": { kind: "variable_declarator", all: [{ has: { field: "value", matches: "ed-unwrap-tuple-call" } }, { matches: "ed-second-of-two" }] },
      "ed-state-setter-site": { kind: "variable_declarator", all: [{ has: { field: "value", matches: "ed-unwrap-state-call" } }, { matches: "ed-second-of-two" }] },
      "ed-reassigned": { resolves: { binding: { occurrence: { inside: { field: "left", any: ASSIGN } } } } },
      "ed-stable-hook-value": {
        any: [
          { resolves: { binding: { site: { matches: "ed-stable-site" } } } },
          { pattern: "$SETR", not: { matches: "ed-reassigned" }, resolves: { binding: { site: { matches: "ed-setter-site" } } } },
        ],
      },
      "ed-state-setter": { pattern: "$SETR", not: { matches: "ed-reassigned" }, resolves: { binding: { site: { matches: "ed-state-setter-site" } } } },
      "ed-top-of-c": { inside: { kind: "statement_block", inside: { field: "body", pattern: "$TC", ...same("TC", "C") } } },
      // A function declared at the top of C that reads no value of C but stable ones.
      "ed-pure-fn-site": {
        pattern: "$PFS",
        any: [
          { any: DECLARED_FN, matches: "ed-top-of-c" },
          {
            kind: "variable_declarator",
            all: [
              { has: { field: "name", kind: "identifier" } },
              { has: { field: "value", any: INLINE_FN } },
              { inside: { any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }], matches: "ed-top-of-c" } },
            ],
          },
        ],
        not: { has: { stopBy: "end", matches: "ed-captured-ref" } },
      },
      "ed-captured-ref": {
        any: REF,
        not: { matches: "ed-stable-hook-value" },
        resolves: {
          binding: {
            site: {
              all: [
                { inside: { stopBy: "end", matches: "ed-fn", pattern: "$CRC", ...same("CRC", "C") } },
                { not: { inside: { stopBy: "end", pattern: "$CRF", ...same("CRF", "PFS") } } },
              ],
            },
          },
        },
      },
      "ed-stable": { any: [{ matches: "ed-stable-hook-value" }, { resolves: { binding: { site: { matches: "ed-pure-fn-site" } } } }] },
      // A dependency path: a name, or a non-computed member of one.
      "ed-chain": {
        any: [
          { kind: "identifier" },
          { kind: "undefined" },
          {
            kind: "member_expression",
            all: [{ has: { field: "object", matches: "ed-chain" } }, { has: { field: "property", any: [{ kind: "property_identifier" }, { kind: "private_property_identifier" }] } }],
          },
        ],
      },
      // What ESTree reads as a declared path: also an optional chain ending in a computed member keyed by a path
      // (`a?.[b.c]` reads as `a.b.c`), which never matches a value the callback reads.
      "ed-declared-path": {
        any: [
          { matches: "ed-chain" },
          {
            kind: "subscript_expression",
            all: [
              { has: { field: "object", matches: "ed-chain" } },
              { has: { field: "index", matches: "ed-chain" } },
              { any: [{ has: { kind: "optional_chain" } }, { has: { field: "object", matches: "ed-optional" } }] },
            ],
          },
        ],
      },
      "ed-optional": { kind: "member_expression", any: [{ has: { kind: "optional_chain" } }, { has: { field: "object", matches: "ed-optional" } }] },
      // A member the dependency path of the name inside it stops before: `.current`, a callee, an assignment target.
      "ed-chain-excluded": {
        kind: "member_expression",
        any: [{ has: { field: "property", regex: "^current$" } }, { inside: { field: "function", kind: "call_expression" } }, { inside: { field: "left", any: ASSIGN } }],
      },
      "ed-chain-stop": { any: [{ not: { kind: "member_expression" } }, { matches: "ed-chain-excluded" }] },
      // The node's path is an element of the dependency array DA of CALL, which runs in C.
      "ed-declared": {
        params: ["CALL", "DA", "C"],
        rule: {
          pattern: "$X",
          any: [
            {
              inside: {
                stopBy: "end",
                kind: "call_expression",
                pattern: "$XCALL",
                ...same("XCALL", "CALL"),
                has: { field: "arguments", has: onArray({ pattern: "$XD", ...same("XD", "DA"), has: { pattern: "$XE", all: [samePath("XE", "X"), { matches: "ed-chain" }] } }) },
              },
            },
            {
              not: { inside: { stopBy: "end", pattern: "$XCALL", ...same("XCALL", "CALL") } },
              inside: {
                stopBy: "end",
                pattern: "$XC",
                ...same("XC", "C"),
                has: { stopBy: "end", pattern: "$XD", ...same("XD", "DA"), has: { pattern: "$XE", all: [samePath("XE", "X"), { matches: "ed-chain" }] } },
              },
            },
          ],
        },
      },
      // A name whose dependency path DA declares, itself or a prefix of it.
      "ed-covered": {
        any: [
          { matches: { "ed-declared": { CALL: "$CALL", DA: "$DA", C: "$C" } } },
          {
            inside: {
              stopBy: { matches: "ed-chain-stop" },
              kind: "member_expression",
              not: { matches: "ed-chain-excluded" },
              matches: { "ed-declared": { CALL: "$CALL", DA: "$DA", C: "$C" } },
            },
          },
        ],
      },
      "ed-missing-ref": { pattern: "$MR", matches: "ed-ref", not: { any: [{ matches: "ed-stable" }, { matches: "ed-covered" }] } },
      ...inCallback("ed-cb-missing", "ed-missing-ref"),
      // A name whose dependency path is SE or extends it.
      "ed-reaches": {
        any: [
          { pattern: "$SR", ...samePath("SR", "SE") },
          { inside: { stopBy: { matches: "ed-chain-stop" }, kind: "member_expression", not: { matches: "ed-chain-excluded" }, pattern: "$SM", ...same("SM", "SE", "reference") } },
        ],
      },
      "ed-ref-reaching": { all: [{ matches: "ed-reaches" }, { matches: "ed-ref" }] },
      ...inCallback("ed-cb-reaching", "ed-ref-reaching"),
      // An element SE of DA that a value the callback reads needs, and no shorter declared path covers.
      "ed-satisfying": {
        pattern: "$SE",
        not: {
          has: {
            stopBy: "end",
            any: [{ kind: "identifier" }, { kind: "member_expression" }],
            pattern: "$PX",
            inside: { stopBy: "end", kind: "array", pattern: "$PD", ...same("PD", "DA"), has: { matches: "ed-chain", pattern: "$PE", ...same("PE", "PX", "reference") } },
          },
        },
        inside: { stopBy: "end", kind: "arguments", has: { matches: "ed-cb-slot", all: [{ matches: "ed-cb-reaching" }] } },
      },
      // A path rooted outside C: a global, or a name of an outer scope.
      "ed-external": {
        any: [
          { kind: "undefined" },
          { kind: "identifier", not: { resolves: { binding: { site: { inside: { stopBy: "end", matches: "ed-fn", pattern: "$XC", ...same("XC", "C") } } } } } },
          { any: [{ kind: "member_expression" }, { kind: "subscript_expression" }], has: { field: "object", matches: "ed-external" } },
        ],
      },
      "ed-ref-current": { kind: "member_expression", has: { field: "property", regex: "^current$" } },
      // An assignment in the callback to a value of C: lost after the render.
      "ed-stale-write": { any: ASSIGN, has: { field: "left", kind: "identifier", matches: "ed-ref" } },
      ...inCallback("ed-cb-stale-write", "ed-stale-write"),
      "ed-missing": { inside: { kind: "arguments", has: { matches: "ed-cb-slot", all: [{ matches: "ed-cb-missing" }] } } },
      // An element no value the callback reads needs; an effect may list extra values, but not outer or `.current` ones.
      "ed-unnecessary": {
        any: [
          { all: [{ inside: { kind: "arguments", inside: { not: { matches: "ed-effect-call" } } } }, onArray({ has: { matches: "ed-declared-path", pattern: "$UE", not: { matches: "ed-satisfying" } } })] },
          onArray({ has: { matches: "ed-declared-path", pattern: "$UE", any: [{ matches: "ed-ref-current" }, { matches: "ed-external" }] } }),
        ],
      },
      "ed-duplicate": onArray({
        has: { matches: "ed-chain", pattern: "$DE", follows: { stopBy: "end", pattern: "$DE1", all: [same("DE1", "DE", "reference"), { matches: "ed-chain" }, { matches: "ed-satisfying-dup" }] } },
      }),
      "ed-satisfying-dup": { pattern: "$SE", matches: "ed-satisfying" },
      // An element of an analysed dependency array.
      "ed-element": { inside: { kind: "array", any: [{ matches: "ed-deps-root" }, { inside: { kind: "as_expression", matches: "ed-deps-root" } }] } },
      "ed-problem": { any: [{ matches: "ed-missing" }, { matches: "ed-unnecessary" }, { matches: "ed-duplicate" }] },
      "ed-settled-deps": { all: [{ matches: "ed-deps-root" }, { not: { matches: "ed-problem" } }] },
      // A value each render builds anew: an object, array, function, class, element, `new`, regex, or a branch of one.
      "ed-construction": {
        any: [
          ...["object", "array", "arrow_function", "function_expression", "generator_function", "class", "jsx_element", "jsx_self_closing_element", "new_expression", "regex"].map(kind => ({ kind })),
          { kind: "ternary_expression", any: [{ has: { field: "consequence", matches: "ed-construction" } }, { has: { field: "alternative", matches: "ed-construction" } }] },
          {
            kind: "binary_expression",
            has: { field: "operator", regex: "^(?:&&|\\|\\||\\?\\?)$" },
            any: [{ has: { field: "left", matches: "ed-construction" } }, { has: { field: "right", matches: "ed-construction" } }],
          },
          { kind: "assignment_expression", has: { field: "right", matches: "ed-construction" } },
          unwrapped("ed-construction"),
        ],
      },
      // A top-level declaration of C whose value each render builds anew (KS).
      "ed-construction-kind": {
        any: [
          { any: [...DECLARED_FN, { kind: "class_declaration" }] },
          { kind: "variable_declarator", all: [{ has: { field: "name", kind: "identifier" } }, { has: { field: "value", matches: "ed-construction" } }] },
        ],
      },
      "ed-construction-site": {
        pattern: "$KS",
        all: [
          { matches: "ed-construction-kind" },
          { any: [{ matches: "ed-top-of-c" }, { inside: { any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }], matches: "ed-top-of-c" } }] },
        ],
      },
      // Inside the effect's cleanup: the function directly below the callback is the one it returns.
      "ed-in-cleanup": {
        inside: {
          stopBy: "end",
          matches: "ed-fn",
          all: [{ inside: { kind: "return_statement" } }, { inside: { stopBy: { matches: "ed-fn" }, matches: "ed-fn", pattern: "$CUF", ...same("CUF", "CB") } }],
        },
      },
      "ed-current-assigned": {
        resolves: { binding: { occurrence: { inside: { kind: "member_expression", has: { field: "property", regex: "^current$" }, inside: { field: "left", any: ASSIGN } } } } },
      },
      "ed-cleanup-current-read": {
        kind: "member_expression",
        all: [{ has: { field: "property", regex: "^current$" } }, { has: { field: "object", kind: "identifier", matches: "ed-ref" } }, { matches: "ed-in-cleanup" }],
      },
      "ed-later-cleanup-read": {
        kind: "member_expression",
        all: [{ matches: "ed-cleanup-current-read" }, { has: { field: "object", kind: "identifier", pattern: "$LCR", ...same("LCR", "CCR", "name") } }],
      },
      // The `current` of the last `X.current` the cleanup reads, for an X no code assigns `.current` to (React's ref).
      "ed-cleanup-current": {
        kind: "property_identifier",
        regex: "^current$",
        pattern: "$CUR",
        inside: {
          field: "property",
          matches: "ed-cleanup-current-read",
          all: [
            { has: { field: "object", pattern: "$CCR", not: { matches: "ed-current-assigned" } } },
            {
              not: {
                any: [
                  { precedes: { stopBy: "end", any: [{ matches: "ed-later-cleanup-read" }, { has: { stopBy: "end", matches: "ed-later-cleanup-read" } }] } },
                  {
                    inside: {
                      stopBy: { any: [{ matches: "ed-cb-slot" }, ...DECLARED_FN] },
                      precedes: { stopBy: "end", any: [{ matches: "ed-later-cleanup-read" }, { has: { stopBy: "end", matches: "ed-later-cleanup-read" } }] },
                    },
                  },
                ],
              },
            },
          ],
        },
      },
      ...inCallback("ed-cb-cleanup-current", "ed-cleanup-current"),
      // A useState setter the effect reads itself, not from a function inside it.
      "ed-direct-set-state": {
        any: REF,
        all: [{ matches: "ed-ref" }, { matches: "ed-state-setter" }, { inside: { stopBy: { matches: "ed-fn" }, matches: "ed-fn", pattern: "$SSF", ...same("SSF", "CB") } }],
      },
      ...inCallback("ed-cb-direct-set-state", "ed-direct-set-state"),
      // The first assignment WA of each value of C in the callback, in the order the scopes are read: outer scopes first.
      "ed-same-write": { any: ASSIGN, all: [{ has: { field: "left", kind: "identifier", pattern: "$WL2", ...same("WL2", "WL", "name") } }, { not: { pattern: "$WA2", ...same("WA2", "WA") } }] },
      "ed-write-before": {
        any: [
          { precedes: { stopBy: "end", any: [{ pattern: "$WB", ...same("WB", "WA") }, { has: { stopBy: "end", pattern: "$WB", ...same("WB", "WA") } }] } },
          { inside: { stopBy: "end", precedes: { stopBy: "end", any: [{ pattern: "$WB", ...same("WB", "WA") }, { has: { stopBy: "end", pattern: "$WB", ...same("WB", "WA") } }] } } },
        ],
      },
      "ed-earlier-write": {
        any: [
          { inside: { stopBy: "end", matches: "ed-scope-node", inside: { stopBy: "end", matches: "ed-scope-node", has: { stopBy: { matches: "ed-scope-node" }, matches: "ed-same-write" } } } },
          { inside: { stopBy: { matches: "ed-scope-node" }, matches: "ed-scope-node", has: { stopBy: { matches: "ed-scope-node" }, all: [{ matches: "ed-same-write" }, { matches: "ed-write-before" }] } } },
        ],
      },
      "ed-first-stale-write": {
        any: ASSIGN,
        pattern: "$WA",
        all: [{ has: { field: "left", kind: "identifier", pattern: "$WL", matches: "ed-ref" } }, { has: { field: "right", pattern: "$WR" } }, { not: { matches: "ed-earlier-write" } }],
      },
      ...inCallback("ed-cb-first-stale-write", "ed-first-stale-write"),
      ...inCallback("ed-cb-reaching-ev", "ed-ref-ev"),
      "ed-ref-ev": { pattern: "$EVR", ...same("EVR", "EV", "source"), matches: "ed-ref" },
      // A callback whose dependencies cannot be read: neither a function nor a name, or a parameter's name.
      "ed-unknown-cb": {
        not: { kind: "comment" },
        any: [
          { not: { any: [...INLINE_FN, { kind: "identifier" }, { kind: "as_expression" }, { kind: "parenthesized_expression" }] } },
          unwrapped("ed-unknown-cb"),
          { matches: "ed-named-cb", resolves: { binding: { site: { matches: "ed-param-site" } } } },
        ],
      },
      // A callback passed by a name that declares no function.
      "ed-other-named-cb": {
        any: [
          unwrapped("ed-other-named-cb"),
          {
            matches: "ed-named-cb",
            resolves: {
              binding: {
                site: { not: { any: [{ matches: "ed-param-site" }, ...DECLARED_FN, { kind: "variable_declarator", has: { field: "value", any: INLINE_FN } }] } },
              },
            },
          },
        ],
      },
    };
    const deps = (...extra) => ({ all: [{ matches: "ed-deps-root" }, ...extra] });
    const atCallback = util => ({ has: { field: "arguments", has: { matches: "ed-cb-slot", all: [{ matches: util }] } } });
    const analysedCall = extra => ({
      kind: "call_expression",
      pattern: "$CALL",
      all: [
        { any: [{ matches: "ed-effect-call" }, { matches: "ed-imperative-call" }, { matches: "ed-memo-call" }] },
        { inside: { stopBy: { matches: "ed-scope-node" }, matches: "ed-scope-node", pattern: "$CSN" } },
        ...extra,
      ],
    });
    return {
      id: "react-hooks-exhaustive-deps",
      tool: H,
      upstream: "react-hooks/exhaustive-deps",
      capabilities: ["syntax", "scope", "compare", "suppression"],
      match: "line+column",
      description:
        "List exactly the component values an effect, useCallback, useMemo or useImperativeHandle callback reads in its dependency array; a missing one leaves the callback reading stale values.",
      shared: U,
      checks: [
        {
          id: "missing-dependency",
          rule: deps({ matches: "ed-missing" }),
          message: "Add `$MR` to this dependency array; the callback reads it, so without it the callback keeps a stale value.",
        },
        {
          id: "unnecessary-dependency",
          rule: deps({ not: { matches: "ed-missing" } }, { matches: "ed-unnecessary" }),
          message: "Remove `$UE` from this dependency array: the callback does not read it, or it is an outer or `.current` value whose change never re-renders the component.",
        },
        {
          id: "duplicate-dependency",
          rule: deps({ not: { matches: "ed-missing" } }, { not: { matches: "ed-unnecessary" } }, { matches: "ed-duplicate" }),
          message: "Remove the repeated `$DE` from this dependency array.",
        },
        {
          id: "dependencies-not-array",
          rule: deps({ not: onArray({}) }),
          message: "Pass the dependencies as an array literal written in place, so each value the callback reads can be checked against it.",
        },
        {
          id: "spread-dependency",
          rule: { kind: "spread_element", matches: "ed-element" },
          message: "List the dependencies one by one instead of spreading another list into this array, so each one can be checked.",
        },
        {
          id: "literal-dependency",
          rule: { any: LITERAL, matches: "ed-element" },
          message: "Remove this literal from the dependency array; it never changes. If you meant the variable it names, list that variable instead.",
        },
        {
          id: "complex-dependency",
          rule: { not: { any: [{ matches: "ed-declared-path" }, { kind: "spread_element" }, ...LITERAL, { kind: "comment" }, { regex: "^[\\[\\],]$" }] }, matches: "ed-element" },
          message: "Move this expression into a variable declared before the hook and list that variable, so the dependency can be checked.",
        },
        {
          id: "effect-event-dependency",
          rule: {
            kind: "identifier",
            pattern: "$EV",
            all: [
              { matches: "ed-element" },
              { resolves: { binding: { site: { kind: "variable_declarator", all: [{ has: { field: "name", kind: "identifier" } }, { has: { field: "value", matches: "ed-unwrap-effect-event" } }] } } } },
              { inside: { stopBy: "end", kind: "arguments", has: { matches: "ed-cb-slot", all: [{ matches: "ed-cb-reaching-ev" }] } } },
            ],
          },
          message: "Remove `$EV` from this dependency array; call a useEffectEvent function from the effect without listing it, since it is not a reactive value.",
        },
        {
          id: "stale-assignment",
          rule: { all: [{ matches: "ed-call-context" }, atCallback("ed-cb-first-stale-write")] },
          anchor: "capture.WR",
          each: "WR",
          message: "Keep `$WL` in a useRef and assign its `.current` instead, or declare it inside the callback; this assignment is lost after each render.",
        },
        {
          id: "ref-current-in-cleanup",
          rule: { all: [{ matches: "ed-call-context" }, { matches: "ed-effect-call" }, atCallback("ed-cb-cleanup-current")] },
          anchor: "capture.CUR",
          each: "CUR",
          message: "Copy `$CCR.current` to a variable inside the effect and use that variable in the cleanup; by the time the cleanup runs the ref may point elsewhere.",
        },
        {
          id: "set-state-without-dependencies",
          rule: { all: [{ matches: "ed-call-root" }, { not: { matches: "ed-has-deps" } }, atCallback("ed-cb-direct-set-state")] },
          message: "Pass a dependency array to this effect; it sets state on every run without one, which can re-render in an endless loop.",
        },
        {
          id: "changes-every-render",
          rule: {
            kind: "identifier",
            all: [
              { resolves: { binding: { site: { matches: "ed-construction-kind" } } } },
              { inside: { kind: "array", any: [{ matches: "ed-settled-deps" }, { inside: { kind: "as_expression", matches: "ed-settled-deps" } }] } },
              { resolves: { binding: { site: { matches: "ed-construction-site" } } } },
            ],
          },
          anchor: "capture.KS",
          message: "Wrap this value in its own useMemo or useCallback, or move it inside the hook that lists it; built anew each render, it reruns that hook every time.",
        },
        {
          id: "async-effect",
          rule: { any: INLINE_FN, all: [{ has: { regex: "^async$", not: { kind: "identifier" } } }, { matches: "ed-effect-cb" }] },
          message: "Make this effect callback synchronous and call an async function from inside it; an async callback returns a promise where React expects a cleanup.",
        },
        {
          id: "missing-callback",
          rule: { all: [{ matches: "ed-hook-call" }, { not: { has: { field: "arguments", has: { matches: "ed-cb-slot" } } } }] },
          message: "Pass a callback to this hook; without one React throws when it runs.",
        },
        {
          id: "memo-without-dependencies",
          rule: { all: [{ matches: "ed-memo-call" }, { has: { field: "arguments", has: { matches: "ed-cb-slot" } } }, { not: { matches: "ed-has-deps" } }] },
          message: "Pass a dependency array to this hook; with only a callback it recomputes on every render and memoizes nothing.",
        },
        {
          id: "unknown-dependencies",
          rule: analysedCall([{ any: [{ matches: "ed-effect-call" }, { matches: "ed-has-deps" }] }, atCallback("ed-unknown-cb")]),
          message: "Pass an inline function to this hook, so the values it reads can be checked against its dependencies.",
        },
        {
          id: "callback-not-function",
          rule: analysedCall([{ matches: "ed-has-deps" }, atCallback("ed-other-named-cb")]),
          message: "Add `$CBN` to this hook's dependency array, or pass an inline function; the value passed as its callback can change between renders.",
        },
      ],
    };
  })(),
];

/** Ports whose policy is written by hand, not from a spec: listed in conformance.json as they are. */
export const HAND_WRITTEN = [
  {
    id: "react-no-array-index-key",
    upstream: { tool: R, rule: "react/no-array-index-key" },
    policy: "rules/react-no-array-index-key.json",
    capabilities: ["syntax", "scope", "value", "suppression"],
    match: "line+column",
  },
  {
    id: "react-iframe-missing-sandbox",
    upstream: { tool: R, rule: "react/iframe-missing-sandbox" },
    policy: "rules/react-iframe-missing-sandbox.json",
    capabilities: ["syntax", "scope", "value", "suppression"],
    match: "line+column",
  },
  {
    id: "react-jsx-no-constructed-context-values",
    upstream: { tool: R, rule: "react/jsx-no-constructed-context-values" },
    policy: "rules/react-jsx-no-constructed-context-values.json",
    capabilities: ["syntax", "scope", "suppression"],
    match: "line+column",
  },
  {
    id: "react-jsx-no-leaked-render",
    upstream: { tool: R, rule: "react/jsx-no-leaked-render" },
    policy: "rules/react-jsx-no-leaked-render.json",
    capabilities: ["syntax", "scope", "suppression"],
    match: "line+column",
  },
  {
    id: "react-jsx-no-script-url",
    upstream: { tool: R, rule: "react/jsx-no-script-url" },
    policy: "rules/react-jsx-no-script-url.json",
    capabilities: ["syntax", "value", "suppression"],
    match: "line+column",
  },
  {
    id: "react-no-access-state-in-setstate",
    upstream: { tool: R, rule: "react/no-access-state-in-setstate" },
    policy: "rules/react-no-access-state-in-setstate.json",
    capabilities: ["syntax", "compare", "suppression"],
    match: "line+column",
  },
  {
    id: "react-no-deprecated",
    upstream: { tool: R, rule: "react/no-deprecated" },
    policy: "rules/react-no-deprecated.json",
    capabilities: ["syntax", "compare", "suppression"],
    match: "line+column",
  },
  {
    id: "react-no-did-mount-set-state",
    upstream: { tool: R, rule: "react/no-did-mount-set-state" },
    policy: "rules/react-no-did-mount-set-state.json",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
  },
  {
    id: "react-no-did-update-set-state",
    upstream: { tool: R, rule: "react/no-did-update-set-state" },
    policy: "rules/react-no-did-update-set-state.json",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
  },
  {
    id: "react-no-object-type-as-default-prop",
    upstream: { tool: R, rule: "react/no-object-type-as-default-prop" },
    policy: "rules/react-no-object-type-as-default-prop.json",
    capabilities: ["syntax", "scope", "value", "suppression"],
    match: "line+column",
  },
  {
    id: "react-no-unknown-property",
    upstream: { tool: R, rule: "react/no-unknown-property" },
    policy: "rules/react-no-unknown-property.json",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
  },
  {
    id: "react-no-unsafe",
    upstream: { tool: R, rule: "react/no-unsafe" },
    policy: "rules/react-no-unsafe.json",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
  },
  {
    id: "react-no-unstable-nested-components",
    upstream: { tool: R, rule: "react/no-unstable-nested-components" },
    policy: "rules/react-no-unstable-nested-components.json",
    capabilities: ["syntax", "scope", "value", "compare", "suppression"],
    match: "line+column",
  },
  {
    id: "react-hooks-error-boundaries",
    upstream: { tool: H, rule: "react-hooks/error-boundaries" },
    policy: "rules/react-hooks-error-boundaries.json",
    capabilities: ["syntax", "scope", "suppression"],
    match: "line+column",
  },
  {
    id: "react-hooks-globals",
    upstream: { tool: H, rule: "react-hooks/globals" },
    policy: "rules/react-hooks-globals.json",
    capabilities: ["syntax", "scope", "suppression"],
    match: "line+column",
  },
  {
    id: "react-hooks-immutability",
    upstream: { tool: H, rule: "react-hooks/immutability" },
    policy: "rules/react-hooks-immutability.json",
    capabilities: ["syntax", "scope", "compare", "suppression"],
    match: "line+column",
  },
  {
    id: "react-hooks-purity",
    upstream: { tool: H, rule: "react-hooks/purity" },
    policy: "rules/react-hooks-purity.json",
    capabilities: ["syntax", "scope", "compare", "suppression"],
    match: "line+column",
  },
  {
    id: "react-hooks-refs",
    upstream: { tool: H, rule: "react-hooks/refs" },
    policy: "rules/react-hooks-refs.json",
    capabilities: ["syntax", "scope", "compare", "suppression"],
    match: "line+column",
  },
  {
    id: "react-hooks-set-state-in-effect",
    upstream: { tool: H, rule: "react-hooks/set-state-in-effect" },
    policy: "rules/react-hooks-set-state-in-effect.json",
    capabilities: ["syntax", "scope", "suppression"],
    match: "line+column",
  },
  {
    id: "react-hooks-set-state-in-render",
    upstream: { tool: H, rule: "react-hooks/set-state-in-render" },
    policy: "rules/react-hooks-set-state-in-render.json",
    capabilities: ["syntax", "scope", "value", "compare", "suppression"],
    match: "line+column",
  },
  {
    id: "react-hooks-static-components",
    upstream: { tool: H, rule: "react-hooks/static-components" },
    policy: "rules/react-hooks-static-components.json",
    capabilities: ["syntax", "scope", "compare", "suppression"],
    match: "line+column",
  },
];
void H;
