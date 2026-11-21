#!/usr/bin/env python3
"""Ran once, for step 2: it reads the engine's code_rule_dsl/mod.rs as it was before step 2 (`git show
be18d02111:native/src/code_rule_dsl/mod.rs`), which no longer holds. Since step 3 the test crates' lib.rs files are
source, kept by hand.

The engine's hosted tests, into test crates above rerail-boot (incremental-plan.md, step 2). Run once.

The engine hosts each crate's tests through facades (`code_rule_dsl/mod.rs`, the lines before the napi items): an
inline module per module path, which re-exports the crate's module by glob, imports what the tests' `use super::*`
reads, and mounts the test files. A test crate takes the facades of its subjects, with their test mounts, and
reads everything else through boot (`pub use rerail_boot::code_rule_dsl::*`, and the same glob at every facade it
keeps, so a facade it keeps hides nothing of boot's). The engine keeps the test mounts no test crate takes: those
whose files read what only the engine has (its tools, `logic::convert`, napi, its crate root), and its own
modules' tests. Its facade section becomes the same shape: boot's glob and the facades of what it keeps.

After it runs, boot's `lib.rs` is source, no longer written from the engine's (boot_mounts.py ran before it).

Usage: test_crates.py NATIVE_DIR [--dry-run]"""
import os, re, sys

native = sys.argv[1]
dry = '--dry-run' in sys.argv
DSL = f'{native}/src/code_rule_dsl'
END = "// cratify: the engine's own items (napi) begin"

# The test crates, each with the module paths (under code_rule_dsl) whose tests it hosts; the longest prefix wins.
GROUPS = {
    'demand': ['demand'],
    'transducer': ['transducer'],
    'logic': ['logic', 'typing', 'sql_executor', 'stats', 'sql_admission', 'residency'],
    'compile': ['compile', 'scope', 'value', 'calls', 'literal', 'identifier', 'eval', 'query', 'php_view',
                'module_graph', 'soundness', 'embed', 'lines', 'regexes',
                # PHP's argument binding, checked against the value tests' PHP oracle (`value::php::tests`)
                'matcher::arguments',
                # Blade's oracle reads the same PHP oracle
                'blade'],
    'matcher': ['matcher', 'workspace', 'findings', 'suppress', 'violations'],
    'lang': ['tape', 'language', 'facts', 'tree', 'vue'],
}
ABOUT = {
    'demand': 'demand: the demand-driven matcher, its virtual tables and its driver',
    'transducer': 'the transducer: the packed programs, their backends and the Bitlex lowering',
    'logic': 'logic and the sessions: the planner, the .rr lowering, the program image, the native set kernels, typing, the SQL executor, the statistics and the residency',
    'compile': 'rule compiling: the compiler, scopes, values, calls, literals and the evaluator',
    'matcher': 'matching: the matcher, the workspace, findings, suppressions and violations',
    'lang': 'the languages: tapes, the language registry, facts, trees, Blade and Vue',
}
# What only the engine has: a test file that reads one of them stays the engine's.
ENGINE_ONLY = re.compile(
    r'crate::(rules|process|model|git_start|path_owner|main_instance|reactor|executors_guard_tests)\b'
    r'|\bslice::napi\b|super::napi::'
    # the napi items of the engine's code_rule_dsl/mod.rs
    r'|code_rule_dsl::(ServerGraphTask|module_config_files|server_graph_async|PackageFacts|package_facts|PackageFactsReader'
    r'|nearest_package|CodeRuleDslRuntime)\b|super::super::(package_facts|reads_package_manifests)\b'
    # what the engine's build script generates (`build_languages.rs`: `with_adapter!`)
    r'|env!\("OUT_DIR"\)')
# Test modules the engine keeps because one it keeps reads them (`workspace::context_tests` reads its helpers).
STAY = set()
# The engine's own files that went down to boot with this step (boot's lib.rs mounts them): the CLI's tools, the
# converters and the rule tests' runner. Their tests are boot's.
TO_BOOT = {'tools', 'logic::convert', 'logic::rr::testing'}
# The engine's facade of what it adds to a module of boot's.
ENGINE_FACADES = {
    'tools': [
        '/// The CLI\'s tools (boot\'s), with the engine\'s napi wrapper and the entry points that boot the program first.',
        'pub mod tools {',
        '    pub use rerail_boot::code_rule_dsl::tools::*;',
        '    mod napi;',
        '    pub use napi::{cli, cli_in, execute_sql_request};',
        '}',
    ],
}
# Dev-dependencies of the engine's that only some test crates read: (crates, the engine's dependency names).
EXTRA_DEV = {
    'lang': lambda n: re.match(r'^rerail-[\w-]+-lean$', n) or n == 'rerail-simdjson',
    'transducer': lambda n: n == 'rerail-parser-tests',
    'logic': lambda n: n == 'rerail-python-lean',
}
# Mounted by every test crate (the tests' helpers).
SHARED = {'test_support'}

lines = open(f'{DSL}/mod.rs').read().split('\n')
end = next(i for i, l in enumerate(lines) if l.startswith(END))

decl = re.compile(r'^\s*(pub(\([^)]*\))?\s+)?mod\s+(r#)?(\w+)\s*;\s*$')
opens = re.compile(r'^\s*(pub(\([^)]*\))?\s+)?mod\s+(r#)?(\w+)\s*\{\s*$')
attr = re.compile(r'^\s*(#\[|///|//)')
path_of = re.compile(r'path\s*=\s*"([^"]*)"')


class Mod:
    def __init__(self, name, attrs, head, path, dir, test):
        self.name, self.attrs, self.head, self.path, self.dir, self.test = name, attrs, head, path, dir, test
        self.body = []  # lines, Mod, or Decl
        self.close = None


class Decl:
    def __init__(self, name, attrs, line, path, file, test):
        self.name, self.attrs, self.line, self.path, self.file, self.test = name, attrs, line, path, file, test
        self.owner = None


def is_test(attrs, name):
    text = ' '.join(attrs)
    return bool(re.search(r'#\[cfg\((all\()?test\b', text)) or name == 'tests' or name.endswith('_tests')


def parse(i, stop, parent):
    pending = []
    while i < stop:
        l = lines[i]
        if parent.close is None and l.strip() == '}' and parent.head is not None and depth_of(l) == depth_of(parent.head):
            parent.body.extend(pending)
            parent.close = l
            return i + 1
        if attr.match(l) and not decl.match(l) and not opens.match(l):
            pending.append(l)
            i += 1
            continue
        m = opens.match(l)
        if m:
            name = m.group(4)
            p = next((path_of.search(a).group(1) for a in pending if path_of.search(a) and '#[' in a), None)
            mod = Mod(name, pending, l, parent.path + [name], os.path.normpath(os.path.join(parent.dir, p or name)),
                      parent.test or is_test(pending, name))
            pending = []
            i = parse(i + 1, stop, mod)
            parent.body.append(mod)
            continue
        m = decl.match(l)
        if m:
            name = m.group(4)
            p = next((path_of.search(a).group(1) for a in pending if path_of.search(a) and '#[' in a), None)
            if p:
                f = os.path.normpath(os.path.join(parent.dir, p))
            else:
                f = os.path.join(parent.dir, name + '.rs')
                if not os.path.exists(f):
                    f = os.path.join(parent.dir, name, 'mod.rs')
            parent.body.append(Decl(name, pending, l, parent.path + [name], f, parent.test or is_test(pending, name)))
            pending = []
            i += 1
            continue
        parent.body.extend(pending)
        pending = []
        parent.body.append(l)
        i += 1
    parent.body.extend(pending)
    return i


def depth_of(l):
    return len(l) - len(l.lstrip(' '))


root = Mod('code_rule_dsl', [], None, [], DSL, False)
parse(0, end, root)


child_decl = re.compile(r'((?:^[ \t]*#\[[^\n]*\][ \t]*\n)*)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?mod\s+(?:r#)?(\w+)\s*;', re.M)


def file_tree(f, modrs, seen):
    """`f` and the files its `mod x;` declarations mount, as rustc resolves them: a `#[path]` is relative to the
    file's directory (and the file it names is a mod-rs file); otherwise a child of a mod-rs file sits beside it,
    and a child of `x.rs` in `x/`."""
    if f in seen or not os.path.exists(f):
        return []
    seen.add(f)
    out = [f]
    for m in child_decl.finditer(open(f).read()):
        p = path_of.search(m.group(1) or '')
        if p:
            out += file_tree(os.path.normpath(os.path.join(os.path.dirname(f), p.group(1))), True, seen)
            continue
        base = os.path.dirname(f) if modrs else f[:-3]
        a, b = os.path.join(base, m.group(2) + '.rs'), os.path.join(base, m.group(2), 'mod.rs')
        out += file_tree(a, False, seen) if os.path.exists(a) else file_tree(b, True, seen)
    return out


def files_of(d):
    """A mounted file and the files it mounts."""
    pathed = any(path_of.search(a) and '#[' in a for a in d.attrs)
    return file_tree(d.file, pathed or os.path.basename(d.file) == 'mod.rs', set())


def walk(m):
    for b in m.body:
        if isinstance(b, Mod):
            yield from walk(b)
        elif isinstance(b, Decl):
            yield b


def group_of(path):
    best, crate = -1, None
    for g, prefixes in GROUPS.items():
        for p in prefixes:
            ps = p.split('::')
            if path[:len(ps)] == ps and len(ps) > best:
                best, crate = len(ps), g
    return crate


stay = []
for d in walk(root):
    if not d.test:
        # a file the engine owns (boot's are re-exports by now), unless it went down to boot here
        d.owner = 'boot' if '::'.join(d.path) in TO_BOOT else 'engine'
        continue
    if d.path[-1] in SHARED and len(d.path) == 1:
        d.owner = 'shared'
        continue
    g = group_of(d.path)
    reads = [f for f in files_of(d) if ENGINE_ONLY.search(open(f).read())]
    if g is None or reads or not os.path.exists(d.file) or '::'.join(d.path) in STAY:
        d.owner = 'engine'
        if g is not None:
            stay.append((d, reads))
    else:
        d.owner = g


def emit(m, crate, depth):
    """`m`'s facade for `crate`, or None when it holds nothing of it."""
    keep = lambda d: d.owner == crate or (d.owner == 'shared') or (crate == 'engine' and d.owner == 'engine')
    out, has = [], False
    for b in m.body:
        if isinstance(b, Mod):
            sub = emit(b, crate, depth + 1)
            if sub:
                out += sub
                has = True
        elif isinstance(b, Decl):
            if keep(b):
                out += b.attrs + [b.line]
                if b.owner != 'shared':
                    has = True
        else:
            out.append(b)
    if not has:
        return None
    if m.head is None:
        return out
    pad = ' ' * (depth_of(m.head) + 4)
    here = '::'.join(m.path)
    glob = [f'{pad}pub use rerail_boot::code_rule_dsl::{here}::*;']
    # Boot's module at this path globs the same crates' modules this facade did, and adds boot's own facades: its
    # glob replaces theirs (two globs naming a module and boot's facade of it would be ambiguous).
    same = re.compile(r'^\s*pub use rerail_\w+::code_rule_dsl::' + re.escape(here) + r'::\*;\s*$')
    kept = []
    for l in out:
        if same.match(l):
            while kept and isinstance(kept[-1], str) and re.match(r'^\s*#\[', kept[-1]):
                kept.pop()
            continue
        kept.append(l)
    out = kept
    return m.attrs + [m.head] + glob + out + [m.close]


def top(crate):
    """The top level: boot's glob and the kept facades. The top's own lines (re-exports) are boot's."""
    out = []
    for b in root.body:
        if isinstance(b, Mod):
            sub = emit(b, crate, 0)
            if sub:
                # The engine's facade that mounts only tests is a test build's.
                if crate == 'engine' and not any(d.owner == 'engine' and not d.test for d in walk(b)):
                    sub = ['#[cfg(test)]'] + sub
                out += sub
        elif isinstance(b, Decl):
            if crate == 'engine' and b.name in ENGINE_FACADES:
                out += ENGINE_FACADES[b.name]
                continue
            keep = b.owner == crate or b.owner == 'shared' or (crate == 'engine' and b.owner == 'engine')
            if keep:
                # The tests' helpers, of which the engine's few tests read a few.
                extra = ['#[allow(dead_code, unused_imports)]'] if crate == 'engine' and b.owner == 'shared' else []
                out += b.attrs + extra + [b.line]
    return out


HEAD = '''//! The engine's tests of {about} (docs/final-shape/proposals/crate-split/incremental-plan.md, step 2).
//!
//! A test crate above boot: it mounts the test files in place, under the facades the engine hosted them through,
//! and reads every other module through boot. Its test binary is booted as it loads (`boot::install_for_tests`), as
//! the engine's is. Written by tools/cratify/test_crates.py, and kept by hand since.
#![cfg(test)]
#![allow(dead_code, unused_imports, hidden_glob_reexports, ambiguous_glob_reexports, private_interfaces, private_bounds)]
#![allow(clippy::len_without_is_empty, clippy::result_unit_err, clippy::new_without_default)]
#![allow(clippy::type_complexity, clippy::too_many_arguments)]
pub use rerail_boot::{{content_store, git_access, git_ignore, git_workspace, panic_boundary, source_tree, threads, tree_watch}};
pub(crate) use rerail_boot::code_rule_dsl::prof::{{lower_hex, purge_allocator_pages}};

// The engine's test-binary rules: each test's time limit (`RERAIL_TEST_TIMEOUT`), and stores that flush nothing
// (`RERAIL_FLUSH=none`, decision 0481), both set before `main`.
#[path = "../../../src/test_watchdog.rs"]
mod test_watchdog;
#[path = "../../../src/test_flush.rs"]
mod test_flush;

/// The program, booted as the engine boots it (`boot::install`), with this binary's own build identities.
pub(crate) fn install_store_functions() {{
    rerail_boot::code_rule_dsl::boot::install_for_tests();
}}

mod test_load {{
    extern "C" fn load() {{
        super::install_store_functions();
    }}
    #[used]
    #[cfg_attr(target_vendor = "apple", unsafe(link_section = "__DATA,__mod_init_func"))]
    #[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
    #[cfg_attr(not(any(target_vendor = "apple", windows)), unsafe(link_section = ".init_array"))]
    static LOAD: extern "C" fn() = load;
}}

#[path = "../../../src/code_rule_dsl"]
pub mod code_rule_dsl {{
    pub use rerail_boot::code_rule_dsl::*;
    pub use rerail_boot::code_rule_dsl::rule_api::CodeRuleDslEditor;
'''


def manifest(name, about):
    src = open(f'{native}/crates/boot/Cargo.toml').read()
    head, rest = src.split('[package]', 1)
    rest = rest.split('[features]', 1)[1]
    feats, rest = rest.split('\n[dependencies]\n', 1)
    out = []
    for l in feats.split('\n'):
        # each feature turns on boot's (a list may span lines: the name goes first)
        l = re.sub(r'^([\w-]+) = \[\s*(\]?)', lambda m: f'{m.group(1)} = ["rerail-boot/{m.group(1)}"' + (']' if m.group(2) else ', '), l)
        out.append(l)
    rest = rest.replace('rerail-boot = { path = ".", features = ["test-hooks"] }',
                        f'rerail-tests-{name} = {{ path = ".", features = ["test-hooks"] }}\n'
                        'rerail-boot = { path = "../boot", default-features = false, features = ["test-hooks"] }')
    eng = open(f'{native}/Cargo.toml').read().split('\n[dev-dependencies]\n', 1)[1].split('\n[', 1)[0].split('\n')
    extra = []
    for k, l in enumerate(eng):
        m = re.match(r'^([\w-]+) = ', l)
        if m and name in EXTRA_DEV and EXTRA_DEV[name](m.group(1)):
            if k and eng[k - 1].startswith('#') and (not extra or extra[-1] != eng[k - 1]):
                extra.append(eng[k - 1])
            extra.append(l.replace('path = "crates/', 'path = "../'))
    if extra:
        rest = rest.replace('\n[dev-dependencies]\n', '\n[dev-dependencies]\n' + '\n'.join(extra) + '\n', 1) if '\n[dev-dependencies]\n' in rest else rest
        rest = rest.replace('[dev-dependencies]\n', '[dev-dependencies]\n' + '\n'.join(extra) + '\n', 1) if not rest.count('\n'.join(extra)) else rest
    gpu = "[target.'cfg(any(target_os = \"macos\", target_os = \"linux\"))'.dev-dependencies]\n"
    rest = rest.replace(gpu, gpu + '# The tape\'s views run as UKL kernels, launched through the device layer (decision 0440 step C).\n'
                        'rerail-gpu = { path = "../../../gpu" }\n', 1)
    return (f'''# The engine's tests of {about} (docs/final-shape/proposals/crate-split/incremental-plan.md, step 2).
# A test crate: its library is its tests (`#![cfg(test)]`). Its manifest is boot's, with boot below it
# (tools/cratify/test_crates.py), and kept by hand since.
[package]
name = "rerail-tests-{name}"
version = "0.1.0"
edition = "2024"
publish = false
description = "The engine's tests of {about}."

[features]''' + '\n'.join(out) + '\n[dependencies]\nrerail-boot = { path = "../boot", default-features = false }\n' + rest)


def indent(ls):
    return '\n'.join(('    ' + l) if l else l for l in ls)


report = []
for g in GROUPS:
    body = top(g)
    n = sum(1 for d in walk(root) if d.owner == g)
    lines_n = sum(sum(1 for _ in open(f)) for d in walk(root) if d.owner == g for f in files_of(d))
    report.append(f'rerail-tests-{g}: {n} mounts, {lines_n} lines')
    if dry:
        continue
    d = f'{native}/crates/tests_{g}'
    os.makedirs(f'{d}/src', exist_ok=True)
    open(f'{d}/src/lib.rs', 'w').write(HEAD.format(about=ABOUT[g]) + indent(body) + '\n}\n')
    open(f'{d}/Cargo.toml', 'w').write(manifest(g, ABOUT[g]))

eng = top('engine')
report.append(f'engine keeps: {sum(1 for d in walk(root) if d.owner == "engine" and d.test)} test mounts')
for d, reads in stay:
    report.append(f'  stays: {"::".join(d.path)} ({", ".join(os.path.relpath(f, DSL) for f in reads[:3]) or "missing"})')
print('\n'.join(report))
if not dry:
    head = ['// The facades re-export their dependency\'s module by glob, and the mounted sources import the same items under',
            '// the same names: the glob\'s name and the private one are one item.',
            '#![allow(hidden_glob_reexports)]',
            '// Every module is boot\'s (rerail-boot). The engine adds napi (the items below, `slice::napi`, the CLI\'s',
            '// entry points in `tools`) and the facades of the tests that read them (tools/cratify/test_crates.py).',
            'pub use rerail_boot::code_rule_dsl::*;']
    # The editor's re-export served the engine's tests of it, which are the test crates' now.
    rest = '\n'.join(lines[end:]).replace(lines[end], "// The engine's own items: napi.", 1).replace('#[cfg(test)]\npub(crate) use rule_api::CodeRuleDslEditor;\n', '')
    open(f'{DSL}/mod.rs', 'w').write('\n'.join(head + eng) + '\n' + rest)
