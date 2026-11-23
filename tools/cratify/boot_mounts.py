#!/usr/bin/env python3
"""rerail-boot's mounts, from the engine's (`code_rule_dsl/mod.rs`, its mount section).

Boot stands where the engine stood, under napi and the CLI: it sees every module the engine sees, at the same
paths, through the same facades (incremental-plan.md, step 1). What it does not take is a mounted file the engine
keeps: each `mod x;` whose path is not in KEEP is dropped, with its attributes and doc comments. The test modules
the engine hosts stay the engine's (step 2 moves them).

Ran once, for step 1b. Since step 2 (split_test_crates.py) the engine's mod.rs is boot's glob and boot's lib.rs is
source: this script no longer applies.

Usage: boot_mounts.py NATIVE_DIR (writes native/crates/boot/src/lib.rs)."""
import re, sys

# The files boot owns, as module paths under code_rule_dsl.
KEEP = {
    'prefilter', 'functors',
    # The tests' helpers (`#[cfg(test)]`): boot's tests compile and evaluate rule documents as the engine's do.
    'test_support',
    'facts::vtab::syntax::ast_wave',
    *(f'language::adapter::{m}' for m in 'c c_pp csharp css go html java kotlin markdown scala swift'.split()),
}
# The engine's own top-level modules boot does not see at all (napi, the CLI, its tests' support).
DROP_TOP = {'waves', 'tools'}
END = "// cratify: the engine's own items (napi) begin; boot's mounts are the lines above (tools/cratify/boot_mounts.py)"

native = sys.argv[1]
lines = open(f'{native}/src/code_rule_dsl/mod.rs').read().split('\n')
end = next(i for i, l in enumerate(lines) if l.strip() == END)
lines = lines[:end]

decl = re.compile(r'^\s*(pub(\([^)]*\))?\s+)?mod\s+(r#)?(\w+)\s*;\s*$')
opens = re.compile(r'^\s*(pub(\([^)]*\))?\s+)?mod\s+(r#)?(\w+)\s*\{\s*$')
out, stack, depth = [], [], 0

def strip_attrs():
    while out and (out[-1].strip().startswith('#[') or out[-1].strip().startswith('///')):
        out.pop()

# What the engine re-exports from boot is boot's own file there.
from_boot = re.compile(r'^(\s*)(pub(\([^)]*\))?\s+)?use rerail_boot::code_rule_dsl::(\w+(?:::\w+)*)::(\w+)\s*;\s*$')
from_boot_top = re.compile(r'^(\s*)(pub(\([^)]*\))?\s+)?use rerail_boot::code_rule_dsl::(\w+)\s*;\s*$')
i = 0
while i < len(lines):
    l = lines[i]
    b = from_boot.match(l) or from_boot_top.match(l)
    if b:
        name = b.groups()[-1]
        if name == 'boot':
            i += 1
            continue
        l = f'{b.group(1)}pub mod {name};'
        # The engine reads it only in its tests (`#[cfg(test)]` on its re-export); boot owns it in every build.
        while out and out[-1].strip() == '#[cfg(test)]':
            out.pop()
    m = decl.match(l)
    if m:
        path = '::'.join([s for s, _ in stack] + [m.group(4)])
        if path not in KEEP:
            strip_attrs()
            i += 1
            continue
        # The engine reads what boot owns: its declaration is `pub`.
        l = re.sub(r'^(\s*)(pub(\([^)]*\))?\s+)?mod ', r'\1pub mod ', l)
    m = opens.match(l)
    # An engine facade over boot's own module (its glob is boot's) is that module's file here.
    if m and i + 1 < len(lines) and re.match(r'^\s*pub use rerail_boot::code_rule_dsl::[\w:]+::\*;\s*$', lines[i + 1]):
        out.append(re.sub(r'\{\s*$', '', re.sub(r'^(\s*)(pub(\([^)]*\))?\s+)?mod ', r'\1pub mod ', l)).rstrip() + ';')
        d = 0
        while True:
            d += lines[i].count('{') - lines[i].count('}')
            i += 1
            if d == 0:
                break
        continue
    if m and not stack and m.group(4) in DROP_TOP:
        # drop the whole inline module
        strip_attrs()
        d = 0
        while True:
            d += lines[i].count('{') - lines[i].count('}')
            i += 1
            if d == 0:
                break
        continue
    if m and not stack:
        # The engine reads every module through boot: a top-level facade is `pub`.
        l = re.sub(r'^(\s*)(pub(\([^)]*\))?\s+)?mod ', r'\1pub mod ', l)
    out.append(l)
    if m:
        stack.append((m.group(4), depth))
    depth += l.count('{') - l.count('}')
    while stack and depth <= stack[-1][1]:
        stack.pop()
    i += 1

HEAD = """//! The engine's composition root (docs/final-shape/proposals/crate-split/incremental-plan.md, step 1).
//!
//! It registers every provider the build selects (the language packs, Metal, the engine's glue) into the core's
//! registries, in one order, once a process (`code_rule_dsl::boot::install`). It holds the glue those registrations
//! name: the prefilter, the functors, the `ast` wave compiler and the waves' glue, and the syntax-only adapters.
//! The engine (napi and the CLI) stands on it, and so can any test binary that needs a booted program.
//!
//! Its sources stay where they were (native/src): this crate mounts them in place. Its mounts are the engine's
//! (`code_rule_dsl/mod.rs`), less the files the engine keeps (tools/cratify/boot_mounts.py; generated, do not edit).
// A mounted module re-exports its dependency's by glob, and its sources import the same items
// under the same names: the glob's name and the private one are one item. The mounts also widen what the crates
// above read (the split's facades, not a design).
#![allow(dead_code, unused_imports, hidden_glob_reexports, ambiguous_glob_reexports, private_interfaces, private_bounds)]
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
// The browser build has no JIT: wasm/cranelift-jit stands in under the same name (wasm/README.md).
#[cfg(target_family = "wasm")]
extern crate cranelift_jit_wasm as cranelift_jit;
// jemalloc's per-thread counters, which the composition hands the facts' accounting (`boot::heap_net`).
#[cfg(not(any(windows, target_family = "wasm", feature = "system-allocator")))]
extern crate tikv_jemalloc_ctl;
pub use rerail_store::content_store;
pub use rerail_store::git_access;
pub use rerail_store::git_workspace;
pub use rerail_store::source_tree;
pub use rerail_workspace::git_ignore;
pub use rerail_workspace::panic_boundary;
pub use rerail_logic::threads;
pub use rerail_tree_watch::tree_watch;
extern crate self as rerail_boot;

// This crate's test binary keeps the engine's test-binary rules: each test's time limit (`RERAIL_TEST_TIMEOUT`),
// and stores that flush nothing (`RERAIL_FLUSH=none`, decision 0481), both set before `main`.
#[cfg(test)]
#[path = "../../../src/test_watchdog.rs"]
mod test_watchdog;
#[cfg(test)]
#[path = "../../../src/test_flush.rs"]
mod test_flush;
pub(crate) use code_rule_dsl::prof::{lower_hex, purge_allocator_pages};
#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {
"""

WAVES = """    /// The waves: the core's (`rerail-logic`), and the glue boot owns (the first slice's wave, findings and
    /// policies, the schedule's batches).
    pub mod waves {
        pub use rerail_logic::code_rule_dsl::waves::{owner, storage};
        #[cfg(any(test, feature = "test-hooks"))]
        pub use rerail_workspace::code_rule_dsl::waves::relational;
        pub mod schedule {
            pub use rerail_logic::code_rule_dsl::waves::schedule::*;
            pub mod batches;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::waves::owner::{Database, Lease};
            #[allow(unused_imports)]
            use crate::code_rule_dsl::waves::owner::sql::SqlOwner;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::Graph;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::vtab::Source;
            #[allow(unused_imports)]
            use crate::code_rule_dsl::facts::vtab::syntax::{Document, fresh, sources};
            #[allow(unused_imports)]
            use crate::content_store::{DerivationInput, DerivationRecord, StoreError, StoreResult};
            #[allow(unused_imports)]
            use sha2::{Digest as _, Sha256};
            #[allow(unused_imports)]
            use std::collections::{BTreeMap, BTreeSet};
            #[allow(unused_imports)]
            use std::sync::Arc;
            #[allow(unused_imports)]
            pub use crate::code_rule_dsl::waves::schedule::lattice::Operations as LatticeOperations;
        }
        pub mod matches;
        pub mod findings;
        pub mod policies;
    }
    /// The composition (`install`).
    pub mod boot;
    pub use rule_api::CompiledCodeRuleDsl;
    #[cfg(test)]
    pub(crate) use rule_api::CodeRuleDslEditor;
}
"""

# A `#[path]` on an inline module that mounts no file here has no use (rustc warns): dropped.
path_attr = re.compile(r'^\s*#\[(cfg_attr\(test,\s*)?path = "[^"]*"\)?\]\s*$')
keep = []
for i, l in enumerate(out):
    if path_attr.match(l) and i + 1 < len(out) and opens.match(out[i + 1]):
        d, j, mounts = 0, i + 1, False
        while j < len(out):
            d += out[j].count('{') - out[j].count('}')
            if j > i + 1 and decl.match(out[j]):
                mounts = True
            if d == 0:
                break
            j += 1
        if not mounts:
            continue
    keep.append(l)
out = keep

body = '\n'.join(('    ' + l) if l else l for l in out)
open(f'{native}/crates/boot/src/lib.rs', 'w').write(HEAD + body + '\n' + WAVES)
