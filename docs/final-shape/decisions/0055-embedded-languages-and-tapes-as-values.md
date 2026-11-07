# 0055: Embedded languages are guest documents the library declares; a tape is a value rules can make

Status: proposed (2026-11-07; design only, nothing built)

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

1. **A guest is a document.** It has its own tape, in one grammar, and a link to the host node it
   came from. It is never spliced into the host's tape.
2. **The library declares guests,** as rules over the host's own facts. A language library says
   what it embeds; no Rust is needed for a region or a string.
3. **Guests are content facts,** cached like any tape: a guest's tape is keyed by its text and
   grammar, so one string parsed in a thousand files is parsed once.
4. **A tape is a value.** `@parse(grammar, text)` makes one inside a rule, and its nodes join like any
   other document's. Library-declared guests are the case where the text is a region of the file and
   the parse happens before evaluation.

## The model: a file is several single-language documents

```
// impl/report.md                         documents of the file
// # Report                               d=7  markdown, primary
// ```python                              d=8  python, guest of d=7 at the code_fence_content
// x = load("a")                                (its nodes are at the .md file's lines 3-4)
// ```
```

- **Each document has one grammar.** Its nodes are `d << 32 | preorder` as today. Kind codes are
  already per grammar (`tag * KIND_STRIDE + local`, `facts::core`), so documents of several grammars
  sit in one batch today; the batch keeps a `Dialect` per tag.
- **A guest's text is its own.** For a region it is the host's bytes. For a value, such as a string
  literal with escapes or a concatenation, it is the decoded text. A `SegmentMap` maps the guest's
  text back to host bytes. A copied stretch maps byte for byte. A written stretch (`\n` for the escape
  `\n`, a wrapper's `(`) maps to the host bytes it stands for. The map already exists and already
  handles both.
- **Positions are the host's.** A guest node's `line`, `end_line`, `col` and a finding's place come
  through the map, so findings land in the file at the right line. `text(n, s)` is the guest's text.
  `docof(n, d)` is the guest document, and `doc(d, path, _)` of a guest is the host's path.

## Decision

### 1. `core.rr` relations

```
// guest(g, host, at, grammar): document g is the code at host node or span `at` of document host,
//   parsed as `grammar`. One row per guest; a file with no guests has no rows.
.decl guest(g: number, host: number, at: loc, grammar: symbol)
// doc_role(d, r): "primary" (the file's own tree), "program" (a guest that is a whole program: a
//   Markdown fence, a <script>), or "fragment" (a part of the host's program: an annotation, a
//   template expression, an SQL string).
.decl doc_role(d: number, r: symbol)
// host_node(n, h): h is the innermost node of the host's tree that spans the bytes guest node n
//   stands for (the matcher's `host`); none for a guest parsed from a decoded value.
.decl host_node(n: node, h: node)
// root(d, n): the root of document d's tree; for a fragment, the expression, not its wrapper.
.decl root(d: number, n: node)

// What core.rr declares today, as views:
parsed(s, g, r) :- guest(d, _, s, g), root(d, r).
```

### 2. The library declares embeds, as rules

```
// embed(at, grammar, role): the code at node or span `at` is in `grammar`. A library derives it from
//   the host's own facts; the engine makes a guest document of each row.
.decl embed(at: loc, grammar: symbol, role: symbol)
// embed_value(at, grammar, role, text): the same, for code written as a value (a string with escapes,
//   a concatenation). The text is the decoded value; the map ties each piece to the host bytes it is
//   decoded from.
.decl embed_value(at: loc, grammar: symbol, role: symbol, text: symbol)
```

Each language library holds its own embeds:

```
// lib/markdown/embeds.rr: a fence whose info string names a language the registry parses.
embed(c, g, "program") :- kind(f, "fenced_code_block"), field(f, "info", i), text(i, t),
    fence_language(t, g), child(f, c, _), kind(c, "code_fence_content").

// lib/python/annotations.rr: a string in a position Ruff reads as a type (`string_annotation`,
//   now a library rule over Ruff's table instead of a native relation).
embed(s, "python-annotation", "fragment") :- string_annotation(s), str_plain(s).
embed_value(s, "python-annotation", "fragment", v) :- string_annotation(s), !str_plain(s), value(s, v).
```

- **A grammar name is a dialect and a start symbol.** The registry gives `python-annotation` as
  Python's expression, and `sql` as the SQL dialect. Tree-sitter has no start symbol, so an
  expression is parsed with a written wrapper, `(` … `)`, as rrlab and Vue already do. The wrapper is
  a written stretch of the map, and `root` is the expression inside it.
- **The embed rules form the embed program,** one per language library. It may read only the host
  document's own facts, and embeds of embeds. It cannot read another file or a policy's relations.
  So it is a file-local stratum, run before every policy, the same for every policy that runs.
- **Writers stay native where a program is written, not cut out.** Vue's script program and Blade's
  compiled view assemble text from pieces in an order of their own, and that is still a `via`. Its
  guests come out in the same relations, so rules cannot tell the difference.

### 3. When guests are made, and how they are cached

For each host document, before its chunk is evaluated:

1. **Run the embed program** on the host's facts. Its rows are a derivation keyed by (host tape hash,
   embed program digest), which is decision 0044's file-local stratum. An unchanged file with an
   unchanged library reads its embeds from the store.
2. **Get the guest's tape.** It is keyed by (sha256 of the guest text, grammar, tape format) in the
   tape cache (0051). The same text in any file, on any branch, is parsed once: "one parse per
   distinct string", as `core.rr` promises.
3. **Record the embedding:** host tape, `at` (a local id), grammar, guest text hash, and the map. The
   map belongs to the host's content and the tape to the guest's, so they are stored apart.
4. **Run the embed program on the new guests** (embeds of embeds: Markdown, then HTML, then
   JavaScript), up to a depth of 4. Each round's guests are smaller than their host. The bound is a
   guard, not a limit anyone should reach.
5. **Build the batch** with hosts and guests together, so every policy's evaluation sees all trees
   from the start. The memo (0052) keeps a file's guests' rows with the file's.

A file with no embeds pays one run of a small, selective program over facts the batch already has,
and stores no rows.

### 4. Which documents a policy sees

A policy names its language (`.language python`). It sees:

- the primary documents of that language;
- **guests of that language whose role is `program`**: Python in a Markdown fence is Python code, as
  the spike decided ("several single-language files joined together"). A policy that only wants
  files says `.scope embedded false`;
- **fragments only through the relations that reach them** (`guest`, `parsed`, `host_node`). An
  annotation is not a module, so module-level rules (an import order, a docstring) must not see one as
  a file.

Kind names are compared per grammar. `kind(n, "identifier")` in a Python policy reads Python
documents' identifiers, not a JavaScript guest's: the policy's language confines it. A rule that
crosses languages (a Markdown host reading its Python guests) names the guest's grammar through
`guest` or `doc_lang(d, l)`.

### 5. A tape is a value: `@parse`

```
// @parse(grammar, text): the tape of text parsed as grammar, a value (its content digest). Equal
//   texts give the same tape. Its nodes are a document's nodes; tape_doc(t, d) names that document.
.functor parse(grammar: symbol, text: symbol): tape
.decl tape_doc(t: tape, d: number)
.decl tape_of(d: number, t: tape)                      // every document's tape, primary ones too

// The SQL a call runs, built from an f-string's pieces, checked as SQL:
sql_root(c, r) :- query_call(c, s), sql_text(s, q), t = @parse("sql", q), tape_doc(t, d), root(d, r).
```

- **Lowering.** `@parse` lowers as `match_all` over symbols does (0054 §2). A context relation holds
  the texts a rule asks about, and a natively made relation gives `(text, grammar, tape)`. The
  evaluator parses each distinct text, or reads its tape from the cache.
- **Its nodes join the chunk.** Each new tape becomes a guest document of the chunk, with a `d` from a
  range of its own. The base relations are rebound with the new documents beside the old ones before
  any stratum that reads them. `Modules::bind` already rebinds a table between statements and keeps
  open cursors. The strata that read the tape's nodes come after the parse relation, as stratification
  already orders them.
- **No place of its own.** A tape made from a rule's text stands nowhere in a file. Its nodes have
  `text`, `kind` and the tree, but no `line`: a rule reports at the node the text came from. When the
  text is a node's own (`@parse(g, n)` of a node or span), the map is the node's, and its nodes have
  the file's positions, which is §3's case.
- **What a rule may do with a tape** in this step: make one, read its document's relations, compare
  two by value (equal digests are equal trees, as `@tree_digest` compares subtrees). Later steps add
  the operations that make new tapes from old: `@splice(t, n, t2)` (a subtree replaced) and
  `@unparse(t)`, which a fix needs to check that its edit still parses.
- **Termination.** `@parse` makes a value that is not on the tree, so a recursion through it is the
  `UnboundedValueCycle` that `front::cycles` refuses, unless the text it parses is strictly shorter
  than the text it came from (a slice, an `@rx_group`), which `front::cycles` already accepts.

### 6. One mechanism for both paths

- The matcher path's guests (`DerivedImage`, `file_documents`) become the same documents. Its
  `embedded` and `host` rules are `guest` and `host_node`, and the converter writes them.
- A future parser written by the transducer (parser-generators/12: "the output is our v9np tape,
  written directly") is the parse in §3 step 2 and in `@parse`: a (grammar, bytes) → tape call. Guests
  need nothing new from it.

## What the prior work says

- **Spike design index §21 and S070 (DESIGN L4148-4186): embedded regions as standalone segment tapes,
  "a multi-language file is several single-language files joined together",** with a sparse map that
  has "none at all for single-language files". *Agrees.* A guest is a document with its own tape, and
  `guest` has no rows for a file without embeds. The spike found embedding by host-grammar
  auto-detection (Markdown fences, HTML script and style). *Departs:* here the library's rules detect
  it, so adding an embedding needs no engine change.
- **S071 (DESIGN L4187-4240; DH L1789-1837): kind codes are per grammar and collide.** Making them
  global in the tape was +16.7% per node for every file and was rejected; Option B made only the
  anchor index global and removed the 13% tax. *Agrees.* The engine's facts already carry
  `tag * KIND_STRIDE + local` and the tape keeps local codes. Guests change neither.
- **S133 (DESIGN L727-747): plan-time name lookups must use the query's grammar**, not the primary
  one: a Rust query answered 0 instead of 71 when Python was also indexed. *Agrees,* and §4 is that
  rule for policies. A policy's kind names resolve in its language's documents. A cross-language
  rule names the guest's grammar.
- **Spike history: storage +11% for 400 HTML files' segments** (DH L1754-1787). *Expected to
  agree.* Guest tapes are small, and string guests are deduplicated by text, which the spike did not
  do. To be measured (step 2).
- **Lessons ledger item 8 (sidecars over the bytes-per-node budget):** *agrees.* There are no
  per-node rows. The embedding record is per guest, and a file without guests has none.
- **Lessons ledger item 20 (partition by file):** *agrees.* The embed program is file-local by
  construction (§2).
- **Decision 0044 (§1, §5): content facts keyed by content hash and grammar, file-local strata keyed
  by tape hash.** *Agrees, and extends it.* A guest tape is an L1 content fact keyed by its text, and
  the embed program's rows are a file-local L4 derivation.
- **0051 and 0052 (tape cache, per-file memo):** *kept.* Guest tapes go in the tape cache, and a
  file's memo entry holds its guests' rows.
- **rrlab (`serve_parsed`, `graft`):** *departs.* rrlab parses what a rule asks for, between rounds,
  with fresh ids and no cache. Here the library decides, the result is cached, and a rule that builds
  text uses `@parse` (§5) inside the same stratification.

## Steps

Each step keeps the replay and the converted gate unchanged, and stops for review.

1. **The `.rr` path reads the guests that exist** (Vue, Blade) as documents: `guest`, `doc_role`,
   `host_node`, and positions through the map. Test: an `.rr` rule over a Vue component's TypeScript
   reports at the `.vue` file's lines.
2. **Library embeds:** `embed` and `embed_value`, the embed program before the batch, and the guest
   tape cache keyed by text. First libraries: Markdown fences, HTML script and style, Python string
   annotations (`string_annotation` as a library rule, `parsed` as the view). Measure, on the
   9,010-file repository: guests made, distinct guest texts, parse time and storage.
3. **Embeds of embeds,** in rounds up to depth 4.
4. **`@parse` inside rules,** with the chunk's documents rebound (§5).
5. **Tape operations for fixes:** `@splice`, `@unparse`.

## Open

- **§4's default is a choice.** Guests whose role is `program` are visible to a policy of their
  language, as in the spike. A Python check would then see Python in Markdown fences, which is often
  partial code. The other choice, guests visible only when a policy opts in, keeps today's findings
  unchanged everywhere.
- Whether a guest that does not parse is withheld as a file is (`core.rr`'s `syntax_error`), or kept with
  its errors. A fence of pseudo-code should not be a finding in a Python policy.
- The grammar names a library may use (`python-annotation`, `sql`, …): where they live (the language
  registry, or `core.rr`), and which SQL dialects the registry has grammars for.
