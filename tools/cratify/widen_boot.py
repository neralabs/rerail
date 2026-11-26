#!/usr/bin/env python3
"""Widens the re-exports of boot's facades that a crate above it reads (incremental-plan.md, step 2).

A facade's restricted re-export (`pub(super) use`, `pub(crate) use`, `pub(in crate::code_rule_dsl) use`) was read
by the engine's own code and tests, in the same crate; a crate above boot reads it across crates, so it is `pub`.
Reads a build's output and widens each re-export a privacy error (E0603) says is defined in boot's lib.rs.

Usage: widen_boot.py BUILD_OUTPUT NATIVE_DIR"""
import re, sys

out, native = open(sys.argv[1]).read(), sys.argv[2]
lib = f'{native}/crates/boot/src/lib.rs'
lines = open(lib).read().split('\n')
hits = set()
for block in re.split(r'\n(?=error)', out):
    if not block.startswith('error[E0603]'):
        continue
    m = re.search(r'is defined here\n\s*--> native/crates/boot/src/lib\.rs:(\d+):', block)
    if m:
        hits.add(int(m.group(1)))
widened = 0
for n in sorted(hits):
    # A name on a continuation line: the statement starts at the nearest line above that begins a `use`.
    while n > 1 and not re.match(r'^\s*(pub(\([^)]*\))?\s+)?use ', lines[n - 1]):
        n -= 1
    new = re.sub(r'^(\s*)pub\((super|crate|in crate::code_rule_dsl|in super::super|in super)\) use ', r'\1pub use ', lines[n - 1])
    new = re.sub(r'^(\s*)use ', r'\1pub use ', new)
    if new != lines[n - 1]:
        lines[n - 1] = new
        widened += 1
    else:
        print(f'not a restricted re-export: {n}: {lines[n - 1].strip()[:120]}')
open(lib, 'w').write('\n'.join(lines))
print(f'boot re-exports widened {widened} of {len(hits)}')
