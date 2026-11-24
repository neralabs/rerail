# 0668. A regex's capture group on the device is a Pike VM over its NFA

Status: in progress (2026-11-24). The function runs on the device; the `ask_c` component that needs it is next.

## Intro

Decision 0661 left one recursive stratum of the 10-file django run on the CPU: `ask_c_found_by_bbf…`, the C include
resolution of `docs/rule-syntax/lib/c/includes.rerail` under demand (`ask_`). Its rules split paths with a regex's
named groups:

```
c_inc_try(c, f, i, 0, p) :- …, match("(?<d>.*)/[^/]*", f), p = cat(d, "/", n).
c_inc_found(c, f, i, r, p) :- c_inc_try(c, f, i, r, p), match("(?<dd>.*)/(?<bb>[^/]*)", p), dir_entry(dd, bb, 0).
```

`rr::lower` makes each group a function: `d = @rx_group(f, "^(?:(?<d>.*)/[^/]*)$", "d")`, after the regex's test. The
CPU's `strings::group` reads it from Rust's `regex` crate (`Regex::captures`): leftmost-first, the group's range of
the text's whole match, or no value when the regex does not match or the group took no part. The device's regex
automata give a test's verdict (a DFA) or a match's span (`replace_all`'s forward and reverse DFAs), never a group's.

## What was built

### The regex's own NFA, as a program

`text::table::group_program(pattern, name)` compiles the pattern with `regex_automata`'s Thompson compiler, which
the `regex` crate itself is built on, and writes its states as a program in the automata block (decision 0478 step
2c), once per pattern and group:

- a header (states, the group's start slot, the anchored start state, the closure's stack at most);
- four words a state: a byte range, sparse ranges (a dense state as its runs), a union's alternates in priority
  order (a binary union as two), a capture (the group's start or end, or another group's, which only passes), a look,
  a match, a fail;
- the ranges and alternates after them.

Refused, named, with the function then the CPU's: a regex not anchored at the text's start (the device starts at the
text's start alone; `rr::lower`'s are always anchored), a look other than the text's start and end, a group the
regex does not name, a program past 2,048 states.

### A Pike VM in the row's stage

`rx_group_range` (`join_stages.uglykl`) walks the program over the row's text as `regex_automata`'s `PikeVM` walks
an NFA:

- A thread list holds each state at most once a position (a sparse set), in priority order, with the group's two
  slots.
- The epsilon closure is depth first in priority order, from a stack in the group's scratch. A capture of the group
  saves its slot and restores it when the search backs out.
- A byte moves each thread in order. A match cuts the threads after it, and the last match found is the
  leftmost-first one.

One lane of the SIMD group walks; the group's lanes then copy the range into the arena as any text function's range
(`TEXTFN_RX_GROUP`, a `ROW_MAP` whose automaton is the program). Every loop is bounded by the text, the states or the
stack. No row where the regex does not match or the group took no part, as on the CPU.

A group read only as positive atoms' keys (`c_inc_found`'s `dir_entry(dd, bb, 0)`) is looked up as an atom
(`ROW_FIND`, decision 0666), as other text functions are. An atom keyed by it alone is then a probe by value too;
before, the compile refused any atom keyed by a made text alone.

## Tests

- `a_groups_program_captures_as_regex_does` (`text::table`): the program walked as the device walks it gives
  `regex`'s group over the C includes' splits, a lazy repeat before a greedy one, overlapping alternatives, a group in
  a repeat, a group that takes no part, multi-byte classes and padded texts. The refusals are each refused.
- `text_functions_on_the_device_are_the_cpus` (`setfix::device`) gained six groups made into the arena (one made
  into a cat after it) and two as atoms' keys, one an atom's only key. Rows and every step's count are the CPU's.
- The device crate's 91 tests pass on Metal (M1 Max), and its 90 on the CPU's device (a Linux host).
