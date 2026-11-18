# 0385: The tail's last whole costs go over the edit

Status: built (2026-11-17), measured on ep2 below. Follows 0380 ("What it does not do").

## Intro

0380 runs the combine's tail again over an edit of the link, each pass onto its last output. Five costs were left
whole, about 0.13 s of an edit's 0.33 s `combine`:

- Opening the last tail run from the store read and decoded all of it: every version's image, and every analysis's
  rows (63,435 rows, 4.5 MB of facts), though a pass asks for a few.
- The file-local split was maintained three times per edit (fact spaces, the narrowing, the answers where documents
  are). Each maintain made every relation's key again, a digest of every rule.
- Three passes ran whole: the projected inputs, the retyped helpers, and the answers where documents are.
- An edit whose last run was eight edits deep would run the whole tail: `rebase` refuses a version that deep, so a
  stored run stays a few layers deep. On ep2 it never happened, for a worse reason (below): the tail's run was
  kept once, so every edit ran from the first run.
- On an edit run, the passes that run over the edit did not print what they print when they run whole (the sources
  on request, the requests narrowed).

## Decision

1. **A stored run's analyses' rows are their own source, decoded when asked for** (`program_text::KeptRun::rows`).
   Each analysis's rows are encoded on their own, one after another, in one source kept under its sha256
   (`rows_alias`). The run's facts say where each is (`pipeline_run_analysis_at`). A reopened version's
   `read_stored` decodes an analysis's rows the first time a pass asks for it. The run's own facts are its
   versions, steps and decisions.
2. **A relation's place depends on its rules and what they read, down.** The split's maintain splits a changed
   head's readers, up. So a pass that reads the split's places for some relations reads them as they were where the
   edit defines none of them, and nothing they read, down:
   - **the requests narrowed** no longer ask for the split over an edit. Their `again` checks that the edit defines
     no request and nothing a request reads, down, as it did before.
   - **the answers where documents are** run over the edit as it is where the last run undid no round, the header is
     as it was, and the edit defines no round's answer nor what it reads, down.
3. **The split's maintain keys only the edit's heads** (`Program::keys_over`). `Delta::heads` names the heads of the
   rules an edit removed and added. Over an edit of rules alone whose heads were derived before and after, the
   last keys are kept and those heads keyed again; the global key reads no rule but which relations are derived.
   Fact spaces reads the split's locations in place, not a copy.
4. **The projected inputs and the retyped helpers run over the edit as it is** where it has no rule of `kind` or
   `docof` (no rule of a helper) and leaves the header as it was.
5. **A run a few edits deep is made shallow, not run whole** (`Run::shallow`, `Builder::flat`). Each version of the
   last run `REBASE_DEPTH` deltas deep or more is made a view of one new base. The base holds every rule the
   version's stack numbers, by its id, its own and those an overlay removed, under the stack's atom and relation
   ids, with its parts and each rule's source. The view leaves out the removed ones. Ids, size and digest are the
   deep version's, so the next edit is an edit of it a layer deep, and every step reads its plan and sources as
   before. The flat version adopts the deep one's analyses (`Store::adopt`). A version whose removed rules are more
   ranges than a view holds stays deep, and the next edit runs whole: that is a program mostly rewritten.
6. **The tail's key names the run kept last, and a run is kept where it pays** (`tail_kept`, `TAIL_REKEPT`). A
   stored run's facts are kept under its key's alias once (`store_run`), and the tail's key is the same for every
   edit. So the first run stood for good: each edit after it wrote a run nothing read, and a run made whole after a
   large edit could not be kept, so every edit after that would run whole. Now each run is kept under its own
   input's identity, and the key is a replaceable pointer to it (the facts memo, as the kept link's, 0373). A run is
   kept again only when it was made whole, or when the edit from the kept run's input reaches 64 rules. Below that,
   each edit is an edit of the kept run's input, and nothing is written.
7. **A program pass runs over an edit of rules alone** (`Pipeline::program_step`). Its last output holds the last
   input's declarations, facts, inputs and outputs, which a pass writing onto it does not state again. 0380's
   `DropRepeated::again` checked no part, and fact spaces only the declarations: over an edit that changed facts,
   the output kept the last run's facts. The engine's program_image tests found it (12 of them failed on "the
   version's program is the tree's after the merges", another program's `env` facts in the output). These tests run
   in `rerail-engine`'s library, not `rerail-logic`'s, so 0380's targeted run had not run them.
8. **What a program pass says when it runs whole, it says again over an edit** (`Kept::said`). The profile's
   decisions a pass makes in `run` are taken with its step (`profile::Deciding`) and kept with the stored run
   (`pipeline_run_step_said`). A pass run over the edit, where its decisions stand, restates them
   (`profile::restate`) to the profile and to every open step. The requests narrowed now say their lines from inside
   the pass, not from its caller.

## Measured

ep2 on the m3 (300 files, 317 rules, `RERAIL_PREPARE=parts`, an edit of B101's body, profiled). Per edit, against
0380 (e86):

| part | 0380 | now |
|---|---|---|
| `combine` | 0.33 s | 0.26 s (e89: the split, the three passes, what passes say) |
| fact spaces' split | 0.025 s | 0.012 s |
| the narrowing, the answers | 0.013 s, 0.015 s | too small for the profile to list |
| `combine` with the rows apart | | 0.24 s (e102) |
| `combine` with no keep on an edit | | 0.23 s (e106; the keep was 0.009 s) |
| the last run, from the store | 0.049 s | 0.032 s (e102) |
| its facts | 4.5 MB, 0.015 s | 0.48 MB, 0.002 s (the rows 2.9 MB apart, decoded when asked) |

- Every pass of the tail runs over the edit (e88, e101), and `RERAIL_COMBINE_TAIL=check` passes; rows 1,334, none
  differing from h1, on each of eleven edits in a row (e91–e101).
- The open's images: 8, read 0.019 s, opened 0.007 s (e102).

## What it does not do

- A stored run's images are read whole when it is opened: each is decompressed, hashed and checked by the store
  and the image's open.
- The congruence classes and the split still read every relation once per maintain (their `Program` and `Rels`).
