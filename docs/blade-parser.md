# Rerail's Blade parser: a port of Forte

Rerail reads Blade templates with a Rust port of Forte 1.1.0 (github.com/fortephp/forte, tag
`v1.1.0`, commit 9c9597d0b7, MIT, by John Koster), the fault-tolerant Blade parser Blade's own
tooling reads templates with. It replaced tree-sitter-blade 0.12.2 (b5291d1), which failed on
real Blade: the templates it parsed with an `ERROR` were 218 of 325 in BookStack, 25 of 197 in
Filament, 258 of 554 in Bagisto and 21 of 127 in laravel/framework (minimal repros of each error
class: `fixtures/blade/blade/grammar-gaps/repros/`, every one of which Forte reads without an error).
This document says what is ported and why, the tree Rerail reads, how rules and the view use it,
how parity with Forte is decided, and how the port is wired into the engine.

## What is ported

`Document::parse($template, $options)` (`Ast/Document/Document.php:164-213`) runs two stages and
keeps their arrays: the lexer (`Lexer::tokenizeCompact`: token records and `LexerError`s) and the
tree builder (`TreeBuilder::buildCompact`: flat node records `{kind, parent, firstChild,
lastChild, nextSibling, tokenStart, tokenCount, genericOffset, data}`, plus `name`, `args` and
`role` extras on directive records). Between them, when the registry accepts every directive, it
trains the registry on the tokens (`trainCompact`). Everything else Forte has is an object API
over these arrays, created lazily. The port is the part that decides structure, under
`native/src/code_rule_dsl/blade/forte/`:

| Forte | Port | What it decides |
|---|---|---|
| `Lexer/Lexer.php`, `ConstructDetector.php`, `Construct.php`, `Concerns/*` (176 KB) | `lexer/` | tokens and lexer errors |
| `Parser/TreeBuilder.php`, `ConstructScanner.php`, `Concerns/*`, `OptionalTags/*` | `parser/` | the flat node array |
| `Parser/Directives/{DirectiveTokenIndex,DirectiveHelper,ArgumentScanner}.php` | `directives/{token_index,helper,arguments}.rs` | pairing, branches, terminators |
| `Parser/Directives/Directives.php` + `resources/directives/*.json` (21 files) | `directives/mod.rs` | which `@word` is a directive, its kind and pairs |
| `Support/ScanPrimitives.php` and the string helpers the two stages call | `scan.rs` | quoted strings, PHP/JS skipping |
| `Ast/Elements/VoidElements.php` | `parser/` | void elements |
| `ElementNode`, `ComponentManager`, `Attributes`, `Attribute`, `DirectiveBlockNode`, `EchoNode`, `Php*Node` reads | `ast.rs` | tag names, component/slot names, attributes, a block's start/end directive, echo/PHP content |

Each PHP method is one Rust function with its name in snake case and a doc comment naming it, as
`literal/phpdoc/{lexer,parser}.rs` port phpstan/phpdoc-parser. PHP's string semantics are
reproduced where Forte relies on them: `$s[$i]` past the end is `""`, `substr` clamps,
`strtolower` and `ctype_*` are ASCII (`ctype_space` includes `\v` and `\f`), `trim`'s default
characters. The one regular expression on the parse path
(`ProcessesDirectives::recoverRepeatedUnterminatedPairedDirective`, a lookbehind-anchored
`preg_match` for a compact terminator inside text) is a byte scan with the same semantics; the
lexer has none (`LineIndex::build`'s `preg_match_all` is only for line numbers).

Not ported, each checked against Forte's code:

- The AST object API (`Ast/`, 270 KB, apart from the reads above): lazy wrappers over the flat
  arrays (`ManagesAst::getNode`), traversal, rendering, offsets. Rerail reads the arrays through
  its own tree (below); the reads the oracle records are ported so the tree's names, attributes
  and block ends are Forte's.
- `Rewriting/` (114 KB): visitors and rewrite passes over a parsed document; `Document::parse`
  never calls it.
- `Querying/` (25 KB) and `DomMapper`: XPath and a DOM projection built from `Document::children()`
  after the parse. Rerail's rules are its query language.
- `Extensions/` (28 KB): `Document::parse` creates an extension registry only when
  `ParserOptions::hasExtensions()`; with none the lexer's and tree builder's extension hooks are
  no-ops. Each call site carries a one-line comment where the PHP calls a hook.
- `Enclaves/` (67 KB): path-matched configuration and rewrite orchestration
  (`EnclavesManager::getParserOptionsForPath`, `Precompiler`). An enclave contributes extensions
  only (`Enclave::getParserOptions`), never a per-path directive registry, so with no extensions
  it changes nothing a parse produces.
- `Diagnostics/` (11 KB): value objects adapting what the parse kept. `Document::diagnostics()`
  converts the lexer's errors; `addParseError` has no caller in Forte, so the tree builder
  records no diagnostics. The port returns the lexer errors and the exception the depth limits
  throw.
- `Components/ComponentManager` beyond `isComponent`/`parseComponentName` with the default
  prefixes (`x-`, `livewire:`, `flux:`), `Facades/`, `ServiceProvider`, `Internal/`: Laravel
  wiring.

## Forte's tree

Tokens carry byte spans and partition the template: every byte is in exactly one token, so the
flat arrays are lossless. A node owns `tokenCount` tokens from `tokenStart`; its children own
sub-ranges, and what no child owns (delimiters, whitespace inside a tag, a directive's name and
arguments) is the node's own. Text, whitespace text included, is `Text` nodes; whitespace
between attributes is `AttributeWhitespace` nodes. Node kinds (`Parser/NodeKind.php`) are
`Root`, `Element`, `Text`, `Echo`, `RawEcho`, `TripleEcho`, `Directive`, `DirectiveBlock`,
`Verbatim`, `PhpBlock`, `PhpTag`, `Comment`, `BogusComment`, `ConditionalComment`,
`BladeComment`, `Doctype`, `Cdata`, `Decl`, `Attribute`, `JsxAttribute`, `UnpairedClosingTag`,
`NonOutput`, `ElementName`, `ClosingElementName`, `AttributeWhitespace`, `AttributeName`,
`AttributeValue`, `ProcessingInstruction` (Forte also declares `Fragment`, which its tree
builder never creates; the port leaves it out, as it leaves out the lexer states and error
reasons Forte declares and never logs, and the registry queries nothing in the parse calls).

A paired directive is a `DirectiveBlock` holding its opening `Directive`, each branch directive
(`@else`, `@elseif`, `@case`, `@empty`) and the closing directive as children; a branch
directive's content is its children, outside its own tokens. The block's token range starts as
the opener's and grows to the closing directive when it closes
(`ProcessesDirectives.php:681-693`). An unclosed block keeps the opener's range while its
children run on: Rerail's spans are therefore the union of a node's tokens and its children's.

Recovery is Forte's: an unclosed element is closed by its parent's end tag, a stray closing tag
is an `UnpairedClosingTag`, an unknown `@word` is text (default registry) or a directive
(accept-all). The tree builder throws a `RuntimeException` past 512 nested elements, 256
directives or 256 conditions (`TreeBuilder.php:35-42`, messages `Maximum element nesting depth
(512) exceeded.` and the like); the port returns that message as `ParseError::Depth`.

## Rerail's tree

`forte/tree.rs` reads a parse as ast-grep-core 0.45.3's `Doc` (`BladeDoc`, nodes `BladeNode`),
as `literal/mod.rs` reads its sub-trees (`SubTree`/`SubDoc`/`SubNode`), with node ids that never
collide with tapes' or tree-sitter's (`ID_FLAG`, a tree ordinal, the preorder index).
`tree::parse_tree`, which tapes, patterns and the view parse with, uses Forte's default directives
and accepts every directive (`ParserOptions::acceptAllDirectives`): an application's or a
package's `@name` (`@ray(...)`, `@livewire(...)`, a `Blade::if` pair) is a `directive` node a
rule can name, as Blade's tooling (tlint, blade-parser) reads it, not text.

- One node per Forte flat node, in preorder of Forte's links; the root is `document`, spanning
  the template.
- Kinds are the AST class `Document::createBuiltInWrapper` wraps the node in, in snake case
  without `Node`: `text`, `element`, `component`, `slot` (`ComponentManager` decides), `directive`,
  `directive_block`, `comment`, `conditional_comment`, `bogus_comment`, `blade_comment`,
  `php_block`, `php_tag`, `verbatim`, `doctype`, `cdata`, `xml_declaration`,
  `processing_instruction`, `escape` (`NonOutput`), `stray_closing_tag` (`UnpairedClosingTag`).
  Where one class serves several node kinds the kind's name is used: `echo`, `raw_echo`,
  `triple_echo`; `element_name`, `closing_element_name`; `attribute`, `jsx_attribute`,
  `attribute_whitespace`, `attribute_name`, `attribute_value`.
- A node's own tokens become leaves in text order, so every byte of the template is in at most
  one leaf: delimiters are anonymous leaves whose kind is their text (`<`, `>`, `/`, `=`, quotes,
  `<!--`, `-->`, `<!DOCTYPE`, `@`, `{{`, `}}`, `{!!`, `!!}`, `{{{`, `}}}`, `{{--`, `--}}`, `(`,
  `)`, `@verbatim`, `@endverbatim`, `@php`, `@endphp`, `<?php`, `<?=`, `<?`, `?>`,
  `<![CDATA[`, `]]>`, `<?xml`), as tree-sitter grammars spell punctuation, so a pattern compares
  them by text. Meaningful tokens are named leaves: `php` (the trimmed PHP of an echo, a
  directive's arguments between `(` and `)`, a `@php` block or a `<?php` tag), `directive_name`,
  `tag_name`, `attribute_name`, `attribute_value`, `bound_attribute_name` (`:x`),
  `escaped_attribute_name` (`::x`), `shorthand_attribute_name` (`:$x`), `comment_text`, `text`,
  `doctype_value`, `conditional_comment_start`, `conditional_comment_end`,
  `processing_instruction_start`, `jsx_attribute_value`, `jsx_shorthand_attribute`,
  `tsx_generic_type`. `text` and `attribute_whitespace` nodes are leaves themselves. Whitespace
  tokens, zero-width tokens (`SyntheticClose`) and the whitespace trimmed off `php` and
  `comment_text` are gaps between leaves, as in tree-sitter trees.
- Fields: `element`/`component`/`slot`: `name`, `closing_name`; `attribute`, `jsx_attribute`:
  `name`, `value`; `directive`: `name`, `arguments`; `echo`, `raw_echo`, `triple_echo`:
  `expression`; `php_block`, `php_tag`: `code`; `directive_block`: `start`, `end`
  (`DirectiveBlockNode::startDirective`/`endDirective`).
- No node is an error node: Forte recovers without them. The lexer's errors (zero-width at their
  offset) and a `ParseError` (the whole template, under a lone `document`) are a list of byte
  ranges on the tree, as a tape's syntax-error ranges are.

Forte's links do not always nest by span. In 11 of the 86,361 recorded trees (suite dump and
corpus dump) a directive block left open where an element closes counts that element's closing
tag and a later `@endX` among its tokens, so it overlaps a sibling (`<ul><li>@if($c)Active@endif<li>…`,
`<x-card><x-slot:content>@if($show)Content@endif</x-slot></x-card>`); in 90 a node has neither
tokens nor children (an `attribute_name` with no tokens in `<div =value>`). The tree resolves
both by rule: a token belongs to the node with the fewest tokens whose range holds it (the
deeper on a tie; directive blocks and the root claim none, their span coming from their
children); a node spans the tokens it owns and its children; a node with neither is
zero-width where its previous sibling ends (or at its parent's start). Under these rules every
recorded tree satisfies the invariants a tree-sitter tree does (children inside their parent,
siblings in order without overlap, every non-whitespace byte in exactly one leaf), which
`tree::tests::trees_are_well_formed` checks over the suite, fixtures and corpora.

The module doc of `tree.rs` is the authoritative list of kinds, leaves and fields.

### Embedded PHP for the view

The view (`blade/view.rs`) copies the PHP a template embeds byte for byte. On the port every such
span is a `php` leaf with its exact byte range, under a node that says what it is:

| Construct | Node | PHP span |
|---|---|---|
| `{{ e }}`, `{!! e !!}`, `{{{ e }}}` | `echo`, `raw_echo`, `triple_echo` | `expression` field |
| `@name(args)` | `directive` (`name` field) | inside the parentheses, to the `)` Blade's compiler balances (`view::balanced`); only where spaces or tabs alone separate the name and the `(`, as Blade's `compileStatements` reads them. Forte also skips line breaks there: the view reads such a `(…)` as template text, and the tape reports that text's syntax errors in place of Forte's errors there (`view::Reading::syntax_errors`) |
| `@php … @endphp` | `php_block` | `code` field |
| `<?php … ?>`, `<?= … ?>` | `php_tag` | everything between the tags; `<?=`'s `code` field |
| `:attr="e"`, `:$name` on `component`, `slot` | `attribute` | `value` without its quotes, or the name after `:` |
| `@if … @elseif … @else … @endif`, loops, `@switch`, `@once`, `@pushIf`, `@push` | `directive_block` with `start`, branch `directive`s, `end`; a directive's body is its children | each directive's arguments |

Forte pairs a block's directives, so the view reads a structure from its block. Where Forte
leaves a control structure unpaired (directives side by side in an attribute's value,
`class="a @if($x) b @endif"`), the view pairs it among its siblings as Blade's compiler does
(`view::flat_structures`); one it cannot close is not read. Forte lexes Blade inside `<script>`
and `<style>` (echoes, comments, directives are nodes) but not their markup: where their text
holds a component tag (`<script type="text/x-template">`), the view reads it as a template of its
own (`Emitter::template`), as Blade's compiler reads any text. It does the same where Forte's
recovery swallowed template text: a `@php` in a `<script>`'s raw text runs, for Forte, to the
end of the template, and for Blade to its first `@endphp`; a directive's arguments run, for
Forte, past the `)` Blade's compiler ends them at (`@js("{$map["("]}")`). An escaped echo Forte
reads as an echo in raw text (`@{{ … }}`) is text, its directives compiled.

### Patterns

ast-grep builds a pattern by parsing its text into the language's `Doc`
(`Pattern::try_new` → `Language::build_pattern` → `PatternBuilder::build(|src| Doc)`), then
matches node against node; a leaf whose text is a metavariable (`Language::extract_meta_var`,
after `pre_process_pattern` turns `$X` into the expando form) binds. `PatternBuilder::build`
accepts any `Doc`, so a Blade pattern is parsed with `tree::parse_tree` into a `BladeDoc`, whose
kinds are numbered as Blade's tapes number them: `{{ $X }}` is an `echo` whose `php` leaf is the
metavariable, `@include($V)` a `directive` whose arguments leaf is. `BladeLang` writes a
metavariable `$X` as `µX` (`pre_process_pattern`, expando `µ`), so a pattern's PHP variables
(`$x`) stay PHP; Forte reads `µX` in an echo or in arguments as opaque PHP bytes, the leaf's
whole text. A pattern Blade reads as text alone (`sprintf($F, $$$A)`) is PHP and matches the
embedded program (`language::rule_pattern`).

## Bounds

Forte bounds nesting (the three depth limits) but not work: lookahead scans, searches over the
directive token index, terminator searches and recovery retries revisit input, and some are
quadratic in PHP. The port spends steps from a budget of 256 per template byte plus 2^20
(`Budget::for_template`); exhausting it is `ParseError::Budget`, an unknown parse. The lexer
spends one step per iteration of its main loop and, on the lookaheads that rewind (JSX
attribute values, balanced JavaScript, TSX generics, bogus comments), the distance scanned; the
tree builder one per token and one per iteration of every search. On the corpora a template
spends at most 1.4 steps per byte (the whole parse); on the suite's templates and their
prefixes at most 2.7 in the lexer and 3.7 in the tree builder. Where Forte's work is
quadratic the budget ends it: 20,000 unclosed `@section('a')` (260 KB, each searching to the
end for its `@endsection`; 5 s in PHP), 200 KB of `<? ` (bogus-comment retries, 19 ms to the
budget) or of `<a <A x` (TSX retries, 41 ms). Nothing recurses: Forte's two recursions
(template literals in `ScanPrimitives`, and `createConditionalComment` calling `processToken`)
run on explicit stacks, so nesting costs heap, not the thread's stack; the scans of
`findLineEnding` and of quoted attribute values, which rescan to the end of the source in PHP,
are single forward passes that find the same positions.

`forte_survives_random_and_truncated_templates` parses, with both registries, 1,500 seeded
random byte strings, 3,000 seeded random assemblies of Blade, HTML, PHP and JavaScript fragments
and about 150 prefixes of each fixture (8,036 inputs, 16,072 parses, all within budget, every
token and node inside the template and the token array), and 13 inputs nesting 20,000 deep
(elements, components, conditions, loops, sections, `@php`, echoes, open parentheses, template
literals in attributes and scripts, conditional comments), each of which ends as Forte 1.1.0
ends it (its element-depth exception, or a tree), but for the unclosed sections above.

The view re-parses what Forte left unread as a template of its own (text in `<script>` and
`<style>`, what Forte's recovery swallowed into a `@php` block or a directive's arguments). It
nests such sub-templates at most 4 deep (real templates need 2) and reads deeper ones as text, so
its work stays linear and its recursion shallow; it polls the tape's budget before each one.
Forte lexes bytes and can end a text node inside a multi-byte character (`<a é@b)`); the view
widens every node to whole characters before it slices the template.
`tree::tests::trees_views_and_tapes_survive_random_and_truncated_templates` runs the same inputs,
and 20,000 nested `<script>`s holding a component tag or a `@php` block, through the tree, the
view and the tape: the tree keeps its invariants, its syntax errors stay inside the template, and
the tape encodes and round-trips.

## License

Forte is MIT (Copyright (c) Johnathon Koster). `THIRD_PARTY_NOTICES` lists Forte's
license for the port (`native/src/code_rule_dsl/blade/forte/`, a derived work) and for the test
material (`fixtures/blade/licenses/forte/`: Forte's license verbatim and `UPSTREAM.md`
naming the suite's templates and the directive JSON copied). The module doc of `forte/mod.rs`
states the derivation.

## Oracle

`fixtures/blade/blade/forte/generate.php` runs Forte 1.1.0 (with laravel/framework 13.34.0, PHP
8.5.11, from a Composer project holding both) and records, per template and registry, the
tokens, the flat nodes with their extras, the lexer errors, what the AST reads above return, or
the depth exception (`forte_tree.php` documents a record):

- `suite.expected.json`: every template Forte's own test suite lexes (9,723 cases,
  `record_suite.php` hooks the lexer during Forte's PHPUnit run), with the registry it was lexed
  with and Forte's tree as an FNV digest (prefix families: one digest per prefix); 11 MB of plain
  JSON, one case per line, most of it the templates. `generate.php --full` spells every tree out.
  `trees_are_well_formed` reads a whole template's tree from the port's parse that hashes to it.
- `fixtures.expected.json`: the Blade view fixtures and the tree-sitter grammar-gap repros, both
  registries, and the default registry's state.
- `corpus/*.expected.json`: 25 templates of each pinned corpus (BookStack f585a7d910, Filament
  ca4f55bb5e, Bagisto 635cab68b3, laravel/framework e5f3d6265f), both registries.
- `--corpus OUT`: every template of the four corpora (1,203), both registries, for the ignored
  `benchmark_forte_corpus`; `--full OUT`: the suite with every tree spelled out.

`t1_reference.php` counts, per corpus, the templates each reference parser reports an error in.

## Parity

The port's output is Forte's, byte for byte, on every template the oracle records: tokens,
flat nodes with their extras, lexer errors, the AST reads and the depth exceptions.

| Oracle | Trees | Differences |
|---|---:|---:|
| Forte's test suite (`suite.expected.json`, prefixes included; FNV digests) | 83,698 | 0 |
| the same, every tree spelled out (`generate.php --full`) | 83,698 | 0 |
| Blade fixtures and grammar-gap repros, both registries | 64 | 0 |
| corpus samples, 25 templates × 2 registries per corpus | 4 × 50 | 0 |
| every corpus template × 2 registries (`benchmark_forte_corpus`): BookStack / Filament / Bagisto / laravel | 650 / 394 / 1,108 / 254 | 0 |

Each stage was also checked alone (`FORTE_PARITY=lexer|parser|ast`) over the full suite dump
and the corpus dump, with 0 differences: the lexer from the template, the tree builder from
Forte's tokens, the AST reads from Forte's tokens and nodes. The tree builder's three depth
limits were checked against Forte with 153 template × limit × registry combinations, from
1/1/1 to 100,000/256/256, all agreeing.

## T1: templates with an error

Per pinned corpus, the templates each parser reports an error in (Forte: a lexer error or a
depth exception; BladeCompiler: illuminate/view 13.34.0 with core directives only and
`withoutComponentTags()` throws, or `php -l` (PHP 8.5.11) rejects its output; tree-sitter-blade
0.12.2 at b5291d1: an `ERROR` or `MISSING` node).

| Corpus | Templates | Port | Forte | Forte, accept-all | BladeCompiler + `php -l` | tree-sitter-blade |
|---|---:|---:|---:|---:|---:|---:|
| BookStack f585a7d910 | 325 | 0 | 0 | 0 | 0 | 218 |
| Filament ca4f55bb5e | 197 | 0 | 0 | 0 | 0 | 25 |
| Bagisto 635cab68b3 | 554 | 18 | 18 | 18 | 0 | 258 |
| laravel/framework e5f3d6265f | 127 | 0 | 0 | 0 | 0 | 21 |

The port reports exactly Forte's templates. All 18 Bagisto templates are one Forte class: a
`@php … @endphp` inside a `<script type="text/x-template">` (raw text). Forte's lexer emits the
`@php` opener, keeps the raw text up to `</script>` (the `@endphp` included) as text, then reads
the rest of the file as the PHP block, so it logs `UnexpectedEof` at the end of the template;
BladeCompiler, which compiles `@php` before any HTML, accepts them. This is a Forte false
positive the port keeps (parity), not a port bug. The view reads those templates as Blade does
(above); a rule over the template's tree does not see the nodes Forte swallowed into the block.

## Performance

Release build, Apple M5 Max, macOS 26.6, median of 10 runs over every template of a corpus,
one process per parser and corpus (`benchmark_forte_throughput`). Peak memory is the growth of
the process's peak resident set while parsing, one tree alive at a time, over its peak after
reading the templates. `port` is the lexer and tree builder (Forte's flat arrays); `port + tree`
adds Rerail's tree over them (`tree::parse_tree`), the counterpart of tree-sitter's tree.
Ten other agents shared the machine, so the absolute numbers are noisy; the ratios held across
two runs.

| Corpus | Bytes | port MB/s | port + tree MB/s | tree-sitter-blade MB/s | port peak | port + tree peak | tree-sitter peak |
|---|---:|---:|---:|---:|---:|---:|---:|
| BookStack | 506,131 | 224 | 77 | 18.6 | 1.2 MB | 1.7 MB | 3.0 MB |
| Filament | 562,189 | 385 | 147 | 23.2 | 1.7 MB | 3.1 MB | 4.8 MB |
| Bagisto | 5,078,393 | 694 | 260 | 38.3 | 1.6 MB | 3.1 MB | 6.1 MB |
| laravel/framework | 99,180 | 335 | 103 | 13.9 | 1.0 MB | 1.3 MB | 2.9 MB |

The port parses 12 to 24 times as fast as tree-sitter-blade, and 4 to 7 times with Rerail's tree
built; its peak is 26 to 39 % of tree-sitter's, 45 to 65 % with the tree.

## Wiring

- **Kinds and fields.** `language::KindCatalog` numbers Blade's own kinds by
  `tree::KIND_NAMES` (ids from 1; 0 is the tape's error code) and PHP's after them, as before.
  Blade's fields are PHP's, then the tree's that PHP lacks (`closing_name`, `expression`,
  `code`, `start`, `end`): a field both name has one id. `kind: comment` names every comment
  kind of the template (`comment`, `blade_comment`, `conditional_comment`, `bogus_comment`) and
  the embedded PHP's.
- **Tapes** (format 4). `TapeImage::encode_template` parses with `tree::parse_tree`, encodes the
  tree in preorder (`encode_blade_tree`), then the view's program under the root as before.
  The tree has no error nodes, so the tape keeps its syntax errors as byte ranges beside the
  records, which `TapeDoc::syntax_error_ranges` returns after the embedded program's ERROR and
  MISSING records. Those are Forte's, except in arguments Forte read after a line break, which
  Blade reads as text: there they are that text's, as a template of its own. `TapeImage::parse`
  picks tree-sitter or Forte by dialect for every caller.
- **Registry.** `languages/blade.json` names `"parser": "forte"`, `"grammar": "native:forte"`
  (`native:` is a grammar Rerail implements, which has no tree-sitter language);
  `code-rule-dsl/capabilities.json` names `forte@1.1.0`. Tag 8 stays Blade's.
- **Build identity.** `native/grammars/tree-sitter-blade` and its build are gone. The grammar
  build id hashes every non-test source under `native/src/code_rule_dsl/blade/` (the port, its
  directive JSON, the view), so no tape parsed by tree-sitter-blade, or by another revision of
  the port or the view, is reused.
- **Rules.** The Blade pack's rules name Forte's kinds: `directive_name` (`@dd`, `@ray`),
  `directive_block` with its `start` field (`@forelse`), `echo`/`raw_echo`,
  `component`/`slot`/`element_name`/`attribute_value`.

### Results

Reports over the pinned corpora (the Bagisto, BookStack and Filament checkouts and laravel's 127
templates), Forte against tree-sitter-blade on the same rules and oracles.

| Check | BookStack | Filament | Bagisto | laravel |
|---|---|---|---|---|
| templates with a syntax error (T1), tree-sitter → Forte | 218 → 0 | 25 → 0 | 258 → 18 | 21 → 0 |
| tape ⇄ tree parity (`benchmark_blade_worktree_oracle_parity`): templates, differences | 325, 0 | 197, 0 | 554, 0 | 127, 0 |
| bytes Blade copies the view reads (`benchmark_blade_region_parity`), tree-sitter → Forte | 146,417 → 146,469 of 146,469 | 181,426 → 181,895 of 181,895 | 655,190 → 655,672 of 655,672 | |
| region disagreements, tree-sitter → Forte | 6 → 0 | 51 → 0 | 76 → 36 | |
| reads decided against PHPStan (`benchmark_blade_definedness_corpus`), tree-sitter → Forte | 569 → 656 | 2,594 → 2,623 | 2,056 → 2,609 | |
| contradictions with PHPStan, tree-sitter → Forte | 0 → 0 | 13 → 9 | 1 → 1 | |

Bagisto's 36 remaining region disagreements are one class tree-sitter-blade had too: `@{{ … }}`
inside a directive's string argument, which Blade copies without its `@`. The contradictions are
a subset of tree-sitter-blade's. The Blade pack's conformance (`npm run conformance:languages --
--pack blade`) passes on the fixtures and the three corpora with no false positive; seven
fixture divergences tree-sitter-blade caused are gone, and its true positives rise from 183 to
557 (BookStack, `tlint-space-after-blade-directives`), 57 to 110 (BookStack,
`tlint-spaces-around-blade-render-content`) and 62 to 110 (Bagisto, `tlint-no-json-directive`).
The false negatives left are in the 18 Bagisto templates above.
