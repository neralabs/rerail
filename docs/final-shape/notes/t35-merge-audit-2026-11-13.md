# t35. What the merges of 2026-11-11 to 2026-11-13 dropped

## Intro

Several agents merge branches into one another every day. The worry was that a merge resolves
conflicts by whichever side looks right in the hunk, not by what each side decided, so
architectural fixes get undone. This note checks that worry against the 120 two-parent merges
made between 2026-11-11 and 2026-11-13. It says what was lost, which resolutions were poor, and
what changed as a result. The rules that came out of it are in `docs/merging.md`.

## How it was checked

1. For each merge, and for each side, every line the side added since the merge base that the
   result lacks, and every line the side deleted that the result has again. 3,937 lines were
   dropped from one side with the other side not touching them; 914 deleted lines came back.
2. Lines found anywhere in the merge's tree count as moved, not lost. 2,445 lines were left, in
   489 (merge, file) groups. None of them is anywhere in today's tree either.
3. Each group was read with its commits on both sides. Four reviewers took one share each. Each
   group was classed as superseded (with the commit or decision that replaced it), lost, or
   unsure.

`scripts/check/merge-audit.py` now does steps 1 and 2 for any merge, in a few seconds.

## What was found

### Most drops were deliberate

About 97% of the dropped lines were replaced on purpose:

- the crate split's mounts and visibility changes (`pub(in …)` to `pub`);
- `Atom.rel` changing type (`.clone()` to `.into()`);
- one of two parallel designs chosen, with the reason in the merge message;
- RER-131's rules taken in their newer form (implied tokens read from the tape's kept tokens,
  not from the source text);
- the tape format: format 15, one definition in `rerail_tape_format`, over Parser-generators'
  stored digest.

Three architectural invariants were checked closely, and every merge kept them: tapes store
named nodes only, no static memo map is added beside the facts, and one tape-format definition.

### What was really lost or broken

- **Six decision numbers held two records each, and no merge renumbered them.** The numbers were
  0047, 0095, 0105, 0107, 0108 (three records) and 0114. Each was made when two branches wrote
  a decision at the same time; every merge since kept both files. Fixed in 916adb316: 0141 to
  0147, with 117 references moved by what they cite.
- **Answered world facts could replace held program facts.** c19a0c146 chose the hub's route
  (answers held by the native engine, 96d4720ab) over final-rerail-shape's (answers written to
  the table, 2911f61c5). That was a sound choice, but it dropped a case 2911f61c5 handled: a
  relation the engine already holds when its answers come. `setfix::retain` replaces the held
  relation, so its program facts were lost. Fixed in 42cab80c9. O149 asks for a test of its own.
- **Test code that no longer compiled.** `logic/tests/kind_planning.rs` lacked 0137's `shapes`
  field, so the logic crate's tests did not build. A benchmarks commit (445936dd5) also used
  `wave::tally`, which the hub's merge b9ac9b84f had replaced with its own since/credit fix. Both
  were merged without building the tests: a build piped through `grep` reports success whatever
  cargo does. Fixed in 42cab80c9 and, on the benchmarks branch, ebbe1516d.

### Resolutions that were poor, though nothing was lost

- **d0f1297e5** had an empty message and took the hub's `mod.rs` whole. Two engine test mounts
  went with it. They are mounted again elsewhere (`native_demand/engine_tests.rs`, and
  `rerail-logic` compiles `matchers::tests` and `integration_tests`), and all pass. But
  `tools/cratify/test-hosts.tsv` still names rerail-engine as their host.
- **824dc395f** (the regenerated split) took one side mechanically on every mount and visibility
  line.
- **5d0f0c53c** (Typing into the hub) has a one-line message for 214 dropped lines. Each drop
  has a reason, but the reasons are in other commits, not in the merge.
- **339a96f0c** widened the cache guard's facade exemption to plain `use` lines. Nothing breaks
  0103 today, but the guard is weaker than before.

### Still open

- d0f1297e5 replaced the pre-wave memo lookup's document-key test (`keys.get(p) != Some(key)`)
  with `b.found.is_none()`. **Settled (293647a70):** the trunk's b01bb2ccc has the keeper find its
  entries by its own test (`keptstats::same` on content and document key), and the merge keeps
  both: a file is looked up when it was found and its keys are the same.

## What happened while fixing it

- **Two renumberings of the same records.** Data-layer-rule-expressivity merged the hub at 09:19
  (946e19d30), between the merge guide and the renumbering. It renumbered the same seven records,
  to 0148 to 0154, and moved only the file names. `docs/merging.md` now says a collision
  inherited from the hub is the hub's to fix. That branch's next merge of the hub should take
  0141 to 0147.
- **A criss-cross merge fooled the audit.** When each side has merged the other, there are two
  merge bases. Against one of them, the other side's own later rewrites look like drops. The
  audit now counts a line as a side's own only if it is new against every base (5494b86c7).
- **A conflict helper emptied `Cargo.toml`.** It opened the file for writing before it built the
  text. The conflict was rebuilt from the index stages (`git merge-file`), and nothing was lost.

## What this changes

- `docs/merging.md`, with a short version in `AGENTS.md`:
  - read both sides' decisions before resolving, and resolve by invariant, never one side whole;
  - keep one definition of each shared format or constant, and keep both sides' tests;
  - renumber only the collisions the merge itself creates;
  - run the audit before committing, and name the superseding commit or decision for every drop;
  - build and run both sides' tests.
- `scripts/check/merge-audit.py`.
