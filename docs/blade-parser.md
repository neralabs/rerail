# Rerail's Blade parser: a port of Forte

Rerail parses Blade with a Rust port of Forte 1.1.0, the fault-tolerant parser Blade's own tooling
uses (github.com/fortephp/forte, tag `v1.1.0`, commit 9c9597d0b7, MIT, by John Koster).

It replaced tree-sitter-blade 0.12.2 (b5291d1), which reported an `ERROR` in hundreds of real
templates ([T1](#t1-templates-with-an-error)). `fixtures/blade/blade/grammar-gaps/repros/` holds a
minimal repro of each error class; Forte reads all of them without an error.

## What is ported

The port covers the two stages that decide structure, under `native/src/code_rule_dsl/blade/forte/`.

- **Lexer** (`Lexer::tokenizeCompact`): token records and `LexerError`s.
- **Tree builder** (`TreeBuilder::buildCompact`): a flat array of node records.

Each PHP method is one snake-cased Rust function whose doc comment names it, as
`literal/phpdoc/{lexer,parser}.rs` ports phpstan/phpdoc-parser.

<details>
<summary>How Forte runs the stages, and the file map</summary>

`Document::parse($template, $options)` (`Ast/Document/Document.php:164-213`) runs the two stages and
keeps their arrays. Node records are `{kind, parent, firstChild, lastChild, nextSibling,
tokenStart, tokenCount, genericOffset, data}`, plus `name`, `args` and `role` extras on directive
records.

Between the stages, when the registry accepts every directive, `trainCompact` trains the registry on
the tokens. The rest of Forte is a lazy object API over the arrays.

| Forte | Port | What it decides |
|---|---|---|
| `Lexer/Lexer.php`, `ConstructDetector.php`, `Construct.php`, `Concerns/*` (176 KB) | `lexer/` | tokens and lexer errors |
| `Parser/TreeBuilder.php`, `ConstructScanner.php`, `Concerns/*`, `OptionalTags/*` | `parser/` | the flat node array |
| `Parser/Directives/{DirectiveTokenIndex,DirectiveHelper,ArgumentScanner}.php` | `directives/{token_index,helper,arguments}.rs` | pairing, branches, terminators |
| `Parser/Directives/Directives.php` + `resources/directives/*.json` (21 files) | `directives/mod.rs` | which `@word` is a directive, its kind and pairs |
| `Support/ScanPrimitives.php` and the string helpers the two stages call | `scan.rs` | quoted strings, PHP/JS skipping |
| `Ast/Elements/VoidElements.php` | `parser/` | void elements |
| `ElementNode`, `ComponentManager`, `Attributes`, `Attribute`, `DirectiveBlockNode`, `EchoNode`, `Php*Node` reads | `ast.rs` | tag names, component/slot names, attributes, a block's start/end directive, echo/PHP content |

</details>

<details>
<summary>PHP semantics the port reproduces</summary>

The PHP string semantics Forte relies on are reproduced: `$s[$i]` past the end is `""`, `substr`
clamps, `strtolower` and `ctype_*` are ASCII (`ctype_space` includes `\v` and `\f`), and `trim`
uses its default characters.

The parse path's one regular expression,
`ProcessesDirectives::recoverRepeatedUnterminatedPairedDirective` (a lookbehind-anchored
`preg_match` for a compact terminator inside text), is a byte scan with the same semantics. The
lexer has none; `LineIndex::build`'s `preg_match_all` only computes line numbers.

</details>

Not ported, each checked against Forte's code: the AST object API, `Rewriting/`, `Querying/`,
`Extensions/`, `Enclaves/`, `Diagnostics/` and Laravel wiring.

<details>
<summary>What each skipped part is, and why it is safe to skip</summary>

- **The AST object API** (`Ast/`, 270 KB, except the reads above): lazy wrappers
  (`ManagesAst::getNode`), traversal, rendering, offsets. Rerail has its own tree; the reads the
  oracle records are ported, so names, attributes and block ends are Forte's.
- **`Rewriting/`** (114 KB): visitors and rewrite passes over a parsed document;
  `Document::parse` never calls it.
- **`Querying/`** (25 KB) **and `DomMapper`**: XPath and a DOM projection built from
  `Document::children()` after the parse. Rerail's rules are its query language.
- **`Extensions/`** (28 KB): created only when `ParserOptions::hasExtensions()`; otherwise the
  lexer's and tree builder's hooks are no-ops. A one-line comment marks each hook call site.
- **`Enclaves/`** (67 KB): path-matched configuration and rewrite orchestration
  (`EnclavesManager::getParserOptionsForPath`, `Precompiler`). An enclave adds only extensions
  (`Enclave::getParserOptions`), never a per-path directive registry, so with no extensions it
  changes nothing a parse produces.
- **`Diagnostics/`** (11 KB): value objects adapting what the parse kept. `Document::diagnostics()`
  converts lexer errors; `addParseError` has no caller in Forte, so the tree builder records none.
  The port returns the lexer errors and the depth-limit exception.
- **Laravel wiring**: `Components/ComponentManager` beyond `isComponent`/`parseComponentName` with
  the default prefixes (`x-`, `livewire:`, `flux:`), `Facades/`, `ServiceProvider`, `Internal/`.

</details>

## Forte's tree

Forte's tree is a lossless flat array of nodes over tokens that partition the template.

- **Token:** a byte span; every byte is in exactly one token.
- **Node:** owns `tokenCount` tokens from `tokenStart`. Children own sub-ranges; the rest (delimiters,
  whitespace inside a tag, a directive's name and arguments) is the node's own.
- **`DirectiveBlock`:** a paired directive. Its children are the opening `Directive`, each branch
  (`@else`, `@elseif`, `@case`, `@empty`) and the closer.

<details>
<summary>Node kinds and block token ranges</summary>

Text, whitespace included, is `Text`; whitespace between attributes is `AttributeWhitespace`. Kinds
(`Parser/NodeKind.php`): `Root`, `Element`, `Text`, `Echo`, `RawEcho`,
`TripleEcho`, `Directive`, `DirectiveBlock`, `Verbatim`, `PhpBlock`, `PhpTag`, `Comment`,
`BogusComment`, `ConditionalComment`, `BladeComment`, `Doctype`, `Cdata`, `Decl`, `Attribute`,
`JsxAttribute`, `UnpairedClosingTag`, `NonOutput`, `ElementName`, `ClosingElementName`,
`AttributeWhitespace`, `AttributeName`, `AttributeValue`, `ProcessingInstruction`.

The port omits what Forte declares but never uses: `Fragment` (the tree builder never creates it),
unlogged lexer states and error reasons, and registry queries the parse never calls.

A branch's content is its children, outside its own tokens. The block's token range starts as the
opener's and grows to the closer's when it closes (`ProcessesDirectives.php:681-693`).

An unclosed block keeps the opener's range while its children run on. Rerail therefore spans a node
by its tokens plus its children's.

</details>

Forte recovers from malformed input without error nodes; past fixed nesting limits the tree builder
throws.

<details>
<summary>Recovery rules and depth limits</summary>

- An unclosed element is closed by its parent's end tag.
- A stray closing tag is an `UnpairedClosingTag`.
- An unknown `@word` is text (default registry) or a directive (accept-all).
- Past 512 nested elements, 256 directives or 256 conditions, the tree builder throws a
  `RuntimeException` (`TreeBuilder.php:35-42`, messages like `Maximum element nesting depth (512)
  exceeded.`). The port returns that message as `ParseError::Depth`.

</details>

## Rerail's tree

`forte/tree.rs` exposes a Forte parse as an ast-grep tree for rules, patterns, tapes and the view.

`tree::parse_tree` accepts every directive, so an application's or package's `@name` (`@ray(...)`,
`@livewire(...)`, a `Blade::if` pair) is a `directive` node, not text, as Blade's tooling (tlint,
blade-parser) reads it.

- **Nodes:** one per Forte flat node, in preorder; the root is `document`, spanning the template.
- **Kinds:** Forte's AST class or node kind names in snake case, such as `element`, `component`,
  `directive`, `echo`.
- **Leaves:** a node's own tokens. Delimiters are anonymous leaves named by their text; meaningful
  tokens are named leaves such as `php`, `directive_name` and `tag_name`.
- **Errors:** no error nodes; errors are byte ranges on the tree.

| Node | Fields |
|---|---|
| `element`, `component`, `slot` | `name`, `closing_name` |
| `attribute`, `jsx_attribute` | `name`, `value` |
| `directive` | `name`, `arguments` |
| `echo`, `raw_echo`, `triple_echo` | `expression` |
| `php_block`, `php_tag` | `code` |
| `directive_block` | `start`, `end` (`DirectiveBlockNode::startDirective`/`endDirective`) |

The module doc of `tree.rs` is the authoritative list of kinds, leaves and fields.

<details>
<summary>How the tree plugs into ast-grep</summary>

The `Doc` is ast-grep-core's, here `BladeDoc` (nodes `BladeNode`), built like `literal/mod.rs`'s
sub-trees (`SubTree`/`SubDoc`/`SubNode`). Node ids (`ID_FLAG`, a tree ordinal, the preorder index)
never collide with tapes' or tree-sitter's.

`tree::parse_tree`, used by tapes, patterns and the view, takes Forte's default directives and
accepts every directive (`ParserOptions::acceptAllDirectives`). Nodes follow the preorder of
Forte's links.

</details>

<details>
<summary>Full kind and leaf lists</summary>

- **Kinds.** The AST class `Document::createBuiltInWrapper` wraps the node in, snake-cased without
  `Node`: `text`, `element`, `component`, `slot` (`ComponentManager` decides), `directive`,
  `directive_block`, `comment`, `conditional_comment`, `bogus_comment`, `blade_comment`,
  `php_block`, `php_tag`, `verbatim`, `doctype`, `cdata`, `xml_declaration`,
  `processing_instruction`, `escape` (`NonOutput`), `stray_closing_tag` (`UnpairedClosingTag`).
  Where one class serves several node kinds, the kind's name is used: `echo`, `raw_echo`,
  `triple_echo`; `element_name`, `closing_element_name`; `attribute`, `jsx_attribute`,
  `attribute_whitespace`, `attribute_name`, `attribute_value`.
- **Leaves.** A node's own tokens become leaves in text order; each byte is in at most one leaf.
  - Delimiters are anonymous leaves whose kind is their text, as in tree-sitter, so patterns
    compare them by text: `<`, `>`, `/`, `=`, quotes, `<!--`, `-->`, `<!DOCTYPE`, `@`,
    `{{`, `}}`, `{!!`, `!!}`, `{{{`, `}}}`, `{{--`, `--}}`, `(`, `)`, `@verbatim`, `@endverbatim`,
    `@php`, `@endphp`, `<?php`, `<?=`, `<?`, `?>`, `<![CDATA[`, `]]>`, `<?xml`.
  - Meaningful tokens are named leaves: `php` (the trimmed PHP of an echo, a directive's arguments
    between `(` and `)`, a `@php` block or a `<?php` tag), `directive_name`, `tag_name`,
    `attribute_name`, `attribute_value`, `bound_attribute_name` (`:x`), `escaped_attribute_name`
    (`::x`), `shorthand_attribute_name` (`:$x`), `comment_text`, `text`, `doctype_value`,
    `conditional_comment_start`, `conditional_comment_end`, `processing_instruction_start`,
    `jsx_attribute_value`, `jsx_shorthand_attribute`, `tsx_generic_type`.
  - `text` and `attribute_whitespace` nodes are leaves themselves.
  - Whitespace tokens, zero-width tokens (`SyntheticClose`) and the whitespace trimmed off `php`
    and `comment_text` are gaps between leaves, as in tree-sitter trees.
- **Errors.** Forte recovers without error nodes. Lexer errors (zero-width at their offset) and a
  `ParseError` (the whole template, under a lone `document`) are byte ranges on the tree, like a
  tape's syntax-error ranges.

</details>

Every recorded tree holds tree-sitter's invariants, even where Forte's links do not nest by span;
`tree::tests::trees_are_well_formed` checks them over the suite, fixtures and corpora.

<details>
<summary>Where spans don't nest, and how the tree resolves it</summary>

Forte's links do not always nest by span. Of 86,361 recorded trees (suite and corpus dumps):

- **11** have a directive block, left open where an element closes, that counts the element's
  closing tag and a later `@endX` among its tokens and so overlaps a sibling
  (`<ul><li>@if($c)Active@endif<li>…`,
  `<x-card><x-slot:content>@if($show)Content@endif</x-slot></x-card>`).
- **90** have a node with neither tokens nor children (an `attribute_name` with no tokens in
  `<div =value>`).

The tree resolves both by rule:

- A token belongs to the node with the fewest tokens whose range holds it, the deeper on a tie.
  Directive blocks and the root claim none; their span comes from their children.
- A node spans the tokens it owns and its children.
- A node with neither is zero-width where its previous sibling ends, or at its parent's start.

Every recorded tree then holds tree-sitter's invariants: children inside their parent, siblings in
order without overlap, every non-whitespace byte in exactly one leaf.

</details>

### Embedded PHP for the view

The view (`blade/view.rs`) copies a template's embedded PHP byte for byte. Each span is a `php` leaf
with its exact byte range, under a node naming the construct:

| Construct | Node | PHP span |
|---|---|---|
| `{{ e }}`, `{!! e !!}`, `{{{ e }}}` | `echo`, `raw_echo`, `triple_echo` | `expression` field |
| `@name(args)` | `directive` (`name` field) | inside the parentheses, to the `)` Blade's compiler balances (`view::balanced`) |
| `@php … @endphp` | `php_block` | `code` field |
| `<?php … ?>`, `<?= … ?>` | `php_tag` | everything between the tags; `<?=`'s `code` field |
| `:attr="e"`, `:$name` on `component`, `slot` | `attribute` | `value` without its quotes, or the name after `:` |
| `@if … @elseif … @else … @endif`, loops, `@switch`, `@once`, `@pushIf`, `@push` | `directive_block` with `start`, branch `directive`s, `end`; a directive's body is its children | each directive's arguments |

Where Forte leaves a structure unpaired or text unread, the view reads it as Blade's compiler does.

<details>
<summary>Where the view departs from Forte</summary>

The view reads `@name(args)` only where spaces or tabs alone separate the name and the `(`, as
Blade's `compileStatements` does. Forte also skips line breaks there; the view reads such a `(…)` as
template text, and the tape reports that text's syntax errors instead of Forte's
(`view::Reading::syntax_errors`).

Forte pairs a block's directives; the view reads a structure from the block. Where Forte leaves a
control structure unpaired (directives side by side in an attribute value,
`class="a @if($x) b @endif"`), the view pairs it among its siblings as Blade's compiler does
(`view::flat_structures`). One it cannot close is not read.

Text Forte leaves unread, the view reads as a template of its own (`Emitter::template`), as Blade's
compiler reads any text:

- **`<script>` and `<style>`.** Forte lexes Blade in them (echoes, comments, directives are nodes)
  but not markup, such as a component tag in `<script type="text/x-template">`.
- **A `@php` in a `<script>`'s raw text.** Forte runs it to the end of the template; Blade, to its
  first `@endphp`.
- **A directive's arguments.** Forte can run them past the `)` where Blade's compiler ends them
  (`@js("{$map["("]}")`).

An escaped echo Forte reads as an echo in raw text (`@{{ … }}`) is text, its directives compiled.

</details>

### Patterns

A Blade pattern is parsed with `tree::parse_tree` like a template. `{{ $X }}` is an `echo` whose
`php` leaf is the metavariable; `@include($V)`, a `directive` whose arguments leaf is.

A pattern Blade reads as text alone (`sprintf($F, $$$A)`) is PHP and matches the embedded program
(`language::rule_pattern`).

<details>
<summary>How ast-grep builds a Blade pattern</summary>

ast-grep parses a pattern into the language's `Doc`
(`Pattern::try_new` → `Language::build_pattern` → `PatternBuilder::build(|src| Doc)`) and matches
node against node. A leaf whose text is a metavariable binds (`Language::extract_meta_var`, after
`pre_process_pattern` turns `$X` into the expando form).

`PatternBuilder::build` accepts any `Doc`, so the pattern becomes a `BladeDoc` whose kinds are
numbered as in Blade's tapes.

`BladeLang` writes a metavariable `$X` as `µX` (`pre_process_pattern`, expando `µ`), so a pattern's
PHP variables (`$x`) stay PHP. Forte reads `µX` in an echo or arguments as opaque PHP bytes, the
leaf's whole text.

</details>

## Bounds

A step budget caps each parse and nothing recurses, so no template hangs the parser or overflows
the stack.

The budget is 256 steps per template byte plus 2^20 (`Budget::for_template`). Exhausting it is
`ParseError::Budget`, an unknown parse.

<details>
<summary>How steps are counted, and observed use</summary>

Forte bounds nesting (the three depth limits), not work: lookahead scans, directive-token-index
searches, terminator searches and recovery retries revisit input, some quadratically in PHP.

- **Lexer:** one step per main-loop iteration, plus the distance scanned on lookaheads that rewind
  (JSX attribute values, balanced JavaScript, TSX generics, bogus comments).
- **Tree builder:** one step per token and per iteration of every search.

Observed maxima: 1.4 steps per byte for a whole parse on the corpora; 2.7 (lexer) and 3.7 (tree
builder) on the suite's templates and their prefixes.

The budget ends Forte's quadratic cases: 20,000 unclosed `@section('a')` (260 KB, each searching to
the end for its `@endsection`), and 200 KB of `<? ` (bogus-comment retries) or of `<a <A x` (TSX
retries).

</details>

<details>
<summary>Recursion, sub-templates and multi-byte text</summary>

Forte's two recursions (template literals in `ScanPrimitives`, and `createConditionalComment`
calling `processToken`) run on explicit stacks, so nesting costs heap, not thread stack.
`findLineEnding` and quoted-attribute-value scans, which rescan to the end of the source in PHP, are
single forward passes that find the same positions.

The view's sub-templates (text Forte left unread, [above](#embedded-php-for-the-view)) nest at most
4 deep (real templates need 2); deeper ones are read as text, keeping work linear and recursion
shallow. The view polls the tape's budget before each one.

Forte lexes bytes and can end a text node inside a multi-byte character (`<a é@b)`); the view widens
every node to whole characters before slicing.

</details>

<details>
<summary>Robustness tests</summary>

`forte_survives_random_and_truncated_templates` parses, with both registries:

- 1,500 seeded random byte strings;
- 3,000 seeded random assemblies of Blade, HTML, PHP and JavaScript fragments;
- about 150 prefixes of each fixture.

All 8,036 inputs (16,072 parses) stay within budget, every token and node inside the template and
the token array. It also runs 13 inputs nesting 20,000 deep (elements, components, conditions,
loops, sections, `@php`, echoes, open parentheses, template literals in attributes and scripts,
conditional comments). Each ends as Forte 1.1.0 ends it (its element-depth exception, or a tree),
except the unclosed sections above.

`tree::tests::trees_views_and_tapes_survive_random_and_truncated_templates` runs the same inputs,
plus 20,000 nested `<script>`s holding a component tag or a `@php` block, through the tree, view and
tape. The tree keeps its invariants, its syntax errors stay inside the template, and the tape
encodes and round-trips.

</details>

## License

The port is a derived work of MIT-licensed Forte, recorded in the notices.

Forte is MIT (Copyright (c) Johnathon Koster). `THIRD_PARTY_NOTICES` lists its license for the port
(`native/src/code_rule_dsl/blade/forte/`, a derived work) and for the test material
(`fixtures/blade/licenses/forte/`: Forte's license verbatim and `UPSTREAM.md`, naming the suite's
templates and the directive JSON copied). The module doc of `forte/mod.rs` states the derivation.

## Oracle

A PHP script records real Forte's output for the port to match.

`fixtures/blade/blade/forte/generate.php` runs Forte 1.1.0 with laravel/framework 13.34.0 on PHP
8.5.11. Per template and registry it records tokens, flat nodes, lexer errors and AST reads, or the
depth exception.

<details>
<summary>Recorded files and flags</summary>

Forte and laravel/framework come from one Composer project holding both. A record holds the flat
nodes with their extras and what the AST reads above return; `forte_tree.php` documents it.

- **`suite.expected.json`**: every template Forte's test suite lexes (9,723 cases;
  `record_suite.php` hooks the lexer during Forte's PHPUnit run), with its registry and Forte's tree
  as an FNV digest (prefix families: one digest per prefix). 11 MB of plain JSON, one case per line,
  mostly templates. `trees_are_well_formed` reads a whole template's tree from the port's parse that
  hashes to it.
- **`fixtures.expected.json`**: the Blade view fixtures and tree-sitter grammar-gap repros, both
  registries, and the default registry's state.
- **`corpus/*.expected.json`**: 25 templates of each pinned corpus (BookStack f585a7d910, Filament
  ca4f55bb5e, Bagisto 635cab68b3, laravel/framework e5f3d6265f), both registries.
- **`--corpus OUT`**: every template of the four corpora (1,203), both registries, for the ignored
  `benchmark_forte_corpus`.
- **`--full OUT`**: the suite with every tree spelled out (`generate.php --full`).

`t1_reference.php` counts, per corpus, the templates each reference parser reports an error in.

</details>

## Parity

The port's output is Forte's, byte for byte, on every recorded template: tokens, flat nodes with
extras, lexer errors, AST reads and depth exceptions.

| Oracle | Trees | Differences |
|---|---:|---:|
| Forte's test suite (`suite.expected.json`, prefixes included; FNV digests) | 83,698 | 0 |
| the same, every tree spelled out (`generate.php --full`) | 83,698 | 0 |
| Blade fixtures and grammar-gap repros, both registries | 64 | 0 |
| corpus samples, 25 templates × 2 registries per corpus | 4 × 50 | 0 |
| every corpus template × 2 registries (`benchmark_forte_corpus`): BookStack / Filament / Bagisto / laravel | 650 / 394 / 1,108 / 254 | 0 |

<details>
<summary>Per-stage and depth-limit checks</summary>

Each stage alone (`FORTE_PARITY=lexer|parser|ast`) also shows 0 differences over the full suite and
corpus dumps: the lexer from the template, the tree builder from Forte's tokens, the AST reads from
Forte's tokens and nodes. The three depth limits agree with Forte on 153 template × limit × registry
combinations, from 1/1/1 to 100,000/256/256.

</details>

## T1: templates with an error

The port flags exactly Forte's templates, far fewer than tree-sitter-blade did.

| Corpus | Templates | Port | Forte | Forte, accept-all | BladeCompiler + `php -l` | tree-sitter-blade |
|---|---:|---:|---:|---:|---:|---:|
| BookStack f585a7d910 | 325 | 0 | 0 | 0 | 0 | 218 |
| Filament ca4f55bb5e | 197 | 0 | 0 | 0 | 0 | 25 |
| Bagisto 635cab68b3 | 554 | 18 | 18 | 18 | 0 | 258 |
| laravel/framework e5f3d6265f | 127 | 0 | 0 | 0 | 0 | 21 |

Bagisto's 18 are one Forte false positive, kept for parity: a `@php … @endphp` inside a
`<script type="text/x-template">`.

<details>
<summary>What counts as an error, and why Forte flags the Bagisto templates</summary>

- **Forte:** a lexer error or a depth exception.
- **BladeCompiler** (illuminate/view 13.34.0, core directives only, `withoutComponentTags()`): it
  throws, or `php -l` (PHP 8.5.11) rejects its output.
- **tree-sitter-blade** 0.12.2 at b5291d1: an `ERROR` or `MISSING` node.

The `<script type="text/x-template">` content is raw text. Forte's lexer emits the `@php` opener and
keeps the raw text up to `</script>`, `@endphp` included, as text. It then reads the rest of the
file as the PHP block and logs `UnexpectedEof` at the end. BladeCompiler compiles `@php` before any
HTML and accepts them.

This is not a port bug. The view reads these templates as Blade does (above); a rule over the
template's tree misses the nodes Forte swallowed into the block.

</details>

## Performance

The port parses 4 to 24 times as fast as tree-sitter-blade, using a fraction of its memory.

| Parser | Speed vs tree-sitter-blade | Peak memory vs tree-sitter-blade |
|---|---|---|
| `port`: lexer and tree builder (Forte's flat arrays) | 12 to 24 times | 26 to 39 % |
| `port + tree`: plus Rerail's tree (`tree::parse_tree`), tree-sitter's counterpart | 4 to 7 times | 45 to 65 % |

Measured by `benchmark_forte_throughput` (release build, every template of a corpus, one process per
parser and corpus) on the four pinned corpora: throughput and the growth of peak resident memory
while parsing, one tree alive at a time.

## Wiring

The port plugs into the kind catalog, tapes, the language registry, build identity and the rules.

- **Kinds and fields:** `language::KindCatalog` numbers Blade's kinds by `tree::KIND_NAMES` and
  fields by `tree::FIELD_NAMES`.
- **Tapes** (format 7): `TapeImage::encode_blade` encodes the tree in preorder.
- **The view's tape:** the view is a derived document with its own PHP tape.
- **Registry:** `languages/blade.json` names `"parser": "forte"`, `"grammar": "native:forte"`.
- **Build identity:** the grammar build id hashes the port's sources, so stale tapes are not reused.
- **Rules:** the Blade pack's rules name Forte's kinds.

<details>
<summary>Details per integration point</summary>

- **Kinds and fields.** Kind ids start at 1; `ERROR` is tree-sitter's code (`u16::MAX`). A Blade
  clause names only the template's kinds. The compiled view is a PHP document of its own: a clause
  reads it with `ast.embedded: compiled-view`, a template rule reaches it with `embedded`.
- **Tapes.** `TapeImage::encode_blade` parses with `tree::parse_tree`, reads the template as Blade
  does (`View::read`), and encodes the tree (`encode_blade_tree`). Syntax errors, as Blade reads
  them, are `ERROR` leaves where they lie, as in tree-sitter's tapes. They are Forte's, except in
  arguments Forte read after a line break, which Blade reads as text and whose errors are that
  text's.
- **The view's tape.** The derived document (`tape::DerivedImage`, a generated one) holds the view's
  PHP text, a source map to the template's bytes, its own PHP tape, and the template errors it reads
  through (`host_errors`). The template's tape holds only its own tree; the store keeps the view's
  tape as its own row (`file_documents`). Both keep tree-sitter's invariants. `TapeImage::parse`
  picks tree-sitter or Forte by dialect for every caller.
- **Registry.** `native:` is a grammar Rerail implements, with no tree-sitter language.
  `code-rule-dsl/capabilities.json` names `forte@1.1.0`. Tag 8 stays Blade's.
- **Build identity.** `native/grammars/tree-sitter-blade` and its build are gone. The grammar build
  id hashes every non-test source under `native/src/code_rule_dsl/blade/` (the port, its directive
  JSON, the view), so no tape from tree-sitter-blade or another revision of the port or view is
  reused.
- **Rules.** The Blade pack's rules (`packs/blade/rules/`, candidates in
  `packs/blade/candidates/rules/`) name Forte's kinds: `directive_name` (`@dd`, `@ray`),
  `directive_block` with its `start` field (`@forelse`), `echo`/`raw_echo`,
  `component`/`slot`/`element_name`/`attribute_value`. A rule over the PHP a template runs reads the
  view (`ast.embedded: compiled-view`).

</details>

### Results

On the pinned corpora Forte reads more of what Blade copies than tree-sitter-blade did, and the Blade
pack finds more with no false positive.

<details>
<summary>Per-corpus results against tree-sitter-blade</summary>

Forte against tree-sitter-blade on the same rules and oracles, over the pinned corpora (the Bagisto,
BookStack and Filament checkouts and laravel's 127 templates). Syntax errors are in
[T1](#t1-templates-with-an-error).

| Check | BookStack | Filament | Bagisto | laravel |
|---|---|---|---|---|
| tape ⇄ tree parity (`benchmark_blade_worktree_oracle_parity`): templates, differences | 325, 0 | 197, 0 | 554, 0 | 127, 0 |
| bytes Blade copies the view reads (`benchmark_blade_region_parity`), tree-sitter → Forte | 146,417 → 146,469 of 146,469 | 181,426 → 181,895 of 181,895 | 655,190 → 655,672 of 655,672 | |
| region disagreements, tree-sitter → Forte | 6 → 0 | 51 → 0 | 76 → 36 | |
| reads decided against PHPStan (`benchmark_blade_definedness_corpus`), tree-sitter → Forte | 569 → 656 | 2,594 → 2,623 | 2,056 → 2,609 | |
| contradictions with PHPStan, tree-sitter → Forte | 0 → 0 | 13 → 9 | 1 → 1 | |

Bagisto's 36 remaining region disagreements are one class tree-sitter-blade had too: `@{{ … }}`
inside a directive's string argument, which Blade copies without its `@`. The contradictions are a
subset of tree-sitter-blade's.

</details>

<details>
<summary>Blade pack conformance</summary>

The Blade pack's conformance (`npm run conformance:languages -- --pack blade`) passes on the
fixtures and the three corpora with no false positive. Seven fixture divergences tree-sitter-blade
caused are gone, and true positives rise:

- 183 → 557 (BookStack, `tlint-space-after-blade-directives`);
- 57 → 110 (BookStack, `tlint-spaces-around-blade-render-content`);
- 62 → 110 (Bagisto, `tlint-no-json-directive`).

The remaining false negatives are in the 18 Bagisto templates above.

</details>
