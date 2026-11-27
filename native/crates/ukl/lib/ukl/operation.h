// ukl/operation.h: an operation's interface (decision 0686, Operations §3), declared once, in UKL.
//
// `DECLARE_KERNEL2(Library, Name, rows...)` is a kernel definition's head without a body: its rows are the
// operation's interface. Each slot is a buffer row: its access is the slot's mode (`ROBUFFER` read, `WOBUFFER`
// written, `RWBUFFER` read and written), its type the kind of block it takes (below), its name the slot's; a
// `RWBUFFER` of a scratch block is the operation's scratch (written and read inside it, nothing kept). Then one
// `ARGS` row: the params' structure, the Rust `params!` struct of that name (`ParamsLayout::ukl`).
//
// The executor's registry is built from these declarations (`rerail_delta_exec`'s build reads them into its table of
// interfaces), and every version of the operation (its Rust twin, its UKL kernels) is registered against one. Nothing
// launches a declaration: on every backend it emits nothing.
#pragma once

// A slot's type: the kind of block it takes (`rerail_blocks::block::kind`), its bytes read through the slot.
typedef uint8_t ukl_facts_block;
typedef uint8_t ukl_scratch_block;
typedef uint8_t ukl_atoms_block;
typedef uint8_t ukl_control_block;
typedef uint8_t ukl_cache_block;
typedef uint8_t ukl_tape_block;

#define DECLARE_KERNEL2(Library, Name, ...)
