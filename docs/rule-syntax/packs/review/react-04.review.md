# Review: react-04

Branch reviewed: `tbelanger/pack-translations-react-04` (4d492cc30b). The reference is eslint-plugin-react 7.37.5 with the
pack's own configuration (`packs/react/upstream/common.mjs`: ESLint 9.39.5, espree for JavaScript, @typescript-eslint/parser
8.70.1 for TypeScript, each rule at its defaults, no settings, so the latest React and the `React` pragma). The translation
side is `rerail-engine run-policies` (`~/pt/bin`) with the branch's `docs/rule-syntax/lib` (`RERAIL_RR_LIB`). Both ran on
pt-w01, in `/tmp/rv-r04`, over 44 case files of my own, each run through all six rules on both sides. run-policies gives
no column, so findings are compared as `rule path:line`. Columns are quoted from ESLint where they matter.

`ptcheck --box pt-w07 --ref tbelanger/pack-translations-react-04 -- --pack react --rules <the six>` (exit 0): all six
`different`, with no original-only finding on any corpus and translation-only findings on samples-react only (11, 3, 5,
20, 24, 8). Every sampled translation-only key is an `eslint-disable` suppression (last section), so none of the defects
below shows in the sampled keys. The suite also lists 3 problems that are outside the translations:
`docs/rule-syntax/packs/react/equivalence.json` still records react-no-unsafe's earlier revision (1, 12 and 21
original-only on examples, fixtures and samples-react; it is now 0, 0 and 0, plus 8 translation-only). The pack does not
gate until that entry is re-recorded.

| Rule | Verdict |
|---|---|
| react-no-find-dom-node | 4 defects: parenthesised callee, `this.#findDOMNode()`, `x[findDOMNode]()` missed; a tagged template reported |
| react-no-render-return-value | 3 defects: a computed key and `ReactDOM[render]` missed; a tagged template reported |
| react-no-string-refs | 1 defect: a comment before the string in `ref={…}` |
| react-no-unescaped-entities | ok |
| react-no-unknown-property | 1 minor defect: `<this …>` in a JavaScript file |
| react-no-unsafe | 7 defects: JSDoc components, `new createReactClass`, parenthesised superclass, abstract methods, overloads, a parenthesised computed key, the `@jsx` pragma |

Most of these are rare in real code. Those likely to matter are react-no-unsafe's JSDoc-declared components and TypeScript
overloads and abstract methods, and react-no-string-refs' comment. react-no-unsafe's JSDoc components, `new
createReactClass`, parenthesised superclass, abstract methods and overloads, and react-no-string-refs' comment, are
regressions: the JSON original handles each of them (`jsdoc-component`, `es5-component`'s `new_expression`,
`component-superclass`'s `parenthesized_expression`, `will-mount`'s `abstract_method_signature` and `method_signature`, and
`has` on the `jsx_expression`). The other defects are in the JSON too.

## react-no-find-dom-node: 4 defects

The plugin reports every `CallExpression` whose callee has `.name === "findDOMNode"` or whose callee's `.property` has
that `.name`. ESTree has no parentheses, a private name's `.name` has no `#`, and a computed member's property is the
identifier itself. The translation asks for a `call_expression` whose `function` field is the identifier or a
`member_expression` with `property` text `findDOMNode`.

### Defect 1: a parenthesised callee is not seen

```jsx
// fd-parens.jsx
import ReactDOM, { findDOMNode } from "react-dom";
export function f(c) {
  const a = (findDOMNode)(c);
  const b = (ReactDOM.findDOMNode)(c);
  return [a, b];
}
```

ESLint: `fd-parens.jsx:3:14`, `fd-parens.jsx:4:14` (react/no-find-dom-node). Translation: nothing. The `function` field is a
`parenthesized_expression`. `tsx/eslint-a-common`'s `callee(c, f)` already strips the parentheses.

### Defect 2: `this.#findDOMNode()` is not seen

```jsx
// fd-private.jsx
class A extends React.Component {
  #findDOMNode() { return null; }
  componentDidMount() {
    this.#findDOMNode();
  }
}
```

ESLint: `fd-private.jsx:4:5`. Translation: nothing. The property is a `private_property_identifier` with the text
`#findDOMNode`; ESTree's `PrivateIdentifier.name` is `findDOMNode`. `tsx/react-class`'s `estree_name` handles this case.

### Defect 3: a computed member `x[findDOMNode]` is not seen

```jsx
// fd-computed.jsx
import { findDOMNode } from "react-dom";
const api = { [findDOMNode.name]: findDOMNode };
api[findDOMNode](this);
```

ESLint: `fd-computed.jsx:3:1`. Translation: nothing. tree-sitter makes this a `subscript_expression`, and ESTree's
`property` is the `findDOMNode` identifier, which has the name. (`api["findDOMNode"]()` is not reported by either: a
`Literal` has no `.name`.)

### Defect 4: a tagged template is reported

```jsx
// fd-tagged.jsx
const findDOMNode = (s) => s;
export const q = findDOMNode`select * from nodes`;
```

ESLint: nothing (a `TaggedTemplateExpression` is not a `CallExpression`). Translation: `fd-tagged.jsx:2`. tree-sitter
parses a tagged template as a `call_expression` whose `arguments` is a `template_string`. ``ReactDOM.findDOMNode`x` `` is
reported in the same way.

Agrees on: `findDOMNode?.(this)`, `ReactDOM?.findDOMNode(this)`, `ReactDOM.findDOMNode?.(this)`, `super.findDOMNode()`,
`a.b.findDOMNode()`, a member split across lines (reported on the object's line by both), `findDOMNode<Element>(c)`.
Neither reports `obj["findDOMNode"]()`, `new findDOMNode()`, `(0, ReactDOM.findDOMNode)(this)`, `findDOMNode!(c)`,
`ReactDOM.findDOMNode!(c)`, `(ReactDOM.findDOMNode as any)(c)` or a bare `findDOMNode` reference.

## react-no-render-return-value: 3 defects

The plugin reports `ReactDOM.render(…)` (callee a `MemberExpression` on the identifier `ReactDOM` whose property has
`.name` `render`) when the call's ESTree parent is a `VariableDeclarator`, `Property`, `ReturnStatement`,
`ArrowFunctionExpression` or `AssignmentExpression`.

### Defect 1: the call as a computed key is not seen

```jsx
// rr-computed-key.jsx
import ReactDOM from "react-dom";
const byInstance = { [ReactDOM.render(<App />, root)]: true };
```

ESLint: `rr-computed-key.jsx:2:23`. Translation: nothing. The call's ESTree parent is the `Property` (its computed key).
In tree-sitter the parent is a `computed_property_name`, which `estree_parent` does not step over.

### Defect 2: `ReactDOM[render](…)` is not seen

```jsx
// rr-computed-member.jsx
import ReactDOM from "react-dom";
const render = "render";
const inst = ReactDOM[render](<App />, root);
```

ESLint: `rr-computed-member.jsx:3:14`. Translation: nothing (`subscript_expression`; ESTree's property is the identifier
`render`).

### Defect 3: a tagged template is reported

```jsx
// rr-tagged.jsx
import ReactDOM from "react-dom";
const x = ReactDOM.render`<div/>`;
```

ESLint: nothing. Translation: `rr-tagged.jsx:2`. This is the same tree-sitter `call_expression` as in react-no-find-dom-node's
defect 4.

Agrees on the kept values `const a = (ReactDOM.render(…))`, `x ||= …`, `x += …`, `x ??= …`, `{ a: … }`, `() => …`,
`() => (…)`, `return …`, `x = y = …`, `obj.prop = …`, `/* c */` before the call, `(0, x = …)`, `w = (/* c */ …)`, a call
split across lines, and `ReactDOM.render<P>(…)`. Neither reports `ReactDOM["render"]`, a destructuring default, a class
field, `ReactDOM.render?.(…)`, `ReactDOM?.render(…)`, an argument, a `?:` branch, a parameter default,
`export default`, `yield`/`await`, `void`, a JSX attribute value, `React.render`, `as any`, `!`, `satisfies` or
`<any>` (`.ts`).

## react-no-string-refs: 1 defect

### Defect: a comment before the string hides it

```jsx
// sr-comment.jsx
export const a = <div ref={/* legacy */ "a"} />;
```

ESLint: `sr-comment.jsx:1:23`. Translation: nothing. `child(v, e, 0)` takes the `jsx_expression`'s first child, which here is
the comment. `tsx/react`'s `prop_value(a, v)` skips comments. The JSON matched with `has` on the `jsx_expression` and
reported this case.

Agrees on `ref="a"`, `ref='f'`, `ref={'b'}`, `ref={("c")}`, `ref={(/* in parens */ "j")}`, `ref={"i" /* trailing */}`,
`ref = "a"`, a `ref` on its own line, `<Foo ref>`, `<Foo.Bar ref>`. Neither reports `` ref={`d`} ``, `ref={'g' + 'h'}`,
`ref={this.k}`, or `"a" as any`, `"b"!` and `"c" satisfies string` (`.tsx`). Without settings the plugin assumes the latest
React and does not check `this.refs`, as the header says.

## react-no-unescaped-entities: ok

Agrees on every case, with one finding per quote: `Don't "quote"` (3 on the line), quotes in multi-line text on each
line, a fragment, `&amp; it's &quot;` (the entities are not quotes), `it's{x}it's`, text in an element inside an attribute,
text that starts with `//` or `/*` or holds `http://` (tree-sitter keeps them as `jsx_text`), a quote at the start of a
line, and `&apos;ok&apos; but ' not`. Neither checks attribute strings or `{"it's"}`. A `>` in JSX text is a parse
error to @typescript-eslint/parser ("Unexpected token. Did you mean `{'>'}` or `&gt;`?"), as the header says.

## react-no-unknown-property: 1 minor defect

The generated tables agree with the plugin's. I loaded `DOM_ATTRIBUTE_NAMES`, `SVGDOM_ATTRIBUTE_NAMES`,
`ATTRIBUTE_TAGS_MAP`, the property lists, `DOM_PROPERTIES_IGNORE_CASE` and `ARIA_PROPERTIES` out of the plugin's source and
diffed them against `react-no-unknown-property.props.rerail`. The attribute map, the tag map, the any-case list and the
ARIA list are equal. The plugin's property list also has `hreflang`, `popovertarget`, `popovertargetaction` and
`rendering-intent`, which the facts leave out. Its case-insensitive `find` returns `hrefLang`, `popoverTarget` and
`popoverTargetAction` first, so those three change nothing. `rendering-intent` changes only the message for an odd casing:
for `<path RENDERING-INTENT>` the plugin says "use 'rendering-intent'" and the translation says "unknown", both at the same
place.

### Defect: `<this …>` in a JavaScript file is not checked

```jsx
// up-this.jsx
export function f() { return <this foo="x" />; }
```

ESLint: `up-this.jsx:1:36`. Translation: nothing. In a `.tsx` file both report it. The JavaScript grammar gives the tag
name the kind `this`, not `identifier`, so `html_tag` does not hold. `<this>` is not an HTML tag, so this matters only for
completeness.

Agrees on every other prop, among them `class`, `onclick`, `checked` off `<input>`, `foo`, `data-xml-foo`,
`data-Foo:bar`, `aria-foo`, `charSet`/`CHARSET` off `<meta>`, `allowFullScreen` off `<iframe>`, `tabindex`/`TABINDEX`,
`stroke-width`, `xlink:href`, `hreflang`, `xmlns:xlink`, `rendering-intent`, `crossorigin` (on `<div>` and `<img>`),
`crossOrigin` off its tags, `abbr` off `<td>`, `transform-origin`, `WEBKITDIRECTORY`, `displaystyle`, a prop on its own line,
a repeated `class` and `<div<string> class>`. Neither reports custom elements, `is`, components, `<a.b>`, `<svg:path>`,
`<fbt>`, `data-*`, ARIA names, `hrefLang`, `popovertarget`, `precedence`, `key`/`ref`/`children`/
`dangerouslySetInnerHTML`, `onPointerDown` or `webkitdirectory` on `<input>`.

## react-no-unsafe: 7 defects

The plugin checks every `ClassDeclaration`, `ClassExpression` and `ObjectExpression` that `isES6Component` or
`isES5Component` accepts. For each member whose `getPropertyName` (`key.name`) is unsafe, it reports the first member with
that name.

### Defect 1: a class declared a component by JSDoc is not seen

```jsx
// us-jsdoc.jsx
/**
 * @extends React.Component
 */
class Legacy {
  UNSAFE_componentWillMount() {}
}
/** @augments React.PureComponent */
export class Legacy2 {
  UNSAFE_componentWillUpdate() {}
}
```

```jsx
// us-jsdoc-export.jsx
/** @extends React.Component */
export default class Legacy {
  UNSAFE_componentWillMount() {}
}
```

ESLint: `us-jsdoc.jsx:5:3`, `us-jsdoc.jsx:9:3`, `us-jsdoc-export.jsx:3:3`. Translation: nothing. `isES6Component` starts with
`isExplicitComponent`, the class's JSDoc `@extends` or `@augments` naming `React.Component` or `React.PureComponent`.
`tsx/react`'s `class_component` has only the `extends` forms. The JSON's `es6-component` has the JSDoc forms
(`jsdoc-component`, `after-jsdoc-component`). The fix belongs in the shared `class_component`, which other class rules use
too.

### Defect 2: `new createReactClass({…})` is not seen

```jsx
// us-new.jsx
const N = new createReactClass({
  UNSAFE_componentWillMount() {},
});
```

ESLint: `us-new.jsx:2:3`. Translation: nothing. `isES5Component` reads `node.parent.callee`, which a `NewExpression` has.
`create_class` asks for `field(c, "function", f)`, and a `new_expression`'s field is `constructor`. The JSON's
`es5-component` has the `new_expression`.

### Defect 3: a parenthesised superclass is not seen

```jsx
// us-parens-super.jsx
class A extends (React.Component) {
  UNSAFE_componentWillMount() {}
}
```

ESLint: `us-parens-super.jsx:2:3`. Translation: nothing (`component_base` gets a `parenthesized_expression`). The JSON's
`component-superclass` steps through parentheses.

### Defect 4: an abstract method is not seen

```tsx
// us-abstract.tsx
import React from "react";
export abstract class Base extends React.Component<{}> {
  abstract UNSAFE_componentWillMount(): void;
}
```

ESLint: `us-abstract.tsx:3:3` (a `TSAbstractMethodDefinition`, whose `key.name` is the method's name). Translation: nothing.
`method_key` has no `abstract_method_signature`, which the JSON's `will-mount` lists.

### Defect 5: with overloads, the implementation is reported instead of the first signature

```tsx
// us-overload.tsx
import React from "react";
export class A extends React.Component<{}> {
  UNSAFE_componentWillReceiveProps(next: {}): void;
  UNSAFE_componentWillReceiveProps(next: {}, ctx: unknown): void;
  UNSAFE_componentWillReceiveProps(next: {}, ctx?: unknown) {}
}
```

ESLint: `us-overload.tsx:3:3`, three times. Each overload signature is a `MethodDefinition`, and every one of them is
reported at the first. Translation: `us-overload.tsx:5`. `method_key` has no `method_signature`, so the "first member" is the
implementation. The JSON's `will-mount` lists `method_signature`.

### Defect 6: a parenthesised computed key is not seen

```jsx
// us-computed-parens.jsx
class A extends React.Component {
  [(UNSAFE_componentWillMount)]() {}
}
```

ESLint: `us-computed-parens.jsx:2:3` (ESTree's key is the identifier). Translation: nothing. `estree_name` takes the
`computed_property_name`'s child 0 only if it is an identifier.

### Defect 7: the `@jsx` pragma is not read

```jsx
// us-pragma.jsx
/** @jsx h */
import { h, Component } from "./my-jsx";
class A extends React.Component {
  UNSAFE_componentWillMount() {}
}
class B extends h.Component {
  UNSAFE_componentWillMount() {}
}
```

ESLint: `us-pragma.jsx:7:3`. With the pragma `h`, `h.Component` is a component and `React.Component` is not. Translation:
`us-pragma.jsx:4`, from `run-policies` and from `run-rr` alike. `tsx/react`'s `pragma_comment` takes `min` over
`visit_index`, which has no rows in this engine, so `pragma(d, "React")` always holds. This is the cause that the
react-practice-a review found for `@jsxImportSource` (`first_source_comment`), here in the `@jsx` comment. It belongs to
the shared library. The JSON hard-codes `React`, so it has this defect too.

Agrees on: `React.Component`, `Component` and `React.PureComponent` classes; a class expression and an anonymous
`export default class`; static members, arrow fields, getters, `#UNSAFE_…`, `[UNSAFE_…]`, `accessor`, an optional method
(`?()`), `override`, `declare`; a member with two decorators (both report the first decorator's line); `createReactClass`
with a method, a `function` value, a shorthand property, a getter, a second argument, `?.(…)`, and
`React.createReactClass`. Neither reports a plain class, a string key (`'UNSAFE_…'()`, `['UNSAFE_…']()`, a `'UNSAFE_…':`
pair), `foo.createReactClass({…})` or a plain object. The first member is chosen as the plugin chooses it: with two members
of one name (`us-dup.jsx`; a field and then a method), both sides report the first one's line. The plugin reports that line
once per member of that name, but the suite compares sets.

## The author's result file

Read after the findings above were written. `react-04.result.json` lists all six rules as `different`, `right:
"original"`, and gives one cause for each: every translation-only key on samples-react is a finding that an
`eslint-disable` comment for the rule silences, and run-policies does not apply `suppress` yet. I checked every key that the
suite's report samples (28 of the 71: 5 per rule, 3 for react-no-render-return-value). On pt-w01 I linted each sample file
with the pinned ESLint and the pack configuration, once with inline config and once with `noInlineConfig`:

| Rule | Keys checked | ESLint with inline config | ESLint with noInlineConfig | Comment |
|---|---|---|---|---|
| react-no-find-dom-node | 5 | 0 | 5 | `eslint-disable-next-line` / `eslint-disable-line react/no-find-dom-node` |
| react-no-render-return-value | 3 | 0 | 3 | `eslint-disable-next-line react/no-render-return-value` |
| react-no-string-refs | 5 | 0 | 5 | next-line, and one file-level `/* eslint-disable react/no-string-refs, … */` |
| react-no-unescaped-entities | 5 | 0 | 5 | file-level `/* eslint-disable react/no-unescaped-entities */` (404.tsx, FAQ.tsx) |
| react-no-unknown-property | 5 | 0 | 5 | next-line, and file-level in integerPropType.test.js |
| react-no-unsafe | 5 | 0 | 5 | next-line, and file-level (`/* eslint-disable react/no-unsafe */` on line 2 of interaction-editor.tsx) |

So the `right` side, `original`, is the tool's answer on every key checked, and the cause is the one the result file gives
(the known run-policies suppression gap).

The NOTES do not mention the defects above, and their "Library changes: none needed" does not hold for react-no-unsafe.
Defects 1 to 3 are in `tsx/react`'s `class_component` and `create_class`, and defects 4 and 6 are in `tsx/react-class`'s
`method_key` and `estree_name`. Other class rules share all of them. The NOTES' ground truth sets
`settings.react.version` to "19.0.0", while the pack's configuration has no settings, which means the latest React. For
these six rules the two give the same answers: the rules test the version only against 0.13, 0.14, 15, 16.1, 16.3, 16.4,
18.3 and 19.
