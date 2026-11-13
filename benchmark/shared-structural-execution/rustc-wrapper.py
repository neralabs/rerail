#!/usr/bin/env python3
"""Match the measured lib-test build without changing dependency optimization."""
import os
import sys

args = sys.argv[1:]
if '--test' in args and '--crate-name' in args and args[args.index('--crate-name') + 1] == 'rerail_engine':
    args += ['-C', 'lto=off', '-C', 'opt-level=3', '-C', 'codegen-units=16']
    if cache := os.environ.get('RERAIL_SHARED_INCREMENTAL'):
        args += ['-C', 'incremental=' + cache]
os.execvp(args[0], args)
