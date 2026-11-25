# Review: misc-untranslated

Branch reviewed: `tbelanger/pack-translations-misc-untranslated` at `6ccec9c7f3`. I reviewed the `.rerail` files, their JSON
originals and the tools without reading the author's NOTES first. Tools ran on `junction-pt-w01`: Bandit 1.9.4
(Python 3.15), lizard 1.24.0, ESLint 9.39.5 with `@typescript-eslint/parser` 8 for `.ts`/`.tsx`, and
`@next/eslint-plugin-next` 16.3.8. Each translation ran with `rerail-engine run-policies` (`run-rr <rule> <root> <out>
<options.json>` where an option is set), using this branch's `docs/rule-syntax/lib`.

| Rule | Verdict |
|---|---|
| `complexity-cyclomatic-java` | 1 defect |
| `next-no-page-custom-font` | 4 defects (2 miss, 2 extra) |
| `eslint-one-var` | 1 defect |
| `bandit-b105-hardcoded-password-string` | 2 defects, plus 1 negligible case |

## complexity-cyclomatic-java

**Defect: the `ceiling` option is ignored.** The JSON declares `ceiling` (integer, default 6) and fires at
`min: {"$option": "ceiling"}`. The pack README documents `.rerail/packs.json` `config: {"ceiling": 10}` for all
8 rules. The translation never reads `option("ceiling", …)`: `cc_report` in `lib/complexity/cyclomatic.rerail`
hardcodes `k > 6` and the text "over the ceiling of 6". The other complexity rules share this library, so the fix
belongs there: compare against the option's value (default 6) and use that value in the message.

```java
// src/main/java/p/Ceiling.java, run with options {"ceiling": 10}
class Ceiling {
    int seven(int a, int b) {
        if (a > 0) { a++; }
        if (b > 0) { b++; }
        for (int i = 0; i < a; i++) { b++; }
        while (b > 10) { b--; }
        return a > b && b > 0 || a < 0 ? 1 : 2;
    }
}
```

- lizard: `Ceiling::seven` CCN 8. With a ceiling of 10, the original does not report it (8 ≤ 10).
- translation (`run-rr … {"ceiling": 10}`): `Ceiling.java:4 CYCLOMATIC_COMPLEXITY`, "`seven` has cyclomatic
  complexity 8, over the ceiling of 6."

The counting itself matched: I compared lizard 1.24.0, standard and `-m`, with the translation on 11 methods. They
covered lambdas (counted in the enclosing method), an anonymous class's method, a local class's method, an enum
constant's method, an interface default method, a constructor, a record's compact constructor, a classic switch, a
pattern switch with `when`, nested ternaries, do/while and multiple catches. The translation reported exactly the
methods over 6 under the pack's rule: a switch counts once, as the lower of lizard's two counts. The one exception
is the compact constructor, which lizard does not see as a function and the pack deliberately scores.

## next-no-page-custom-font

All four defects concern the plugin's own order-dependent and type-exact checks.

**Defect 1 (miss): a Document subclass declared before `export default class` is treated as the default
component.** In the plugin, `exportDeclarationType` is set only when the traversal reaches `ExportDefaultDeclaration`.
For a `<link>` visited before that point, the `ClassDeclaration` branch is not taken yet. The token scan finds no
identifier after `export default class`, so `parentComponent` is undefined and the link is reported. The
translation's `default_component` reads the whole file and accepts any class extending the `next/document` import.

```jsx
// pages/_document.js
import Document, { Html, Head, Main, NextScript } from 'next/document'

class Helper extends Document {
  render() {
    return <link href="https://fonts.googleapis.com/css2?family=Inter" rel="stylesheet" />
  }
}

export default class MyDocument extends Document {
  render() {
    return <Html><Head /><body><Main /><NextScript /></body></Html>
  }
}
```

- ESLint: `pages/_document.js:5 @next/next/no-page-custom-font` ("Using `<link />` outside of `<Head>` …").
- translation: nothing.

**Defect 2 (miss): a class *expression* extending Document counts as the default component.** The plugin requires
`ancestor.type === 'ClassDeclaration'`. `class_kind` includes tree-sitter's `class`, which the anonymous
`export default class extends Document` needs, so it also matches every class expression in the file. Accept
`class` only when it is the default export itself.

```jsx
// pages/_document.js
import Document, { Html, Head, Main, NextScript } from 'next/document'

export default class MyDocument extends Document {
  render() {
    return <Html><Head /><body><Main /><NextScript /></body></Html>
  }
}

const Other = class extends Document {
  render() {
    return <link href="https://fonts.googleapis.com/css2?family=Inter" rel="stylesheet" />
  }
}
```

- ESLint: `pages/_document.js:11 @next/next/no-page-custom-font`.
- translation: nothing.

**Defect 3 (extra): `export default abstract class` falls through to the token rule.** typescript-eslint gives an
abstract class `type: 'ClassDeclaration'`. tree-sitter gives `abstract_class_declaration`, which neither `class_kind`
nor `declared` lists. The third `default_component` rule then takes the first word, `abstract`, as the component's
name, so the link inside the Document is reported.

```tsx
// pages/_document.tsx
import Document, { Html, Head, Main, NextScript } from 'next/document'

export default abstract class MyDocument extends Document {
  render() {
    return (
      <Html>
        <Head>
          <link href="https://fonts.googleapis.com/css2?family=Inter" rel="stylesheet" />
        </Head>
        <body><Main /><NextScript /></body>
      </Html>
    )
  }
}
```

- ESLint: nothing.
- translation: `pages/_document.tsx:8 @next/next/no-page-custom-font`.

**Defect 4 (extra): the plugin reads only the last `href`.** `NodeAttributes` keeps one entry per name, and the last
one wins. `attr_str` holds for every `href` (`lib/tsx/next.rerail` says so), and this rule does not narrow it to the
last one.

```jsx
// pages/index.js
export default function Home() {
  return <link href="https://fonts.googleapis.com/css2?family=Inter" href="/local.css" rel="stylesheet" />
}
```

- ESLint: nothing.
- translation: `pages/index.js:2 @next/next/no-page-custom-font`.

Checked and the same on both sides: a link outside `pages` (`components/Font.jsx`, reported with the "single page"
message); `class MyDocument extends Document {…}; export default MyDocument` with the link inside `<Head>` (not
reported); a sibling `function Side()` link in the same file (reported); `href={"…"}`.

## eslint-one-var

**Defect: `declare global { … }` opens a block scope in the translation but not in ESLint.** typescript-eslint parses
`declare global { … }` as a `TSModuleDeclaration` whose body is a `TSModuleBlock`. one-var pushes no block for that,
so `let`/`const` inside share the enclosing scope. tree-sitter-typescript parses `declare global` as an
`ambient_declaration` holding a `statement_block` directly, with no `module`/`internal_module` node. So
`namespace_body` (`lib/tsx/statements.rerail`) does not exclude it, and `block_scope_at` makes it a scope.
`namespace_body` also needs the `statement_block` child of an `ambient_declaration`, which is `declare global`'s body.

```ts
// j1_declare_global.ts
const x = 1;
declare global {
  const y: number;
}
export {};
```

- ESLint: `j1_declare_global.ts:3 one-var` "Combine this with the previous 'const' statement."
- translation: nothing.

The rest matched ESLint exactly. With default options I checked 20 cases; they included static blocks, a for head,
switch cases, `if (a) var u;`, `var` in a namespace, `declare module`, `namespace`, `export const`, definite
assignment, TypeScript `using`, an empty statement between declarations, and the require quirk
(`var a = 1; var fs = require("fs");` is not reported). I also ran seven option sets (`run-rr` with the options
object) on one 20-statement file: `never`, `consecutive`, `{separateRequires, var/let/const: always}`,
`{initialized: never}`, `{var: never, let: consecutive, const: always}`, `{initialized: consecutive, uninitialized:
always}` and `{uninitialized: consecutive}`. All were identical to ESLint with the same options (2–18 keys each).

## bandit-b105-hardcoded-password-string

**Defect 1 (miss): parenthesised assignment targets.** CPython strips the parentheses, so the target is a
`Name`/`Attribute`/`Subscript`. tree-sitter-python parses a parenthesised left-hand side as a pattern, not a
`parenthesized_expression`, so `unparen` leaves it as is and `target_name`/`subscript_key` fail. The rule's header
says "parentheses change nothing". That holds for values and comparison operands, but not for targets.

```python
(password) = "a"
password = ("b")
(self.token) = "c"
(d["secret"]) = "d"
x = (password) = "e"
```

- Bandit: lines 1, 2, 3, 4, 5 (B105).
- translation: line 2 only.

**Defect 2 (miss): a comment inside a comparison shifts the operand indexes.** `child(c, l, 0)` / `child(c, r, 1)`
count named children, and a comment is one. When a comment falls between the left operand and the first comparator,
`child(c, r, 1)` is the comment, so the comparison is never matched.

```python
def check(password):
    if (password  # the user's
            == "admin"):
        return True
```

- Bandit: line 3 (B105).
- translation: nothing.

Skip comments when indexing the operands. `cmp_pair` in `lib/python/ast.rerail` has the same indexing.

**Negligible: a str literal as the subscripted *value*.** Bandit's Subscript branch tests any str whose parent is a
`Subscript`, so `"api_token"[0] = "x"` is reported at line 1. The translation only looks at the slice. The code
always raises at runtime, so I would not change the rule for it.

Checked and the same on both sides, across 46 lines:
- assignments: chained targets, `+=`, annotated assignments (plain and subscript), the walrus operator, tuple and
  list targets, a parameter default;
- dict keys with `None`/bytes/`...`/`1j`/`-1`/f-string values, a dict comprehension, and a `t"…"` value (Python
  3.15: not a Constant, neither side reports);
- subscripts: `d["password",]`, `d[("password")]`, `d["x"]["password"]`, `d[k]["password"]`, `obj.d["token"]`,
  `d["Token\n"]` (Python's `$` before a final newline), and a comma inside a comment in a subscript;
- comparisons: `is not`, `not in`, a chained `== … !=` (one finding per str operand), a str first comparator
  followed by a non-str one, multi-line and implicitly concatenated strings (the first line);
- the name regex on `PASSWORD`, `my_pass`, `passwd_hash`, `tokens`, `api_secret_key`, `secrete` and `pas_wd`.

## The author's result files

I read these after writing the findings above.

| Rule | Status | `right` | Check | Agrees |
|---|---|---|---|---|
| `complexity-cyclomatic-java` | same | – | The corpora never set `ceiling`, so the defect above cannot show there. The author's NOTES records that the option is not read and defers it as a library change. It is still a divergence from the original. | – |
| `next-no-page-custom-font` | same | – | None of the four defects above occurs in the pack's fixtures or samples. | – |
| `eslint-one-var` | different, 130 translation-only on rerail-typescript | translation | ESLint 9.39.5 (default options, typescript-eslint parser) on the corpus's `src/engine/code-rule/code-rule-schema.ts` and `test/support/ui-modules.d.ts` reports 127 + 10 keys. That includes all 127 in the schema file and the sample keys `ui-modules.d.ts:132, 203, 276, 308`. The translation on the same two files gives the identical 137 keys. | yes |
| `bandit-b105-hardcoded-password-string` | different, 4 translation-only on fixtures | original | Bandit 1.9.4 on `bandit/rerail/nosec/B105.py` reports lines 1 and 11 only, and 1, 3, 5, 7, 9, 11 with `--ignore-nosec`. The translation reports 1, 3, 5, 7, 9, 11. The 4 extra keys are the `# nosec` lines, which is the known run-policies gap (suppression comments not applied). `suppress/nosec.rs` maps test names such as `hardcoded_password_string` to ids as Bandit does, so line 9 should be suppressed once the gap is fixed (not observed: run-policies cannot show it). | yes |
