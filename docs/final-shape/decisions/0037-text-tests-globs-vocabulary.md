# 0037: Text tests are globs over typed text; they run once per distinct string

Status: accepted and built (2026-11-05, the user; list step 30)

## Intro

Many data-layer rule clauses look at text:

- the name of a function or variable (`_..._cache`);
- the path of a file (`impl/**/tests/**`);
- the value of a string literal (`"SELECT ..."`);
- sometimes a whole line of code (`dialect.name == "postgres"`).

The source rules write all of these as regexes. Rules here are Datalog and contain no regex
(decision 0036), so we need two things:

1. a small, readable way to write a text test in a rule (the syntax);
2. a way to run those tests fast over 9,010 files and 6 million names (the execution).

This record proposes both. In short:

- **Rules say what to match, in globs.**
- **The engine never tests a row.** It tests each distinct string once, for all rules together,
  and the rules join against the result.

## What the text tests are (measured)

The Pulling-it-all-together workstream has already rewritten the data-layer pack
(`391457bc2`, "Rewrite heavy data-layer rule shapes"):

- Of the 112 clauses that searched file text, 108 are now syntax clauses. The other 4 test only
  the path.
- Fixture parity was exact for every changed rule.

Its research (`research/regex.md` §1.2) found that **none of the 113 file-text clauses is really
textual**. Each one described code structure, comments or docstrings as text.

The regexes left in the rewritten pack (`classify.py` over all 60 rules: 1,307 distinct, 7,115
uses) group like this:

| Shape | Distinct | Example |
|---|---:|---|
| exact set of strings | 684 | `^(?:compile\|lower)$` |
| prefix / suffix / contains set | 355 | `_async$`, `^Sql` |
| a name shape with character classes or gaps | 67 | `^_\w*cache\w*$`, `^[A-Z]`, `^apply_.*_effect$` |
| a path | 54 | `(?:^\|/)tests/`, `^(?:api/)?impl/` |
| code written as text | 49 | `dialect\.name\s*==\s*["']postgres` |
| words inside a string | 18 | `(?i)\b(?:SELECT\|INSERT)\b`, `(?i)ON\s+DELETE` |
| long exact sets (cut off in the census) | 87 | `^(?:execute\|execute_async\|…)$` |

So:

- about 80% are exact strings or prefix, suffix or contains sets;
- most of the rest are glob-shaped names and paths;
- 49 are code structure, which the rules should state as structure;
- 18 look for words inside string values.

## Decision, part 1: syntax

### Text comes from typed relations

A rule never reads raw file bytes. It reads text through relations whose second column is a
string of a known kind:

| Relation | Holds | Kind of text |
|---|---|---|
| `name(n, s)` | an identifier, or the dotted text of an attribute or dotted name: `a.b.c`, without spaces | name: word characters and dots |
| `str(n, s)` | the decoded content of a string literal: no prefix, no quotes | any text |
| `strprefix(n, p)` | a string's prefix letters, lowercased (`f`, `rb`, `""`) | letters |
| `comment(n, s)` | a comment's text, without the `#` | any text |
| `doc(d, p, m)` | a file's path `p` and module name `m` | path, module |

Knowing the kind of text lets the engine simplify a test. A name holds only word characters and
dots, so `_\w*cache\w*` on a name is the glob `_*cache*` exactly.

### One pattern language: globs

```text
$s = "conn"                          % equal
$s in {"conn", "db", "cursor"}       % one of a set (a constant relation)
@glob($s, "_*cache*")                % * any run, ? one character
@glob($s, "[A-Z]*")                  % [A-Z] a class, [!_] not in it
@glob($s, "{*Step,*Intent,ServerPlan}")   % {a,b} alternatives
@glob($p, "impl/**/tests/**")        % in a path, * stays in one segment and ** crosses
@iglob($s, "*_info")                 % ignoring case
@word($s, "SELECT")                  % a whole word in prose: "SELECT x", not "SELECTED"
@iword($s, "on delete")              % words in sequence, any spaces between, any case
```

- `@prefix`, `@suffix`, `@contains` and their `i` forms stay as readable spellings of `x*`, `*x`
  and `*x*`.
- Any test takes a set in place of one pattern, meaning "any of these":
  `@iword($s, {"SELECT", "INSERT", "UPDATE", "DELETE"})`.

Examples from the pack, before and after:

```text
% connection-caches: a private attribute whose name mentions a cache
%   regex:  ^_\w*(?:branch_tip|revision_tip|branch_cache)\w*$
cache_attr($n) :- name($n, $s),
    @glob($s, {"_*branch_tip*", "_*revision_tip*", "_*branch_cache*"}).

% backend-type-strings: dialect.name == "postgres"
%   regex:  dialect\.name\s*==\s*["'](postgresql|sqlite|…)["']
backend_test($c) :- node(_, $c, comparison_operator), op($c, "=="),
    child($c, $l, 0), name($l, "dialect.name"),
    child($c, $r, 1), str($r, $v), $v in backend_names.

% sql strings: a string whose value starts a SQL statement
%   regex:  (?i)^\s*(?:CREATE|ALTER|DROP)\b
ddl_string($n) :- str($n, $v), first_word($v, $w), @iglob($w, {"create", "alter", "drop"}).
```

The last example shows the limit of the language: "starts with a word" is `first_word`, a derived
relation, not a pattern feature. The language stays small, and each structural need gets a named
relation.

### What is not in the language

- **No regex anywhere.** There are no `\s*`, no back-references and no counted repeats.
- **Code shapes are written as structure.** The 49 code-as-text regexes become joins over `node`,
  `child`, `name` and `op`, as the sibling rewrites already do. Matches inside comments and
  strings then go away; regex.md §1.3 measured those as false positives (18% of raw matches).
- **Line facts are kept for the two clauses that need them** (regex.md class b): `line($n, $l)`
  joined with `comment`, for `# noqa: async-executor`, and the file's last line for the
  1,500-line limit.

### The converter

The converter turns a regex into a glob only when the result is exact for that column's kind of
text:

- `^`, `$` and `.*` become ends and `*`;
- `\w*` on a name becomes `*`;
- character classes and alternation carry over directly.

Anything else stays a named gap for a hand rewrite. It never guesses.

## Decision, part 2: execution

### Every text column is a vocabulary

- Text values are interned as symbols. The engine already does this ("atoms mode", `sym`).
- Measured on nera-data-layer (regex.md §2.2):
  - 6.0M identifier occurrences;
  - **231,862 distinct names**, 26 times fewer;
  - interning takes 0.22 s on one thread, at ingest.
- String values, comments and paths are interned the same way. There are 9,010 distinct paths.

### Tests run once per distinct string, for all rules together

1. **Collect.** The planner gathers every text test in the program (all 60 rules) and groups them
   by column kind.
2. **Recognize alternatives.** It normalizes the tests:
   - spellings become globs;
   - sets become alternatives;
   - a relation whose rules differ only in a test on the same variable, such as
     `p($s) :- @glob($s, "a*"). p($s) :- @glob($s, "*b").`, becomes one test with two
     alternatives.
3. **Compile each group into the cheapest matcher that is exact:**

   | Tests | Matcher | Measured |
   |---|---|---|
   | exact strings | a hash set of symbol ids; strings missing from the vocabulary match nothing | 396 of 444 identifier patterns (regex.md §2.1) |
   | prefix, suffix, contains sets | one Aho-Corasick automaton (multi-string search) per group | — |
   | words | Aho-Corasick over words, plus a word-boundary check | — |
   | other globs | small DFAs (state machines), at most about 64 patterns each | the hybrid of hash + one DFA: build 0.44 s, 3.0 MB, run 0.11 s (M) |

   - DFAs are split into groups because one big DFA blows up: 639 patterns in one DFA took 676 s
     and 201 MB to build.
   - Case-insensitive tests run over each term's lowercased form, computed once.
   - A DFA is an internal step that the glob compiles into. Rule authors never see a regex.
4. **Run each matcher over the vocabulary,** chunk at a time on the host (SIMD, DFA) or the GPU,
   never per node. The output is a relation over symbol ids: a bitmap, or a list when sparse.
5. **Rules join on integers.** `name($n, $s), @glob($s, "_*cache*")` becomes a join of `name` with
   the result relation `t17(sym)`:
   - in SQL, an integer join;
   - in the tape, a `TEXT_ID` bit lookup (spec-transducer opcode 0x0c), one load per node.

### What this buys

- **A selective test drives the plan.** The engine starts from the few matching symbols and
  reads their postings: the value index.
  - The AST-index spike's design index (§1 item 7) found the value index mandatory: 15.8 times
    faster literal seeks on SQLite.
  - Its text-candidate hierarchy (§4b) puts "exact token / any-of: vidx" first.
- **Negation is a complement over the vocabulary,** not a per-row test.
- **Warm runs evaluate only new symbols.** The vocabulary only grows, and results are cached by
  each matcher's digest. A warm run costs about zero.
- **It removes a per-row callback.** Today `str_test` is a SQL function called once per row.
  - The lessons ledger's row 17 forbids per-row callbacks in the SQL phase: a dynamic regex per
    term once cost 14.3 s.
  - Row 18 forbids replacing regex with LIKE or INSTR string games; structured facts with exact
    equality are the rule. Code-as-text clauses therefore become structure, not substring tests.

### Expected cost (D, from the sibling's measurements)

| Run | Matching the vocabulary | Lookups in rules (6M at about 1 ns) | Total |
|---|---|---|---|
| Cold | 0.11–0.25 s | about 6 ms | **about 0.1–0.3 s** |
| Warm | about 0 (new symbols only) | about 6 ms | **about 0.01 s** |

That fits the gate's 1–2 s cold and milliseconds warm.

## Steps

1. **Adopt the sibling's rewritten pack** as a recorded COPY from Pulling-it-all-together, then
   rerun the converter and the replay.
2. **Add the text relations** `str`, `strprefix` and `comment`, and dotted text in `name`.
3. **Syntax:** sets as patterns, `@glob` and `@iglob` on any text, `@word` and `@iword`. Teach the
   converter the exact regex-to-glob cases.
4. **Execution:** the vocabulary matchers and result relations. Drop the per-row `str_test`.
5. **Measure** on nera-data-layer `2b1504ad`: CPU cold and warm, memory, and key sets against the
   recordings.

## Not chosen

- **A regex or "text pattern" built-in.** It matches the recordings most easily, but it puts a
  second language into the rules, and per-node text scans are what cost 9–17 s today.
- **A trigram index.** The spike index (§4b) revised this decision repeatedly and ended at "no
  for this corpus size". After lowering, regex.md §3 found it has no consumer in this ruleset.
- **One combined automaton for every test.** It is slower: a `RegexSet` took 92.6 s against 0.52 s
  pattern by pattern (regex.md §1.1), and determinization blows up past about 450 patterns.

## Built (2026-11-05)

- **Syntax** (`logic::strings`, `logic::front`):
  - `@glob`, `@iglob`, `@word`, `@iword`, `@equals`, `@prefix`, `@suffix`, `@contains`, and their `i` forms;
  - each takes one pattern or a set `{a, b}`;
  - `$s in {…}` and `!@test(…)`.
- **Text relations** (`facts::vtab::syntax`): `name(n, s)` (an identifier, or an attribute's dotted text),
  `str(n, s)` (the content between the quotes, where there is no interpolation) and `strprefix(n, p)`.
  `comment` is not built yet; 4 clauses need it.
- **Execution:**
  - `strings::as_relations` turns each test whose string comes from a text relation into an input relation
    `strtest_<k>(s)`, one per distinct (test, pattern set).
  - Each batch fills those relations from its distinct strings. A per-symbol bit cache tests each string once
    per run.
  - Tests on strings a rule computes itself (`@cat`) stay per row in SQL.
- **File-local split** (`typing::file_local`): test relations, and derived relations that only filter strings
  from text relations, count as file-local. Otherwise a rule that reads them would run cross-file and hold
  whole base relations in memory. `syntax_error`'s column is a document column.
- **Converter:**
  - It reads `rules-ast`, the sibling's rewritten pack.
  - A regex becomes globs only where the result is exact for its kind of text. A test,
    `regexes_become_exact_globs`, compares each glob set with its regex on sample strings.
  - String-literal regexes become `strprefix` and `str` tests.
  - Two forms are recorded as approximations:
    - a module-text search for names becomes a search of the file's names;
    - `_*` becomes at most three underscores.
- **Coverage and replay:**
  - Clauses converted: 165/562 before, 315/650 now.
  - 155 recorded events now have all their clauses converted, and 153 of them give exactly the recorded
    findings.
  - One miss is a mention inside a string literal, which the structural rule ignores by design (§1.3 of the
    sibling's regex.md).
  - The other miss is a clause that is on only under `oracleAnalyzer`.

### Measured on nera-data-layer `2b1504ad`

Full corpus: 9,010 files, 28.2M nodes, 104 chunks. CPU only (user + system), at load 18–90.

| Rule (all clauses converted) | Text test relations | Strings tested | Text test CPU | Run CPU | Peak RSS |
|---|---:|---:|---:|---:|---:|
| admin-async-purity | 10 | 594,447 | 0.50 s | 49.1 s | 227 MB |
| plan-scope-side-effects | 19 | 594,447 | 0.42 s | 61.7 s | 330 MB |
| serverplan-grammar-surface | 9 | 594,447 | 0.34 s | 52.5 s | 239 MB |
| removed-compat-surfaces | 16 | 594,447 | 0.50 s | 51.0 s | 277 MB |
| versioned-lifecycle-legacy-surfaces | 10 | 594,447 | 0.33 s | 43.4 s | 318 MB |

- **Text tests.** They cost 0.3–0.5 s cold per rule, near the 0.1–0.3 s estimate given the load.
- **The rest of each run.**
  - Most of a run is parsing (13–24 s) and the rules' local plans (17–40 s), not text tests.
  - Each rule is measured as its own run, so the parse is paid every time.
- **The split fix.**
  - Before it, admin-async-purity peaked at 2,245 MB and plan-scope-side-effects at 2,153 MB, both with a
    cross-file part.
  - After it, every rule above stays under 1 GB.
- **async-sync-boundary** (63 clauses) now runs past the 2-minute cap. Its local plan is about 19,000
  statements over 104 chunks. That is open: its plan shape, not text tests.
