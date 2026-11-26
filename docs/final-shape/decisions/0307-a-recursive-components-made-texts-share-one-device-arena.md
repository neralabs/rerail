# 0307: A recursive component's made texts share one device arena, interned at its end

Status: built (audited 2026-11-23). Decided (the user, 2026-11-16); built the same day. In a session, `hierarchy`
reached the device once it made levels (decision 0311).

## Intro

**The problem.** On Metal, `device::fixpoint` runs a recursive component round by round, and each round's batch has
a text arena of its own. A text a rule makes in one round can't be read in the next: it is no symbol yet, and its
arena is gone. So the device refuses any recursive component that makes text, and the four shortlex-least-route
recursions (O62: `hierarchy`, `sink`, `project_sink`) run on the CPU. Each makes its route each round
(`@concat(r0, ...)`).

**The decision.** The rounds of one recursive component share one device arena:

- A text made in any round is written to that arena and named by its offset there.
- Later rounds read it in place.
- The texts the component keeps are interned into the store's atoms once, when the component ends.
- There is no host trip between rounds.

The alternative, interning on the device each round into a device intern table, was not taken. It costs a hash
insert per made text per round, and needs a table that grows on the device.

## How (to build)

- **Arena.** One arena for the component, sized from the device budget (`budget::metal_share`). A component whose
  arena would pass it runs on the CPU, with the reason in the profile, as other refusals do.
- **Values.** A made text is an arena reference that the fold kernels and the text functions read as they read a
  symbol's bytes.
- **End.** At the component's end, the texts its kept rows name are interned (the atoms' `intern_all`, one call), and
  the rows are rewritten to symbols.
- **Still missing for the four recursions** (O62, in `device::fixpoint`'s order):
  - `@printf` on the device (hierarchy's `"%012d."`);
  - the shortlex fold: `lat_best` and `lat_merge` comparing a value's text by (characters, bytes), a word at a time.

## Built (2026-11-16)

- **Arena.** `device::fixpoint` makes one `mj::Arena` for a component whose head is a `shortlex_min` lattice
  (`arena_max()`), and every round's batch writes into it (`Batch::with_shared_arena`). The texts its rules name as
  constants are put there first (`Arena::put`), so every value of the column is an arena text.
- **Where a made text may go.** Only into a `shortlex_min` head's value column. The compile (`own_texts`, passed by
  `SolverDevice::compile_join`) reads that column as an arena text and refuses it as a key, a test, a fused or a
  negated atom's column. Placement (`Shape::add`'s `texts_kept`) lets such a stratum make texts in its rounds.
- **The fold.** `lat_best` and `lat_merge` take the arena. Code 3 compares two texts by characters, then bytes, a
  big-endian word of eight at a time (`lat_text_less`).
- **End.** The kept rows' arena texts are interned in one `intern_all`. The top stays as it is.
- **`@printf`.** It lowers to `concat(zero_pad(n, w), ".")` (`printf_pieces`). The device now makes `zero_pad`
  (`TEXTFN_ZERO_PAD`: the sign, then zeros to the width, 0 to 64, as the host's `{:0w$}`). A cat of made texts
  (`@concat(r0, @zero_pad(..))`, a three-part `@concat`) binds a variable on the device (`is_made_cat`).

**Tests.** The coverage gate's probes (`plan/coverage.rs`), each with SQLite's rows on the device:

- `shortlex_min over symbols`;
- `characters, then bytes`: labelled edges, where "ab" must beat "ba" and "é" (one character, two bytes) must beat
  "aa". With the compare made bytes-only, the device gave "aa", and the gate failed;
- `of printf steps` (hierarchy's shape);
- `of a three-part concat` (the sinks' shape, with `@to_string` in place of `@id`).

`@zero_pad`, `@concat` of three texts and both `@printf` probes moved to the device outside recursion too.

## Found (2026-11-16)

- **Levels.** On public_api_triples with `--metal`, `hierarchy` is placed on the GPU and refused at run time: "a
  recursive component that keeps levels (its maintenance runs on the CPU)". Every corpus run is a session's cross
  part, which keeps levels for a stratum with the `levels` qualifier (0090), and `--full` does too. The device
  hands its rows back once, at the end, so the host would give them one level. It needs each row's level, the
  round that made it (a lattice row: the round that improved it), as the CPU's `lbase + wave + 1`. Decided and
  built as 0311: `hierarchy` now runs on the device in public_api_triples' and internal_dead_code's session runs.
- **The sinks run nowhere.** `serverplan_state_tripwires.state_flow_1.rr` and `state_flow_project_1.rr` fail their
  types (`relation start is read by the rule for arg_before and not declared`). No executor has `@id` (no SQL for it,
  no native function).

## Closure

The four recursions run on Metal and give SQLite's rows.
