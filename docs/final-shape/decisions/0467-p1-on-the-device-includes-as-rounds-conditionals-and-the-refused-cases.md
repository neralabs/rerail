# 0467: P1 on the device: includes as rounds, the conditionals, and the cases refused until now

(0467 on Go-C-and-C-support and here, 0487 on rr-align at roll-up 21, 0501 on investigate-bend-vs-datalog, renumbered past 0499 at its merge into that hub; 0467 again here at roll-up 26, Q445.)

Status: accepted (2026-11-19); being built in the steps below. Builds 0436's evaluator passes 2
to 4 ("includes", "definitions", "conditionals") and the step 0448 left open ("until P1's
conditionals and includes are on the device"). Numbered past the highest in any worktree (0482).

The user (2026-11-19), after being told the device evaluator was a complete macro expander but
not a complete preprocessor: "please go back through this session history and the decisions,
and tell me the correct design for including which we already agreed to. Then build items 1, 2
and 3". The items:

1. `#if`, `#ifdef` and `#elif` decided on the device. Until now the spike decided them on the
   host (`stream::parallel::p1_of`).
2. `#include` resolved on the device, in rounds, with the host loading files between them.
3. The cases refused until now: `#line`, a call across a stretch's end, `__COUNTER__`,
   `__INCLUDE_LEVEL__`, `__FILE_NAME__`, `__BASE_FILE__`, `_Pragma`, and the `__has_*` feature
   tests.

## Intro

The device runs the evaluation's later passes already: the names, the definitions' table, the
text's calls, their expansion and the output (0436 passes 1, 3, 5 to 7; 0448; 0468). What it
reads from the host is P1's answer, the unit's layout (`pp_expand::Layout`): which file
instances the unit has, in what order, at which slots, and which of their directives and
stretches are active. This decision moves P1 itself onto the device.

## The agreed design, from the record

- 0436 (the user, 2026-11-18): the evaluator runs "includes, as rounds; the definitions'
  lookups; the conditionals; …". "The CPU only loads files and their tapes between include
  rounds." Pass 2: "each `#include` matched to the unit's file table, with instances laid out by
  a scan per depth. Files not yet loaded go back to the CPU, which maps them, gets their tapes
  (kept, or parsed now) and starts the next round."
- The user, the same day: "#includes, which are a recursion"; "you can do a Metal include pass
  first if you want, this will need to be interleaved with CPU work to actually memory map or
  load the files".
- 0482, P1: a round reads the state the last one left (the include tree, each instance's active
  groups, the pragmas its text made) and lays the unit out (sizes bottom-up, first positions
  top-down), sorts the active definitions by (name, position) with `push_macro` and `pop_macro`
  as rows, evaluates each test at its position, each alone, takes each chain's first true arm,
  and resolves each active include at its position, dropping those an earlier `#pragma once` or
  `#import` of their file skips. The rounds stop when one leaves the state it read; every
  dependency points backwards, so that fixpoint is the serial reading.
- 0436's next step, as written: "the instances' layout (sizes bottom-up, bases top-down, a scan a
  depth), the definitions' table (rows sorted by key and position, a search per use,
  `push_macro`/`pop_macro` matched by depth), the tests, the chains' first true arm and the
  groups' activity, and the includes (each resolved by the host between rounds, the files it
  loads added to the unit)."
- The spike stays as a cross-check of the passes' arrays, and the serial pass is the reference
  for the output (the user, 2026-11-18).

## Decision

### 1. Rounds: the device decides, the host loads

- An evaluation is P1's iterations, then the expansion, in one command, as now (R5, R9 of 0436,
  0448). P1 iterates on the device until its state stops changing, a bounded loop on the
  persistent grid, its iterations separated by grid barriers.
- An include whose file the unit's file table does not hold is a **request**: its key, written
  to the arena. At P1's fixpoint, an evaluation with requests stops before the expansion.
- The host then answers each request, and nothing else: it searches the configuration's paths,
  reads the file, gets its tape (kept by content, or parsed now), admits both to the residency,
  and adds a row to the file table (a row that finds nothing too). Then it runs the evaluation
  again, as a new round. The host reads no other answer, and decides nothing.
- A round is the same fixed handful of launches (0468 §4). The rounds are the include tree's
  depth at most, and fewer where one round's requests reach several levels.

### 2. The instance tree and its layout

- An **instance** is a file entered at an include: its file, its parent instance and the
  directive that entered it, its depth. The predefines and the main file are the two roots.
  Instances are appended as they are first entered, keyed by (parent instance, directive, file),
  so an instance keeps its identity from one iteration to the next.
- Each instance has a **child word** for each of its file's directives (the instance its include
  enters, or none) and an **active word** for each of its file's groups.
- The layout, an iteration from the last state:
  - the instances in the tree, by depth;
  - each one's **size** (its directives, its text tokens, and its entered children's sizes),
    bottom-up, a phase a depth;
  - each directive's **shift** (the sizes of the children its earlier directives entered), a
    segmented scan over each instance's directives;
  - each one's **base**, top-down, a phase a depth: a child starts at the slot after its
    directive.
  - Then a directive's slot is `base + d + text_before + shift[d]`, and a stretch's likewise:
    `pp_expand::Layout`'s arrays, now made in the arena.

### 3. The definitions' table

The table the expansion reads already (0448 §2) is made from the layout's active directives.
`push_macro` and `pop_macro` become rows too: after the sort by (key, slot), a pop's value is the
value at its push, pushes and pops matched per name by depth (a segmented scan of +1 and −1, and
a search for the push of the pop's depth). A pop with no push changes nothing; a push makes no
row.

### 4. The tests

- `#ifdef`, `#ifndef`, `#elifdef`, `#elifndef`: a lookup of the name at the directive's slot.
- `#if`, `#elif`: the directive's line is a list of the expansion (0448), fenced at its end, read
  as the serial pass reads an `#if` (`in_if`): `defined X` and `defined ( X )` are answered at
  the line's slot and not expanded, and `__has_include` and `__has_include_next` are operators.
  Its expanded tokens are then evaluated by one thread a test, as `expr::eval` does: 64-bit
  values, signed unless an operand is unsigned, `&&`, `||` and `?:` evaluating only what they
  take, an identifier left as 0, `true` as the dialect has it.
- `__has_include`'s name is matched through the file table, as an include's is (a miss is a
  request). The other `__has_*` tests (`__has_feature`, `__has_builtin`, `__has_attribute`, …)
  are answered from the configuration's table, given as configuration input as the built-ins'
  names are.
- A chain's taken arm is its first true arm (an atomic minimum of the arms' indices). A group is
  active where its arm is taken and its parent group is active (a pointer-jumping pass over the
  groups' nesting).

### 5. The includes

- An active include's name: its header token, or its line expanded as in §4 and spelled as
  `header_of` spells it (`<` … `>` with their blanks).
- Its key: the name's key, whether it is `<…>`, the including file for a quoted name, and for
  `#include_next` the search directory its file was found by. The key is searched in the file
  table; a miss is a request (§1).
- It enters nothing where an earlier `#pragma once` or `#import` of its file is active before
  it, where its file's guard is defined at its slot (clang's multiple-include reading), or past
  the configuration's depth.

### 6. The cases refused until now

- **`#line`** and line markers: a row each (its slot, its line, its file's literal). `__LINE__`
  is the physical line plus the latest row's offset before it (a search), and `__FILE__` that
  row's literal.
- **A call across a stretch's end**: the top-level text is the unit's active stretches in order,
  so a call's arguments read on into the next active stretch.
- **`__COUNTER__`**: each expansion of it in the serial reading's order, ranked by a scan.
- **`__INCLUDE_LEVEL__`**: the instance's depth. **`__FILE_NAME__`**: its file's last path
  component, as a literal. **`__BASE_FILE__`**: the main file's literal.
- **`_Pragma`**: in text, its string destringized into a pragma; a `push_macro`, `pop_macro` or
  `once` it makes is a row of P1's next iteration, as 0482's events are.
- **The `__has_*` tests in text**: answered as in §4.

## Requirements, each held by a test

| | Requirement | Test |
|---|---|---|
| R1 | The layout made on the device from the spike's tree equals the spike's slots. | `the_layout_is_the_spikes` |
| R2 | The definitions' table with push and pop rows equals the spike's `resolve_rows`. | `push_and_pop_are_the_spikes` |
| R3 | Every `#ifdef`-family test, chain and group equals the spike's at the fixpoint. | `the_groups_are_the_spikes` |
| R4 | Every `#if` value equals the serial pass's `if_value`, over shapes covering each operator, the unsigned rules, short circuits, `defined`, `__has_include` and the configuration's tests. | `if_values_are_the_serial_passes` |
| R5 | The include tree equals the spike's: literal and expanded names, quoted and angle, `#include_next`, guards, `#pragma once`, `#import`, the depth's bound, a file not found. | `the_include_tree_is_the_spikes` |
| R6 | The host does nothing between rounds but answer requests, and a round is the same handful of launches. | a guard over the round's runner |
| R7 | Each case of §6 gives the serial pass's tokens. | `the_refused_cases_are_the_serial_passes` |
| R8 | The corpus: every unit, its main file alone given, equals the serial pass, with no refusals. | `the_corpus_preprocesses_on_the_device` (ignored, in slices) |

## Steps

1. The layout on the device from a given tree (§2; R1).
2. Push and pop rows (§3; R2).
3. The `#ifdef` family, the chains and the groups, iterated on the device (§4; R3).
4. The includes with literal names, the file table, requests and rounds (§1, §5; R5, R6).
5. `#if` expressions, `defined`, `__has_include` and the configuration's tests (§4; R4).
6. Expanded include names (§5; R5).
7. The cases of §6 (R7).
8. The corpus from its main files (R8). The spike's P1 becomes a cross-check only.

## Built

- **Step 1: the layout on the device (§2; R1).** `pp_p1.uglykl`, as phases of `pp_setup`
  between the definitions and the visibility.
  - The host gives the tree (`PpNode`: an instance's file, parent, directive, depth, unit) and,
    until step 3, the spike's activity. `pp_expand::Layout` holds no slots any more: no
    `insts`, `dir_slots`, `gap_slots` or `unit_floors`.
  - The device makes them: each instance's words (`dir_first`, `gap_first`: a scan of its
    file's directive counts), its size bottom-up (a phase a depth; a child adds its size to its
    parent's with an atomic add, and writes it at its directive), the stretches' shifts (one
    segmented scan), the units' floors (a scan), the bases top-down (a phase a depth), and
    every directive's and stretch's slot.
  - The spike's `P1Inst` gives each instance's parent, directive and depth.
  - R1: `every_name_sees_the_spikes_definition` compares the device's slots (`Vis::slots`)
    with the spike's, on its shapes and the first 40 corpus units: equal.
  - All 18 evaluator tests pass. The corpus, in batches of up to 300 units: 3,132 units equal,
    the same 3 refused, 4 launches each.
- **Step 2: push and pop rows (§3; R2).**
  - The front reads each `#pragma`'s act from its tape (`pp_pragma_act`: `once`, and
    `push_macro` or `pop_macro` with `( "name"`, its key the string's bytes inside the quotes),
    into `DIR_ACTS`.
  - An active push or pop is a row of the definitions' table, its type in `RTYPES`.
  - After the rows' sort, where the unit has any (`N_STACK`), three phases over the sorted rows:
    each pop's push (a walk back over its name's rows counting the pops between: the stack's
    matching), each matched pop's source (the latest row before its push that gives its name a
    value: a definition, an undef, a built-in, or a matched pop), and a pointer jumping until
    each source is a value's row (at most 32 rounds, each a barrier, ending at the first that
    moves none).
  - A push, and a pop with no push, hold `V_SKIP`: a lookup walks past them, as
    `resolve_rows` drops them. A walk past 256 rows of one name refuses the unit.
  - The spike's `p1_of` no longer refuses push and pop: its `vis` already reads them.
  - R2: `push_and_pop_are_the_spikes`, six shapes (nested pushes, a push of a name not defined,
    pops with no push, across an include, other names between, a malformed push), C and C++:
    every name's meaning equals the spike's. All 19 evaluator tests pass; the corpus is
    unchanged (3,132 equal, the same 3 refused, 4 launches each).
- **Step 3: the `#ifdef` family, the chains and the groups, iterated on the device (§4; R3).**
  - The host gives no activity any more. `Layout` holds the tree and, until step 5, each
    `#if`'s and `#elif`'s value (`if_values`, the spike's, read only at those directives).
  - Each instance has a group word for each of its file's groups (a scan of the front's group
    counts). The first iteration has each instance's own group active, nothing else, as the
    spike's `fresh`.
  - An iteration, all in `pp_setup`: each directive's and stretch's activity from its group;
    the definitions' table from the active rows (flags, scan, rows, sort, push and pop); each
    arm's test where its chain's parent group is active (`#else` true, the `#ifdef` family a
    lookup of the name at the directive's slot, a missing name as the spike reads it); each
    chain's taken arm by an atomic minimum of its true arms' indices; each group's activity by
    a pointer jumping over the groups' nesting (own flag and parent, two pairs of buffers, at
    most 20 rounds); then the groups against the last iteration's.
  - The iterations end at the first that changes no group word; at most 64, past which the
    unit is refused. The rows' table the expansion reads is the last iteration's, made from the
    fixpoint's activity.
  - The stretches' sort moved out of the loop: it depends on the slots alone.
  - R3: `the_groups_are_the_spikes` (six shapes: nested chains, `#elifdef`/`#elifndef`,
    dependent tests in a chain of five, guards, built-ins tested, a nameless `#ifndef`), and
    `lookups_agree` compares every directive's and stretch's activity with the spike's on every
    shape and the first 40 corpus units: equal.
  - All 20 evaluator tests pass. The corpus: 3,132 units equal, the same 3 refused, 4 launches
    each; GPU time 160 ms against 147 ms (M5 Max, under the other sessions' load).
- **Step 4: the includes with literal names, the file table, requests and rounds (§1, §5; R5,
  R6).**
  - The host gives the roots alone (each unit's predefines and main file). The device grows the
    tree: an active include's file is found by its key in the file table, and entering it
    appends an instance (its file, parent, directive, depth, unit), made once for its directive
    word and file, so an instance keeps its index and its words across iterations.
  - Each iteration lays the tree out again: an instance is live where its parent is and the
    parent's directive enters it now (a phase a depth); a dead instance has size 0, its words
    are inactive, its slots `PP_NONE` (sorted last). The stretches' sort moved after the loop.
  - The key (`p1_inc_find`): the name's key (the header's bytes inside `<…>`, or the string's
    inside its quotes), whether it is `<…>`, the including file's index in its unit for a quoted
    name, the search entry its file was found by for an `#include_next` (the main file's is an
    `#include`), then the unit. A table row gives the file, or none found; a miss is a request.
  - An include enters nothing past an earlier active `#pragma once` or `#import` of its file
    (each file's least such slot, an atomic minimum), where its file's guard is defined at its
    slot, or past the configuration's depth (no request then either, as `Pp::include`).
  - The iterations end at the first that changes no group and no include. With requests, or a
    refusal, the round ends before the expansion (`PG_STOP`): `px_expand` and `pp_check` see it
    and return.
  - The host's runner (`c_directives::eval::rounds`): each round places the unit's files
    (resident documents are not placed again), runs it, and answers each request by
    `Files::find` and `Files::read`, adding the table's row; nothing else. R6's guard
    (`the_host_only_loads_files_between_rounds`) reads its source.
  - The stores are sized by P1's rung (`Caps`: a unit's files entered `2 << rung` times each);
    the device refuses a round past them (`P1_ROOM`) and it runs again a rung up.
  - Each unit's first output token comes from the device (`P1_UFIRST`, its first stretch in
    the unit's order): the host no longer knows a unit's words.
  - The `#if` values, until step 5, are the spike's by (unit, slot within the unit): the
    device's slots equal the spike's (R1), so the table is searched by the directive's slot.
  - Refused until a later step: an expanded include name (step 6), `#embed`, `#line` (step 7).
  - R5: `the_include_tree_is_the_spikes`, seven shapes (quoted names by the includer's
    directory and by the search, `<…>`, one name resolving differently from two includers,
    `#include_next`, guards, `#pragma once`, `#import`, a header entered twice, an include a
    later definition decides, a missing file, a self-include to the depth's bound), C and C++.
    `lookups_agree` now starts every unit from its main file and compares, by slot, every live
    instance's directives and stretches (file, index, activity) and every name's meaning.
  - All 22 evaluator tests pass. The corpus, from main files alone: 3,130 units equal, at most
    2 rounds of 4 launches; refused: `#line` 2, a call across a stretch's end 1, an expanded
    include name 2 (step 6; the spike read them on the host before). GPU time 184 ms against
    160 ms: P1's room makes larger arenas, so batches split at 125 units, not 250.
- **Step 5: `#if` expressions, `defined`, `__has_include` and the configuration's tests (§4;
  R4).** `pp_if.uglykl`, as phases of P1's iterations, after the definitions' table and
  before the tests.
  - The host gives no `#if` value any more: `Layout` holds the dialect's operators (the
    `__has_*` tests by their names' keys, from `macros::BUILTINS`, and in C++ the alternative
    tokens, from `stream::ALT`), the configuration's answers (`Config::features`, by the test's
    name's key and its operand's), and whether `true` is 1.
  - Each active `#if`'s and `#elif`'s line is a top of its own, its directive word a stretch:
    its tokens (its `OPERANDS`' leaves) read in place through `IF_NODES`, at its directive's
    slot. The expansion's top-level passes, its queue and its places are the text's, bound to
    the lines (`IF_TOP`) instead of the active stretches (`TEXT_TOP`). Inside the kernel the
    queue is drained a bounded number of times (`PX_DRAIN`), and between tops its slots and the
    arena's tops are reset (`PX_RESET`: the queue is linear).
  - The lines' names are looked up as `in_if` reads them (`p1_if_tokens`): `defined` and the
    operand it reads raw (the token after it, or after its `(` and the one after that) are no
    names; a `__has_*` test whose name means the built-in there, followed by `(`, is marked,
    and its operand (to its `)`, by the depth's close table and a max scan) is read raw.
  - One thread a test evaluates its line's output (`p1_if_eval`), as `expr::eval` does:
    operator precedence over two stacks (no recursion), 64-bit values with the unsigned rules,
    a division by zero carried as a flag that `&&`, `||` and `?:` drop where they do not take
    the operand (the serial pass's `live`), any other error making the test false; integer and
    character constants as `expr::number` and `expr::char_value` read them (phase-2 bytes);
    `defined` answered by a lookup at the directive's slot; the configuration's tests by their
    table (`__is_identifier` 1 where it holds no row); `__has_include`, `__has_include_next`
    and `__has_embed` through the file table as an include's key (a miss is a request, so the
    round ends and the host answers it); C++'s alternative tokens as their operators.
  - `lookups` lays the expansion's stores out too, and reruns at the next rung where the
    lines' expansion runs out of room.
  - Refused, by name: `defined` made by a macro's expansion, or read inside a call's arguments
    (undefined behaviour, C17 §6.10.1p4; the corpus has none: `P1_IF_DEFINED`), and an
    expression past the evaluator's 48-deep stacks or a constant past 128 bytes
    (`P1_IF_BIG`). A `__has_include` whose operand is a macro is step 6's.
  - R4: `if_values_are_the_serial_passes`, 117 expressions in one unit (each operator and its
    precedence, the unsigned rules, short circuits, errors, character constants, `true`,
    macros in the line, both forms of `defined` before and after a definition and after an
    `#undef`, the three header tests found, not found and empty, the configuration's answers,
    the alternative tokens), `#elif` chains, nested conditionals and a test a header's
    definition decides, in C and C++: the output is the serial pass's, and both values occur.
  - All 23 evaluator tests pass; the Metal gates pass. The corpus, from main files: 3,129
    units equal, at most 2 rounds of 4 launches; refused: `#line` 2, a call across a stretch's
    end 1, an expanded include name 3 (one more than step 4: a `__has_include` of a macro,
    step 6). GPU time 206 ms against 184 ms: each iteration expands the tests' lines.
- **Step 6: expanded include names (§5; R5).** `pp_if.uglykl`.
  - A macro-form include's line (`#include MACRO`, `#include_next`, `#import`, within the
    depth's bound) is a line of §4's pass, its names looked up as text's are; a header test's
    operand that starts with neither a string nor `<` is expanded too (`Pp::header_of`).
  - The output keeps each token's space flag beside its ref, so the name is spelled from the
    expansion as `header_of` spells it: a string's bytes inside its quotes, or the tokens from
    `<` to `>`, a blank where space was before one (`p1_header_of`, `p1_header_bytes`).
  - One lookup serves the includes and the header tests (`p1_find`, `p1_find_header`), so a
    literal name and an expanded one are keyed alike. A request carries its name's bytes, as
    the device spelled them (`P1_RNAMES`): the host searches for them and reads no source.
  - The refusal `P1_EXPANDED_NAME` goes.
  - R5: `expanded_include_names_are_the_spikes`, 7 shapes in C and C++: a string, `<…>` from
    object-like and function-like macros, a stringized operand, names defined in an earlier
    header, a name with blanks, a name that spells nothing, one not found, `#include_next` of a
    macro, `__has_include` of macros. The tree, every name's meaning and the output are the
    spike's.
  - All 24 evaluator tests pass; the Metal gates pass. The corpus, from main files: 3,132 units
    equal, at most 2 rounds of 4 launches; refused: `#line` 2, a call across a stretch's end 1.
    GPU time 251 ms against 206 ms, at a load average of 55 (not comparable).
- **Step 7: the cases of §6 (R7).** One commit each (7a–7f); R7's test,
  `the_refused_cases_are_the_serial_passes`, grows a shape with each, in C and C++, 4 launches.
  - 7a, `__INCLUDE_LEVEL__`, `__FILE_NAME__`, `__BASE_FILE__`: each top stretch (and each
    directive's line) knows its instance (`SINST`, `IF_INST`). A built-in reads its instance's
    depth, its file's last component's literal (placed by the host beside `__FILE__`'s, from the
    configuration's paths), or its unit's main file's literal.
  - 7b, the `__has_*` tests in text: a test the configuration answers is a function-like name
    (`L_FEATURE`). Its call's raw operand, spelled together, keys the configuration's table, as in
    an `#if`, and the call is one number token. `__has_include` and `__has_embed` stay plain names
    in text, as clang reads them.
  - 7c, `__COUNTER__`: each use is a number token with room for its digits and a row of the
    uses' table (`CNT`). Once every list is placed, a use's value is the count of its unit's
    live uses before it. Two uses' order is where their chains part: an argument's list is
    ordered by its parameter's first plain use in the body, the serial pass's `pre`, and comes
    before the call's rescan; a list is ordered by its place in its parent; a top call by its
    top token. A list a claim rolled back is never placed, so it is not counted: the serial pass
    reads it raw.
    - **Refused, by name:** `__COUNTER__` in a directive's line (`X_COUNTER`). Its value would
      count text expanded only after P1. The corpus has none.
  - 7d, `_Pragma`: a function-like built-in whose call leaves no tokens. Its pragma goes to the
    sink, not the output.
    - **Refused, by name (`X_PRAGMA`):** a pragma that would make a row (`once`, `push_macro`,
      `pop_macro`), and an operand that is a macro's (its string is known only once expanded).
      Those rows need the text expanded inside P1's iterations, which §6 asks for and this step
      does not build. **Open**: either the text pass joins P1's iterations where a unit has such a
      `_Pragma`, or the refusal stays. The corpus has none.
  - 7e, `#line` and line markers: an active one's operands (its `LINE_REST` or `MARKER_REST`)
    are a line of §4's pass, expanded.
    - At each iteration's end, its row is made from the output: its offset (the line it names
      less the physical line after it) and, with a string, its presumed path's and that path's
      last component's literals, in two scratch tokens of its own.
    - Two segmented max scans over each instance's directive words give the latest row, and the
      latest row with a file, before each directive and each stretch.
    - `__LINE__` is the physical line plus the latest row's offset at the call's end; `__FILE__`
      and `__FILE_NAME__` are the latest named row's literals.
    - A row that changed is a change, so the iterations go on until the rows hold too.
  - 7f, a call across a stretch's end: a top call's arguments read on past directives, through
    the run of its file's stretches with nothing of another file between (`RSTART`, `RUN_END`).
    The depth's scan and the close table are scoped to the run.
    - The call's slot is its `)`'s stretch's, since the serial pass expands it once its
      arguments are read; an exit's at the top level the same.
    - A function-like name's `(` must be in its own stretch: the serial pass looks for it past
      no directive, at the top level and for an exit alike.
    - **Refused (`X_ACROSS`):** a call whose arguments run into an included file or past their
      file's end. The serial pass ends such a call with its file. The corpus has none.
    - `a_batch_is_its_units` now refuses a call past its file's end.
  - All 25 evaluator tests pass; the Metal gates pass. The corpus, from main files: **3,135 units
    equal, none refused**, at most 2 rounds of 4 launches. GPU time 245 ms over the three slices,
    at a load average of 18–22.
- **Step 8: the corpus from its main files (R8).** `the_corpus_preprocesses_on_the_device`
  (was 0448's `the_corpus_expands_on_the_device`) builds each unit from its main file alone,
  through `on_device`: the rounds, with no spike run. It fails on any refusal, and on a round
  other than 4 launches.
  - The spike's P1 is now a cross-check only: `unit_of` runs it beside `on_device`, for the tests
    that compare the passes' arrays (`lookups_agree`, the tree's, the groups', the names'). Every
    other test, and the corpus, builds through `on_device`.
  - The corpus, in its three slices (0/500, 500/600, 1100/all): **3,135 units equal the serial
    pass (7,610,374 tokens), none refused**, at most 2 rounds of 4 launches. GPU time 271 ms over
    34 runs, at a load average of 22–52.
  - **Open, from step 7:** the `_Pragma` rows (`once`, `push_macro`, `pop_macro`, or a macro
    operand), `__COUNTER__` in a directive's line, and a call into an included file or past its
    file's end stay refused by name (`X_PRAGMA`, `X_COUNTER`, `X_ACROSS`). The corpus has none of
    them.
