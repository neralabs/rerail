#!/usr/bin/env python3
"""rerail-boot's Cargo.toml from the engine's (native/Cargo.toml): the same features and dependencies, re-rooted at
native/crates/boot, without what only the addon and the CLI need (the cdylib, the build script, napi's build, the
examples, the parsers' measurement dev-dependencies). Also adds `rerail-boot/<feature>` to each of the engine's
features that forwards to its crates, and the engine's dependency on boot. Usage: boot_manifest.py NATIVE_DIR"""
import re, sys
native = sys.argv[1]
src = open(f'{native}/Cargo.toml').read()

def section(text, name):
    m = re.search(r'^\[' + re.escape(name) + r'\]\n(.*?)(?=^\[|\Z)', text, re.S | re.M)
    return m.group(1) if m else ''

def reroot(text):
    def fix(m):
        p = m.group(1)
        if p.startswith('crates/'): p = '../' + p[len('crates/'):]
        elif p == '.': p = '../..'
        else: p = '../../' + p
        return f'path = "{p}"'
    return re.sub(r'path = "([^"]+)"', fix, text)

features = section(src, 'features')
deps = section(src, 'dependencies')
targets = re.findall(r"^(\[target\.'[^']+'\.dependencies\]\n.*?)(?=^\[|\Z)", src, re.S | re.M)
dev = section(src, 'dev-dependencies')
keep_dev = [l for l in dev.split('\n') if re.match(r'(rerail-(facts|lang|lang-[a-z]+|logic|rr|sessions|transducer|classic|compiler) =|ast-grep-language =|rusqlite =|tempfile =)', l)]
dev_targets = re.findall(r"^(\[target\.'[^']+'\.dev-dependencies\]\n.*?)(?=^\[|\Z)", src, re.S | re.M)
dev_targets = [re.sub(r'^rerail-gpu = .*\n', '', t, flags=re.M) for t in dev_targets]
dev_targets = [re.sub(r'^# The tape.s views run as UKL kernels.*\n', '', t, flags=re.M) for t in dev_targets]

out = f'''# The engine's composition root (docs/final-shape/proposals/crate-split/incremental-plan.md, step 1): the
# providers the build selects, registered in one order, and the engine's glue they need. Its sources stay where they
# were, mounted in place (src/lib.rs). Its manifest is the engine's, re-rooted (tools/cratify/boot_manifest.py), and
# kept by hand since.
[package]
name = "rerail-boot"
version = "0.1.0"
edition = "2024"
publish = false
description = "The composition root: every provider registered once, in one order; the prefilter, the waves' glue, the syntax-only adapters."

[features]
{features.rstrip()}

[dependencies]
{reroot(deps).rstrip()}

{chr(10).join(reroot(t).rstrip() + chr(10) for t in targets)}
[dev-dependencies]
{reroot(chr(10).join(keep_dev))}

{chr(10).join(reroot(t).rstrip() + chr(10) for t in dev_targets)}'''
open(f'{native}/crates/boot/Cargo.toml', 'w').write(out)

# The engine: every feature that forwards to its crates forwards to boot's too; boot is a dependency.
def add(m):
    name, body = m.group(1), m.group(2)
    if 'rerail-classic/' not in body or 'rerail-boot/' in body: return m.group(0)
    return f'{name} = [{body}, "rerail-boot/{name}"]'
lines = src.split('\n')
feat_start = lines.index('[features]')
feat_end = next(i for i in range(feat_start + 1, len(lines)) if lines[i].startswith('['))
body = '\n'.join(lines[feat_start:feat_end])
body = re.sub(r'^([a-z0-9-]+) = \[(.*?)\]$', add, body, flags=re.M | re.S)
src2 = '\n'.join(lines[:feat_start]) + '\n' + body + '\n' + '\n'.join(lines[feat_end:])
src2 = src2.replace('rerail-classic = { path = "crates/classic", default-features = false }\n',
    'rerail-classic = { path = "crates/classic", default-features = false }\n'
    '# The composition root and the glue under napi and the CLI (incremental-plan.md, step 1).\n'
    'rerail-boot = { path = "crates/boot", default-features = false }\n', 1)
src2 = src2.replace('rerail-classic = { path = "crates/classic", default-features = false, features = ["test-hooks"] }\n',
    'rerail-classic = { path = "crates/classic", default-features = false, features = ["test-hooks"] }\n'
    'rerail-boot = { path = "crates/boot", default-features = false, features = ["test-hooks"] }\n', 1)
open(f'{native}/Cargo.toml', 'w').write(src2)
