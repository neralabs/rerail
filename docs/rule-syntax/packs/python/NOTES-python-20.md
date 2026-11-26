# python-20: N813, N814, N815, N816, N817, N818

All six follow Ruff 0.17.6 (`ruff check --select <CODE>` on a box). Ground truth was taken on the differing fixtures
and on scratch files of scope, import and Unicode edge cases (one of them run both at module level and in a class).
The translations report exactly Ruff's (path, line, code) keys on all of them, apart from Ruff's `invalid-syntax` rows.
`ptcheck -- --pack python --rules <the six>`: all six `same` on examples, fixtures and samples-python.

## Shared library: `lib/python/python-20.rerail`

All six rules share two things that `python/naming` does not model, so they live in a new library:

- **Names as Ruff reads them.** Ruff's parser NFKC-normalises identifiers, as CPython does, and pep8-naming's case
  tests use `char::is_uppercase` / `is_lowercase` (Unicode's Uppercase/Lowercase properties). `python/naming` tests
  raw text with `[A-Z]`/`[a-z]`. `ruff_norm` (`@nfkc`), `ruff_cased_lower/upper`, `ruff_camel`, `ruff_mixed`,
  `ruff_acronym` (`\p{Uppercase}` regexes) and `ruff_ignored` (ignore-names globbed against the normalised name) do
  the same over normalised names.
- **Import names without their spelling.** `ruff_alias_import` / `ruff_import_full` read a dotted name as its
  identifiers joined by `.`. `python/naming`'s `alias_import` takes the text after the last `.`, which breaks on
  `xml . etree` and on a line continuation.
- **Stores in Ruff's current scope** (N815/N816). `ruff_store(n, y, s)` covers `store_name`, `case` captures
  (`py_capture`) and `*x` in a `with … as (a, *x)` target. Its scope `s` is the one Ruff's visitor has current:
  parameter defaults and a comprehension's first iterable (nested too) belong to the scope around them. Class bases,
  PEP 695 type parameters and a `type` alias's value belong to neither the class nor the module.

Proposed for `python/naming` at integration: replace its case relations and `alias_import` with these. Its N801-N812
rules have the same NFKC/Unicode gap (see `NOTES-py-d-n.md`, A1). The `\p{…}` classes work in the engine's regexes.

## Per rule

- **N813** (`ruff-n813-camelcase-imported-as-lowercase`). Was: missed `from mod import ǅx as dzx` (Ruff: `Džx` is
  camel case; the raw `ǅx` matched `[^A-Z]*[a-z]…` as lowercase), and reported `from m import Ǳ as dz` and
  `Ⅴ as five` (Ruff: `DZ`, `V` are upper case, not camel). Fixed by normalising.
- **N814** (`ruff-n814-camelcase-imported-as-constant`). Was: missed `Camel as Ǆ` / `Ⅻ` / `Ŀ` because their
  one-character raw aliases fell under Ruff's one-character exemption. Ruff counts the normalised alias (`DŽ`,
  `XII`, `L·`), so the length test is now `match("..+", a)` on the normalised alias. `Camel as Ⅰ` (`I`) stays exempt.
- **N815** (`ruff-n815-mixed-case-variable-in-class-scope`). Was: missed `case` captures (`case int(fN)`,
  `case fN2 as fN3`, `case [wX, (yZ | yZ)]`), walruses in a parameter default (`def f(self, x=(cD := 1))`) and in a
  comprehension's first iterable (`[x for x in (gH := q)]`), and `*uV` in `with ctx() as (sT, *uV)`. Counterexamples
  Ruff does not report, and neither does the translation: `class K((iJ := object))` in a class body, `class P[T: (qR
  := int)]`, `type TA = (oP := int)`, walruses in annotations. TypedDict detection is unchanged. The branch version on
  cbf1a4bdf4 uses `class_anc` (the py_names engine), which main lacks and which was not ported. It gives the same
  findings on a single file.
- **N816** (`ruff-n816-mixed-case-variable-in-global-scope`). Same fixes as N815: captures (`case fN1 as fN2`,
  `{**fN3}`) and walruses in a first iterable, nested ones too: `ts = (a for a in [b for b in (fooD := [1])])` is
  reported, while `[[y for y in (fooA := x)] for x in …]` is not (the inner comprehension sits in the outer one's
  element).
- **N817** (`ruff-n817-camelcase-imported-as-acronym`). Was: missed `import xml.\⏎ etree.cElementTree as ET` (the
  name was read as the text after the last `.` of the first line), and reported
  `from xml . etree import ElementTree as ET` (the module text kept its spaces, so the `xml.etree.ElementTree`
  convention did not match). Only Ruff's default import conventions are known (`ET` is the only acronym among them).
  `flake8-import-conventions.aliases` / `extend-aliases` options are not read, as before.
- **N818** (`ruff-n818-error-suffix-on-exception-name`). Was: missed bases spelled `Ｅxception`, `𝐄xcept𝐢on`,
  `Exceptⅰon`, `ᴱrror`, and reported `class Ｅrror(Exception)` (Ruff's name is `Error`). Fixed by normalising the
  class name and the bases. A parenthesised base `class A((Exception))` is now a base too, as in Ruff's AST.

## Engine gaps

None needed for these rules.
