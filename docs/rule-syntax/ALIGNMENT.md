# One rule language across the branches

Four branches write Rerail rules, in three notations:

- **this branch** (More-ergonomic-rule-syntax): Soufflé Datalog with six additions, `.rr` files;
- **Final rerail shape** (wolfeel): "Rerail logic", a Datalog dialect with a parser, a stratifier and magic sets in
  the engine, `.rdl` files. Its decisions 0036 (tree axes are built-ins computed as tapes) and 0037 (text tests are
  globs over typed text) are the newest statements about rule syntax;
- **Pulling it all together**: `rerail.policy/v1` JSON, with new lowering (tree neighbourhoods, set algebra,
  vocabulary predicates) and specified named derived facts. No new surface syntax;
- **Data-layer rule expressivity**: `rerail.policy/v1` JSON with SQL stages, and new facts: CPython AST fields
  from an isolated worker, syntax errors, ordered reachability, runtime observations. No new surface syntax.

This page settles each construct once, as close to Soufflé as the semantics allow, and says what this branch took
from the others. Where the engine on wolfeel already parses something, the table gives the one-token mapping, so
that its converter can print this form, or its parser accept it.

## The rule for choosing

1. **Soufflé's spelling wins where Soufflé has the construct:** variables, types, aggregates, lattices, magic sets,
   comments and built-in constraints.
2. **The semantics win where they differ.** A Soufflé spelling with other semantics, such as wolfeel's `@cat`
   (a dotted join) beside Soufflé's `cat` (plain concatenation), gets a different name.
3. **New constructs look like Soufflé's nearest one.** A text test is a built-in constraint like `match` and
   `contains`: pattern first, subject second, no `@`.

## Constructs

| Construct | Soufflé | wolfeel `.rdl` | this branch, now | Why |
|---|---|---|---|---|
| variable | `x` | `$x` | `x` | rule 1. The `.rdl` converter prints bare names; `$` stays inside ast-grep patterns, where it is ast-grep's |
| types | `number`, `symbol`, `unsigned`, `float` | `int`, `sym`, `node`, `flat` | `number`, `symbol`, plus `node`, `span`, `loc` | rule 1. `int`→`number`, `sym`→`symbol`; `node` is Rerail's own and stays |
| comment | `//`, `/* */` | `//` | `//`, `/* */` | rule 1. 0037's `%` examples are not valid in either |
| constant set | a relation of facts | `.kindset k = "a" \| "b"`, `{"a", "b"}`, `$s in {…}` | a relation of facts: `scope("module"). scope("class_definition").` | rule 1. One kind of set, joinable, nameable; the planner sees that it is constant |
| count | `n = count : { a(x), b(x) }` | `$n = count { one atom }` | Soufflé's | rule 1. A body of several atoms gets a helper relation in the engine, not in the rule |
| sum | `s = sum v : { … }` | — | Soufflé's | test-budget needs it |
| lattice | `.lattice flat<> { Bottom -> …, Top -> …, Lub -> …, Glb -> …, Leq -> … }`; a column `v: flat<>` | `.lattice cv(n: node; v: flat)`, `merge @join widen k`, `top` | Soufflé's declaration and column form. Soufflé has no widening: wolfeel's `widen k` stays, after the declaration, as the one non-Soufflé word | rule 1. Soufflé's grammar (`src/parser/parser.yy`, `src/ast/Lattice.cpp`): `.lattice NAME<> { OP -> arg, … }`, operators Bottom, Top, Lub, Glb, Leq. How `Lub`'s functor argument is written is still to check against Soufflé's tests |
| demand (magic sets) | `.pragma "magic-transform" "found, cv"` | `.demand found`, `.demand *` | the pragma | rule 1. wolfeel also transforms through gated negation, which Soufflé does not; same spelling, wider semantics |
| text test | `match(re, s)`, `contains(sub, s)` | `@glob($s, p)`, `@prefix`, `@suffix`, `@contains`, `@word`, `i` forms; alone means `= 1` | `glob(p, s)`, `prefix(p, s)`, `suffix(p, s)`, `contains(sub, s)`, `word(w, s)`, `iglob`, `iprefix`, `isuffix`, `icontains`, `iword` | rule 3: built-in constraints in Soufflé's argument order. Sets of patterns come from a relation: `cache_pat(p), glob(p, s)` |
| regex | `match(re, s)` | none (0037) | `match(re, s)`, captures `(?<x>…)` bind `x` | departure from 0037, below |
| typed text | — | `text`, `name` (dotted), `str`, `strprefix`, `comment`, `op`, `intval` | the same names; `string_value` is now `str` | 0037. `value` (a literal's normalised value, numbers too) stays beside `str` |
| dotted join | — | `@cat(a, b)` = `a.b`, reversible | `@dot(a, b)`, and `split_dot(q, m, x)` for the reverse | rule 2: Soufflé's `cat` is plain concatenation |
| ancestor of a kind | — | `inside($n, $a, kindset)`, `has($a, $n, kindset)`, `nearest($n, $a, set)` | `inside(n, a, set)`, `has(a, n, set)`, `nearest(n, a, set)`, plus `within(n, a)` (any ancestor) and `within_stop(n, a, stop)` | 0036: tape built-ins. `set` is a relation of kinds or one kind as a string |
| syntax error | — | `syntax_error(d)` | `syntax_error(d, line, msg)` (was `parse_error`) | wolfeel's name; Data-layer-rule-expressivity's columns (CPython's line and message) |
| findings | — | the fixed relation `violation(rule, clause, code, path, line)` | `.violation V(at, …) "msg"` | `.violation` lowers to that relation: rule = policy, clause = V, code and line from the key |
| scope | — | generated `in_scope`/`out_scope` with `@glob` | `.scope dirs …`, `.scope tests false` | the directive lowers to the same globs |
| matcher atom | — | `ast($D, $N, { json })` | ``n ~ `pattern` `` | both stay: `~` for patterns, `ast(…)` for an ast-grep rule object the translator has not lowered |
| unknowns | — | paired relations (`call_open`), `must_`/`may_` | `must` / `may` | the keywords lower to wolfeel's paired relations |
| include | `#include` | none (each file repeats the base `.decl`s) | `#include` | rule 1 |

## Text: globs first, regex where a glob cannot say it

Decision 0037 removes regex from rules: globs over typed text, run once per distinct string. This branch adopts
the globs, the typed text relations and the execution, and keeps `match` for what globs cannot express.

**Adopted.**

- The glob family above. `*` is any run without `/`; `**` any run; `?` one character other than `/`;
  `[A-Z]`, `[!_]` a class; `{a,b}` alternatives; `\` escapes. On a name, which holds no `/`, `*` and `**` agree.
- Execution: each distinct string is tested once, for all rules together. Exact sets become hash sets of symbol
  ids, affix sets Aho-Corasick automata, other globs small DFAs; rules join on integers.
- The translator prints a glob test wherever the source regex is exactly one, and `match` nowhere else
  (`site/textglob.py`). It never guesses: a construct whose glob reading differs for some string, such as `\w*`,
  a negated class or `.+`, keeps `match`. Over this branch's 930 translated rules and hand ports there are 2,123
  distinct regexes:

  | Becomes | Distinct |
  |---|---:|
  | `glob` (exact strings, sets, path and name shapes) | 1,868 |
  | `prefix`, `suffix`, `contains` | 64 |
  | `iglob`, `icontains` (case-insensitive) | 17 |
  | `match` (regexes a glob cannot state, including 27 with captures) | 174 |

  Each conversion was checked by fuzzing: strings generated from the glob and at random, compared with
  `re.fullmatch` on the original. No pattern disagreed.

**Kept: `match`, in a restricted dialect.**

- **Captures.** 27 regexes bind variables: SEC codes in a comment, the `l2070` of a layer path, a signature in a
  script line. A glob tests; it cannot bind.
- **Language packs.** 0037 measured the data-layer pack. The other packs keep a regex here and there, spread
  across them (Python 50 distinct, React 28, JavaScript 21, Rust 17, PHP 12, the rest under 10). Most are
  small grammars: the syntax of a number literal (`0[xbo][0-9A-Fa-f_]*_(?:8|16|32|64)`), runs of a character
  class, case-insensitive inline groups.
- **Text checks.** `.scope text` documents (Markdown, signatures in source lines) are text by definition.

The dialect is the one Pulling it all together compiles: Rust `regex` without look-around or back-references,
ASCII `\b`, so every pattern is a DFA. Captures are extracted after a match, once per distinct string, never per
node. The checker warns when a `match` pattern is exactly a glob. That keeps 0037's point, that most text tests are
not regexes, without losing the ones that are.

**One correction to 0037.** It says `\w*` on a name becomes `*` exactly. Its `name` relation holds dotted text
(`a.b.c`), and `\w` does not match `.`, so the conversion is exact only on undotted names. The translator here does
not make it.

## Taken from Pulling it all together

- **Classifying `match`:** a literal set becomes an index join, a general pattern is tested once per distinct term,
  and nodes with no term fall back to a text scan. That is the lowering for `match` and the glob family alike.
- **Named derived facts** (decision 0005): a library relation must name a concept, have at least two users, and
  have at most as many rows per document as nodes. The libraries (`python/ast.rr`, `signing.rr`) already follow
  the second rule; the other two are now stated in data-layer/DESIGN.md.
- **Stop rules and field edges** (`stopBy: <rule>`, `has … field`, `nthChild.ofRule`): already `within_stop` with
  a helper relation, `field(p, f, c)`, and `child(p, c, i)` with a count.

## Taken from Data-layer rule expressivity

- **Not taken: the isolated CPython worker.** That branch gets `@unparse`, `@ast_dump`, `str_part` and
  `syntax_error`'s line and message from `python -I -S` running `ast.parse`. Rerail runs no CPython, in any form
  (decided 2026-11-05). `str_part` is decoded natively by the literal rules; `syntax_error` comes from a native
  parser that follows CPython's grammar (Ruff's), keyed by line; `@unparse` is a native printer for messages only.
- **`@sha256(s)`** was already in core. Its `body_sha256` and `dump_sha256` fields hash `ast.dump` text; here
  the allowlist that stores them is re-signed once with `@tree_digest` instead (there is no `@ast_dump`).
- **Lambda parameters.** Its batch-nine audit found lambda parameters attributed to the enclosing `def`.
  `python/ast.rr` does not have that defect: `sig_param` takes a function's parameters from its own `parameters`
  field, and `py_frame` treats a lambda as its own frame.
- **Ordered first-match reachability.** It reproduces the scripts' depth-first search, in edge order, with a
  visiting set and a memo shared across roots (ordered_first.rs). This branch now has the same semantics as two
  built-ins, `dfs_first` and `dfs_first_memo` (data-layer/DESIGN.md, addition 7), computed set-based except
  inside cycles. LA-011's "first in visitation order" needs no search: it is `min` over the visit index.
- **Runtime observations as JSON parameters** become run artifacts (data-layer/RUN-FACTS.md). One difference: it
  says recorded observations never replace fresh ones; here an artifact may be reused when the digest of its
  inputs is unchanged.

## What the other branches would change

| Branch | Change |
|---|---|
| wolfeel | the `.rdl` converter prints bare variables, `number`/`symbol`, relations of names instead of `{…}` sets, and `glob(p, s)` order; the parser accepts both orders during the move. `@cat` becomes `@dot`. `.demand` becomes the Soufflé pragma. `syntax_error` gains line and message |
| Pulling it all together | none: its lowering is under the language, and `match` classification is reused |
| Data-layer rule expressivity | its SQL stages map to rules; `rx_extract(text, re, n)` is `match` with a named capture; `verify_ed25519` is `@ed25519_ok`; `code`/`message` output columns are `code {c}` and message variables |
