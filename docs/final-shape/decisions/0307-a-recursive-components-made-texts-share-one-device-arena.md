# 0307: A recursive component's made texts share one device arena, interned at its end

Status: decided (the user, 2026-11-16); not yet built (O62, Metal).

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

## Closure

The four recursions run on Metal and give SQLite's rows.
