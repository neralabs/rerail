# NOTES python-01: RUF001, RUF002, RUF005, RUF006, RUF008, RUF012, RUF013, RUF016

Ground truth: ruff 0.17.6 on a fleet box (`ruff check --select <CODE> --no-cache`). Last check:
`ptcheck --files … -- --pack python --rules <the eight>`, exit 0: seven `same`, RUF013 `different` by one key where
the translation follows Ruff.

New library: `lib/python/python-01.rerail` (rt_app and friends, typedef_node, doc_piece), shared by RUF001, RUF002,
RUF005, RUF008, RUF012, RUF013 and RUF016.

## Per rule

- **RUF001** (same). Followed Ruff in three places.
  - A string annotation is reported unless annotations are deferred. The rule dropped every parsed annotation string
    (`!parsed_in(_, s)`), so it missed `y: List["Fоo"]` and `t: "Fоo" | None` under `from __future__ import
    annotations` (probe lines 6 and 9), and `x: '''"""'Literal["ﮨ"]'"""'''` (confusables.py:61). Ruff reports all
    three. It skips only a deferred annotation's root string and the strings parsed out of it:
    `from __future__ import annotations; x: "List['Fоo']"` gets no finding, and the translation never reaches that
    tree, because it only enumerates the file's string_content nodes. Survey candidate 0b29205463 used `ann_tree`,
    which also drops the nested strings Ruff reports, so I did not adopt it.
  - `"dоc",` is a tuple, not a docstring. RUF001 reports it (RUF002-tuple-statement-is-not-docstring.py:2).
  - A word made only of look-alikes is reported: `{𝑞𝑢𝑒𝑟𝑦}` (FAST003.py:262), `Ｔｈｉｓ`. With the same fix, the
    42 django locale keys go away, e.g. `"j F Y، ساعت G:i"` (fa/formats.py:7): `ساعت` has one look-alike out of four
    non-ASCII letters, and Ruff does not report it. See the engine gap below.
- **RUF002** (same). Uses doc_piece: `"dоc",` is not a docstring. The count fix brings back `"""Ｔｈｉｓ is"""`
  (D404-joined-and-non-ascii-first-pieces.py:26).
- **RUF005** (same). Ruff parses string type definitions and runs RUF005 on what they contain:
  `def f(a: "foo + [1]")`, `List["foo + [1]"]`, `cast("(1,) + foo", w)`, `TypeVar("T", bound="[1] + foo")`, an unresolved
  `Literal['[1] + foo']` inside a string annotation, `"[1]" "+ foo"` and `"[1] + \x66oo"` (probe lines 2–8, 12, 13).
  The translation now also starts from typedef_node. `"  [1] + foo"` is F722 in Ruff (the text starts with a space)
  and gets no finding, so typedef_str leaves out strings that start with a space or a tab.
- **RUF006** (same). Ruff ties each unused binding to its statement, so an unread comprehension variable in
  `t = asyncio.create_task(asyncio.gather(*[fetch() for _ in urls]))` reports the task even though `t` is awaited
  (probe line 5). Generator `(fetch(u) for u in urls)` is not reported. Followed Ruff with a third clause: a
  comprehension `for` target that nothing reads, inside an `… = <task call>` or `…: T = <task call>` statement.
  Checked against ruff on `self.t = …[… for a, b in urls if a]` (reported, `b`), `x: int = …{… for a in urls for b in a}`
  (reported), `y = z = …` (not reported: two targets), and a nested comprehension and a lambda that read the variable
  (not reported).
- **RUF008, RUF012** (same). Ruff's AST makes `Union[(int, str)]` the same as `Union[int, str]`, but ruff-classes'
  type_app saw one argument, a tuple. So `a: Union[(int, str)] = []` was reported, where Ruff takes it as immutable
  (RUF008 probe lines 7, 8, 10; RUF012 probe lines 5, 6, 9, 10). rt_immutable follows Ruff's is_immutable_annotation
  over rt_app: Union and Annotated count only with a tuple subscript (`Union[(int,)]` is immutable), and Optional only
  without one.
- **RUF013** (different: 1 translation-only key on samples-python; right: translation). Two fixes:
  - The rule reported nothing: `shown(t, m) :- …, !parsed(m, "annotation", _)` never holds when m is not a string
    (engine gap below). The negation now goes through `ann_parsed(s) :- typedef_str(s), parsed(…)`.
  - It now uses rt_app, so `Annotated[(int, 1)]`, `Union[(int, str)]` and `Literal[(None, 1)]` read as Ruff reads
    them (RUF013-parenthesized-tuple-slices.py). In string annotations, `"Annotated[int, x]" = None` is reported and
    `"  int" = None` (F722) is not, as in Ruff.
  - The remaining key: modin/pandas/series.py:1514 `method: str = None` in `class Series`, which binds
    `str = CachedAccessor(…)` later, at line 1821, under `from __future__ import annotations`. Ruff reports it.
    Minimal case, which Ruff reports:
    `from __future__ import annotations` / `class S:` / `    def f(self, method: str = None): ...` / `    str = 1`.
    The original seems to resolve `str` to the class attribute. Right: translation.
- **RUF016** (same). Like RUF005, Ruff checks string type definitions: `x: "[1]['a'] | [2][1.5:'b':None]"` and
  `y: list["(1,)[1.0]"]` (probe lines 7, 8). indexed now also starts from typedef_node, and the tuple-index tests use
  negations instead of counts (engine gap below).

## Engine gaps (worked around; worth fixing in the engine)

1. **A count does not see a parsed string's nodes.** `count : { field(s, "subscript", _) }` is 0 when s is a node of an
   annotation guest, though `field(s, "subscript", i)` holds. Every count that can reach a guest is now a negation
   (RUF016 index_tuple/later_index, lit_type's non_str_part, typedef_other_part) or a child index (rt_at). rt_at
   numbers a subscript's arguments by child index, so a comment among them shifts the numbers.
2. **Two counts bound to one variable.** In `n = count : {A}, n = count : {B}`, the clause held whenever count B was
   1, whatever count A was. Measured: (4 non-ASCII, 1 look-alike) held, and so did (8, 1); (4, 4), (9, 2) and (7, 4)
   did not. The table's confusable_word_counts now binds `n` and `m` and tests `n = m`.
3. **`!parsed(m, g, _)` on a node that is not a string never holds.** Guard it with a string test first.
4. **A relation over every node does not reach guests.** `kind(b, "binary_operator")` with b free lists the file's
   nodes only, even after a parse was asked for. A rule that wants guest nodes must bind them first (typedef_node).
   Relations derived with the node bound (list_like, unparen, negated helpers) do work on guests.
5. **Recursion that adds to a number is refused**: `rt_at(m, i + 1, b) :- rt_at(m, i, a), …` gives "unbounded value
   cycle". Hence the child index in (1).

## Library changes proposed

- ruff-classes' `type_app`/`ann_input`/`immutable_ann` should read a lone tuple subscript (`X[(a, b)]`) as the argument
  list, and should not number arguments with a count (gap 1). python-01's rt_* relations do both and can replace them.
- python/ast's `doc_stmt` should reject `"doc",` (a one-element tuple statement). python-01's doc_piece does this.
- typing's `type_ctx` together with typedef_str/typedef_node: "the nodes of a string type definition that Ruff
  checks". Other ruff expression rules have the same originalOnly gap on string annotations.

## Generated table

`ruff-ruf001-ambiguous-unicode-character-string.confusables.rerail` lines 22–25 (confusable_word_counts) were edited
by hand for gap 2. `gen-ruff-ruf001-ambiguous-unicode-character-string.py` lines 81–82 still write the old clause and
need the same change (`m = count : {{ confusable_letter(w, _) }}, n = m.`). I did not touch it because it is outside
this batch's files. The table is shared with RUF003, which now comes out `same` (equivalence.json still lists it as
different, 0/3 on fixtures).
