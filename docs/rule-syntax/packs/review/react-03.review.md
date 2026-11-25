# Review: react-03

Branch reviewed: `tbelanger/pack-translations-react-03` at `5c66a2b6c0`. I read the six `.rerail` files, their JSON
originals, the shared libraries they include (`lib/tsx/react.rerail`, `react-class.rerail`, `names.rerail`) and the
plugin's source (`lib/rules/*.js`, `lib/util/makeNoMethodSetStateRule.js`) before the author's NOTES and result file.
The tool ran on a fleet box: ESLint 9.39.5 (espree 10.4.0), `eslint-plugin-react` 7.37.5,
`@typescript-eslint/parser` 8.70.1 for `.ts`/`.tsx`, with the pack's recording config (`upstream/common.mjs`:
`sourceType: "module"` everywhere, no settings, no globals, every rule at its defaults). Each translation ran with
`rerail-engine run-policies` and this branch's `docs/rule-syntax/lib`. Every counterexample below is a whole file;
line:column comes from ESLint, run-policies gives the line. None of the files carries a suppression comment.

| Rule | Verdict |
|---|---|
| `react-jsx-no-target-blank` | 2 defects (parentheses, 7 forms; a multi-line `rel`), 3 minor |
| `react-jsx-no-undef` | 2 defects (the plugin's first-child-scope lookup; three TypeScript declarations) |
| `react-no-access-state-in-setstate` | 3 defects (logical operators, `x[i]`, parentheses), 1 minor |
| `react-no-danger` | ok |
| `react-no-did-mount-set-state` | 3 defects, shared with did-update (parentheses, destructuring defaults, `accessor`), 1 minor |
| `react-no-did-update-set-state` | the same 3 defects and minor as did-mount |

`ptcheck --ref tbelanger/pack-translations-react-03 -- --pack react --rules <the six>` on pt-w07: 0 original-only
everywhere; translation-only only on samples-react (target-blank 1, no-undef 1, access-state 6, no-danger 8,
did-mount 5, did-update 15). Its `problems` list says four declared counts are stale (target-blank fixtures 5 → 0,
no-undef samples-react 3 → 1, access-state fixtures 1/0 → 0/0 and samples-react 39/2 → 0/6).

## react-jsx-no-target-blank

The plugin, for an `<a>` opening element: the last `target` may be `_blank` (a `Literal`, or a `{…}` holding a
`Literal` or a `ConditionalExpression` with a `Literal` branch, case ignored); the last `href` is a `Literal` matching
`/^(?:\w+:|\/\/)/` or some `href` is a `{…}`; and the last `rel` is not secure. Secure means every value of
`getStringFromValue` is a string whose `split(' ')` words include `noreferrer`. Ternaries pair by the tests' `.name`.
Agreed: `_BLANK`, `//x` and `mailto:` hrefs, a bare `rel`, repeated `target`/`href` (last wins; any dynamic `href`
counts), `` `noreferrer ${x}` `` vs `` `${x} noreferrer` ``, and paired ternaries (`c ? "_blank" : "_self"` with
`c ? "noreferrer" : null`, the reverse, and `"_BLANK"` which `indexOf('_blank')` does not find).

**Defect 1 (miss and extra): parentheses.** ESTree drops them, so the plugin sees a `Literal` or a
`ConditionalExpression` through any number of them. The translation reads tree-sitter's `parenthesized_expression`
as another kind everywhere: `may_blank` and `blank_lit` (target value, ternary, branch), `rel_expr`/`rel_value`/
`rel_branch` (rel value, ternary, branch), and `paired` (a test `(t)` is no identifier). The JSON original unwrapped
them (`blank-literal`, `ok-literal`), so lines 1, 3, 4 and 6 are regressions from it.

```jsx
// tb-paren.jsx
const a = <a target={("_blank")} href="https://x.example">x</a>;
const b = <a target={(open ? "_blank" : null)} href="https://x.example">x</a>;
const c = <a target={open ? ("_blank") : null} href="https://x.example">x</a>;
const d = <a target="_blank" href="https://x.example" rel={("noreferrer")}>x</a>;
const e = <a target="_blank" href="https://x.example" rel={(dev ? "noreferrer" : "noopener noreferrer")}>x</a>;
const f = <a target="_blank" href="https://x.example" rel={dev ? ("noreferrer") : "noreferrer"}>x</a>;
const g = <a target={(t) ? "_blank" : null} href={u} rel={t ? "noreferrer" : "x"}>x</a>;
```

- ESLint: `tb-paren.jsx:1:11`, `2:11`, `3:11` `react/jsx-no-target-blank`.
- translation: `tb-paren.jsx:4`, `5`, `6`, `7`.

**Defect 2 (extra): a `rel` string over two lines.** `noreferrer(s)` matches `(.* )?noreferrer( .*)?` without
`(?s)`, so a newline before the space (`"noopener\n  noreferrer"`, which `split(' ')` makes `["noopener\n", "",
"noreferrer"]`) fails the match. `risky_link` already uses `(?s)`; `noreferrer` needs it too.

```jsx
// tb-newline.jsx
const a = <a target="_blank" href="https://x.example" rel="noopener
  noreferrer">x</a>;
```

- ESLint: nothing.
- translation: `tb-newline.jsx:1`.

**Minor (extra): `\w` is Unicode in the engine and ASCII in JavaScript.** `risky_link`'s `(\w+:|//)` takes
`ü:` as a scheme; the plugin's `/^(?:\w+:|\/\/)/` (no `u` flag) does not. `[A-Za-z0-9_]` matches the plugin.

```jsx
// tb-unicode.jsx
const a = <a target="_blank" href="ü:x">x</a>;
```

- ESLint: nothing. translation: `tb-unicode.jsx:1`.

**Minor (miss): a test named `undefined`.** The plugin pairs by `test.name`, and the identifier `undefined` has the
name `"undefined"`; tree-sitter gives it the kind `undefined`, so `paired` counts it with the nameless tests (`x.y`)
and pairs it with them. The JSON original's `named-test` listed `undefined` with the identifiers.

```jsx
// tb-undefined.jsx
const a = <a target={undefined ? "_blank" : null} href={u} rel={x.y ? "noreferrer" : "x"}>x</a>;
```

- ESLint: `tb-undefined.jsx:1:11`. translation: nothing.

**Minor (both ways): HTML entities in attribute strings.** The plugin reads the `Literal`'s decoded value;
`jsx_string` keeps the text as written (`lib/tsx/react.rerail` says entities are not modelled).

```jsx
// tb-entity.jsx
const a = <a target="_blan&#107;" href="https://x.example">x</a>;
const b = <a target="_blank" href="https://x.example" rel="noreferrer&#32;noopener">x</a>;
```

- ESLint: `tb-entity.jsx:1:11`. translation: `tb-entity.jsx:2`.

## react-jsx-no-undef

The plugin looks the tag's root name up in the node's scope and every scope above it up to the module scope (in a
module; globals never count), then adds the variables of **the module scope's first child scope and that scope's
first child** ("Temporary fix for babel-eslint", `scope.childScopes[0]`). Agreed: DOM, namespaced and `this.X`
tags, `<lower.X>` (checked by `lower`), `Map`/`Object` (globals do not count), `var` hoisted out of a block,
block-scoped function declarations, catch, `for…of`, parameters (destructured, rest), named function and class
expressions, destructured declarators, `import type`, `import { default as X }`, `import * as NS`, `declare const`,
`declare class`, `enum`, `namespace`, `type`, `interface`, type parameters and overload signatures.

**Defect 1 (extra): the first child scope's names.** A name declared in the file's first function (or class, or
block), or in that scope's first child, counts everywhere in the file. `declared_at` only reads the scopes that
enclose the tag.

```jsx
// nu-first-scope.jsx
function setup() { const Icon = load(); }
export function Toolbar() { return <Icon />; }
```

- ESLint: nothing.
- translation: `nu-first-scope.jsx:2` `'Icon' is not defined.`

The same holds one level down: with `class First { render() { const Inner = 1; } }` first in the file, `<Inner />`
anywhere is not reported (checked; the translation reports it).

**Defect 2 (extra): three TypeScript declarations bind nothing.** `names.rerail`'s `decl_site` has no
`abstract_class_declaration`, no `function_signature` (`declare function`), and no `import_alias`
(`import X = require(…)`); typescript-eslint's scope manager declares all three. `declared_at` already adds enums and
namespaces by hand; these three need the same (or `decl_site` needs them, which other rules would then see).

```tsx
// nu-ts-decls.tsx
import Legacy = require("legacy");
declare function Widget(): any;
abstract class Base {}
export const z = <><Legacy /><Widget /><Base /></>;
```

- ESLint: nothing.
- translation: `nu-ts-decls.tsx:4` three times (`'Legacy'`, `'Widget'`, `'Base'`).

## react-no-access-state-in-setstate

Agreed: a read directly in the first argument (and inside an arrow updater there, which the plugin also reports), a
read in a function-valued property of that argument (not reported), `this?.state`, `this[state]` (computed
identifier), `this["state"]` (not), a spread first argument, the variable path through `x.y`, `{ k: x }`, `{ x }`,
`x + 1` and `(x)`, not across scopes (a block, an arrow), file order (a `setState` before the declarator), the method
path through a bare call and one call level up, `this.getX()` (not), a method defined after the call (not), a
`function` class field, createReactClass specs, and `const { state } = this` in a component, in a plain function, as
`{ state: s }` (not: the plugin records the key, `state`) and as `["state"]` (not).

**Defect 1 (extra): `&&`, `||` and `??`.** The plugin's Identifier visitor climbs `BinaryExpression` parents only;
ESTree makes `&&`, `||` and `??` a `LogicalExpression`, so `{ n: next || 0 }` is not a use. tree-sitter makes all of
them `binary_expression`, and `above_binary` climbs every one. The JSON original excluded them (`non-logical-binary`).

```jsx
// as-logical.jsx
class A extends React.Component {
  f() { const next = this.state.n; this.setState({ n: next || 0 }); }
  g() { const q = this.state.q; this.setState({ q: q && 1 }); }
  h() { const r = this.state.r; this.setState({ r: r ?? 1 }); }
}
```

- ESLint: nothing.
- translation: `as-logical.jsx:2`, `3`, `4`.

**Defect 2 (miss): `x[i]`.** `list[0]` is a computed `MemberExpression` whose `object` is `list`, so the plugin counts
it. `used_as_value` accepts `member_expression` only, not `subscript_expression` (the JSON original had both).

```jsx
// as-subscript.jsx
class A extends React.Component {
  g() { const list = this.state.list; this.setState({ first: list[0] }); }
}
```

- ESLint: `as-subscript.jsx:2:22`.
- translation: nothing.

**Defect 3 (miss): parentheses around `this` or the callee.** The plugin checks `callee.object.type ===
'ThisExpression'` and `node.object.type === 'ThisExpression'` on ESTree, which has no parentheses. `this_prop`
(`lib/tsx/react-class.rerail`) needs `kind(t, "this")` and `this_call` needs the callee itself to be `this.x`.

```jsx
// as-paren.jsx
class A extends React.Component {
  a() { (this.setState)({ x: this.state.x }); }
  b() { this.setState({ x: (this).state.x }); }
}
```

- ESLint: `as-paren.jsx:2:30`, `3:28`.
- translation: nothing.

**Minor (miss): methods with no name.** A method whose key has no `.name` (`['compute']() {}`, `'compute'() {}`) is
recorded with `methodName: undefined`, and a call inside the first argument whose callee is no identifier
(`Object.assign(…)`, `this.helper()`) has `methodName` undefined too, so the plugin reports every such method's read.
`method_name_of` binds no name for these keys.

```jsx
// as-unnamed.jsx
class A extends React.Component {
  ['compute']() { return this.state.x; }
  onClick() { this.setState({ x: Object.assign({}, this.props) }); }
}
```

- ESLint: `as-unnamed.jsx:2:26`.
- translation: nothing.

## react-no-danger

ok. The plugin reports a `dangerouslySetInnerHTML` attribute whose element type matches `/^[a-z]/`. Agreed on
`<div>`, `<Foo>` (not), `<foo.Bar>`, `<Foo.bar>` (not), `<svg:foreignObject>`, `<this.Foo>` (a DOM element by the
regex), a valueless prop, and two of them on one element (two reports).

## react-no-did-mount-set-state and react-no-did-update-set-state

Both use `makeNoMethodSetStateRule`, and both translations use `set_state_in` (`lib/tsx/react-class.rerail`), so every
finding holds for both. The plugin: a `CallExpression` whose callee is a `MemberExpression` on `ThisExpression` with
the property name `setState`; walking its ancestors outward, it counts `*Function(Expression|Declaration)` (arrows
included) and reports at the first `Property`, `MethodDefinition`, `ClassProperty` or `PropertyDefinition` whose
`key.name` is the method's name while the count is at most 1. Agreed: methods, function and arrow class fields, a
property with no function, `this?.setState`, `[componentDidMount]()`, a string key (not), an outer key through an
inner one, two nested arrows (not), a generator, an async arrow, and a class inside the method.

**Defect 1 (miss): parentheses.** `(this.setState)(…)` and `(this).setState(…)` are ESTree's plain callee.
`this_call`/`this_prop` read tree-sitter's `parenthesized_expression` as another kind. The JSON originals stepped
through them (`stopBy: not parenthesized_expression`, `matches: "this"`).

```jsx
// sm-paren.jsx
class A extends React.Component {
  componentDidMount() { (this.setState)({ a: 1 }); }
  componentDidUpdate() { (this).setState({ a: 1 }); }
}
```

- ESLint: `sm-paren.jsx:2:26` `react/no-did-mount-set-state`, `3:26` `react/no-did-update-set-state`.
- translation: nothing.

**Defect 2 (miss): a shorthand destructuring default.** `{ componentDidMount = … }` is an ESTree `Property` keyed
`componentDidMount` whose value is an `AssignmentPattern`; tree-sitter makes it an `object_assignment_pattern`, which
`keyed_kind` lacks (the JSON originals had it). The same holds in a parameter list.

```jsx
// sm-default.jsx
class A extends React.Component {
  load() { const { componentDidMount = this.setState({}) } = this.props; }
  save() { const { componentDidUpdate = this.setState({}) } = this.props; }
}
```

- ESLint: `sm-default.jsx:2:40` `react/no-did-mount-set-state`, `3:41` `react/no-did-update-set-state`.
- translation: nothing.

**Defect 3 (extra): `accessor` fields.** typescript-eslint makes `accessor x = …` an `AccessorProperty`, which the
plugin does not look at. tree-sitter makes it a `public_field_definition`, which `keyed_kind` takes. The JSON original
of did-update excluded it (`not has ^accessor$`).

```tsx
// sm-accessor.tsx
import React from "react";
class B extends React.Component {
  accessor componentDidMount = this.setState({ a: 1 });
  accessor componentDidUpdate = () => { this.setState({ a: 1 }); };
}
```

- ESLint: nothing.
- translation: `sm-accessor.tsx:3` `react-no-did-mount-set-state`, `4` `react-no-did-update-set-state`.

**Minor (miss): a parenthesised computed key.** `[(componentDidMount)]` has an `Identifier` key in ESTree;
`estree_name` reads only a bare identifier inside `computed_property_name`.

```jsx
// sm-computed-paren.jsx
class A extends React.Component {
  m() { const o = { [(componentDidMount)]: this.setState({}) }; }
}
```

- ESLint: `sm-computed-paren.jsx:2:44`. translation: nothing.

## The author's result file

All six entries are `different` with `right: "original"`: every translation-only key is a line under a suppression
comment, which run-policies does not apply (known). I ran ESLint on one or more sample keys per entry, from the
samples-react corpus on the box, twice: with inline comments honoured (the recording) and with `noInlineConfig`.

| Rule | Sample key | Comment | ESLint, comments honoured | ESLint, comments ignored |
|---|---|---|---|---|
| jsx-no-target-blank | excalidraw `DropdownMenuItemLink.tsx:35` | `eslint-disable-next-line` | nothing on 35 | `35:5` |
| jsx-no-undef | ant-design alert `index.test.js:64` | `eslint-disable-next-line` | nothing on 64 | `64:31` |
| no-access-state-in-setstate | superset `CategoricalDeckGLContainer.jsx:105` | file-level `/* eslint-disable … */` (line 4) | nothing | `105:66`, `203:10`, `221:29` |
| no-access-state-in-setstate | docusaurus IdealImage `index.js:314` | `eslint-disable-line` | nothing | `314:32`, `321:10` |
| no-danger | docusaurus Mermaid `index.tsx:25` | `eslint-disable-next-line` | nothing on 25 | `25:7` |
| no-danger | ant-design `CodePreview.tsx:83` | `biome-ignore lint/security/noDangerouslySetInnerHtml` | `83:19` | `83:19` |
| no-danger | polar `layout.tsx:60` | `oxlint-disable-next-line react/no-danger` | `60:13` | `60:13` |
| no-did-mount-set-state | blueprint `portal.tsx:103` | `/* eslint-disable-next-line */` | nothing on 103 | `103:9` |
| no-did-update-set-state | react-select `Select.js:191` | `eslint-disable-line` | nothing on 191 | `191:4` |

`right: "original"` is confirmed against the tool on every ESLint-suppressed key. On the two no-danger keys under
biome and oxlint comments, ESLint reports the line, as the translation does; the original honours the rule's biome
and oxlint `suppress` lines. The NOTES say so and count the original right as the product's behaviour. That holds
for the product, but against the tool these two keys side with the translation, and the entry's reason ("all
suppressions the rule declares and the original honours") should say ESLint reports them.

The NOTES' three fixes hold on my cases: paired ternary `rel`s are secure again, `import { default as X }` binds, and
access-state follows source order and takes `const { state } = this` outside a component. Every defect above comes
from a case written for this review; ptcheck's corpora show none of them as a translation/original difference.
