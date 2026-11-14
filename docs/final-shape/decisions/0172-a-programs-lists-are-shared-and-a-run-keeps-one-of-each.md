# 0172: A program's lists are shared, and a run keeps one of each

Status: built (2026-11-13). Follows decision 0166 (interned names). Amends decision 0162's walk.

## Intro

After decision 0166, reading the 141 stored program texts of a warm dl1 run made 260k allocations. Almost all of
them were the program's own lists: an atom's arguments, a rule's body, a declaration's columns, a fact's row. Each
was its own `Vec`.

Most of those lists are copies. The run's programs are made from the same rule files, so the same rules, columns
and rows come back text after text:

| list | made | distinct in each text (summed) | distinct in the run |
|---|---|---|---|
| argument lists | 90.5k | 31.9k | 7.3k |
| declarations' columns | 52.5k | 29.4k | 1.3k |
| fact rows | 26.9k | 26.4k | 7.2k |
| rule bodies | 22.4k | 21.8k | 10.9k |

So the lists are shared, as names are, and each list is made once in a run.

```
// before: two programs that both say  p(x, y) :- e(x, y).  hold two head lists, two bodies, two argument lists
// now:    they hold the one body, whose atom holds the one argument list; a clone of either program copies none
```

## Decision

### 1. A program's lists are `front::List`

`List<T>` (in `front::name`, beside `Name`) is an `Arc<[T]>`, with no allocation for the empty list. A clone is a
count. A copy of a program, which several passes make, copies no list.

- `Atom::args` and `Term::Call`'s arguments are `List<Term>`.
- `Rule::body` is `List<Literal>`.
- `Program::decls`' columns are `List<(Name, Name)>`.
- A fact's row, in `Program::facts` and `program_facts::Facts`, is `List<Value>`.

A `List` reads as a slice. A pass that changes one in place calls `make_mut`, which copies the list first only when
another holder shares it. A change of length goes through `edit` or the `Vec`-like `push`, `insert`, `remove`,
`retain`, `truncate` and `extend`. Each of those makes the list again, once, so a pass that builds a list item by
item builds a `Vec` and makes the list at the end (`plan::corpus::matchers` does). `retain` makes nothing when it
keeps every item.

### 2. A run keeps one of each list: `front::Lists`

`Lists` is a table of distinct lists, one for each kind. `run_observed` makes one at the run's start. Every program
the run reads from a stored text takes its lists from it:

- the prepared program (`prepared::lookup`);
- the combined program (`combined::lookup`);
- the plans' rewritten programs (`choices::Durable`).

All three reach `program_text::load` or `read_in`. The table goes when the run ends. It is not a static, so a long
process keeps no list of a run that is over. `program_text::read` without a run makes a table of its own.

A table is hash-consed, and shallow. A list's items are hashed and compared by the addresses of what they hold:
each name (interned, decision 0166) and each inner list (the table's own). Two lists made through the table are
equal exactly when their items' parts are the same. So a body is found without walking its atoms' arguments
again. An item holding a list made elsewhere is found less often, never wrongly: a hit means every part is the
same.

### 3. The walk gathers each list on a stack, and takes it from the table

`program_text::from_tape` gathers each list's items on a stack for that kind. Nested lists, such as a call's
arguments inside an argument list, are gathered above. The finished list is looked up in the table. A list met
before is that list, and makes nothing.

Within one text, an argument list or a declaration's columns whose text was met before is taken again by its text,
without walking it. The same text is the same list.

The stacks are kept in the run's `Lists` between reads, so their room is made once a run. Declarations are gathered
and their map built at once (full B-tree nodes). The text's maps are sized from its node count.

## Measured

Same set as decisions 0162 and 0166: the 141 texts (6.18 MB), a release build (opt-level 3) of the readers alone,
one `Lists` a pass as a run makes. Counts are per pass, whole-process counts less a run that only reads the texts.

| | instructions | cycles | walk allocations |
|---|---|---|---|
| decision 0166 | 1,186 M | 250 M (load 44–55) | 260k |
| shared lists, deep hashing | 1,227 M | | 87.5k |
| shared lists, shallow hashing (this) | 1,062 M | 304 M (load 82) | 81.8k |
| of which the walk alone | about 603 M (0166: 726 M) | | |

That is 10% fewer instructions than decision 0166, and 39% fewer than the statement reader (1,752 M). The cycles
were taken at very different loads and say little.

Hashing each list deeply (each name's bytes, each inner list again) cost more instructions than the allocations it
saved. Hashing shallowly took that back, and more.

What the 81.8k allocations of a cold read are:

| | allocations |
|---|---|
| the run's distinct lists | 37.2k |
| names new to the process (decision 0166's interner) | about 9.4k |
| the rest of the program's form | about 35k |

The last row is what a second read of all 141 texts, its names and lists all met, still makes: 35.2k. It is mostly
the program's other `String`s, which are not names yet:

- inputs and outputs, 8.9k;
- the facts' relations, 2.8k;

plus their vectors, and the maps' nodes. Making `inputs`, `outputs` and the facts' keys `Name`s is the next step.
It is about 1,150 uses, so it is not taken here.

End to end, dl1 (the 44 data-layer policies) on a fresh store, debug build, CPU, strict SQL, at load 8 to 13: a
first run took 189.5–193.5 G instructions and 55.6–56.5 G cycles; warm runs took 83.4–83.5 G and 25.7 G (decision
0166: 85.3–86.4 G at load 34 to 42). Every run gave the same 48 findings. These are debug numbers and decide
nothing here.

## Tests

- `front::name::list_tests`: a shared list is copied before it changes, and only then. A run's table finds a list
  by its parts.
- `program_text::tests::a_runs_programs_share_their_lists`: two programs read through one table share their
  columns, rows, bodies and arguments.
- The tests of decisions 0162 and 0166, and the passes' tests, which now build and change `List`s.

## Prior work consulted

- **Decision 0166:** interned names, which the shallow table's hash stands on.
- **Decision 0162:** the walk.
- **Decision 0155:** a stored program is its `.rr` text. Here the texts' common parts become shared lists when they
  are read.
- **No static memo maps.** The table is the run's, made and dropped with it, not a process static.
