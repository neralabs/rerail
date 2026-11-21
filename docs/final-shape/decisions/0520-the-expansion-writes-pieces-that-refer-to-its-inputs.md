# 0520: the expansion writes pieces that refer to its inputs, never copies of them

The user, 2026-11-20, after 0478 §10's reclaiming: "you are getting the writing all wrong. We need
to write out a format that's not just text but allows us to copy text from the inputs and the
definitions too. Our input and output text formats should include two things: 1. text copied
from an input from offset x to offset y; 2. text produced by our run (which can then be copied by
the next one). That way we are not continually expanding gigantic blocks of text but are
referring back to work already done."

## Intro

The device preprocessor (0448, 0478) expands a top level's calls as a tree of lists. Each list
holds its tokens as columns in the arena, a ref, flags, a depth and an origin a token, and every
step that moves tokens copies them:

- a call's substitution writes its body with each argument's tokens copied in;
- an argument's list is its raw tokens copied from the call's parent, and once expanded it is
  copied again, flattened into a block of its own, which the substitution copies a third time;
- an exit's join copies the rests of the frames it climbs;
- the output is every token written again, one word a token, plain text included.

So what a run holds grows with what it does, not with what it reads. The rooms that ran out in
Boost (0478) were these copies. This decision replaces them with **pieces**: a list, an
argument's expansion and the output are each a short array of references to text that already
exists, and nothing is copied but the references.

## The root cause, measured (before this decision)

`a_chains_use_is_flat_in_its_length` (UKL's CPU backend, 2026-11-20) runs 0478's chains, a state of
120 tokens passed down through 25, 50 and 100 levels, and reads each store's most use:

| Chain | Tokens (25, 50, 100) | Lists | Arguments' entries |
|---|---|---|---|
| plain | 1,408 · 1,920 · 3,072 | 26 · 51 · 101 | 10 · 14 · 23 |
| an argument scanned | 3,456 · 4,992 · 7,168 | 37 · 67 · 128 | 13 · 19 · 27 |
| an exit | 2,052 · 3,592 · 5,128 | 59 · 114 · 220 | 21 · 38 · 54 |

Every store grows with the chain's length. Each level keeps its list, about 20 to 50 tokens of
copies, and its entries until the top level is placed. Two other causes made the climb look like
room, and are fixed with this decision's first step (0478 §10's figures):

- **The work bound was a room.** A top's items were bounded at 4 times the queue's room and
  refused as room (`X_ROOM`). So a unit that does much but holds little needed a rung for its work.
  The bound is now fixed (`WORK`, 2^18 items a top), and past it the unit is refused for work
  (`X_WORK`), which grows no rung.
- **The givings were capped** (`PX_GIVES`, 192). Once a long top spent them, every mark was turned
  off and each store grew with the rest of the work. The host now bounds the givings by each
  room's gap and the ring's pauses within the work bound (`WORDS` 15). Reaching that bound is the
  queue's refusal, never a silent stop.

## Decision

### 1. A piece

A piece is a run of tokens that already exist, read in place:

| Kind | Its tokens | Read as |
|---|---|---|
| `P_TOP` | the top level's tokens `start .. start + len` (an input, from offset x to y) | `top_ref`, `top_space` |
| `P_BODY` | definition `def`'s body items `start .. start + len` (a definition's text) | `d.body + i`, its item's `I_SPACE` |
| `P_REF` | one token the run produced: a paste's, a `#`'s, a number, a built-in's (a scratch token, or the name a paste spells) | the ref itself |

Each piece also has:

- its flags: all of its tokens painted (`PF_NOEXP`), and the first token's space where it differs
  from its source's (`PF_SPACE_SET`, `PF_SPACE`);
- the origin `__LINE__` reads for its tokens (`exp`, as `texp` was: 0 reads each token's own).

A piece array keeps each piece's place, the tokens before it (`at`). Token `j` is in the last
piece whose `at` is not past `j`, a binary search.

A run's produced text is a `P_REF` piece. Every later piece array that holds that token copies
the piece, not the token's bytes.

### 2. A list is its pieces

- A macro's substitution writes pieces:
  - a run of the body's literal items is one `P_BODY` piece;
  - an argument, raw or expanded, is the pieces that cover it in its source;
  - a `#` is a `P_REF`;
  - a `##` joins the last token of the piece before it with the first of the piece after it,
    each trimmed by one, the result a `P_REF` in the `##`'s place.

  Pieces never refer to a list. They are path-compressed when made: a range of a list's pieces is
  copied as the pieces of the inputs those cover. So a read is one binary search, and a list's
  pieces can be given back when it is dead, as its tokens were, whatever reads its result.
- An argument's list is a view of its call's source, `lo .. hi` there, with no pieces of its own.
  A view of a view is made a view of the first one's source.
- An argument's expansion, its flatten, is pieces: its own runs between its live children, and
  each child's expansion, in order. A substitution copies them, not its tokens.
- An exit's join is the pieces covering each frame's rest.
- Nothing stores a token's depth. A scan carries it, and a search for a `)` from a `(` or across a
  frame's rest reads it relatively, as the top level's close table does for the top.
- A list's painted names (`T_PAINT`) are counted in the list. Its walk, which writes its
  expansion, finds them again (a name its chain disables) and makes each its own `PF_NOEXP` piece.

### 3. The output is pieces

The output a top level writes is pieces too:

- a run of top tokens outside every call is one `P_TOP` piece, not a token a word;
- each call's expansion is its pieces.

The directives' evaluation (`pp_if`) reads the output's tokens through the pieces. The host reads
them, and makes tokens only where its caller asks for tokens: the tests' comparison with the
serial pass.

### 4. What a pending level keeps

A level of a chain is pending until the call it holds is done, and keeps, in that time, its list's
record and nothing more. Four things made it keep more (step 4, measured below):

1. **A done call's span stayed in its parent's pieces.** The pieces wholly inside a done call's
   span (and its exits' claim) leave the parent's array in a pause: the pieces after them move
   down, and the block's end past its pieces goes back to the pool (`pgave` counts what is given
   from the end). They are not given back in place: a read finds a token by a binary search of the
   places (`pat`), which may land on any piece of the array, and a piece given back is written by
   its next taker. No item runs in a pause, no record names a piece of a list (its readers search
   the places, which stay in order), and no reader reads a done span's tokens.
2. **A call's tokens from its exit's name on stayed.** Its exit's join copied them, and its walk
   stops at the name, so once the exit is made they leave the array too (where no call of the list
   has a claim: one would hold the name, and the exit would not be its own).
3. **A pause came only once a store's front moved.** A run that took blocks from the pool alone
   spent all it held before the front moved, so each pause came a gap above the last one's peak,
   and the peak grew with the chain's length. A take from a pool now counts toward the store's
   mark as the front's growth does (`CTL` 20–22), within an allowance of givings (`CTL` 23: twice
   the fronts'). Past the allowance the fronts alone count, as before: the room may run out, and the
   unit runs a rung up, but no unit is refused for its pauses.
4. **A call whose expansion is empty but for its exit kept its record.** Its record was only the
   link from its parent to its exit's call, which is in the same parent. In a pause the exit's call
   takes its place among the parent's children (from its `lo`), and it is dead. This happens only
   where the call is certainly live: every call before it final, none with an exit that could claim
   it, and not past the parent's own exit's name. Then no rollback drops it, and it is as placed (its
   arguments' `__COUNTER__` uses are counted). `__COUNTER__`'s order ranks siblings by their spans'
   ends, which never tie. An exit's join and its call take their ids one at a time, as ids come
   back (the join's once its call is done), and the join keeps its call's id (`call`).

The serial pass keeps a frame a nested call: a level's `(` … `)` is read while its frame is still on
the stack. So the lists' count is one a level, and every other store is flat.

### 5. What stays

- The lists, the queue, the claims and the exits (0448, 0467).
- The pools (0478 §10): the pieces' store takes and gives blocks as the tokens' did.
- The serial pass as the oracle: every eval test.

## Steps

0. The work bound and the givings' bound (above), and `a_chains_use_is_flat_in_its_length`.
   The test is ignored until step 4 makes it pass.
1. Every read of a list's tokens goes through one accessor (`px_tok`). The scan's, the exit's
   and the close search's depths are carried, not stored. Behaviour unchanged, and every eval test
   passes.
2. Lists, argument expansions and joins as pieces (§1, §2). `tok`, `tflag`, `tchild`, `tdepth`
   and `texp` go. The flat test passes on tokens (now pieces) and arguments.
3. The output as pieces (§3), `pp_if` and the host's read through them.
4. A pending level's record is all that stays of it (§4). Done.

   | Chain (25, 50, 100 levels) | Tokens (pieces) | Lists | Arguments' entries |
   |---|---|---|---|
   | plain, after step 2 | 104, 204, 404 | 26, 51, 101 | 26, 51, 59 |
   | plain | 24, 24, 24 | 26, 51, 101 | 5, 5, 5 |
   | through a scanned argument, after step 2 | 238, 464, 890 | 53, 112, 186 | 35, 61, 87 |
   | through a scanned argument | 28, 28, 28 | 30, 55, 105 | 4, 4, 4 |
   | through an exit, after step 2 | 552, 1064, 2088 | 65, 121, 228 | 42, 57, 77 |
   | through an exit | 44, 44, 44 | 29, 54, 104 | 3, 3, 3 |

   Each store's most use in the run (`Exp::room`), its blocks given back every 16 of its growth.

## Requirements

| # | Requirement | Test |
|---|---|---|
| R1 | Every store but the output's, the scratch's and the lists' is flat in a chain's length, at 25, 50 and 100 levels: plain, through a scanned argument, through an exit. The lists grow by at most one a level (the serial pass's frames). | `a_chains_use_is_flat_in_its_length` |
| R2 | Every eval test equals the serial pass after each step. | the eval tests |
| R3 | A unit refused for work is refused as `X_WORK`, and grows no rung. | `a_unit_past_its_room_runs_again_larger` (unchanged), the work bound's reason in `against_serial` |
