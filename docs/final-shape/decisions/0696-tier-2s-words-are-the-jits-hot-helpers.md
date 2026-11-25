# 0696: Tier 2's words are the JIT's hot helpers

Status: built (the user, 2026-11-25: "3x is too much of a penalty to pay; I was hoping for 20-30%", then "please
address all of these, measuring as you go"). Extends 0310 (one inlining pass) to tier 2's own functions. Follows 0685.

## Intro

The JIT made each of tier 2's words (`TOKEN`, a token mark, a node's open, leaf and close, …) a function of its own
(`outline`, `WordFn`), and every place of the word called it. Its state stayed in `ctx` (`Ctx::state`). The lean
parser and the generated Rust have the same helpers, but rustc inlines them into the rules, and their state is in
registers. A profile of rr's tier 2 found the JIT's time in those calls: each call's prologue and epilogue, the state
stored to `ctx` before the call and loaded back after it, and the token marks each read through a call.

## Decisions

1. **The words' functions are hot helpers in 0310's pass.** `rerail_clif::inline::with_helpers` takes the module's
   own helpers, by id, each body legalized once. Every call of one is inlined from that one body, in the pass that
   inlines the tape's hot reads (`native_cpu_codegen::define_functions_inlining`). The policy is the callee's mark,
   as 0310 has it: a word's kind, never the call's site. The words stay one body each in the lowering, so a word is
   still written once.
2. **A local check is not hot** (`WordFn::Check`). It is a loop over the token's words. Inlined into a loop of its
   caller's, such as JSON's string loop, it takes the registers that hold that loop's state, which the JSON test
   holds to "no frame access" (`simdjson_forms_starts_are_simdjsons_indexes`). It stays a `preserve_most` call.
3. **The state in registers after inlining.** With `ctx` pinned, `state_in_registers` runs on each function after
   the inlining pass (the pass's `after` hook). Every read of the pinned register is `ctx`, so a field an inlined word
   stores and the next one loads is one value, and the stores that no later load reads are dropped.
4. **A token mark asks its token's kind first.** Where `TOKEN` keeps the token's code (`Mem::Taken`, from tier 1's
   records), a mark tests its offset-0 element on that code before it reads any record, as the lean parser asks the
   kind before it peeks (`token_pattern`'s `code`). A token of another kind fails there. This is the JIT's lowering
   only: the mark's program and Metal's lowering are unchanged.
5. **A word set is one bit test.** Three words or more whose codes fall within 64 of each other are tested as one
   set's bit (`elem_test`), as rustc compiles a `match` on them, not as a chain of compares.
6. **`own_calls` counts the calls left after inlining**, in each function the module defines, so the ladder's check
   (`ladder.rs`: each `blr` is one of tier 2's own calls) reads the code that is compiled.
7. **An inlined instruction takes its caller word's place** (`srcloc`), since Cranelift's inliner drops the callee's.
   `word_ranges` and the profile then attribute each instruction to a word.
8. **Jumps threaded** (`native_cpu_codegen::thread_jumps`), on every function the JIT defines, after the switches
   are lowered: a block whose one instruction is a `brif` on one of its own parameters is passed over. A `jump` into
   it becomes that `brif` on the argument it passes, and a branch into it with a constant argument goes to the side
   the constant takes. A token mark's answer and an inlined word's result were made as 0 or 1 at both ends of the
   test and tested again at the join; now neither is made. A block whose parameters are used anywhere but its own
   branch stays.
9. **`state_in_registers` reads each instruction once a pass.** The values that are `ctx` are found once (a set), and
   the constant fields' fixed point is a worklist over each block's writes, not a sweep of every instruction.
   Forwarding a field across a join (a value every predecessor holds) was tried and was slower: it holds the fields
   in registers through the marks' chains, where reloading them from `ctx` costs less. It stays forwarding along
   chains of one predecessor.
10. **A thread keeps its tape's buffers for the next tape** (`Spare`, as `IDS` keeps the code ids): each buffer a
    tape drops goes back to its own slot (its rows, sizes, offsets, tails, bases, code keys, supertypes, open nodes,
    returns, frames, chains), and `sized` takes it again where it holds what the next file needs, as simdjson's
    parser keeps its room from one document to the next. The image the caller takes is its own. A pool that handed
    any buffer large enough to any use was tried first and was slower.
11. **The record stride is compiled in** (`Cut::stride`, `stride_known`). A `Parser` compiles its tier 2 at the
    stride of its own tier 1's records (`cpu_stride`: rows on the CPU), and with `ctx` pinned each read of the
    stride is that constant. `CompiledStream::stride` says which, and `call` refuses records of another. Records of
    another stride (the device's, columns, `RecordLayout::choose`) take a tier 2 that reads the stride from `ctx`,
    compiled on first use (`Parser::tier2_at`, `again_at`, `Wide::at_any`); `CompiledStream::run` lays the
    interpreter's records out in the stride its stream reads.
12. **Holding fields across rules is not done this way.** With `Cut::held` set again (the token's index alone), the
    words, which read and write the state in `ctx`, read a value the rule held elsewhere and refused every file.
    Holding a field needs each word's function to take the held fields as parameters and give them back.

## Not done

From the list the user asked for:

- Metal's path to the tier 2 of its own stride (`tier2_on::image_on`) is not run: Metal runs are paused. So is a
  token mark asked switch-first in Metal's lowering.
- The working row (format 15's key, start and end) is still written and read back; nodes are not written to the
  columns directly. The generated Rust's writer (`rerail_tape_format::Writer`) also keeps each node's key, start and
  end and finishes them into the image, so the row is not the JIT's alone. The JIT's passes over the rows (blocks,
  commit, uncode, finish) are about a sixteenth of tier 2's samples in the profile.
- The state is not passed as the words' and rules' arguments (`held`, `pass` stay empty). The generated Rust keeps
  its state in `self` as the JIT keeps it in `ctx`, and Cranelift's code reloads a field where rustc's forwards it.
  Forwarding every field across joins (what arguments would give inlined words) was slower (decision 9 above).
- Inlining makes the rules' code larger, and Cranelift's compile of tier 2 takes longer with it. The words of 80 to
  150 instructions, inlined, are what the run's time needs: leaving them calls takes back most of the compile and
  most of the run's gain.
