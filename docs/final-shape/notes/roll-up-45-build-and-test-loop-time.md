# Roll-up 45: build-and-test-loop-time (c4d091a46f) onto Pulling-it-all-together, read by architecture

Ours: Pulling-it-all-together at 8b1dc1c67e (roll-up 44, Q465). Theirs: build-and-test-loop-time at c4d091a46f. It
is farm tooling only (farm 54–57):

- compile speeds measured per host, shown by `status`, and Windows syncing one at a time;
- a crate built against a replaced one compiles again, and setup keeps an unmerge;
- `bench --table`;
- the README's macOS first-run check over ssh;
- each host's CPU and memory found by its health probe, with `test-remote --arch`, `--memory` and
  `--cpu-feature` (`features`, `RERAIL_FARM_CPU_*`).

## Items

1. **`test-remote`'s main: theirs' line.** Both sides added `trees += [t for t in pack_data() …]`: ours (the pack's
   data sent with every run) and theirs' merge of the same change. Theirs' line also takes `parse`'s three new
   results (`arch`, `memory_gb`, `features`). One `pack_data` call remains, as both sides intend.
2. Every other file merged without a conflict. No engine code is touched.

## Checks

- The script compiles (`py_compile`).
- A real run, `test-remote --os linux --arch aarch64 --remote-only`, ran a trivial script on an aarch64 Linux host,
  through the merged selection.
