# 0711: Tier 2's writer ops keep the caller's registers

Status: built (2026-11-26, roll-up 135's follow-up on the hub). Extends 0696 §2 and 0434 §6 (R17).

## Intro

Decision 0696 holds JSON's string loop to "no frame access". The words of that loop, and of the continue states,
keep their state in registers, and the JSON test checks the machine code for loads and stores off `sp`
(`simdjson_forms_starts_are_simdjsons_indexes`). Its cold calls are `preserve_most`, so they keep the caller's
registers.

Format 16 is now the only form tier 2 writes (0626, 0631), and the loop writes its nodes as it goes, by the writer's
ops (`w_leaf`, `w_close`, `w_fail`, …; 0434 §6). Tier 2's module defined those ops with the platform's calling
convention. A call to one clobbers every caller-saved register, so the loop's state could live only in the
callee-saved ones, and one value spilled. The value was the last token's end, which `TOKEN` compares each start
against. The test was red on the hub from the moment format 16 became the only form. Parser-generators' line stayed
green because its default tier 2 was still format 15, which makes no calls in the loop.

## Decisions

1. **`writer::define_ops` takes the ops' calling convention** (`cc`). The module it defines them in is either the
   JIT's writer (`jit::Writer`, which Rust calls) or tier 2's own module. Each op calls the others with the same
   convention, by the signature `define_ops` returns.
2. **Tier 2 defines its ops `PreserveMost`** (`native_cpu_stream`, the module's `TableFns`), as its own cold calls
   are (0696 §2). The words keep their state in registers across a node written; an op saves what it uses.
3. **The JIT's writer keeps the platform's convention** (`default_call_conv`). Rust calls those ops, and Rust has
   no `preserve_most`. So does the relocation test's module.

## Checked

- `simdjson_forms_starts_are_simdjsons_indexes` passes (word 94 was a store to `[sp, #240]`).
- parser-tests' json and rr modules, 18/18; rerail-clif 19/19; compiler 46/46; cpu-stream 72/72.
- python-parser 25/25; kotlin-parser 70/70; rr-parser 56/56, without its three GPU tests.

The cost of the ops' larger saves against the loop's spills is not measured.
