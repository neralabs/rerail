# Batch misc-untranslated: next

## next-no-page-custom-font: same

check-rr refused revision 2: `default_component: match_all over $v, a node or span not bound by source, src_line, kind
or a capture`. The third `default_component` clause ran `match` over the export's node. It now reads the node's
`text` and matches that: the same test (the first identifier token after `export default`, the plugin's token scan).

Final ptcheck: same (fixtures 11 both; examples and samples-next have no findings on either side).

Library changes: none (the review fixes below too). Engine gaps: none found.

### Review fixes

The review found four divergences from eslint-plugin-next 16.3.8, none in the pack's corpora. The plugin decides at
each `<link>` from what its traversal has seen so far; the translation read the whole file. `default_component` is
replaced by `allowed(o)`, which follows the plugin's state at the link:

- the default export counts only once its statement starts before the link (`seen`, by start offset);
- seen and a ClassDeclaration (`class_declaration`, `abstract_class_declaration`, or the `class` that is the default
  export itself): a class *declaration* around the link extending next/document's default import;
- seen and a FunctionDeclaration (`function_declaration`, `generator_function_declaration`, or the anonymous default
  function): a function declaration with the same id, the token scan's name when the default one has none;
- otherwise the token scan (`scan_name`): the first Identifier token after `export default` (`async` and `abstract`
  are identifier tokens; `class`, `function` and the other keywords give none), matched against any declaration with
  that id.
- the link's href is its last `href` attribute (`last_href`), as NodeAttributes keeps one entry per name.

| Case (review) | ESLint | Before | After |
|---|---|---|---|
| 1. `class Helper extends Document` with a link, above `export default class` | line 5 | nothing | line 5 |
| 2. `const Other = class extends Document` with a link, below `export default class` | line 11 | nothing | line 11 |
| 3. `export default abstract class MyDocument extends Document`, link in `<Head>` | nothing | line 8 | nothing |
| 4. `<link href="…fonts…" href="/local.css">` in pages/index.js | nothing | line 2 | nothing |

Further cases, ESLint 9.39.5 with the plugin (espree for .js, typescript-eslint for .tsx) against `run-rr`, identical
on all 15 files: a sibling function in a Document; `export default MyDocument` after the class; an anonymous
`export default function` (allowed) and one with a later `href` overriding; a function above `export default function
Doc2`; `export default async function ()` (reported: the scan finds `async`); `export default withX(Doc)` (Doc's link
reported: the scan names `withX`); an anonymous `export default class extends D` beside another subclass; a function
above, and one named `abstract` below, `export default abstract class`; components/Font.jsx; an arrow component
`const Doc = () => …` above `export default Doc`; a valueless and an expression last href; `export default function*`.

ptcheck after the fix: same (fixtures 11 both; examples and samples-next none on either side).
