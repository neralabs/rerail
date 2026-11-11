# 0055: Embedded languages are guest documents the library declares; a tape is a value rules can make

Status: accepted (2026-11-07). The `.rr` path's guests are built for Python and Rust files: file-local embeds, `sub_tape` and `parsed` requests, embeds of embeds, `.guests` and `.on guest_error` (see [What was built](#what-was-built)). Decision 0058 builds hiding, kinds across grammars, normalized strings and `@parse`, `@splice`, `@unparse`. Global embeds and the matcher path's guests are not built yet.

## Intro

Code often holds code in another language:

- a Markdown file's fenced blocks, an HTML file's `<script>` and `<style>`;
- a Vue component's scripts and template expressions, a Blade view's compiled PHP;
- strings: Python's `x: "List[int]"` and `cast("Foo", y)`, SQL in `execute("SELECT …")`, a regex, a
  LINQ-like query, JSON in a test fixture.

Rules need the inner code as a tree: to resolve the names in a string annotation, to check the SQL a
call runs, to lint the JavaScript in a page. Today there are two partial answers:

- **The matcher path** has it for Vue and Blade. A file is a *host* document plus *guest* documents,
  each guest with its own tape (`tape::DerivedImage`, stored in `file_documents`) and a map from its
  text back to the host's bytes (`embed::segments::SegmentMap`). JSON rules cross with `embedded` and
  `host`. But which guests exist is Rust: each embedding is a `via` in the language registry, and
  adding one is a new Rust module.
- **The `.rr` path** has none of it. `core.rr` declares `parsed(s, grammar, root)` and
  `string_annotation(s)`, and both are read as empty (O76, O80). The batch holds one tree per file,
  and drops any other root. rrlab grafts trees into its node space on request, between Soufflé
  rounds, with fresh ids. That is a demand loop per run, and nothing is cached.

This record makes embedding one mechanism for both paths:

1. **A tape is a fact, and a document is a tape placed somewhere.** A tape is a tree in one grammar,
   keyed by its content. A document is a tape at a place: a file, or a node or span of another
   document, with a map to that place's bytes. A guest is never spliced into its host's tape.
2. **The library declares guests** with `embed(host, at, grammar, role)`, as rules over any facts.
   A language library says what it embeds; no Rust is needed for a region or a string.
3. **Guests are generic facts.** Any rule, in any file's evaluation, can read any document's guests,
   as it reads any document's nodes. `sub_tape(at, grammar, t)` asks for one inside a rule.
4. **Tapes are cached by content:** one string parsed in a thousand files is parsed once.
5. **A policy sees guests by default,** and one flag turns that off. What happens to a guest that does
   not parse is the policy's choice too: skip it, keep its partial tree, or report it.

## The model: tapes, and documents that place them

```
// impl/report.md                         documents of the file
// # Report                               d=7  markdown, primary: tape T1 at the file
// ```python                              d=8  python, guest of d=7: tape T2 at the code_fence_content
// x = load("a")                                (its nodes are at the .md file's lines 3-4)
// ```
// docs/other.md holds the same fence:    d=9  python, guest of its own host: the same tape T2
```

- **A tape is content.** Its key is (sha256 of its text, grammar, tape format). Its facts are the
  ones that do not depend on a place: `kind`, `child`, `field`, `text`, `@tree_digest`. They are
  computed once per tape and shared by every document that places it.
- **A document places a tape.** Its nodes are `d << 32 | preorder` as today, and the facts that
  depend on the place come through the document's map: `line`, `end_line`, `col`, and a finding's
  position. `docof(n, d)` is the document. `doc(d, path, _)` of a guest is its host's path, so
  findings land in the file at the right line.
- **Each document has one grammar.** Kind codes are already per grammar (`tag * KIND_STRIDE +
  local`, `facts::core`), so documents of several grammars sit in one batch today. The batch keeps a
  `Dialect` per tag.
- **A guest's text is its own.** For a region it is the host's bytes. For a value, such as a string
  literal with escapes or a concatenation, it is the decoded text. The `SegmentMap` maps the guest's
  text back to host bytes. A copied stretch maps byte for byte. A written stretch (`\n` for the escape
  `\n`, a wrapper's `(`) maps to the host bytes it stands for. The map already exists and already
  handles both.

## Decision

### 1. `core.rr` relations

```
// tape_of(d, t): document d places tape t. Every document has one: primary documents and guests.
.decl tape_of(d: number, t: tape)
// guest(g, host, at, grammar): document g is the code at node or span `at` of document `host`,
//   parsed as `grammar`. One row per guest, readable from any file's evaluation.
.decl guest(g: number, host: number, at: loc, grammar: symbol)
// doc_role(d, r): "primary" (the file's own tree), "program" (a guest that is a whole program: a
//   Markdown fence, a <script>), or "fragment" (a part of the host's program: an annotation, a
//   template expression, an SQL string).
.decl doc_role(d: number, r: symbol)
// host_node(n, h): h is the innermost node of the host's tree that spans the bytes guest node n
//   stands for (the matcher's `host`); none for a guest parsed from a decoded value.
.decl host_node(n: node, h: node)
// doc_root(d, n): the root of document d's tree; for a fragment, the expression, not its wrapper.
.decl doc_root(d: number, n: node)

// What core.rr declares today, as a view:
parsed(s, g, r) :- guest(d, _, s, g), doc_root(d, r).
```

### 2. The library declares embeds, as rules keyed by their host

```
// embed(host, at, grammar, role): the code at node or span `at` of document `host` is in `grammar`.
//   The engine makes a guest document of each row.
.decl embed(host: number, at: loc, grammar: symbol, role: symbol)
// embed_value(host, at, grammar, role, text): the same, for code written as a value (a string with
//   escapes, a concatenation). The text is the decoded value; the map ties each piece to the host
//   bytes it is decoded from.
.decl embed_value(host: number, at: loc, grammar: symbol, role: symbol, text: symbol)
```

The host is a variable, bound by the body like any other. There is no `THIS_FILE`: Datalog has no
current file, and a rule over every document is the same rule as one over "this" document.

```
// lib/markdown/embeds.rr: a fence whose info string names a language the registry parses.
embed(d, c, g, "program") :- node(d, f, "fenced_code_block"), field(f, "info", i), text(i, t),
    fence_language(t, g), child(f, c, _), kind(c, "code_fence_content").

// lib/python/annotations.rr: a string in a position Ruff reads as a type (`string_annotation`,
//   now a library rule over Ruff's table instead of a native relation).
embed(d, s, "python-annotation", "fragment") :- string_annotation(s), str_plain(s), docof(s, d).
embed_value(d, s, "python-annotation", "fragment", v) :-
    string_annotation(s), !str_plain(s), value(s, v), docof(s, d).

// lib/python/sql.rr: the string passed to a function another module declares as taking SQL.
//   Its body reads another file's facts, so it is a global embed (§3).
embed(d, s, "sql", "fragment") :- call_arg(c, 0, s), callee(c, f), sql_parameter(f, 0), docof(s, d).
```

- **A grammar name is a dialect and a start symbol.** The registry gives `python-annotation` as
  Python's expression, and `sql` as the SQL dialect. Tree-sitter has no start symbol, so an
  expression is parsed with a written wrapper, `(` … `)`, as rrlab and Vue already do. The wrapper is
  a written stretch of the map, and `root` is the expression inside it.
- **`at` must be a node or span of `host`.** An `embed` row whose `at` is not in `host` is refused at
  run time, the way a finding without a document is. So the host is a key the engine checks; it never
  re-places code at another file.
- **Writers stay native where a program is written, not cut out.** Vue's script program and Blade's
  compiled view assemble text from pieces in an order of their own, and that is still a `via`. Its
  guests come out in the same relations, so rules cannot tell the difference.

### 3. `sub_tape`: asking for a guest inside a rule

```
// sub_tape(at, grammar, t): t is the tape of the code at node or span `at`, parsed as `grammar`,
//   placed at `at` (its nodes have the file's positions).
.decl sub_tape(at: loc, grammar: symbol, t: tape)

// Every SQL string a call runs, checked as SQL, with no library embed:
bad_sql(s) :- query_call(c, s), sub_tape(s, "sql", t), tape_of(d, t), guest(d, _, s, _),
    syntax_error(d, _, _).
```

`sub_tape` is `embed` used in a body. A rule that reads `sub_tape(at, g, t)` with `at` and `g`
bound is lowered to an `embed` row for the context that binds them (`embed(d, at, g, "fragment")`
with `docof(at, d)`), and a view over what the engine makes:

```
sub_tape(at, g, t) :- guest(d, _, at, g), tape_of(d, t).
```

So the two forms are one mechanism. `embed` is the library's declaration, which every policy shares.
`sub_tape` is one rule's request. Both make the same guest, and the same tape, cached by its text.

### 4. When guests are made, and how they are cached

An embed rule is placed by what its body reads, as decision 0044 places every stratum:

- **A file-local embed** reads only its host document's own facts, and embeds of embeds (the
  Markdown and annotation rules above). It runs before the batch is built, per host document:
  1. **Run the file-local embed rules** on the host's facts. Their rows are a derivation keyed by
     (host tape hash, embed program digest), 0044's file-local stratum. An unchanged file with an
     unchanged library reads its embeds from the store.
  2. **Get the guest's tape** by (sha256 of the guest text, grammar, tape format) from the tape cache
     (0051). The same text in any file, on any branch, is parsed once: "one parse per distinct
     string", as `core.rr` promises.
  3. **Record the document:** host tape, `at` (a local id), grammar, guest tape, and the map. The map
     belongs to the host's content and the tape to the guest's, so they are stored apart.
  4. **Run the embed rules on the new guests** (embeds of embeds: Markdown, then HTML, then
     JavaScript), up to a depth of 4. Each round's guests are smaller than their host. The bound is a
     guard, not a limit anyone should reach.
  5. **Build the batch** with hosts and guests together, so every stratum sees those trees from the
     start. The memo (0052) keeps a file's guests' rows with the file's.
- **A global embed** reads another file's facts (the `sql_parameter` rule above), or a policy's
  relations, as `sub_tape` in a policy does. It runs in the evaluation, in the stratum after the facts
  it reads. Its guests are made there (§6's mechanism), and the strata that read them come after.
  Its rows are keyed by what they read, as 0044's higher layers are. The tapes are still cached by
  text, so a global embed costs a parse only for a text never seen before.

A file with no embeds pays one run of a small, selective program over facts the batch already has,
and stores no rows. This record does not make every embed global, though that would be one rule
instead of two. A global embed's guests exist only after the strata it reads, so no other stratum of
that evaluation sees them as files, and its rows are recomputed when any file they read changes.
Markdown fences and annotations need neither cost.

### 5. Which documents a policy sees

A policy names its language (`.language python`). By default it sees:

- the primary documents of that language;
- **every guest of that language,** programs and fragments: Python in a Markdown fence and a string
  annotation are both Python documents.

One flag turns guests off:

```
.guests off          // only primary documents; guests are reachable through `guest` and `sub_tape`
.guests on           // the default
```

`.guests off` hides guests as documents, not as facts: `guest`, `parsed`, `host_node` and
`sub_tape` still reach them.

A fragment does not look like a file to a rule. Its root is an expression, not a `module`. A rule
about a module (an import order, a module docstring) matches nothing in it, and a rule about an
expression (a call, a name) matches it as it would match code. A rule that must tell them apart reads
`doc_role(d, r)`.

Kind names are compared per grammar. `kind(n, "identifier")` in a Python policy reads Python
documents' identifiers, not a JavaScript guest's: the policy's language confines it. A rule that
crosses languages (a Markdown host reading its Python guests) names the guest's grammar through
`guest` or `doc_lang(d, l)`.

### 6. Guests that do not parse

A primary document that does not parse is withheld, and `.on syntax_error report CODE` reports it
(c6fc887f4). A guest is often partial code: a Markdown fence of pseudo-code, a template expression
cut off at a placeholder. So the policy says what a broken guest is:

```
.on guest_error skip                 // the default: the guest is withheld, nothing is reported
.on guest_error partial              // the guest's partial tree is kept, ERROR nodes and all
.on guest_error warn CODE            // withheld, and a warning at the guest's first error
.on guest_error report CODE          // withheld, and an error at the guest's first error
.on guest_error partial warn CODE    // kept, and a warning
.on guest_error partial report CODE  // kept, and an error
```

- **Withheld means as a document.** A skipped guest still has its `guest` row and its
  `syntax_error(d, line, msg)` row, so a rule can read them. It has no nodes in the policy's
  documents, so no check runs on half a tree.
- **The verdict is the grammar's,** as for a primary document: Ruff's for Python (`core_rr::verdict`),
  tree-sitter's first error for the others. A `fragment` is judged in its start symbol: an annotation
  that is a valid expression parses, though it is not a module.
- **The tape does not depend on the choice.** Tree-sitter always makes a tree, so the cache holds one
  tape per text, with its error rows. Skip and partial differ only in whether the policy's batch binds
  its nodes. Two policies with different choices share the parse.
- **A withheld host has no guests.** Its embed rules have no facts to run on.
- **Per grammar.** `.on guest_error sql report SQL001` sets the choice for one grammar's guests; the
  form without a grammar sets the rest.

### 7. A tape is a value: `@parse`

```
// @parse(grammar, text): the tape of text parsed as grammar, a value (its content key). Equal texts
//   give the same tape. tape_of(d, t) names the document that places it in this evaluation.
.functor parse(grammar: symbol, text: symbol): tape

// The SQL a call runs, built from an f-string's pieces, checked as SQL:
sql_root(c, r) :- query_call(c, s), sql_text(s, q), t = @parse("sql", q), tape_of(d, t), doc_root(d, r).
```

- **`@parse` is `sub_tape` for text with no place.** It lowers as `match_all` over symbols does (0054
  §2). A context relation holds the texts a rule asks about, and a natively made relation gives
  `(text, grammar, tape)`. The evaluator parses each distinct text, or reads its tape from the cache.
- **Its nodes join the chunk.** Each new tape is placed as a document of the chunk, with a `d` from a
  range of its own and the role `fragment`. The base relations are rebound with the new documents
  beside the old ones before any stratum that reads them. `Modules::bind` already rebinds a table
  between statements and keeps open cursors. The strata that read the tape's nodes come after the
  parse relation, as stratification already orders them. A global embed (§4) and `sub_tape` in a
  policy use this same path.
- **No place of its own.** A tape made from a rule's text stands nowhere in a file. Its document has
  `text`, `kind` and the tree, but no `line`: a rule reports at the node the text came from.
- **What a rule may do with a tape** in this step: make one, read its document's relations, compare
  two by value (equal keys are equal trees, as `@tree_digest` compares subtrees). Later steps add
  the operations that make new tapes from old: `@splice(t, n, t2)` (a subtree replaced) and
  `@unparse(t)`, which a fix needs to check that its edit still parses.
- **Termination.** `@parse` makes a value that is not on the tree, so a recursion through it is the
  `UnboundedValueCycle` that `front::cycles` refuses, unless the text it parses is strictly shorter
  than the text it came from (a slice, an `@rx_group`), which `front::cycles` already accepts. Embeds
  of embeds (§4) end the same way: each guest's text is a part of its host's.

### 8. One mechanism for both paths

- The matcher path's guests (`DerivedImage`, `file_documents`) become the same documents. Its
  `embedded` and `host` rules are `guest` and `host_node`, and the converter writes them.
- A future parser written by the transducer (parser-generators/12: "the output is our v9np tape,
  written directly") is the parse in §4 step 2 and in `@parse`: a (grammar, bytes) → tape call. Guests
  need nothing new from it.

## What the prior work says

- **Spike design index §21 and S070 (DESIGN L4148-4186): embedded regions as standalone segment tapes,
  "a multi-language file is several single-language files joined together",** with a sparse map that
  has "none at all for single-language files". *Agrees.* A guest is a document with its own tape, and
  `guest` has no rows for a file without embeds. A policy sees guests by default (§5), as the spike's
  queries saw segments. The spike found embedding by host-grammar auto-detection (Markdown fences,
  HTML script and style). *Departs:* here the library's rules detect it, so adding an embedding needs
  no engine change.
- **S071 (DESIGN L4187-4240; DH L1789-1837): kind codes are per grammar and collide.** Making them
  global in the tape was +16.7% per node for every file and was rejected; Option B made only the
  anchor index global and removed the 13% tax. *Agrees.* The engine's facts already carry
  `tag * KIND_STRIDE + local` and the tape keeps local codes. Guests change neither.
- **S133 (DESIGN L727-747): plan-time name lookups must use the query's grammar**, not the primary
  one: a Rust query answered 0 instead of 71 when Python was also indexed. *Agrees,* and §5 is that
  rule for policies. A policy's kind names resolve in its language's documents. A cross-language
  rule names the guest's grammar.
- **Spike history: storage +11% for 400 HTML files' segments** (DH L1754-1787). *Expected to
  agree.* Guest tapes are small, and tapes are deduplicated by text (a tape is content, a document
  only places it), which the spike did not do. To be measured (step 2).
- **Lessons ledger item 8 (sidecars over the bytes-per-node budget):** *agrees.* There are no
  per-node rows. A document is one record per guest, and a file without guests has none.
- **Lessons ledger item 20 (partition by file, global machinery for the rest):** *agrees.* A
  file-local embed runs per file before the batch. A global embed is the "rest", and runs where 0044
  places cross-file strata (§4).
- **Decision 0044 (§1, §5): content facts keyed by content hash and grammar, file-local strata keyed
  by tape hash, higher layers by what they read.** *Agrees, and extends it.* A tape is an L1 content
  fact keyed by its text. A file-local embed is an L4 derivation. A global embed is a higher layer,
  and its tapes are still L1.
- **0051 and 0052 (tape cache, per-file memo):** *kept.* Guest tapes go in the tape cache, and a
  file's memo entry holds its file-local guests' rows.
- **Withholding (c6fc887f4: a document that does not parse is withheld):** *extended.* §6 gives a
  guest the same withholding, plus the partial tree, as a policy choice.
- **rrlab (`serve_parsed`, `graft`):** *departs.* rrlab parses what a rule asks for, between rounds,
  with fresh ids and no cache. Here the library or the rule asks (`embed`, `sub_tape`), the result is
  cached, and the parse sits inside the same stratification.

## Steps

Each step keeps the replay and the converted gate unchanged, and stops for review.

1. **The `.rr` path reads the guests that exist** (Vue, Blade) as documents: `tape_of`, `guest`,
   `doc_role`, `host_node`, positions through the map, `.guests on|off`, and `.on guest_error`. Test:
   an `.rr` rule over a Vue component's TypeScript reports at the `.vue` file's lines; a broken
   template expression is skipped, kept partial, or reported as the policy says.
2. **File-local embeds:** `embed` and `embed_value`, the embed rules before the batch, and the guest
   tape cache keyed by text. First libraries: Markdown fences, HTML script and style, Python string
   annotations (`string_annotation` as a library rule, `parsed` as the view). Measure, on the
   9,010-file repository: guests made, distinct guest texts, parse time and storage.
3. **Embeds of embeds,** in rounds up to depth 4.
4. **Guests inside the evaluation:** `@parse`, `sub_tape`, and global embeds, with the chunk's
   documents rebound (§7).
5. **Tape operations for fixes:** `@splice`, `@unparse`.

## What was built

A worked example first. This policy finds the names in string annotations:

```
// Each identifier inside a string annotation, at its place in the file.
.decl ident(x: symbol, l: number, c: number)
ident(x, l, c) :- kind(s, "string"), parsed(s, "annotation", _),
                  guest(g, _, s, _), node(g, i, "identifier", _, _),
                  text(i, x), line(i, l), col(i, c).
```

Over this file:

```python
x: "List[int]" = []            # List at 1:4, int at 1:9: the string's bytes are copied
z: "A\x42" = 1                 # AB at 4:3: a decoded value stands at its string
q: "Opt['Baz']" = 3            # Baz at 6:9: the inner string is parsed in round 2
b: b"Nope" = 4                 # nothing: a bytes literal is no annotation
```

How it runs, per chunk (`logic/plan/corpus.rs`, `logic/plan/corpus/guests.rs`):

1. **The rule asks.** Lowering (`rr/lower.rs`, `embed_demand`) turns each `parsed(s, "g", r)` or
   `sub_tape(s, "g", t)` into `rr_embed_req(s, "g")`. Its body is a context relation of the rule's
   other literals that bind `s`. Literals that read the guest are left out: those joined to `r` or
   `t`, and the guest relations themselves. Without that, the request would wait on its own answer.
2. **The embed slice runs.** The rules that make `embed`, `embed_value` and `rr_embed_req` are cut
   from the file-local program (`embed_slice`). They run over the chunk's batch before anything else.
3. **The guests are parsed.** Each request names a host node (`host << 32 | local`). Its text is
   chosen as follows:
   - A plain string's contents are a copied stretch of the file, so positions map by an offset
     (`Placement::Copied`).
   - An escaped string, or a concatenation, gives its decoded value, and every node stands at the
     string (`Placement::At`).
   - Bytes and f-strings give no guest.
   - Any other node gives its own text.

   The tape comes from `core::build_cached`, keyed by text, so equal texts are parsed once.
4. **The batch is rebuilt** with the hosts and the new guest documents. The slice runs again, up to
   4 rounds. A string inside a parsed annotation is asked for in round 2, and its placement is
   composed with its host's (`Placement::then`). After that, the file-local program runs once over
   the final batch.

What a guest document is (`facts/vtab/syntax.rs`, `facts/vtab/syntax/guests.rs`):

- **Its key** is in `2^30..2^31`, hashed from its host's path, its place and its grammar. Files now
  take `1..2^30` (`memo::doc_keys`). Every node id stays positive, and a memoised row's guest nodes
  keep their ids.
- **Its nodes** are its root's subtree. For the `annotation` grammar, the text is parsed as Python
  `( … )`, and the root is the expression inside the parentheses, as rrlab does.
- **Its positions** (`line`, `col`, `end_line`) are its file's, through the placement. It has no
  `source`, `src_line` or `line_count` rows, because its text is the file's.
- **Its rows:**
  - `tape_of(d, t)`, `doc_role(d, r)` and `doc_root(d, n)`, for every document (`doc_root`, as
    `root` is a name rule files already use);
  - `guest(g, host, at, grammar)`, `parsed(at, grammar, root)` and `sub_tape(at, grammar, t)`, for
    guests;
  - `host_node(n, h)`, made only when a rule reads it.
- **Its rows are its file's.** The memo attributes them to the host file (`host_of`).
- **A broken guest** is withheld unless the policy says `partial`. Its `syntax_error` line is
  mapped into the file. `.on guest_error … report CODE` lowers to a `violation` over
  `syntax_error`, `guest` and `doc`. `warn` also sets `.severity guest_error warning`. A file's own
  `.on syntax_error` now excludes guests (`!guest(d, _, _, _)`).

The libraries' `parsed(s, "annotation", r)` (`names.rr`, `typing.rr`, `annotations.rr`,
`ruff-classes.rr`, `typing-context.rr`) now has rows. `parsed` is no longer in the corpus runner's
`NOT_MADE` list. `string_annotation` still is (O76), because Ruff's table of the call positions
that take a type as a string is not built.

Where this differs from the decision above:

- **`.guests off`** first turned off the libraries' embeds. Decision 0058 made it what §5 asked for:
  embeds are always made, and `.guests off` hides guests from rules that do not reach them through
  a guest relation.
- **Grammars** are `annotation` or a language label from the registry (`python`, `rust`, `sql`,
  `typescript`, …). A guest's kinds resolve in its own grammar where a rule reaches it (0058 §2).
- **The corpus runner collects `.py` and `.rs` files only**, so Markdown, HTML, Vue and Blade hosts
  wait for it. The matcher path's Vue and Blade guests are not yet these documents (step 1's test).
- **Embeds are file-local.** An `embed` whose body reads another file's facts runs inside the chunk,
  over the chunk's files only. Global embeds and `@parse` (§7) are not built.

Tests (`logic/rr/tests.rs`):

- `string_annotations_are_guests_at_their_host_lines`: plain, escaped, concatenated, nested and
  bytes strings;
- `guests_that_do_not_parse_follow_the_policy`: skip and report, then partial;
- `embeds_declare_guests_and_guests_off_leaves_them_out`: `embed`, `embed_value`, `sub_tape`'s
  shared tape, and `.guests off`.

The replay is unchanged (3988 303 298 0; 47 s user and 11 s system CPU, load average about 127).
The converted gate's checks read no `parsed`.

## Open

- ~~The grammar names a library may use, and which SQL dialects have grammars.~~ Decision 0082 §1–§2:
  the language registry (dialect labels and `fragments`); one SQL grammar, `sql`.
- ~~Whether a global embed's guests should be documents of a later run.~~ Decision 0082 §3: no; they
  are documents in a later wave of the same run.
