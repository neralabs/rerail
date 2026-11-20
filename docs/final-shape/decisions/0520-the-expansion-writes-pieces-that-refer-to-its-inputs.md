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

### 4. What stays

- The lists, the queue, the claims and the exits (0448, 0467).
- The pools (0478 §10): the pieces' store takes and gives blocks as the tokens' did.
- The serial pass as the oracle: every eval test.

## Steps

0. The work bound and the givings' bound (above), and `a_chains_use_is_flat_in_its_length`.
   The test is ignored until step 2 makes it pass.
1. Every read of a list's tokens goes through one accessor (`px_tok`). The scan's, the exit's
   and the close search's depths are carried, not stored. Behaviour unchanged, and every eval test
   passes.
2. Lists, argument expansions and joins as pieces (§1, §2). `tok`, `tflag`, `tchild`, `tdepth`
   and `texp` go. The flat test passes on tokens (now pieces) and arguments.
3. The output as pieces (§3), `pp_if` and the host's read through them.
4. A pending level's record is all that stays of it. The lists' count is then one a pending
   level, the serial pass's frame a nested call. A tail call taking its parent's place is the
   step after, for the lists' count to be flat too.

## Requirements

| # | Requirement | Test |
|---|---|---|
| R1 | Every store but the output's and the scratch's is flat in a chain's length, at 25, 50 and 100 levels: plain, through a scanned argument, through an exit. | `a_chains_use_is_flat_in_its_length` |
| R2 | Every eval test equals the serial pass after each step. | the eval tests |
| R3 | A unit refused for work is refused as `X_WORK`, and grows no rung. | `a_unit_past_its_room_runs_again_larger` (unchanged), the work bound's reason in `against_serial` |
