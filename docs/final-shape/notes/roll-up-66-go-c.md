# Roll-up 66: Go-C-and-C-support (a39040bb03) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 33f4aef25f. Theirs: Go-C-and-C-support at a39040bb03. The one merge base is
cb23f84a49, theirs' pin in roll-up 56. Theirs brings eleven commits of its own and no merges (decision 0588):
- the expansion is one loop phase. The loop's stacks are in the group's memory, and a long `#if` line is read by
  its SIMD group;
- §6: a batch's units are a team each, and P1's text room follows a round's requests. The grid is 64 groups at most;
- a list's next entries are looked at a lane a token, and its plain ones are written whole by their lanes. A call's
  argument is one step;
- a reader of the output keeps its place, and a run the system stopped runs again;
- the loop's step and its registers, measured.

## Items

1. **A slab's refusals, by layer.**
   - The lease is the pool's, so its refusals (no device, the block, its device buffer) print under ours'
     `RERAIL_POOL_DEBUG`, as pool.rs' do, through one closure. Theirs' closure under `RERAIL_PP_WHY` goes, and so
     does ours' closure named `refused`, which shadowed theirs' function of that name.
   - The residency's room loop is pp's, so its refusals go through theirs' `refused`, under `RERAIL_PP_WHY`, as
     0588 says.
   - Ours' rule from 87fe4b4a28 holds: only a capacity refusal is counted and retried. Any other error, no device
     among them, is the caller's as it is. Theirs' arm turned every error into capacity.
2. **The setup's split** (`the_setup_splits_one_unit_a_barriered_function`): theirs' 9 libraries, with `pp_unit`.
   Calls are 5 deep: the base's 5, less ours' level (16z's block read in the public read, 0576) and plus theirs'
   (`pp_unit`). The test measures 9 and 5.
3. **The command's number** (`pp_expand::RUNS`, `Expanded::run`) is an id counter, append-only. It goes in the
   global-state list as `stays`, as the ANE's and the cost model's counters do.

## Audit

`scripts/check/merge_audit.py` reports 20 findings, all in `slabs.rs` and the split test: each is a line of one side
replaced in items 1 and 2.

## Checks

On spark2, CPU, one platform:
- `cargo check --workspace --tests` is clean. The crate DAG, global-state and thread-state checks pass.
- rerail-metal's `the_setup_splits_one_unit_a_barriered_function` and `an_evicted_slab_is_recycled_and_its_rooms_read_zero`
  pass.
- Theirs' new tests in rerail-c-directives pass: `a_long_if_line_is_read_by_its_group` and
  `the_expansion_is_one_loop_phase`. They ran under `cargo test`: nextest lists none of that crate's tests, though
  its test binary lists them.
