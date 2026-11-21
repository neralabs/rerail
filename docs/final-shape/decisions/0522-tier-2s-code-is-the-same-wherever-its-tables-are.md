# 0522: Tier 2's code is the same wherever its tables are; the ASCII split leaves a region's vectors to it

Status: done by the implementer, while fixing failing tests (the user, 2026-11-20: "please address these").
Follows 0423.

## Intro

Two faults were found on the way.

**`json_tests::simdjson_forms_starts_are_simdjsons_indexes` failed one run in three**, on the same binary: "word 7
reads or writes the frame". The CLIF of every run was the same but for constants, which were the tables' heap
addresses.
- An address is moved into a register in as many instructions as it has halfwords that are not zero (`movz`, `movk`).
- Every failing run had a table at an address whose low halfword was zero, and no passing run did.
- The different instruction counts changed regalloc2's choices in a loop at the edge of the registers. One more
  callee-saved register and one spill slot followed, read 106 times.

**`regex_block::tests::a_patterns_block_program_runs_alike_on_every_executor` was refused** for
`(?m)^class\s+(?<c>\w+)`: "region 0's value %87 on the vectors made outside its branch (decision 0423)".
- `%87` is a UTF-8 class that is known zero on an all-ASCII block.
- The ASCII split (`ascii_split`) made it in its non-ASCII way, which runs on blocks the region skips, and passed it
  out to the region's member that reads it as a word.

## The decision

1. **Tier 2 reads a table's address from a symbol** (`native_cpu_stream::table_address`):
   - The symbol is one imported symbol, `TABLES`, at address 0. A table's address is its offset, so the relocation
     (`0 + addend`) writes the address into the code's literal.
   - The code is then one instruction of the same form wherever the table is. Nothing is copied: the tables stay
     where they are.
   - This covers the rows, the sets, the ends, the sequences' bytes, the classes, the codes, and the record pass's
     tables when tier 2 reads them (`native_cpu_block::tables_symbols`).
   - Tier 1 keeps its constants (`tables_consts`), which `clif_rust` reads in tier 1's IR.
2. **A region's value on the vectors leaves the ASCII split for no member of that region.** The region makes it in
   its own branch, as 0423 says.
3. Of the classes of one set, a body pack takes the first test made, not the first the hash map gives
   (`native_cpu_lanes::body_pack`). The tests of one set are the same test, so this changes no result. It makes the
   choice the same in every run.

The refusal of 0423 now names the instruction it found.

## Checks

- Ten runs of `simdjson_forms_starts_are_simdjsons_indexes`: all pass, and tier 2's machine code is the same in
  every run once each literal's address is taken out (before: 2 to 3 failures in 6 runs).
- `a_patterns_block_program_runs_alike_on_every_executor` passes.
- The parser tests (89, all but the benches, Metal and the corpus writer) pass.

## Not checked

- Whether tier 1's code also changes with its tables' addresses. It keeps them as constants.
- Nothing about speed was measured.
