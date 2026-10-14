// The React pack's policies as compact specs, one entry per pack rule (`candidate: true`: a port that does not pass
// its fixtures or the V1 rule validation gates yet, written to packs/react/candidates/). build.mjs turns each into a
// rerail.policy/v1 document (scope, violation, eslint suppression and a shared `define` are added there) and lists it,
// with the HAND_WRITTEN ports, in packs/react/conformance.json. Edit here, then run build.mjs.
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

/** react-in-jsx-scope: a binding of `React` an element sees, approximated syntactically (see the policy description). */
const declares = name => ({
  any: [
    { kind: "import_statement", has: { stopBy: "end", kind: "identifier", regex: `^${name}$` } },
    {
      any: [{ kind: "lexical_declaration" }, { kind: "variable_declaration" }],
      has: {
        kind: "variable_declarator",
        has: {
          field: "name",
          any: [
            { kind: "identifier", regex: `^${name}$` },
            { has: { stopBy: "end", any: [{ kind: "identifier" }, { kind: "shorthand_property_identifier_pattern" }], regex: `^${name}$` } },
          ],
        },
      },
    },
    { any: [{ kind: "function_declaration" }, { kind: "class_declaration" }], has: { field: "name", regex: `^${name}$` } },
    { kind: "export_statement", has: { field: "declaration", matches: "declares-react" } },
  ],
});
const REACT_SCOPE_UTILS = {
  "declares-react": declares("React"),
  "react-visible": {
    any: [
      { inside: { stopBy: "end", kind: "program", has: { matches: "declares-react" } } },
      { inside: { stopBy: "end", kind: "statement_block", has: { matches: "declares-react" } } },
      { inside: { stopBy: "end", any: [...FUNCTION_KINDS, { kind: "function_declaration" }, { kind: "method_definition" }], matches: "react-parameter" } },
    ],
  },
  "react-parameter": {
    any: [
      { has: { field: "parameters", has: { stopBy: "end", kind: "identifier", regex: "^React$", binds: { form: "parameter" } } } },
      { has: { field: "parameter", kind: "identifier", regex: "^React$" } },
    ],
  },
  "other-pragma-file": {
    inside: { stopBy: "end", kind: "program", has: { kind: "comment", regex: "@jsx\\s", not: { regex: "@jsx\\s+React(?:[.\\s]|$)" } } },
  },
};

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
    candidate: true,
    tool: R,
    upstream: "react/no-danger",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/no-danger (default options): a `dangerouslySetInnerHTML` attribute on a DOM element (a tag whose name starts with a lower-case letter, jsx-ast-utils' elementType).",
    checks: [
      {
        id: "upstream",
        rule: { ...prop("dangerouslySetInnerHTML"), inside: { stopBy: "neighbor", any: OPENING, ...DOM_TAG } },
        message: "Dangerous property 'dangerouslySetInnerHTML' found",
      },
    ],
  },
  {
    id: "react-no-find-dom-node",
    candidate: true,
    tool: R,
    upstream: "react/no-find-dom-node",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/no-find-dom-node: a call whose callee is `findDOMNode` or a member `.findDOMNode`.",
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
        message: "Do not use findDOMNode. It doesn’t work with function components and is deprecated in StrictMode. See https://reactjs.org/docs/react-dom.html#finddomnode",
      },
    ],
  },
  {
    id: "react-no-is-mounted",
    candidate: true,
    tool: R,
    upstream: "react/no-is-mounted",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/no-is-mounted: `this.isMounted()` inside an object property or a method (ESTree Property / MethodDefinition ancestor).",
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
        message: "Do not use isMounted",
      },
    ],
  },
  {
    id: "react-no-render-return-value",
    candidate: true,
    tool: R,
    upstream: "react/no-render-return-value",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/no-render-return-value (latest React): a `ReactDOM.render(...)` call whose value is used: its parent (parentheses aside) is a declarator, a property, a return, an arrow body or an assignment.",
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
        message: "Do not depend on the return value from ReactDOM.render",
      },
    ],
  },
  {
    id: "react-jsx-no-duplicate-props",
    candidate: true,
    tool: R,
    upstream: "react/jsx-no-duplicate-props",
    capabilities: ["syntax", "compare", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/jsx-no-duplicate-props (case-sensitive): an attribute whose plain name an earlier attribute of the same element already has; namespaced names and spreads are skipped.",
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
        message: "No duplicate props allowed",
      },
    ],
  },
  {
    id: "react-no-children-prop",
    candidate: true,
    tool: R,
    upstream: "react/no-children-prop",
    capabilities: ["syntax", "scope", "value", "arguments", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/no-children-prop (allowFunctions: false): a `children` JSX attribute, and a createElement call (isCreateElement, default pragma) whose second argument is an object literal with a `children` key.",
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
    candidate: true,
    tool: R,
    upstream: "react/jsx-no-comment-textnodes",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/jsx-no-comment-textnodes: JSX text with a line that starts (after whitespace) with `//` or `/*`.",
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
    candidate: true,
    tool: R,
    upstream: "react/void-dom-elements-no-children",
    capabilities: ["syntax", "scope", "value", "arguments", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/void-dom-elements-no-children: a void DOM element (area, base, br, col, embed, hr, img, input, keygen, link, menuitem, meta, param, source, track, wbr) with children, or with a `children` or `dangerouslySetInnerHTML` attribute (one report each); the same through createElement with an object-literal props argument.",
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
        message: "Void DOM element cannot receive children.",
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
        message: "Void DOM element cannot receive children.",
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
        message: "Void DOM element cannot receive children.",
      },
    ],
  },
  {
    id: "react-self-closing-comp",
    candidate: true,
    tool: R,
    upstream: "react/self-closing-comp",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/self-closing-comp (component: true, html: true): an element with a closing tag and no children but whitespace containing a line break; namespaced tags only when lower-case (DOM).",
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
    candidate: true,
    tool: R,
    upstream: "react/no-string-refs",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/no-string-refs (latest React, noTemplateLiterals: false): a `ref` attribute whose value is a string literal, bare or in braces. From React 18.3 on the rule no longer reports `this.refs` reads.",
    checks: [
      {
        id: "upstream",
        rule: { kind: "jsx_attribute", all: [{ has: { kind: "property_identifier", regex: "^ref$" } }, { has: { any: [{ kind: "string" }, { kind: "jsx_expression", has: { matches: "string-in-parens" } }] } }] },
        utils: { "string-in-parens": { any: [{ kind: "string" }, { kind: "parenthesized_expression", has: { matches: "string-in-parens" } }] } },
        message: "Using string literals in ref attributes is deprecated.",
      },
    ],
  },
  {
    id: "react-no-direct-mutation-state",
    candidate: true,
    tool: R,
    upstream: "react/no-direct-mutation-state",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/no-direct-mutation-state: an assignment or update whose target is a member chain rooted at `this.state`, inside a class component (nearest class extends Component / PureComponent / React.Component / React.PureComponent) or a function that is a property of the object passed to createReactClass; in a class constructor only when inside a call. Reported at `this`. Function components (Components.detect's stateless detection) are not modelled.",
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
    candidate: true,
    tool: R,
    upstream: "react/no-danger-with-children",
    capabilities: ["syntax", "scope", "arguments", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/no-danger-with-children: an element with both `dangerouslySetInnerHTML` and children (a first child that is not a line break, or a `children` prop), props read through `{...name}` spreads of a variable initialised with an object literal; and `x.createElement(type, props, ...)` with `dangerouslySetInnerHTML` in props (an object literal or a variable initialised with one) and a third argument or a `children` prop.",
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
    candidate: true,
    shared: BUTTON_UTILS,
    tool: R,
    upstream: "react/button-has-type",
    capabilities: ["syntax", "scope", "value", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/button-has-type (button, submit and reset allowed): a `<button>` without a `type` attribute (jsx-ast-utils getProp, case-insensitive) or whose first `type` value is not a literal button/submit/reset (each leaf of a conditional is checked; a non-literal leaf is reported at itself); the same for `React.createElement('button', props)` (isCreateElement, default pragma).",
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
        message: "Invalid value for button type attribute",
      },
      ...["", "consequence", "alternative"].flatMap(branch => [
        {
          id: `jsx-invalid${branch ? `-${branch}` : ""}`,
          rule: { any: BUTTON_TAG, has: { kind: "jsx_attribute", matches: "first-type-attribute", has: { kind: "jsx_expression", has: { matches: leafUtil("invalid", branch) } } } },
          utils: BUTTON_UTILS,
          message: "Invalid value for button type attribute",
        },
        {
          id: `jsx-complex${branch ? `-${branch}` : ""}`,
          rule: { any: BUTTON_TAG, has: { kind: "jsx_attribute", matches: "first-type-attribute", has: { kind: "jsx_expression", has: { matches: leafUtil("complex", branch) } } } },
          anchor: "capture.LEAF",
          utils: BUTTON_UTILS,
          message: "The button type attribute must be specified by a static string or a trivial ternary expression",
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
          message: "Invalid value for button type attribute",
        },
        {
          id: `create-element-complex${branch ? `-${branch}` : ""}`,
          rule: { matches: "button-create-element", has: { field: "arguments", has: { kind: "object", nthChild: 2, has: { kind: "pair", matches: "ce-first-type-property", has: { field: "value", matches: leafUtil("complex", branch) } } } } },
          anchor: "capture.LEAF",
          utils: BUTTON_UTILS,
          message: "The button type attribute must be specified by a static string or a trivial ternary expression",
        },
      ]),
      {
        id: "create-element-complex-shorthand",
        rule: { matches: "button-create-element", has: { field: "arguments", has: { kind: "object", nthChild: 2, has: { kind: "shorthand_property_identifier", regex: "^type$", pattern: "$LEAF", matches: "ce-first-type-property" } } } },
        anchor: "capture.LEAF",
        utils: BUTTON_UTILS,
        message: "The button type attribute must be specified by a static string or a trivial ternary expression",
      },
    ],
  },
  {
    id: "react-jsx-key",
    candidate: true,
    shared: JSX_KEY_UTILS,
    tool: R,
    upstream: "react/jsx-key",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/jsx-key (default options): an element (not a fragment) without a `key` attribute that is an array literal's element, or that a `.map(fn)` / `.from(x, fn)` callback returns: the arrow's body, a branch of its conditional body, the right side of its logical body, or a return statement directly in the callback's body or in nested if/else branches; never inside a `React.Children.toArray(...)` / `Children.toArray(...)` call.",
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
    candidate: true,
    tool: R,
    upstream: "react/jsx-no-undef",
    capabilities: ["syntax", "scope", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/jsx-no-undef (allowGlobals: false, module code): a JSX tag name (not a DOM tag) or the root of a member tag name that no file-local binding declares; globals, even configured ones, do not count in a module.",
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
  // Ships off when promoted (NER-1797): its pack.json entry takes `"on": false`, mirroring plugin:react/jsx-runtime.
  // Under React 17's automatic JSX runtime (tsconfig `"jsx": "react-jsx"`, babel `runtime: "automatic"`, Next.js,
  // Vite) `React` need not be in scope, so the rule flags every JSX file and steers agents to add an unused
  // `import React`. Upstream has no option for this either, and a policy cannot read the project's runtime. A repo should
  // turn it on itself when its ESLint config enables react/react-in-jsx-scope or it uses the classic runtime. Evaluation
  // runs must not attach this candidate directly.
  {
    id: "react-react-in-jsx-scope",
    candidate: true,
    tool: R,
    upstream: "react/react-in-jsx-scope",
    capabilities: ["syntax", "scope", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/react-in-jsx-scope (pragma React): a JSX opening element or fragment where no `React` binding is visible: none declared at the top level (import, var/let/const, function), in an enclosing block's own declarations, or as an enclosing function's parameter. Files whose `@jsx X` comment names another pragma are not checked: the pragma is a per-file fact a policy cannot read.",
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
    ],
  },
  {
    id: "react-no-unescaped-entities",
    candidate: true,
    tool: R,
    upstream: "react/no-unescaped-entities",
    capabilities: ["syntax", "suppression"],
    match: "line",
    description: "eslint-plugin-react 7.37.5 react/no-unescaped-entities (forbid >, \", ', }): one finding per such character on the first line of a JSX text node (the k-th check holds where that line has at least k of them; ESLint reports each character at its own column). Characters on a text node's later lines are not reported: a finding is anchored at a node, and a text node has no node per line.",
    checks: [1, 2, 3, 4, 5, 6, 7, 8].map(count => ({
      id: `entity-${count}`,
      rule: { kind: "jsx_text", regex: `\\A(?:[^\\n>"'}]*[>"'}]){${count}}` },
      message: "HTML entity must be escaped.",
    })),
  },
  {
    id: "react-jsx-no-target-blank",
    candidate: true,
    tool: R,
    upstream: "react/jsx-no-target-blank",
    capabilities: ["syntax", "value", "compare", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react 7.37.5 react/jsx-no-target-blank (default options: links only, enforceDynamicLinks always, allowReferrer false): an `<a>` whose last `target` is possibly `_blank` (a string, or a conditional with a `_blank` branch), with an external last `href` string (`scheme:` or `//`) or any `href` expression, and whose last `rel` does not give `noreferrer` on every value it may take (a string, the first quasi of a template, both branches of a conditional, or, when target and rel test the same name, the branch target is `_blank` on).",
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
    candidate: true,
    shared: Object.fromEntries(Object.entries(HOOKS_UTILS).filter(([name]) => name !== "class-field" && name !== "class-function")),
    tool: H,
    upstream: "react-hooks/rules-of-hooks",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react-hooks 7.1.1 react-hooks/rules-of-hooks: a hook call (`use`, `useX`, `Namespace.useX`) reported at its callee when it runs (its nearest function, ESLint's code path) in a component or hook (a function named `X…`/`useX`, or memo/forwardRef's callback) conditionally (an if/conditional/logical/switch/catch/default branch, or after an earlier return in the function); in a class method or field; in another named function; at the top level; in an anonymous callback inside a component or hook; in a loop; in an async component or hook; and `use` in try/catch. Code paths are approximated syntactically.",
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
        message: "React Hooks must be called in the exact same order in every component render, only in React function components or custom React Hook functions.",
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
          kind: "identifier",
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
        message: "React Hook \"use\" cannot be called in a try/catch block.",
      },
    ],
  },
  {
    id: "react-hooks-exhaustive-deps",
    candidate: true,
    tool: H,
    upstream: "react-hooks/exhaustive-deps",
    capabilities: ["syntax", "scope", "compare", "suppression"],
    match: "line+column",
    description: "eslint-plugin-react-hooks 7.1.1 react-hooks/exhaustive-deps, the missing-dependency report only: the dependency array literal of useEffect / useLayoutEffect / useInsertionEffect / useCallback / useMemo (and React.*), when the inline callback reads a name bound in the function that calls the hook (outside the callback) that no dependency's root names; useState/useReducer setters and dispatchers, useRef, useEffectEvent and useTransition results count as stable.",
    checks: [
      {
        id: "missing-dependency",
        rule: {
          kind: "array",
          pattern: "$DEPS",
          all: [
            { nthChild: 2, inside: { kind: "arguments", inside: { kind: "call_expression", pattern: "$CALL", matches: "deps-hook", all: [{ inside: { stopBy: { matches: "fn" }, matches: "fn", pattern: "$C" } }] } } },
            {
              follows: {
                stopBy: "end",
                any: [{ kind: "arrow_function" }, { kind: "function_expression" }],
                pattern: "$CB",
                nthChild: 1,
                has: {
                  stopBy: "end",
                  kind: "identifier",
                  pattern: "$R",
                  all: [
                    { not: { inside: { field: "name", kind: "variable_declarator" } } },
                    {
                      resolves: {
                        binding: {
                          site: {
                            all: [
                              { inside: { stopBy: "end", matches: "fn", pattern: "$SC", compare: { left: "SC", op: "eq", right: "C", text: "node" } } },
                              { not: { inside: { stopBy: "end", pattern: "$SCB", compare: { left: "SCB", op: "eq", right: "CB", text: "node" } } } },
                              { not: { matches: "stable-site" } },
                            ],
                          },
                        },
                      },
                    },
                    { not: { inside: { stopBy: "end", kind: "array", pattern: "$D2", compare: { left: "D2", op: "eq", right: "DEPS", text: "node" } } } },
                  ],
                },
              },
            },
            { not: { has: { stopBy: "end", kind: "identifier", pattern: "$E", compare: { left: "E", op: "eq", right: "R" } } } },
          ],
        },
        utils: {
          ...HOOKS_UTILS,
          "deps-hook": {
            has: {
              field: "function",
              any: [
                { kind: "identifier", regex: "^(?:useEffect|useLayoutEffect|useInsertionEffect|useCallback|useMemo)$" },
                {
                  kind: "member_expression",
                  all: [{ has: { field: "object", kind: "identifier", regex: "^React$" } }, { has: { field: "property", regex: "^(?:useEffect|useLayoutEffect|useInsertionEffect|useCallback|useMemo)$" } }],
                },
              ],
            },
          },
          "stable-site": { any: [{ matches: "stable-hook-site" }, { matches: "literal-const-site" }] },
          "literal-const-site": {
            kind: "variable_declarator",
            inside: { kind: "lexical_declaration", has: { regex: "^const$" } },
            has: { field: "value", any: [{ kind: "string" }, { kind: "number" }, { kind: "true" }, { kind: "false" }, { kind: "null" }, { kind: "regex" }] },
          },
          "stable-hook-site": {
            kind: "variable_declarator",
            has: {
              field: "value",
              kind: "call_expression",
              has: { field: "function", regex: "^(?:React\\.)?(?:useRef|useState|useReducer|useEffectEvent|useTransition|useActionState)$" },
            },
            not: { has: { field: "name", kind: "array_pattern" } },
          },
        },
        message: "React Hook has a missing dependency. Either include it or remove the dependency array.",
      },
    ],
  },
];

/** Ports whose policy is written by hand, not from a spec: listed in conformance.json as they are. */
export const HAND_WRITTEN = [
  {
    id: "react-no-array-index-key",
    upstream: { tool: R, rule: "react/no-array-index-key" },
    policy: "candidates/rules/react-no-array-index-key.json",
    capabilities: ["syntax", "suppression"],
    match: "line+column",
  },
];
void H;
